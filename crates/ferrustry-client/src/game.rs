//! Логика клиента, не зависящая от окна: ввод, камера, инструменты стройки, фиксированный шаг
//! симуляции, сборка кадра. Тестируется без GPU и без оконной системы.

use std::collections::BTreeSet;

use ferrustry_content::{BlockId, BlockKind, ContentRegistry};
use ferrustry_render::atlas::palette;
use ferrustry_render::{Atlas, Camera, Ghost, Hud, SpriteBatch, SpriteTable, build_world, rgba};
use ferrustry_sim::{Command, Dir, TICKS_PER_SECOND, TilePos, World};

/// Кнопка мыши.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Левая.
    Left,
    /// Правая.
    Right,
    /// Средняя.
    Middle,
}

/// Клавиша (уже отображённая из физической раскладки).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Цифра 1..9 — слот панели.
    Digit(u8),
    /// Поворот.
    Rotate,
    /// Пауза.
    Pause,
    /// Сброс инструмента.
    Cancel,
    /// Быстрое сохранение.
    Save,
    /// Быстрая загрузка.
    Load,
    /// Ускорить симуляцию.
    Faster,
    /// Замедлить симуляцию.
    Slower,
    /// Панорама вверх.
    PanUp,
    /// Панорама вниз.
    PanDown,
    /// Панорама влево.
    PanLeft,
    /// Панорама вправо.
    PanRight,
}

/// Событие ввода.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// Курсор в пикселях окна.
    CursorMoved([f32; 2]),
    /// Нажатие кнопки мыши.
    MouseDown(Button),
    /// Отпускание кнопки мыши.
    MouseUp(Button),
    /// Колесо: положительное — от себя (приближение).
    Wheel(f32),
    /// Клавиша нажата / отпущена.
    Key(Key, bool),
    /// Новый размер окна.
    Resize([f32; 2]),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Build(TilePos),
    Remove(TilePos),
    Pan([f32; 2]),
}

/// Внешний запрос к окну/файловой системе, который игра не выполняет сама.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Сохранить мир.
    Save,
    /// Загрузить мир.
    Load,
}

const QUICKSAVE: &str = "ferrustry-quicksave.fsave";

/// Состояние клиента.
pub struct Game {
    /// Контент.
    pub reg: ContentRegistry,
    /// Мир.
    pub world: World,
    /// Камера.
    pub camera: Camera,
    hotbar: Vec<BlockId>,
    tool: Option<usize>,
    rot: Dir,
    paused: bool,
    speed: u32,
    cursor: [f32; 2],
    drag: Option<Drag>,
    pan_keys: [bool; 4],
    pending: Vec<Command>,
    accumulator: f64,
    message: Option<(String, f64)>,
    requests: Vec<Request>,
    fps: RateMeter,
    tps: RateMeter,
    ghosts: Vec<Ghost>,
    dirs: Vec<Dir>,
}

/// Скользящий счётчик частоты.
#[derive(Debug, Default, Clone)]
struct RateMeter {
    window: f64,
    count: u32,
    value: f64,
}

impl RateMeter {
    fn tick(&mut self, dt: f64, events: u32) {
        self.window += dt;
        self.count += events;
        if self.window >= 0.5 {
            self.value = f64::from(self.count) / self.window;
            self.window = 0.0;
            self.count = 0;
        }
    }
}

impl Game {
    /// Новая игра на мире `world`; камера смотрит на центр карты.
    pub fn new(reg: ContentRegistry, world: World, viewport: [f32; 2]) -> Self {
        let order = [
            "conveyor",
            "bore",
            "smelter",
            "steel_forge",
            "wind_turbine",
            "power_node",
            "core",
        ];
        let hotbar = order.iter().filter_map(|n| reg.block_by_name(n)).collect();
        let center = [
            world.grid().width() as f32 / 2.0,
            world.grid().height() as f32 / 2.0,
        ];
        Self {
            reg,
            world,
            camera: Camera::new(center, 32.0, viewport),
            hotbar,
            tool: None,
            rot: Dir::Right,
            paused: false,
            speed: 1,
            cursor: [viewport[0] / 2.0, viewport[1] / 2.0],
            drag: None,
            pan_keys: [false; 4],
            pending: Vec::new(),
            accumulator: 0.0,
            message: None,
            requests: Vec::new(),
            fps: RateMeter::default(),
            tps: RateMeter::default(),
            ghosts: Vec::new(),
            dirs: Vec::new(),
        }
    }

