//! Логика клиента, не зависящая от окна: экраны и меню, ввод, камера, инструменты стройки,
//! фиксированный шаг симуляции, сборка кадра. Тестируется без GPU и без оконной системы.

use std::collections::BTreeSet;

use ferrustry_content::{BlockId, BlockKind, ContentRegistry};
use ferrustry_render::atlas::palette;
use ferrustry_render::{Atlas, Camera, Ghost, Hud, SpriteBatch, SpriteTable, build_world, rgba};
use ferrustry_sim::combat::Outcome;
use ferrustry_sim::mapgen::{self, Difficulty, MapSettings, MapSize};
use ferrustry_sim::schematic::Schematic;
use ferrustry_sim::{Command, CommandError, Detail, Dir, TICKS_PER_SECOND, TilePos, World};

use crate::ui::{Rect, Ui, text_width};

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
    /// Esc: сброс инструмента / меню.
    Cancel,
    /// Быстрое сохранение.
    Save,
    /// Быстрая загрузка.
    Load,
    /// Ускорить симуляцию.
    Faster,
    /// Замедлить симуляцию.
    Slower,
    /// Справка.
    Help,
    /// Режим копирования схемы.
    Copy,
    /// Сохранить текущую схему.
    SaveSchematic,
    /// Загрузить схему.
    LoadSchematic,
    /// Следующая категория построек.
    NextCategory,
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
    /// Масштаб экрана ОС (DPI): 1.0, 1.25, 1.5, 2.0...
    Scale(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Build(TilePos),
    Remove(TilePos),
    Copy(TilePos),
    Pan([f32; 2]),
}

/// Экран игры.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Главное меню (мир живёт на фоне).
    MainMenu,
    /// Игра.
    Playing,
    /// Меню паузы (симуляция стоит).
    PauseMenu,
    /// Настройки новой игры.
    NewGame,
}

/// Внешний запрос к окну/файловой системе, который игра не выполняет сама.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Сохранить мир.
    Save,
    /// Загрузить мир.
    Load,
    /// Закрыть игру.
    Quit,
    /// Сохранить схему в файл.
    SaveSchematic(Schematic),
    /// Загрузить схему из файла.
    LoadSchematic,
}

const QUICKSAVE: &str = "ferrustry-quicksave.fsave";
const SCHEMATIC_FILE: &str = "ferrustry-schematic.fschem";

/// Категории панели построек (вкладки).
const CATEGORIES: [(&str, &[&str]); 5] = [
    (
        "LOGISTICS",
        &["conveyor", "splitter", "sorter", "bridge_conveyor", "depot"],
    ),
    ("PRODUCTION", &["bore", "smelter", "steel_forge", "core"]),
    ("DEFENSE", &["gun_turret", "arc_tower", "iron_wall"]),
    (
        "POWER",
        &[
            "wind_turbine",
            "burner",
            "power_node",
            "battery",
            "heat_engine",
        ],
    ),
    (
        "FLUIDS+HEAT",
        &[
            "pipe",
            "condenser",
            "cooling_tower",
            "heat_pipe",
            "radiator",
        ],
    ),
];

/// Состояние клиента.
pub struct Game {
    /// Контент.
    pub reg: ContentRegistry,
    /// Мир.
    pub world: World,
    /// Камера.
    pub camera: Camera,
    screen: Screen,
    seed: u64,
    /// Постройки текущей категории (слоты панели).
    hotbar: Vec<BlockId>,
    categories: Vec<(&'static str, Vec<BlockId>)>,
    category: usize,
    tool: Option<BlockId>,
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
    ui: Ui,
    ui_scale: f32,
    show_help: bool,
    quicksave_exists: bool,
    inspected: Option<TilePos>,
    last_wave: u32,
    copy_mode: bool,
    paste: Option<Schematic>,
    map_size: MapSize,
    difficulty: Difficulty,
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
    /// Новая игра на мире `world`, открывается главное меню.
    pub fn new(reg: ContentRegistry, world: World, viewport: [f32; 2]) -> Self {
        let categories: Vec<(&'static str, Vec<BlockId>)> = CATEGORIES
            .iter()
            .map(|(name, blocks)| {
                (
                    *name,
                    blocks
                        .iter()
                        .filter_map(|n| reg.block_by_name(n))
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|(_, v)| !v.is_empty())
            .collect();
        let hotbar = categories.first().map(|c| c.1.clone()).unwrap_or_default();
        let center = [
            world.grid().width() as f32 / 2.0,
            world.grid().height() as f32 / 2.0,
        ];
        Self {
            reg,
            world,
            camera: Camera::new(center, 32.0, viewport),
            screen: Screen::MainMenu,
            seed: 1,
            hotbar,
            categories,
            category: 0,
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
            ui: Ui::default(),
            ui_scale: 1.0,
            show_help: false,
            inspected: None,
            last_wave: 0,
            copy_mode: false,
            paste: None,
            map_size: MapSize::Medium,
            difficulty: Difficulty::Normal,
            quicksave_exists: std::path::Path::new(QUICKSAVE).exists(),
        }
    }

    /// Сразу перейти к игре (минуя меню) — для `--load`, скриншотов и тестов.
    pub fn start_playing(&mut self) {
        self.screen = Screen::Playing;
    }

    /// Текущий экран.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Показать сообщение на несколько секунд.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), 3.5));
    }

    /// Забирает накопленные запросы к окружению.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Путь быстрого сохранения.
    pub fn quicksave_path() -> &'static str {
        QUICKSAVE
    }

    /// Путь файла схемы.
    pub fn schematic_path() -> &'static str {
        SCHEMATIC_FILE
    }

    /// Результат загрузки схемы: включает режим вставки.
    pub fn schematic_loaded(&mut self, s: Option<Schematic>) {
        match s {
            Some(s) => {
                self.notify(format!("SCHEMATIC LOADED: {} BUILDINGS", s.entries.len()));
                self.tool = None;
                self.copy_mode = false;
                self.paste = Some(s);
            }
            None => self.notify("NO SCHEMATIC TO LOAD"),
        }
    }

    /// Результат сохранения схемы.
    pub fn schematic_saved(&mut self, ok: bool) {
        self.notify(if ok {
            "SCHEMATIC SAVED"
        } else {
            "SCHEMATIC SAVE FAILED"
        });
    }

    /// Левый нижний угол вставки: схема центрируется под курсором.
    fn paste_origin(&self, s: &Schematic) -> TilePos {
        let t = self.cursor_tile();
        TilePos::new(t.x - s.width / 2, t.y - s.height / 2)
    }

    /// Результат сохранения (вызывает окружение).
    pub fn saved(&mut self, ok: bool) {
        self.quicksave_exists |= ok;
        self.notify(if ok { "GAME SAVED" } else { "SAVE FAILED" });
    }

    /// Результат загрузки (вызывает окружение): при успехе — сразу в игру.
    pub fn loaded(&mut self, world: Option<World>) {
        match world {
            Some(w) => {
                self.world = w;
                self.drag = None;
                self.pending.clear();
                self.last_wave = self
                    .world
                    .wave_info()
                    .map_or(0, |(n, _)| n.saturating_sub(1));
                self.screen = Screen::Playing;
                self.notify("GAME LOADED");
            }
            None => self.notify("NO SAVE TO LOAD"),
        }
    }

    fn new_game(&mut self) {
        let settings = MapSettings {
            size: self.map_size,
            difficulty: self.difficulty,
            seed: self.seed,
        };
        match mapgen::generate(&self.reg, &settings) {
            Ok(w) => {
                self.world = w;
                self.camera.center = [
                    self.world.grid().width() as f32 / 2.0,
                    self.world.grid().height() as f32 / 2.0,
                ];
                self.camera.zoom = 32.0;
                self.tool = None;
                self.drag = None;
                self.pending.clear();
                self.paused = false;
                self.last_wave = 0;
                self.paste = None;
                self.copy_mode = false;
                self.inspected = None;
                self.screen = Screen::Playing;
                self.message = Some((
                    "CHOOSE A BUILDING BELOW (1-5, TAB: CATEGORY), THEN CLICK THE MAP".into(),
                    8.0,
                ));
            }
            Err(e) => self.notify(format!("CANNOT CREATE MAP: {e}")),
        }
    }

    /// Выбранная постройка.
    pub fn selected_block(&self) -> Option<BlockId> {
        self.tool
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
            Input::Scale(s) => self.ui_scale = (s * 4.0).round().max(4.0) / 4.0,
            Input::CursorMoved(p) => {
                if let Some(Drag::Pan(last)) = self.drag {
                    self.camera.pan_pixels(p[0] - last[0], p[1] - last[1]);
                    self.drag = Some(Drag::Pan(p));
                }
                self.cursor = p;
                self.ui.mouse = p;
            }
            Input::Key(k, down) => self.key(k, down),
            _ if self.screen != Screen::Playing => self.menu_mouse(input),
            Input::MouseDown(Button::Left) if self.ui.blocks(self.cursor) => {
                self.ui.pressed_at = Some(self.cursor);
            }
            Input::MouseUp(Button::Left) if self.ui.pressed_at.is_some() => self.ui.released = true,
            Input::Wheel(d) => self.camera.zoom_at(1.15f32.powf(d), self.cursor),
            Input::MouseDown(Button::Middle) => self.drag = Some(Drag::Pan(self.cursor)),
            Input::MouseDown(Button::Left) if self.paste.is_some() => {
                if let Some(s) = &self.paste {
                    let cmds = s.commands(self.paste_origin(s));
                    self.queue(cmds);
                }
            }
            Input::MouseDown(Button::Left) if self.copy_mode => {
                self.drag = Some(Drag::Copy(self.cursor_tile()));
            }
            Input::MouseDown(Button::Right) if self.paste.is_some() || self.copy_mode => {
                self.paste = None;
                self.copy_mode = false;
                self.drag = None;
            }
            Input::MouseDown(Button::Left) => {
                if self.selected_block().is_some() {
                    self.drag = Some(Drag::Build(self.cursor_tile()));
                } else {
                    let t = self.cursor_tile();
                    if self.world.grid().building(t).is_some() {
                        self.inspected = Some(t); // клик по постройке без инструмента — осмотр
                    } else {
                        self.inspected = None;
                        self.notify("CHOOSE A BUILDING BELOW (1-5, TAB: CATEGORY)");
                    }
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
                    (Some(Drag::Copy(start)), Button::Left) => {
                        self.drag = None;
                        match self.world.copy_schematic(&self.reg, start, end) {
                            Some(sch) => {
                                self.notify(format!(
                                    "COPIED {}: CLICK TO PLACE, R ROTATE, F6 SAVE",
                                    sch.entries.len()
                                ));
                                self.copy_mode = false;
                                self.paste = Some(sch);
                            }
                            None => self.notify("NOTHING TO COPY HERE"),
                        }
                    }
                    _ => {} // отпущена другая кнопка — жест продолжается
                }
            }
        }
    }