    /// Показать сообщение на несколько секунд.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), 3.0));
    }

    /// Забирает накопленные запросы к окружению (сохранение/загрузка).
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Путь быстрого сохранения.
    pub fn quicksave_path() -> &'static str {
        QUICKSAVE
    }

    /// Выбранная постройка.
    pub fn selected_block(&self) -> Option<BlockId> {
        self.tool.and_then(|i| self.hotbar.get(i).copied())
    }

    fn cursor_tile(&self) -> TilePos {
        let w = self.camera.screen_to_world(self.cursor);
        TilePos::new(w[0].floor() as i32, w[1].floor() as i32)
    }

    /// Левый нижний тайл постройки размера `size`, центрированной под курсором.
    fn origin_for(&self, tile: TilePos, size: u8) -> TilePos {
        let off = (i32::from(size) - 1) / 2;
        TilePos::new(tile.x - off, tile.y - off)
    }

    /// Обрабатывает событие ввода.
    pub fn handle(&mut self, input: Input) {
        match input {
            Input::Resize(v) => self.camera.viewport = v,
            Input::CursorMoved(p) => {
                if let Some(Drag::Pan(last)) = self.drag {
                    self.camera.pan_pixels(p[0] - last[0], p[1] - last[1]);
                    self.drag = Some(Drag::Pan(p));
                }
                self.cursor = p;
            }
            Input::Wheel(d) => self.camera.zoom_at(1.15f32.powf(d), self.cursor),
            Input::MouseDown(Button::Middle) => self.drag = Some(Drag::Pan(self.cursor)),
            Input::MouseDown(Button::Left) => {
                if self.selected_block().is_some() {
                    self.drag = Some(Drag::Build(self.cursor_tile()));
                }
            }
            Input::MouseDown(Button::Right) => {
                if self.tool.is_some() && self.drag.is_none() {
                    self.tool = None; // правый клик с инструментом — снять инструмент
                } else {
                    self.drag = Some(Drag::Remove(self.cursor_tile()));
                }
            }
            Input::MouseUp(b) => {
                let end = self.cursor_tile();
                match (self.drag, b) {
                    (Some(Drag::Build(start)), Button::Left) => {
                        self.drag = None;
                        let cmds = self.build_commands(start, end);
                        self.queue(cmds);
                    }
                    (Some(Drag::Remove(start)), Button::Right) => {
                        self.drag = None;
                        let cmds = self.remove_commands(start, end);
                        self.queue(cmds);
                    }
                    (Some(Drag::Pan(_)), Button::Middle) => self.drag = None,
                    _ => {} // отпущена другая кнопка — жест продолжается
                }
            }
            Input::Key(k, down) => self.key(k, down),
        }
    }

    fn key(&mut self, k: Key, down: bool) {
        let idx = match k {
            Key::PanUp => Some(0),
            Key::PanDown => Some(1),
            Key::PanLeft => Some(2),
            Key::PanRight => Some(3),
            _ => None,
        };
        if let Some(i) = idx {
            self.pan_keys[i] = down;
            return;
        }
        if !down {
            return;
        }
        match k {
            Key::Digit(d) => {
                let i = usize::from(d.saturating_sub(1));
                self.tool = if self.tool == Some(i) || i >= self.hotbar.len() {
                    None
                } else {
                    Some(i)
                };
            }
            Key::Rotate => {
                self.rot = match self.rot {
                    Dir::Right => Dir::Down,
                    Dir::Down => Dir::Left,
                    Dir::Left => Dir::Up,
                    Dir::Up => Dir::Right,
                }
            }
            Key::Pause => self.paused = !self.paused,
            Key::Cancel => {
                self.tool = None;
                self.drag = None;
            }
            Key::Save => self.requests.push(Request::Save),
            Key::Load => self.requests.push(Request::Load),
            Key::Faster => self.speed = (self.speed * 2).min(8),
            Key::Slower => self.speed = (self.speed / 2).max(1),
            _ => {}
        }
    }

    /// Команды стройки от тайла `start` до `end`. Ленты — «уголком» (сначала по X, затем по Y)
    /// с направлением по ходу протяжки; остальное — в линию с шагом в размер постройки.
    pub fn build_commands(&self, start: TilePos, end: TilePos) -> Vec<Command> {
        let Some(block) = self.selected_block() else {
            return Vec::new();
        };
        let def = self.reg.block(block);
        let size = i32::from(def.size);
        let is_belt = matches!(def.kind, BlockKind::Conveyor { .. });
        let mut out = Vec::new();
        if start == end {
            out.push(Command::Place {
                block,
                origin: self.origin_for(start, def.size),
                rot: self.rot,
            });
            return out;
        }
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        if is_belt {
            let hdir = if dx >= 0 { Dir::Right } else { Dir::Left };
            let vdir = if dy >= 0 { Dir::Up } else { Dir::Down };
            let sx = dx.signum();
            let mut x = start.x;
            while x != end.x {
                out.push(Command::Place {
                    block,
                    origin: TilePos::new(x, start.y),
                    rot: hdir,
                });
                x += sx;
            }
            // угловой и вертикальный участок
            let sy = dy.signum();
            let mut y = start.y;
            loop {
                let rot = if dy == 0 { hdir } else { vdir };
                out.push(Command::Place {
                    block,
                    origin: TilePos::new(end.x, y),
                    rot,
                });
                if y == end.y {
                    break;
                }
                y += sy;
            }
        } else {
            let horizontal = dx.abs() >= dy.abs();
            let n = if horizontal { dx.abs() } else { dy.abs() } / size;
            for i in 0..=n {
                let t = if horizontal {
                    TilePos::new(start.x + dx.signum() * i * size, start.y)
                } else {
                    TilePos::new(start.x, start.y + dy.signum() * i * size)
                };
                out.push(Command::Place {
                    block,
                    origin: self.origin_for(t, def.size),
                    rot: self.rot,
                });
            }
        }
        out
    }

    /// Команды сноса всех построек в прямоугольнике (каждая — один раз).
    pub fn remove_commands(&self, a: TilePos, b: TilePos) -> Vec<Command> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for y in a.y.min(b.y)..=a.y.max(b.y) {
            for x in a.x.min(b.x)..=a.x.max(b.x) {
                let p = TilePos::new(x, y);
                if let Some(id) = self.world.grid().building(p)
                    && seen.insert(id)
                {
                    out.push(Command::Remove { at: p });
                }
            }
        }
        out
    }

    fn queue(&mut self, cmds: Vec<Command>) {
        if self.paused {
            // на паузе стройка применяется сразу, без хода симуляции
            let mut failed = 0;
            for c in &cmds {
                failed += usize::from(self.world.apply(&self.reg, c).is_err());
            }
            self.report_failures(failed, cmds.len());
        } else {
            self.pending.extend(cmds);
        }
    }

    fn report_failures(&mut self, failed: usize, total: usize) {
        if failed > 0 && total == 1 {
            self.notify("CANNOT BUILD HERE");
        } else if failed > 0 {
            self.notify(format!("{failed} OF {total} NOT PLACED"));
        }
    }

    /// Продвигает игру на `dt` секунд реального времени: панорама, фиксированные тики (30 Hz).
    pub fn update(&mut self, dt: f64) {
        let dt = dt.clamp(0.0, 0.25);
        self.fps.tick(dt, 1);
        let pan = 900.0 * dt as f32;
        let k = self.pan_keys;
        self.camera.pan_pixels(
            (f32::from(u8::from(k[2])) - f32::from(u8::from(k[3]))) * pan,
            (f32::from(u8::from(k[0])) - f32::from(u8::from(k[1]))) * pan,
        );
        if let Some((_, ttl)) = &mut self.message {
            *ttl -= dt;
            if *ttl <= 0.0 {
                self.message = None;
            }
        }
        let mut ticks = 0;
        if !self.paused {
            let step = 1.0 / f64::from(TICKS_PER_SECOND);
            self.accumulator += dt * f64::from(self.speed);
            // не больше 8 тиков за кадр — защита от «спирали смерти»
            while self.accumulator >= step && ticks < 8 {
                let cmds = std::mem::take(&mut self.pending);
                let results = self.world.tick(&self.reg, &cmds);
                let failed = results.iter().filter(|r| r.is_err()).count();
                self.report_failures(failed, results.len());
                self.accumulator -= step;
                ticks += 1;
            }
            if ticks == 8 {
                self.accumulator = 0.0;
            }
        }
        self.tps.tick(dt, ticks);
    }

    /// Собирает кадр: слой мира и слой интерфейса.
    pub fn frame(
        &mut self,
        atlas: &Atlas,
        table: &SpriteTable,
        world_batch: &mut SpriteBatch,
        hud: &mut Hud,
    ) {
        world_batch.clear();
        self.ghosts.clear();
        if let Some(block) = self.selected_block() {
            let end = self.cursor_tile();
            let start = match self.drag {
                Some(Drag::Build(s)) => s,
                _ => end,
            };
            for c in self.build_commands(start, end) {
                if let Command::Place { block, origin, rot } = c {
                    let valid = self.world.can_place(&self.reg, block, origin).is_ok();
                    self.ghosts.push(Ghost {
                        block,
                        origin,
                        rot,
                        valid,
                    });
                }
            }
            let _ = block;
        }
        build_world(
            world_batch,
            table,
            &self.reg,
            &self.world,
            &self.camera,
            &self.ghosts,
            &mut self.dirs,
        );
        if let Some(Drag::Remove(start)) = self.drag {
            let end = self.cursor_tile();
            let (x0, x1) = (start.x.min(end.x), start.x.max(end.x) + 1);
            let (y0, y1) = (start.y.min(end.y), start.y.max(end.y) + 1);
            world_batch.push(
                atlas.uv(table.white),
                [(x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0],
                [(x1 - x0) as f32, (y1 - y0) as f32],
                0.0,
                [0.9, 0.15, 0.1, 0.35],
            );
        }
        self.build_hud(atlas, table, hud);
    }

    fn build_hud(&self, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        hud.begin(vh);
        let white = table.white;
        let panel = [0.02, 0.025, 0.04, 0.82];
        let orange = rgba(palette::FORGE_ORANGE);
        let steel = rgba(palette::STEEL_WHITE);
        let titanium = rgba(palette::TITANIUM);

        // заголовок и статус
        hud.rect(atlas, white, [0.0, 0.0, 360.0, 64.0], panel);
        hud.text(atlas, 12.0, 10.0, 3.0, "FERRUSTRY", orange);
        let status = format!(
            "TICK {}  FPS {:.0}  TPS {:.0}  X{}{}",
            self.world.tick_count(),
            self.fps.value,
            self.tps.value,
            self.speed,
            if self.paused { "  PAUSED" } else { "" }
        );
        hud.text(atlas, 12.0, 40.0, 2.0, &status, steel);

        // ресурсы в ядрах
        let core = self.world.total_core_items();
        let lines: Vec<String> = self
            .reg
            .items()
            .iter()
            .enumerate()
            .map(|(i, d)| {
                format!(
                    "{:<11}{:>6}",
                    d.name.replace('_', " "),
                    core.get(i).copied().unwrap_or(0)
                )
            })
            .collect();
        let pw = 230.0;
        hud.rect(
            atlas,
            white,
            [vw - pw, 0.0, pw, 18.0 + lines.len() as f32 * 18.0],
            panel,
        );
        for (i, l) in lines.iter().enumerate() {
            hud.text(atlas, vw - pw + 10.0, 8.0 + i as f32 * 18.0, 2.0, l, steel);
        }

        // панель построек
        let slot = 64.0;
        let gap = 6.0;
        let n = self.hotbar.len() as f32;
        let total = n * slot + (n - 1.0) * gap;
        let x0 = (vw - total) / 2.0;
        let y0 = vh - slot - 34.0;
        for (i, &b) in self.hotbar.iter().enumerate() {
            let x = x0 + i as f32 * (slot + gap);
            let selected = self.tool == Some(i);
            if selected {
                hud.rect(
                    atlas,
                    white,
                    [x - 3.0, y0 - 3.0, slot + 6.0, slot + 6.0],
                    orange,
                );
            }
            hud.rect(atlas, white, [x, y0, slot, slot], panel);
            hud.batch.push(
                table.block(b),
                [x + slot / 2.0, vh - (y0 + slot / 2.0)],
                [slot - 12.0, slot - 12.0],
                0.0,
                [1.0; 4],
            );
            hud.text(
                atlas,
                x + 4.0,
                y0 + 4.0,
                2.0,
                &(i + 1).to_string(),
                if selected { orange } else { titanium },
            );
        }
        let label = match self.selected_block() {
            Some(b) => format!(
                "{}  ROT {:?}",
                self.reg.block(b).name.replace('_', " "),
                self.rot
            ),
            None => "SELECT 1-7".to_owned(),
        };
        let lw = label.len() as f32 * 12.0;
        hud.text(atlas, (vw - lw) / 2.0, y0 - 22.0, 2.0, &label, steel);
        hud.text(
            atlas,
            10.0,
            vh - 14.0,
            1.0,
            "LMB BUILD/DRAG  RMB REMOVE/CANCEL  R ROTATE  WASD/MMB PAN  WHEEL ZOOM  SPACE PAUSE  +/- SPEED  F5 SAVE  F9 LOAD",
            titanium,
        );

        // подсказка под курсором
        let t = self.cursor_tile();
        let under = self
            .world
            .grid()
            .building(t)
            .and_then(|id| self.world.buildings().get(id))
            .map(|b| self.reg.block(b.block).name.replace('_', " "));
        let tip = match under {
            Some(n) => format!("{},{}  {}", t.x, t.y, n),
            None => format!("{},{}", t.x, t.y),
        };
        hud.text(
            atlas,
            self.cursor[0] + 16.0,
            self.cursor[1] + 16.0,
            2.0,
            &tip,
            steel,
        );

        if let Some((m, _)) = &self.message {
            let w = m.len() as f32 * 18.0;
            hud.rect(
                atlas,
                white,
                [(vw - w) / 2.0 - 12.0, 76.0, w + 24.0, 40.0],
                panel,
            );
            hud.text(atlas, (vw - w) / 2.0, 86.0, 3.0, m, orange);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ferrustry_sim::scenario;

    fn game() -> Game {
        let reg = ContentRegistry::base().unwrap();
        let world = scenario::demo_map(&reg, 1).unwrap();
        Game::new(reg, world, [1280.0, 720.0])
    }

    fn click_tile(g: &mut Game, t: TilePos) {
        let p = g
            .camera
            .world_to_screen([t.x as f32 + 0.5, t.y as f32 + 0.5]);
        g.handle(Input::CursorMoved(p));
    }

    #[test]
    fn belt_drag_makes_l_shape_with_flow_directions() {
        let mut g = game();
        g.handle(Input::Key(Key::Digit(1), true));
        let cmds = g.build_commands(TilePos::new(0, 0), TilePos::new(3, -2));
        let placed: Vec<(TilePos, Dir)> = cmds
            .iter()
            .map(|c| match c {
                Command::Place { origin, rot, .. } => (*origin, *rot),
                Command::Remove { .. } => unreachable!(),
            })
            .collect();
        assert_eq!(
            placed,
            vec![
                (TilePos::new(0, 0), Dir::Right),
                (TilePos::new(1, 0), Dir::Right),
                (TilePos::new(2, 0), Dir::Right),
                (TilePos::new(3, 0), Dir::Down),
                (TilePos::new(3, -1), Dir::Down),
                (TilePos::new(3, -2), Dir::Down),
            ]
        );
    }

    #[test]
    fn mouse_build_and_remove_reach_the_world() {
        let mut g = game();
        let before = g.world.buildings().len();
        g.handle(Input::Key(Key::Digit(1), true));
        let a = TilePos::new(20, 20);
        let b = TilePos::new(29, 20);
        click_tile(&mut g, a);
        g.handle(Input::MouseDown(Button::Left));
        click_tile(&mut g, b);
        g.handle(Input::MouseUp(Button::Left));
        g.update(1.0 / 30.0 + 1e-6);
        assert_eq!(g.world.buildings().len(), before + 10);
        // правый клик с инструментом снимает инструмент, второй — начинает снос
        g.handle(Input::MouseDown(Button::Right));
        assert!(g.selected_block().is_none());
        g.handle(Input::MouseUp(Button::Right));
        click_tile(&mut g, a);
        g.handle(Input::MouseDown(Button::Right));
        click_tile(&mut g, b);
        g.handle(Input::MouseUp(Button::Right));
        g.update(1.0 / 30.0 + 1e-6);
        assert_eq!(g.world.buildings().len(), before);
    }

    #[test]
    fn fixed_step_runs_30_ticks_per_second_and_pauses() {
        let mut g = game();
        for _ in 0..60 {
            g.update(1.0 / 60.0);
        }
        assert!(
            (29..=31).contains(&g.world.tick_count()),
            "{}",
            g.world.tick_count()
        );
        g.handle(Input::Key(Key::Pause, true));
        let t = g.world.tick_count();
        g.update(1.0);
        assert_eq!(g.world.tick_count(), t);
        // большой рывок времени не вызывает больше 8 тиков
        g.handle(Input::Key(Key::Pause, true));
        g.update(10.0);
        assert!(g.world.tick_count() - t <= 8);
    }

    #[test]
    fn frame_contains_ghosts_and_hud() {
        let mut g = game();
        let reg = ContentRegistry::base().unwrap();
        let atlas = Atlas::build(&reg);
        let table = SpriteTable::new(&atlas, &reg);
        let (mut wb, mut hud) = (SpriteBatch::default(), Hud::default());
        g.frame(&atlas, &table, &mut wb, &mut hud);
        let base = wb.len();
        g.handle(Input::Key(Key::Digit(3), true));
        g.frame(&atlas, &table, &mut wb, &mut hud);
        assert_eq!(wb.len(), base + 1, "one ghost for the smelter");
        assert!(hud.batch.len() > 50);
    }
}