    /// Мышь в меню: только левая кнопка для интерфейса.
    fn menu_mouse(&mut self, input: Input) {
        match input {
            Input::MouseDown(Button::Left) => self.ui.pressed_at = Some(self.cursor),
            Input::MouseUp(Button::Left) if self.ui.pressed_at.is_some() => self.ui.released = true,
            _ => {}
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
            self.pan_keys[i] = down && self.screen == Screen::Playing;
            return;
        }
        if !down {
            return;
        }
        match (self.screen, k) {
            (Screen::PauseMenu, Key::Cancel) => self.screen = Screen::Playing,
            (Screen::Playing, Key::Cancel) => {
                if self.show_help {
                    self.show_help = false;
                } else if self.tool.is_some()
                    || self.drag.is_some()
                    || self.inspected.is_some()
                    || self.paste.is_some()
                    || self.copy_mode
                {
                    self.tool = None;
                    self.drag = None;
                    self.inspected = None;
                    self.paste = None;
                    self.copy_mode = false;
                } else {
                    self.screen = Screen::PauseMenu;
                }
            }
            (Screen::Playing, Key::Digit(d)) => {
                let i = usize::from(d.saturating_sub(1));
                self.select(i);
            }
            (Screen::Playing, Key::Rotate) if self.paste.is_some() => {
                self.paste = self.paste.as_ref().map(|s| s.rotated_cw(&self.reg));
            }
            (Screen::Playing, Key::Copy) => {
                self.copy_mode = !self.copy_mode;
                self.paste = None;
                self.tool = None;
                if self.copy_mode {
                    self.notify("DRAG A FRAME AROUND BUILDINGS TO COPY");
                }
            }
            (Screen::Playing, Key::SaveSchematic) => match &self.paste {
                Some(s) => self.requests.push(Request::SaveSchematic(s.clone())),
                None => self.notify("COPY SOMETHING FIRST (C)"),
            },
            (Screen::Playing, Key::LoadSchematic) => self.requests.push(Request::LoadSchematic),
            (Screen::NewGame, Key::Cancel) => self.screen = Screen::MainMenu,
            (Screen::Playing, Key::Rotate) => {
                self.rot = match self.rot {
                    Dir::Right => Dir::Down,
                    Dir::Down => Dir::Left,
                    Dir::Left => Dir::Up,
                    Dir::Up => Dir::Right,
                }
            }
            (Screen::Playing, Key::Pause) => self.paused = !self.paused,
            (Screen::Playing, Key::NextCategory) => {
                let n = self.categories.len().max(1);
                self.set_category((self.category + 1) % n);
            }
            (Screen::Playing, Key::Help) => self.show_help = !self.show_help,
            (Screen::Playing | Screen::PauseMenu, Key::Save) => self.requests.push(Request::Save),
            (_, Key::Load) => self.requests.push(Request::Load),
            (Screen::Playing, Key::Faster) => self.speed = (self.speed * 2).min(8),
            (Screen::Playing, Key::Slower) => self.speed = (self.speed / 2).max(1),
            _ => {}
        }
    }

    fn select(&mut self, i: usize) {
        let Some(&b) = self.hotbar.get(i) else { return };
        self.paste = None;
        self.copy_mode = false;
        self.tool = if self.tool == Some(b) { None } else { Some(b) };
    }

    fn set_category(&mut self, k: usize) {
        if let Some((_, blocks)) = self.categories.get(k) {
            self.category = k;
            self.hotbar = blocks.clone();
        }
    }

    /// Прямоугольник вкладки категории.
    pub fn tab_rect(&self, k: usize) -> Rect {
        let u = self.u();
        let (w, h, gap) = (150.0 * u, 26.0 * u, 6.0 * u);
        let n = self.categories.len() as f32;
        let total = n * w + (n - 1.0) * gap;
        let x0 = (self.camera.viewport[0] - total) / 2.0;
        let y = self.hotbar_rect(0)[1] - 70.0 * u;
        [x0 + k as f32 * (w + gap), y, w, h]
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
            let results: Vec<_> = cmds
                .iter()
                .map(|c| self.world.apply(&self.reg, c))
                .collect();
            self.report_failures(&results);
        } else {
            self.pending.extend(cmds);
        }
    }

    fn report_failures(&mut self, results: &[Result<(), CommandError>]) {
        let failed = results.iter().filter(|r| r.is_err()).count();
        let Some(Err(first)) = results.iter().find(|r| r.is_err()) else {
            return;
        };
        let why = match first {
            CommandError::NotEnough(item) => format!("NOT ENOUGH {}", self.item_name(*item)),
            CommandError::Occupied => "TILE IS OCCUPIED".to_owned(),
            CommandError::NotBuildable => "CANNOT BUILD ON THIS FLOOR".to_owned(),
            CommandError::OutOfBounds => "OUTSIDE THE MAP".to_owned(),
            other => format!("{other}").to_uppercase(),
        };
        if results.len() == 1 {
            self.notify(why);
        } else {
            self.notify(format!("{failed} OF {} NOT PLACED: {why}", results.len()));
        }
    }

    /// Продвигает игру на `dt` секунд реального времени: панорама, фиксированные тики (30 Hz).
    /// В меню паузы симуляция стоит; в главном меню мир живёт на фоне.
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
        let running = match self.screen {
            Screen::MainMenu | Screen::NewGame => true,
            Screen::Playing => !self.paused && self.world.outcome() == Outcome::Running,
            Screen::PauseMenu => false,
        };
        if running {
            let step = 1.0 / f64::from(TICKS_PER_SECOND);
            self.accumulator += dt * f64::from(self.speed);
            // не больше 8 тиков за кадр — защита от «спирали смерти»
            while self.accumulator >= step && ticks < 8 {
                let cmds = std::mem::take(&mut self.pending);
                let results = self.world.tick(&self.reg, &cmds);
                self.report_failures(&results);
                self.accumulator -= step;
                ticks += 1;
            }
            if ticks == 8 {
                self.accumulator = 0.0;
            }
        }
        self.tps.tick(dt, ticks);
        if self.screen == Screen::Playing
            && let Some((next, _)) = self.world.wave_info()
        {
            let arrived = next.saturating_sub(1);
            if arrived > self.last_wave {
                self.last_wave = arrived;
                let n: u32 = World::wave_composition(&self.reg, arrived)
                    .iter()
                    .map(|(_, c)| c)
                    .sum();
                self.message = Some((format!("WAVE {arrived}: {n} ENEMIES INCOMING"), 5.0));
            }
        }
    }

    /// Собирает кадр: слой мира и слой интерфейса (и обрабатывает клики по интерфейсу).
    pub fn frame(
        &mut self,
        atlas: &Atlas,
        table: &SpriteTable,
        world_batch: &mut SpriteBatch,
        hud: &mut Hud,
    ) {
        world_batch.clear();
        self.ghosts.clear();
        let over_ui = self.ui.blocks(self.cursor);
        if self.screen == Screen::Playing
            && !over_ui
            && let Some(_) = self.selected_block()
        {
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
        }
        if self.screen == Screen::Playing
            && !over_ui
            && let Some(s) = &self.paste
        {
            for c in s.commands(self.paste_origin(s)) {
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
        if let Some(Drag::Copy(start)) = self.drag {
            let end = self.cursor_tile();
            let (x0, x1) = (start.x.min(end.x), start.x.max(end.x) + 1);
            let (y0, y1) = (start.y.min(end.y), start.y.max(end.y) + 1);
            world_batch.push(
                atlas.uv(table.white),
                [(x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0],
                [(x1 - x0) as f32, (y1 - y0) as f32],
                0.0,
                [0.2, 0.55, 0.95, 0.3],
            );
        }
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

        let mut ui = std::mem::take(&mut self.ui);
        ui.begin();
        hud.begin(self.camera.viewport[1]);
        match self.screen {
            Screen::MainMenu => self.main_menu(&mut ui, atlas, table, hud),
            Screen::NewGame => self.new_game_screen(&mut ui, atlas, table, hud),
            Screen::Playing => self.play_hud(&mut ui, atlas, table, hud),
            Screen::PauseMenu => {
                self.play_hud(&mut ui, atlas, table, hud);
                self.pause_menu(&mut ui, atlas, table, hud);
            }
        }
        if self.screen == Screen::Playing
            && let Outcome::Lost { .. } = self.world.outcome()
        {
            self.game_over(&mut ui, atlas, table, hud);
        }
        self.toast(atlas, table, hud);
        ui.end();
        self.ui = ui;
    }

    /// Масштаб интерфейса (по DPI, не меньше 1).
    fn u(&self) -> f32 {
        self.ui_scale.max(1.0)
    }

    /// Прямоугольники кнопок вертикального меню по центру экрана.
    pub fn menu_rects(&self, n: usize, top: f32) -> Vec<Rect> {
        let u = self.u();
        let (w, h, gap) = (340.0 * u, 52.0 * u, 12.0 * u);
        let x = (self.camera.viewport[0] - w) / 2.0;
        (0..n)
            .map(|i| [x, top + i as f32 * (h + gap), w, h])
            .collect()
    }

    /// Прямоугольник слота панели построек.
    pub fn hotbar_rect(&self, i: usize) -> Rect {
        let u = self.u();
        let (slot, gap) = (64.0 * u, 6.0 * u);
        let n = self.hotbar.len() as f32;
        let total = n * slot + (n - 1.0) * gap;
        let x0 = (self.camera.viewport[0] - total) / 2.0;
        let y0 = self.camera.viewport[1] - slot - 40.0 * u;
        [x0 + i as f32 * (slot + gap), y0, slot, slot]
    }

    fn main_menu(&mut self, ui: &mut Ui, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, 0.72]);
        let title = "FERRUSTRY";
        let ts = 10.0 * u;
        let ty = vh * 0.16;
        hud.text(
            atlas,
            (vw - text_width(title, ts)) / 2.0,
            ty,
            ts,
            title,
            rgba(palette::FORGE_ORANGE),
        );
        let sub = "REFORGE THE INDUSTRY";
        hud.text(
            atlas,
            (vw - text_width(sub, 3.0 * u)) / 2.0,
            ty + 84.0 * u,
            3.0 * u,
            sub,
            rgba(palette::STEEL_WHITE),
        );
        let desc = "INDUSTRIAL STRATEGY, REENGINEERED IN RUST";
        hud.text(
            atlas,
            (vw - text_width(desc, 2.0 * u)) / 2.0,
            ty + 116.0 * u,
            2.0 * u,
            desc,
            rgba(palette::TITANIUM),
        );

        let r = self.menu_rects(3, ty + 170.0 * u);
        if ui.button(hud, atlas, white, r[0], "NEW GAME", 3.0 * u, true) {
            self.screen = Screen::NewGame;
        }
        if ui.button(
            hud,
            atlas,
            white,
            r[1],
            "CONTINUE",
            3.0 * u,
            self.quicksave_exists,
        ) {
            self.requests.push(Request::Load);
        }
        if ui.button(hud, atlas, white, r[2], "QUIT", 3.0 * u, true) {
            self.requests.push(Request::Quit);
        }
        let ver = concat!("V", env!("CARGO_PKG_VERSION"), "  EARLY PROTOTYPE");
        hud.text(
            atlas,
            12.0 * u,
            vh - 22.0 * u,
            2.0 * u,
            ver,
            rgba(palette::TITANIUM),
        );
    }

    fn game_over(&mut self, ui: &mut Ui, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.08, 0.0, 0.0, 0.6]);
        let t = "CORE DESTROYED";
        let ty = vh * 0.25;
        hud.text(
            atlas,
            (vw - text_width(t, 6.0 * u)) / 2.0,
            ty,
            6.0 * u,
            t,
            rgba(palette::FORGE_ORANGE),
        );
        let waves = self
            .world
            .wave_info()
            .map_or(0, |(n, _)| n.saturating_sub(2));
        let sub = format!(
            "SURVIVED {waves} WAVES  -  {} ENEMIES DESTROYED",
            self.world.kills()
        );
        hud.text(
            atlas,
            (vw - text_width(&sub, 2.0 * u)) / 2.0,
            ty + 64.0 * u,
            2.0 * u,
            &sub,
            rgba(palette::STEEL_WHITE),
        );
        let r = self.menu_rects(2, ty + 110.0 * u);
        if ui.button(hud, atlas, white, r[0], "NEW GAME", 3.0 * u, true) {
            self.screen = Screen::NewGame;
        }
        if ui.button(hud, atlas, white, r[1], "MAIN MENU", 3.0 * u, true) {
            self.screen = Screen::MainMenu;
            self.tool = None;
        }
    }

    /// Раскладка экрана новой игры: размеры, сложности, «случайный seed», «старт», «назад».
    pub fn new_game_layout(&self) -> ([Rect; 3], [Rect; 3], Rect, Rect, Rect) {
        let u = self.u();
        let vw = self.camera.viewport[0];
        let (bw, bh, gap) = (170.0 * u, 44.0 * u, 12.0 * u);
        let row = |y: f32| -> [Rect; 3] {
            let x0 = (vw - (3.0 * bw + 2.0 * gap)) / 2.0;
            [0, 1, 2].map(|i| [x0 + i as f32 * (bw + gap), y, bw, bh])
        };
        let top = self.camera.viewport[1] * 0.22;
        let sizes = row(top + 60.0 * u);
        let diffs = row(top + 160.0 * u);
        let random = [vw / 2.0 + 110.0 * u, top + 240.0 * u, 170.0 * u, bh];
        let start = [(vw - 340.0 * u) / 2.0, top + 320.0 * u, 340.0 * u, 52.0 * u];
        let back = [(vw - 340.0 * u) / 2.0, top + 384.0 * u, 340.0 * u, 52.0 * u];
        (sizes, diffs, random, start, back)
    }

    fn new_game_screen(&mut self, ui: &mut Ui, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        let (orange, steel, titanium) = (
            rgba(palette::FORGE_ORANGE),
            rgba(palette::STEEL_WHITE),
            rgba(palette::TITANIUM),
        );
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, 0.78]);
        let top = vh * 0.22;
        let t = "NEW GAME";
        hud.text(
            atlas,
            (vw - text_width(t, 5.0 * u)) / 2.0,
            top - 50.0 * u,
            5.0 * u,
            t,
            orange,
        );
        let (sizes, diffs, random, start, back) = self.new_game_layout();
        let label = |hud: &mut Hud, y: f32, s: &str| {
            hud.text(
                atlas,
                (vw - text_width(s, 2.0 * u)) / 2.0,
                y,
                2.0 * u,
                s,
                titanium,
            );
        };
        label(hud, top + 34.0 * u, "MAP SIZE");
        for (i, (name, size)) in [
            ("SMALL", MapSize::Small),
            ("MEDIUM", MapSize::Medium),
            ("LARGE", MapSize::Large),
        ]
        .into_iter()
        .enumerate()
        {
            if ui.button(hud, atlas, white, sizes[i], name, 2.0 * u, true) {
                self.map_size = size;
            }
            if self.map_size == size {
                let r = sizes[i];
                hud.rect(
                    atlas,
                    white,
                    [r[0], r[1] + r[3] - 3.0 * u, r[2], 3.0 * u],
                    orange,
                );
            }
        }
        label(hud, top + 134.0 * u, "DIFFICULTY (WAVE PACE)");
        for (i, (name, d)) in [
            ("EASY", Difficulty::Easy),
            ("NORMAL", Difficulty::Normal),
            ("HARD", Difficulty::Hard),
        ]
        .into_iter()
        .enumerate()
        {
            if ui.button(hud, atlas, white, diffs[i], name, 2.0 * u, true) {
                self.difficulty = d;
            }
            if self.difficulty == d {
                let r = diffs[i];
                hud.rect(
                    atlas,
                    white,
                    [r[0], r[1] + r[3] - 3.0 * u, r[2], 3.0 * u],
                    orange,
                );
            }
        }
        let (first, spacing) = self.difficulty.timing_secs();
        let pace = format!(
            "FIRST WAVE {}:{:02}, THEN EVERY {}:{:02}",
            first / 60,
            first % 60,
            spacing / 60,
            spacing % 60
        );
        label(hud, diffs[0][1] + diffs[0][3] + 10.0 * u, &pace);
        let seed = format!("SEED {}", self.seed);
        hud.text(
            atlas,
            random[0] - 20.0 * u - text_width(&seed, 2.0 * u),
            random[1] + 14.0 * u,
            2.0 * u,
            &seed,
            steel,
        );
        if ui.button(hud, atlas, white, random, "RANDOM", 2.0 * u, true) {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64);
            self.seed = (nanos ^ self.seed.rotate_left(17)) % 1_000_000;
        }
        if ui.button(hud, atlas, white, start, "START", 3.0 * u, true) {
            self.new_game();
        }
        if ui.button(hud, atlas, white, back, "BACK", 3.0 * u, true) {
            self.screen = Screen::MainMenu;
        }
    }

    fn pause_menu(&mut self, ui: &mut Ui, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, 0.5]);
        let top = vh * 0.28;
        let r = self.menu_rects(5, top);
        let panel = [
            r[0][0] - 24.0 * u,
            top - 70.0 * u,
            r[0][2] + 48.0 * u,
            r[4][1] + r[4][3] - top + 94.0 * u,
        ];
        ui.panel(hud, atlas, white, panel, [0.02, 0.025, 0.04, 0.92]);
        let t = "PAUSED";
        hud.text(
            atlas,
            (vw - text_width(t, 4.0 * u)) / 2.0,
            top - 50.0 * u,
            4.0 * u,
            t,
            rgba(palette::FORGE_ORANGE),
        );
        let s = 3.0 * u;
        if ui.button(hud, atlas, white, r[0], "RESUME", s, true) {
            self.screen = Screen::Playing;
        }
        if ui.button(hud, atlas, white, r[1], "SAVE  (F5)", s, true) {
            self.requests.push(Request::Save);
        }
        if ui.button(
            hud,
            atlas,
            white,
            r[2],
            "LOAD  (F9)",
            s,
            self.quicksave_exists,
        ) {
            self.requests.push(Request::Load);
        }
        if ui.button(hud, atlas, white, r[3], "MAIN MENU", s, true) {
            self.screen = Screen::MainMenu;
            self.tool = None;
        }
        if ui.button(hud, atlas, white, r[4], "QUIT", s, true) {
            self.requests.push(Request::Quit);
        }
    }

    fn play_hud(&mut self, ui: &mut Ui, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        let panel = [0.02, 0.025, 0.04, 0.82];
        let orange = rgba(palette::FORGE_ORANGE);
        let steel = rgba(palette::STEEL_WHITE);
        let titanium = rgba(palette::TITANIUM);

        // заголовок, статус и волны
        ui.panel(hud, atlas, white, [0.0, 0.0, 380.0 * u, 84.0 * u], panel);
        if let Some((n, left)) = self.world.wave_info() {
            let secs = left / u64::from(TICKS_PER_SECOND);
            let enemies = self.world.units().count();
            let wave = format!(
                "WAVE {n} IN {}:{:02}  ENEMIES {enemies}  KILLS {}",
                secs / 60,
                secs % 60,
                self.world.kills()
            );
            let col = if left < 30 * u64::from(TICKS_PER_SECOND) {
                orange
            } else {
                steel
            };
            hud.text(atlas, 12.0 * u, 60.0 * u, 2.0 * u, &wave, col);
        }
        hud.text(atlas, 12.0 * u, 10.0 * u, 3.0 * u, "FERRUSTRY", orange);
        let status = format!(
            "TICK {}  FPS {:.0}  TPS {:.0}  X{}{}",
            self.world.tick_count(),
            self.fps.value,
            self.tps.value,
            self.speed,
            if self.paused { "  PAUSED" } else { "" }
        );
        hud.text(atlas, 12.0 * u, 40.0 * u, 2.0 * u, &status, steel);

        // кнопка меню в правом верхнем углу и ресурсы ядер под ней
        let mb = [vw - 110.0 * u, 8.0 * u, 100.0 * u, 34.0 * u];
        if ui.button(hud, atlas, white, mb, "MENU", 2.0 * u, true) {
            self.screen = Screen::PauseMenu;
        }
        let core = self.world.total_core_items();
        let pw = 240.0 * u;
        let rows = self.reg.items().len() as f32;
        ui.panel(
            hud,
            atlas,
            white,
            [vw - pw, 50.0 * u, pw, 30.0 * u + rows * 18.0 * u],
            panel,
        );
        hud.text(
            atlas,
            vw - pw + 10.0 * u,
            58.0 * u,
            2.0 * u,
            "CORE STORAGE",
            orange,
        );
        for (i, d) in self.reg.items().iter().enumerate() {
            let l = format!(
                "{:<11}{:>6}",
                d.name.replace('_', " "),
                core.get(i).copied().unwrap_or(0)
            );
            hud.text(
                atlas,
                vw - pw + 10.0 * u,
                78.0 * u + i as f32 * 18.0 * u,
                2.0 * u,
                &l,
                steel,
            );
        }

        // вкладки категорий
        for k in 0..self.categories.len() {
            let r = self.tab_rect(k);
            let name = self.categories[k].0;
            if ui.button(hud, atlas, white, r, name, 2.0 * u, true) {
                self.set_category(k);
            }
            if k == self.category {
                hud.rect(
                    atlas,
                    white,
                    [r[0], r[1] + r[3] - 3.0 * u, r[2], 3.0 * u],
                    orange,
                );
            }
        }

        // панель построек: клик выбирает, наведение — подсказка
        let mut tooltip = None;
        for i in 0..self.hotbar.len() {
            let r = self.hotbar_rect(i);
            let b = self.hotbar[i];
            ui.block(r);
            let selected = self.tool == Some(b);
            let hover = ui.hovered(r);
            if selected || hover {
                let c = if selected {
                    orange
                } else {
                    rgba(palette::MOLTEN_GOLD)
                };
                hud.rect(
                    atlas,
                    white,
                    [
                        r[0] - 3.0 * u,
                        r[1] - 3.0 * u,
                        r[2] + 6.0 * u,
                        r[3] + 6.0 * u,
                    ],
                    c,
                );
            }
            hud.rect(atlas, white, r, panel);
            let pad = 6.0 * u;
            hud.batch.push(
                table.block(b),
                [r[0] + r[2] / 2.0, vh - (r[1] + r[3] / 2.0)],
                [r[2] - 2.0 * pad, r[3] - 2.0 * pad],
                0.0,
                if self.affordable(b) {
                    [1.0; 4]
                } else {
                    [0.3, 0.3, 0.32, 1.0]
                },
            );
            let key = match i {
                0..=8 => (i + 1).to_string(),
                9 => "0".to_owned(),
                _ => String::new(),
            };
            hud.text(
                atlas,
                r[0] + 4.0 * u,
                r[1] + 4.0 * u,
                2.0 * u,
                &key,
                if selected { orange } else { titanium },
            );
            if ui.clicked(r) {
                self.select(i);
            }
            if hover {
                tooltip = Some((i, r));
            }
        }
        let label = match (tooltip, self.selected_block()) {
            (Some((i, _)), _) => self.block_info(self.hotbar[i]),
            (None, Some(b)) => format!(
                "{}   R: ROTATE ({:?})   RMB: CANCEL",
                self.block_info(b),
                self.rot
            ),
            (None, None) => {
                "CHOOSE A BUILDING (CLICK OR 1-5, TAB: CATEGORY), THEN CLICK THE MAP".to_owned()
            }
        };
        let r0 = self.hotbar_rect(0);
        let lw = text_width(&label, 2.0 * u);
        ui.panel(
            hud,
            atlas,
            white,
            [
                (vw - lw) / 2.0 - 10.0 * u,
                r0[1] - 34.0 * u,
                lw + 20.0 * u,
                26.0 * u,
            ],
            panel,
        );
        hud.text(
            atlas,
            (vw - lw) / 2.0,
            r0[1] - 28.0 * u,
            2.0 * u,
            &label,
            steel,
        );
        hud.text(
            atlas,
            10.0 * u,
            vh - 24.0 * u,
            2.0 * u,
            "F1 HELP   ESC MENU",
            titanium,
        );

        // подсказка под курсором (только над картой и только в игре)
        if self.screen == Screen::Playing && !ui.blocks(self.cursor) {
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
                self.cursor[0] + 16.0 * u,
                self.cursor[1] + 16.0 * u,
                2.0 * u,
                &tip,
                steel,
            );
        }

        self.inspector(ui, atlas, table, hud);

        if self.show_help {
            let lines = [
                "CONTROLS",
                "",
                "1-5 / CLICK         CHOOSE BUILDING",
                "TAB / CLICK TAB     BUILDING CATEGORY",
                "C + DRAG            COPY BUILDINGS (THEN CLICK TO PASTE)",
                "F6 / F7             SAVE / LOAD SCHEMATIC",
                "CLICK A BUILDING    INSPECT (SET SORTER FILTER)",
                "LEFT CLICK          BUILD",
                "LEFT DRAG           BUILD A LINE (BELTS TURN)",
                "R                   ROTATE",
                "RIGHT CLICK         CANCEL TOOL",
                "RIGHT DRAG          REMOVE AREA",
                "WASD / ARROWS / MMB MOVE CAMERA",
                "WHEEL               ZOOM",
                "SPACE               PAUSE",
                "+ / -               SIMULATION SPEED",
                "F5 / F9             QUICK SAVE / LOAD",
                "ESC                 MENU",
                "",
                "BORES ON ORE -> BELTS -> SMELTER (NEEDS POWER)",
                "-> BELTS -> CORE.  TURBINES MUST TOUCH THE SMELTER",
                "OR CONNECT THROUGH A POWER NODE.",
            ];
            let s = 2.0 * u;
            let w = lines.iter().map(|l| text_width(l, s)).fold(0.0, f32::max) + 40.0 * u;
            let h = lines.len() as f32 * 18.0 * u + 40.0 * u;
            let (x, y) = ((vw - w) / 2.0, (vh - h) / 2.0);
            ui.panel(hud, atlas, white, [x, y, w, h], [0.02, 0.025, 0.04, 0.94]);
            for (i, l) in lines.iter().enumerate() {
                let c = if i == 0 { orange } else { steel };
                hud.text(
                    atlas,
                    x + 20.0 * u,
                    y + 20.0 * u + i as f32 * 18.0 * u,
                    s,
                    l,
                    c,
                );
            }
        }
    }

    /// Панель осмотра выбранной постройки (слева); у сортировщика — выбор фильтра.
    fn inspector(&mut self, ui: &mut Ui, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        let Some(at) = self.inspected else { return };
        let Some(ins) = self.world.inspect(&self.reg, at) else {
            self.inspected = None; // постройку снесли
            return;
        };
        let u = self.u();
        let white = table.white;
        let steel = rgba(palette::STEEL_WHITE);
        let orange = rgba(palette::FORGE_ORANGE);
        let mut lines: Vec<String> = Vec::new();
        let pct = |a: u64, b: u64| (a * 100).checked_div(b).unwrap_or(0);
        match &ins.detail {
            Detail::Conveyor { items_on_segment } => {
                lines.push(format!("ITEMS ON SEGMENT {items_on_segment}"))
            }
            Detail::Drill {
                ore,
                ore_tiles,
                buffer,
            } => match ore {
                Some(o) => {
                    lines.push(format!("MINING {}", self.item_name(*o)));
                    lines.push(format!("ORE TILES {ore_tiles}  BUFFER {buffer}"));
                }
                None => lines.push("NO ORE UNDER THIS BORE".to_owned()),
            },
            Detail::Crafter {
                inputs,
                outputs,
                progress,
                ready,
            } => {
                for (i, n) in inputs {
                    lines.push(format!("IN  {:<11}{n:>4}", self.item_name(*i)));
                }
                for (i, n) in outputs {
                    lines.push(format!("OUT {:<11}{n:>4}", self.item_name(*i)));
                }
                lines.push(format!("PROGRESS {}%", progress / 10));
                if !ready {
                    lines.push("WAITING FOR INPUT / OUTPUT SPACE".to_owned());
                }
            }
            Detail::Core { items } => {
                for (i, n) in items.iter().enumerate() {
                    lines.push(format!(
                        "{:<11}{n:>6}",
                        self.reg.items()[i].name.replace('_', " ")
                    ));
                }
            }
            Detail::Generator {
                power,
                fuel,
                running,
            } => {
                lines.push(if *running {
                    format!("OUTPUT {power} / TICK")
                } else {
                    "NO FUEL".to_owned()
                });
                if let Some((item, n)) = fuel {
                    lines.push(format!("FUEL {} {n}", self.item_name(*item)));
                }
            }
            Detail::Depot {
                items,
                total,
                capacity,
            } => {
                lines.push(format!("STORED {total} / {capacity}"));
                for (i, n) in items {
                    lines.push(format!("{:<11}{n:>6}", self.item_name(*i)));
                }
                lines.push("OUTPUT: FRONT SIDE".to_owned());
            }
            Detail::ArcTurret { charge } => {
                lines.push(format!("CHARGE {}%", charge / 10));
            }
            Detail::PowerNode => lines.push("CONNECTS BUILDINGS IN RANGE".to_owned()),
            Detail::Battery { stored, capacity } => {
                lines.push(format!(
                    "CHARGE {stored} / {capacity} ({}%)",
                    pct(*stored, u64::from(*capacity))
                ));
            }
            Detail::Splitter { item } => {
                lines.push(format!(
                    "HOLDING {}",
                    item.map_or("NOTHING".to_owned(), |i| self.item_name(i))
                ));
            }
            Detail::Sorter { item, filter } => {
                lines.push(format!(
                    "FILTER {}",
                    filter.map_or("NONE".to_owned(), |i| self.item_name(i))
                ));
                lines.push(format!(
                    "HOLDING {}",
                    item.map_or("NOTHING".to_owned(), |i| self.item_name(i))
                ));
            }
            Detail::Network => {}
            Detail::HeatEngine { output } => lines.push(format!("OUTPUT {output} POWER / TICK")),
            Detail::Turret { ammo } => lines.push(format!("AMMO {ammo} SHOTS")),
            Detail::Wall => {}
            Detail::Bridge { items, linked } => {
                lines.push(if *linked {
                    "LINKED: SENDING".to_owned()
                } else {
                    "END: OUTPUT FORWARD".to_owned()
                });
                lines.push(format!("BUFFER {}", items.len()));
            }
        }
        if ins.health < ins.max_health {
            lines.push(format!("HEALTH {} / {}", ins.health, ins.max_health));
        }
        if let Some(h) = &ins.heat {
            let state = if h.temperature >= 1000 {
                "OVERHEATED"
            } else if h.temperature > ferrustry_sim::thermal::THROTTLE_FROM {
                "SLOWING DOWN"
            } else {
                "OK"
            };
            lines.push(format!("HEAT {}%  {state}", h.temperature / 10));
            lines.push(format!("HEAT +{} -{} / TICK", h.generated, h.removed));
        }
        if let Some(f) = &ins.fluid {
            let name = f.fluid.map_or("EMPTY".to_owned(), |x| {
                self.reg.fluid(x).name.to_uppercase()
            });
            lines.push(format!("{name} {} / {}", f.amount, f.capacity));
        }
        if let Some(p) = &ins.power {
            lines.push(format!(
                "POWER {}%  +{} -{}",
                p.satisfaction / 10,
                p.production,
                p.demand
            ));
            if p.capacity > 0 {
                lines.push(format!("NET BATTERIES {} / {}", p.stored, p.capacity));
            }
        }
        let title = self.reg.block(ins.block).name.replace('_', " ");
        let s = 2.0 * u;
        let row = 18.0 * u;
        let sorter = matches!(ins.detail, Detail::Sorter { .. });
        let n_items = self.reg.items().len();
        let w = 300.0 * u;
        let buttons_h = if sorter {
            (n_items as f32 + 1.0) * (row + 6.0 * u) + 24.0 * u
        } else {
            0.0
        };
        let h = 44.0 * u + lines.len() as f32 * row + buttons_h;
        let (x, y) = (8.0 * u, 74.0 * u);
        ui.panel(hud, atlas, white, [x, y, w, h], [0.02, 0.025, 0.04, 0.9]);
        hud.rect(atlas, white, [x, y, w, 3.0 * u], orange);
        hud.text(atlas, x + 12.0 * u, y + 12.0 * u, s, &title, orange);
        for (i, l) in lines.iter().enumerate() {
            hud.text(
                atlas,
                x + 12.0 * u,
                y + 36.0 * u + i as f32 * row,
                s,
                l,
                steel,
            );
        }
        if sorter {
            let mut by = y + 36.0 * u + lines.len() as f32 * row + 12.0 * u;
            hud.text(
                atlas,
                x + 12.0 * u,
                by,
                s,
                "SET FILTER:",
                rgba(palette::TITANIUM),
            );
            by += row;
            let opts: Vec<Option<ferrustry_content::ItemId>> = std::iter::once(None)
                .chain((0..n_items).map(|i| Some(ferrustry_content::ItemId(i as u16))))
                .collect();
            for (k, opt) in opts.into_iter().enumerate() {
                let r = [
                    x + 12.0 * u,
                    by + k as f32 * (row + 6.0 * u),
                    w - 24.0 * u,
                    row + 2.0 * u,
                ];
                let label = opt.map_or("NONE".to_owned(), |i| self.item_name(i));
                if ui.button(hud, atlas, white, r, &label, s, true) {
                    self.queue(vec![Command::Configure { at, item: opt }]);
                }
            }
        }
    }

    fn item_name(&self, i: ferrustry_content::ItemId) -> String {
        self.reg.item(i).name.replace('_', " ").to_uppercase()
    }

    fn stacks(&self, v: &[ferrustry_content::ItemStack]) -> String {
        v.iter()
            .map(|s| format!("{} {}", s.count, self.item_name(s.item)))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Хватает ли ресурсов на постройку (в песочнице — всегда).
    fn affordable(&self, b: BlockId) -> bool {
        if self.world.rules().infinite_resources {
            return true;
        }
        let have = self.world.total_core_items();
        self.reg
            .block(b)
            .cost
            .iter()
            .all(|s| have.get(usize::from(s.item.0)).copied().unwrap_or(0) >= u64::from(s.count))
    }

    /// Краткое описание постройки для подсказки (с ценой).
    fn block_info(&self, b: BlockId) -> String {
        let d = self.reg.block(b);
        let name = d.name.replace('_', " ");
        let what = match &d.kind {
            BlockKind::Conveyor { .. } => format!("{name}: MOVES ITEMS"),
            BlockKind::Drill { .. } => format!("{name} {0}X{0}: MINES ORE UNDER IT", d.size),
            BlockKind::Crafter {
                inputs,
                outputs,
                power_use,
                heat,
                ..
            } => format!(
                "{name} {}X{}: {} -> {}, POWER {}{}",
                d.size,
                d.size,
                self.stacks(inputs),
                self.stacks(outputs),
                power_use,
                if *heat > 0 {
                    format!(", HEAT +{heat}")
                } else {
                    String::new()
                }
            ),
            BlockKind::Generator {
                power, fuel: None, ..
            } => format!("{name}: +{power} POWER"),
            BlockKind::Generator {
                power,
                fuel: Some(fuel),
                burn_ticks,
                ..
            } => format!(
                "{name} {0}X{0}: +{power} POWER WHILE BURNING {1} ({burn_ticks} TICKS EACH)",
                d.size,
                self.item_name(*fuel)
            ),
            BlockKind::Depot { capacity } => format!(
                "{name} {0}X{0}: STORES {capacity} ITEMS, OUTPUTS FORWARD",
                d.size
            ),
            BlockKind::ArcTurret {
                range,
                damage,
                power_use,
                ..
            } => format!(
                "{name} {0}X{0}: RANGE {range}, DAMAGE {damage}, IGNORES ARMOR, POWER {power_use}",
                d.size
            ),
            BlockKind::PowerNode { range } => format!("{name}: LINKS POWER WITHIN {range} TILES"),
            BlockKind::Core { capacity } => {
                format!("{name} {0}X{0}: STORES {capacity} OF EACH ITEM", d.size)
            }
            BlockKind::Splitter => format!("{name}: SPLITS ITEMS TO THE OTHER 3 SIDES"),
            BlockKind::Sorter => format!("{name}: FILTER STRAIGHT, REST SIDEWAYS"),
            BlockKind::Bridge { range, .. } => {
                format!("{name}: JUMPS {range} TILES TO NEXT BRIDGE")
            }
            BlockKind::Battery { capacity } => format!("{name}: STORES {capacity} POWER"),
            BlockKind::Turret {
                range,
                damage,
                ammo,
                shots_per_item,
                ..
            } => format!(
                "{name} 2X2: RANGE {range}, DAMAGE {damage}, AMMO {} ({shots_per_item} SHOTS)",
                self.item_name(*ammo)
            ),
            BlockKind::Wall => format!("{name}: {} HEALTH, ENEMIES PREFER TO GO AROUND", d.health),
            BlockKind::Pipe { capacity } => format!("{name}: CARRIES FLUID ({capacity})"),
            BlockKind::Condenser {
                rate, power_use, ..
            } => {
                format!("{name} 2X2: MAKES {rate} WATER/TICK, POWER {power_use}")
            }
            BlockKind::CoolingTower { rate, cooling, .. } => {
                format!("{name} 2X2: {rate} WATER -> -{} HEAT", rate * cooling)
            }
            BlockKind::HeatPipe { .. } => format!("{name}: CONNECTS HEAT BUILDINGS"),
            BlockKind::Radiator { dissipation, .. } => format!("{name}: -{dissipation} HEAT/TICK"),
            BlockKind::HeatEngine {
                max_heat,
                power_per_heat_permille,
                ..
            } => format!(
                "{name} 2X2: {max_heat} HEAT -> {} POWER",
                max_heat * power_per_heat_permille / 1000
            ),
        };
        if d.cost.is_empty() || self.world.rules().infinite_resources {
            what
        } else {
            format!("{what}  COST {}", self.stacks(&d.cost))
        }
    }

    fn toast(&self, atlas: &Atlas, table: &SpriteTable, hud: &mut Hud) {
        if let Some((m, _)) = &self.message {
            let u = self.u();
            let vw = self.camera.viewport[0];
            let s = 2.0 * u;
            let w = text_width(m, s);
            let r = [(vw - w) / 2.0 - 14.0 * u, 76.0 * u, w + 28.0 * u, 34.0 * u];
            hud.rect(atlas, table.white, r, [0.02, 0.025, 0.04, 0.9]);
            hud.rect(
                atlas,
                table.white,
                [r[0], r[1] + r[3] - 3.0 * u, r[2], 3.0 * u],
                rgba(palette::FORGE_ORANGE),
            );
            hud.text(
                atlas,
                (vw - w) / 2.0,
                86.0 * u,
                s,
                m,
                rgba(palette::STEEL_WHITE),
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ferrustry_sim::scenario;

    struct Env {
        atlas: Atlas,
        table: SpriteTable,
        wb: SpriteBatch,
        hud: Hud,
    }

    fn env() -> Env {
        let reg = ContentRegistry::base().unwrap();
        let atlas = Atlas::build(&reg);
        let table = SpriteTable::new(&atlas, &reg);
        Env {
            atlas,
            table,
            wb: SpriteBatch::default(),
            hud: Hud::default(),
        }
    }

    fn game() -> Game {
        let reg = ContentRegistry::base().unwrap();
        let world = scenario::demo_map(&reg, 1).unwrap();
        Game::new(reg, world, [1280.0, 720.0])
    }

    fn frame(g: &mut Game, e: &mut Env) {
        g.frame(&e.atlas, &e.table, &mut e.wb, &mut e.hud);
    }

    fn move_to(g: &mut Game, p: [f32; 2]) {
        g.handle(Input::CursorMoved(p));
    }

    fn center(r: Rect) -> [f32; 2] {
        [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
    }

    /// Клик левой кнопкой по точке экрана с кадрами до и после (как в реальном цикле).
    fn click(g: &mut Game, e: &mut Env, p: [f32; 2]) {
        move_to(g, p);
        frame(g, e);
        g.handle(Input::MouseDown(Button::Left));
        frame(g, e);
        g.handle(Input::MouseUp(Button::Left));
        frame(g, e);
    }

    fn click_tile(g: &mut Game, t: TilePos) {
        let p = g
            .camera
            .world_to_screen([t.x as f32 + 0.5, t.y as f32 + 0.5]);
        g.handle(Input::CursorMoved(p));
    }

    fn playing() -> (Game, Env) {
        let (mut g, mut e) = (game(), env());
        frame(&mut g, &mut e);
        // «NEW GAME» — первая кнопка главного меню
        let ty = 720.0 * 0.16;
        let new_game = g.menu_rects(3, ty + 170.0)[0];
        click(&mut g, &mut e, center(new_game));
        assert_eq!(g.screen(), Screen::NewGame);
        let (_, _, _, start, _) = g.new_game_layout();
        click(&mut g, &mut e, center(start));
        assert_eq!(g.screen(), Screen::Playing);
        (g, e)
    }

    #[test]
    fn starts_in_main_menu_and_new_game_button_works() {
        let (g, _) = playing();
        assert!(g.selected_block().is_none());
    }

    #[test]
    fn clicking_hotbar_selects_building_without_building_under_it() {
        let (mut g, mut e) = playing();
        let before = g.world.buildings().len();
        g.set_category(1);
        let slot = center(g.hotbar_rect(1));
        click(&mut g, &mut e, slot);
        assert_eq!(g.selected_block(), g.reg.block_by_name("smelter"));
        g.update(0.2);
        assert_eq!(
            g.world.buildings().len(),
            before,
            "click on the panel must not build"
        );
        // клик по тому же слоту снимает выбор
        click(&mut g, &mut e, slot);
        assert!(g.selected_block().is_none());
    }

    #[test]
    fn belt_drag_makes_l_shape_with_flow_directions() {
        let (mut g, _) = playing();
        g.handle(Input::Key(Key::Digit(1), true));
        let cmds = g.build_commands(TilePos::new(0, 0), TilePos::new(3, -2));
        let placed: Vec<(TilePos, Dir)> = cmds
            .iter()
            .map(|c| match c {
                Command::Place { origin, rot, .. } => (*origin, *rot),
                _ => unreachable!(),
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
        let (mut g, mut e) = playing();
        let before = g.world.buildings().len();
        g.handle(Input::Key(Key::Digit(1), true));
        let a = TilePos::new(90, 60);
        let b = TilePos::new(99, 60);
        click_tile(&mut g, a);
        frame(&mut g, &mut e);
        g.handle(Input::MouseDown(Button::Left));
        click_tile(&mut g, b);
        g.handle(Input::MouseUp(Button::Left));
        g.update(1.0 / 30.0 + 1e-6);
        assert_eq!(g.world.buildings().len(), before + 10);
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
    fn click_on_map_without_tool_explains_what_to_do() {
        let (mut g, mut e) = playing();
        g.message = None;
        click_tile(&mut g, TilePos::new(92, 68));
        frame(&mut g, &mut e);
        g.handle(Input::MouseDown(Button::Left));
        assert!(
            g.message
                .as_ref()
                .is_some_and(|(m, _)| m.contains("CHOOSE A BUILDING"))
        );
    }

    #[test]
    fn escape_opens_pause_menu_which_stops_simulation() {
        let (mut g, mut e) = playing();
        g.handle(Input::Key(Key::Cancel, true));
        assert_eq!(g.screen(), Screen::PauseMenu);
        let t = g.world.tick_count();
        g.update(1.0);
        assert_eq!(g.world.tick_count(), t);
        frame(&mut g, &mut e);
        let resume = g.menu_rects(5, 720.0 * 0.28)[0];
        click(&mut g, &mut e, center(resume));
        assert_eq!(g.screen(), Screen::Playing);
    }

    #[test]
    fn fixed_step_runs_30_ticks_per_second_and_pauses() {
        let (mut g, _) = playing();
        let t0 = g.world.tick_count();
        for _ in 0..60 {
            g.update(1.0 / 60.0);
        }
        assert!(
            (29..=31).contains(&(g.world.tick_count() - t0)),
            "{}",
            g.world.tick_count() - t0
        );
        g.handle(Input::Key(Key::Pause, true));
        let t = g.world.tick_count();
        g.update(1.0);
        assert_eq!(g.world.tick_count(), t);
        g.handle(Input::Key(Key::Pause, true));
        g.update(10.0);
        assert!(g.world.tick_count() - t <= 8);
    }

    #[test]
    fn frame_contains_ghosts_and_hud() {
        let (mut g, mut e) = playing();
        move_to(&mut g, [300.0, 300.0]);
        frame(&mut g, &mut e);
        let base = e.wb.len();
        g.handle(Input::Key(Key::Digit(3), true));
        frame(&mut g, &mut e);
        assert_eq!(e.wb.len(), base + 1, "one ghost for the smelter");
        assert!(e.hud.batch.len() > 50);
    }

    #[test]
    fn clicking_a_building_opens_inspector_and_sorter_filter_can_be_set() {
        let (mut g, mut e) = playing();
        // поставить сортировщик (слот 3) в свободном месте и снять инструмент
        g.handle(Input::Key(Key::Digit(3), true));
        // расчищенная площадка у ядра (центр 96,64 средней карты) гарантирована генератором
        let t = TilePos::new(100, 68);
        click_tile(&mut g, t);
        frame(&mut g, &mut e);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        g.update(1.0 / 30.0 + 1e-6);
        g.handle(Input::Key(Key::Cancel, true));
        assert!(g.selected_block().is_none());
        // клик по нему без инструмента — осмотр
        click_tile(&mut g, t);
        frame(&mut g, &mut e);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        assert_eq!(g.inspected, Some(t));
        frame(&mut g, &mut e);
        // кнопки фильтра: NONE, затем ресурсы по порядку; выбрать IRON INGOT (ID 2)
        let u = 1.0;
        let (x, y, row) = (8.0 * u, 74.0 * u, 18.0 * u);
        let lines = 2.0; // FILTER, HOLDING
        let by = y + 36.0 * u + lines * row + 12.0 * u + row;
        let k = 1.0 + 2.0; // NONE + iron_ore, carbon -> iron_ingot
        let p = [x + 40.0 * u, by + k * (row + 6.0 * u) + row / 2.0];
        click(&mut g, &mut e, p);
        g.update(1.0 / 30.0 + 1e-6);
        let ingot = g.reg.item_by_name("iron_ingot");
        match g.world.inspect(&g.reg, t).unwrap().detail {
            Detail::Sorter { filter, .. } => assert_eq!(filter, ingot),
            d => panic!("{d:?}"),
        }
        assert!(
            g.selected_block().is_none(),
            "clicking the inspector must not build"
        );
    }

    #[test]
    fn not_enough_resources_is_explained() {
        let (mut g, mut e) = playing();
        // ядро стоит 500 слитков + 100 стали, на старте 150 слитков
        g.tool = g.reg.block_by_name("core");
        click_tile(&mut g, TilePos::new(91, 67));
        frame(&mut g, &mut e);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        g.update(1.0 / 30.0 + 1e-6);
        let msg = g
            .message
            .as_ref()
            .map(|(m, _)| m.clone())
            .unwrap_or_default();
        assert_eq!(msg, "NOT ENOUGH IRON INGOT");
    }

    #[test]
    fn losing_the_core_stops_the_game_and_offers_a_new_one() {
        let (mut g, mut e) = playing();
        let brute = g.reg.unit_by_name("brute").unwrap();
        // ядро стартовой базы — в центре карты; окружить его тяжёлыми врагами
        let core = g
            .world
            .buildings()
            .iter()
            .find(|(_, b)| b.tag == ferrustry_sim::KindTag::Core)
            .map(|(_, b)| b.origin)
            .unwrap();
        let sub = ferrustry_sim::combat::SUB;
        for k in 0..24 {
            let (x, y) = (core.x - 1 + (k % 5), core.y - 1 + (k / 5) % 5);
            if g.world.grid().building(TilePos::new(x, y)).is_none() {
                g.world
                    .spawn_unit(&g.reg, brute, x * sub + 128, y * sub + 128);
            }
        }
        for _ in 0..400 {
            g.update(1.0 / 30.0 + 1e-6);
            if g.world.outcome() != Outcome::Running {
                break;
            }
        }
        assert!(matches!(g.world.outcome(), Outcome::Lost { .. }));
        let t = g.world.tick_count();
        g.update(1.0);
        assert_eq!(g.world.tick_count(), t, "simulation stops after defeat");
        frame(&mut g, &mut e);
        let vh = 720.0;
        let new_game = g.menu_rects(2, vh * 0.25 + 110.0)[0];
        click(&mut g, &mut e, center(new_game));
        assert_eq!(g.screen(), Screen::NewGame);
        let (_, _, _, start, _) = g.new_game_layout();
        click(&mut g, &mut e, center(start));
        assert_eq!(g.world.outcome(), Outcome::Running);
        assert_eq!(g.screen(), Screen::Playing);
    }

    #[test]
    fn copy_frame_then_paste_and_rotate() {
        let (mut g, mut e) = playing();
        g.handle(Input::Key(Key::Digit(1), true));
        click_tile(&mut g, TilePos::new(90, 60));
        frame(&mut g, &mut e);
        g.handle(Input::MouseDown(Button::Left));
        click_tile(&mut g, TilePos::new(93, 60));
        g.handle(Input::MouseUp(Button::Left));
        g.update(1.0 / 30.0 + 1e-6);
        let before = g.world.buildings().len();
        // C + рамка вокруг ленты
        g.handle(Input::Key(Key::Copy, true));
        assert!(g.selected_block().is_none());
        click_tile(&mut g, TilePos::new(89, 59));
        frame(&mut g, &mut e);
        g.handle(Input::MouseDown(Button::Left));
        click_tile(&mut g, TilePos::new(94, 61));
        g.handle(Input::MouseUp(Button::Left));
        let sch = g.paste.clone().unwrap();
        assert_eq!((sch.width, sch.height), (4, 1));
        // вставка выше; призраки видны
        click_tile(&mut g, TilePos::new(92, 68));
        frame(&mut g, &mut e);
        assert_eq!(g.ghosts.len(), 4);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        g.update(1.0 / 30.0 + 1e-6);
        assert_eq!(g.world.buildings().len(), before + 4);
        // R поворачивает схему; ПКМ выходит из режима вставки
        g.handle(Input::Key(Key::Rotate, true));
        let r = g.paste.as_ref().unwrap();
        assert_eq!((r.width, r.height), (1, 4));
        g.handle(Input::MouseDown(Button::Right));
        assert!(g.paste.is_none());
    }

    #[test]
    fn hidpi_scale_enlarges_ui() {
        let mut g = game();
        let r1 = g.hotbar_rect(0);
        g.handle(Input::Scale(1.5));
        let r2 = g.hotbar_rect(0);
        assert!(r2[2] > r1[2] * 1.4);
    }
}
