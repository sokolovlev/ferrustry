//! Редактор карт: рельеф, руды, ядра, точки появления врагов и правила партии.
//!
//! Редактор правит обычный мир в режиме песочницы (симуляция стоит, стройка бесплатна).
//! «PLAY» снимает с него [`MapDef`] с правилами и стартовым запасом редактора и создаёт
//! из неё партию, а «SAVE» записывает ту же карту в `.fmap` (в открытый файл или в новый
//! `maps/map-NNN.fmap`). Каждый мазок кисти и клик — шаг отмены: снимок карты ([`MapDef`],
//! ~6 байт на тайл), а не всего мира (~30 байт на тайл с полем путей).

use ferrustry_content::{FloorId, ItemId, OreId};
use ferrustry_render::atlas::palette;
use ferrustry_render::{Atlas, Hud, SpriteBatch, SpriteTable, rgba};
use ferrustry_sim::buildings::KindTag;
use ferrustry_sim::map::MapDef;
use ferrustry_sim::mapgen::{self, Difficulty, MapSettings};
use ferrustry_sim::{Command, Dir, Rules, TICKS_PER_SECOND, TilePos, World};

use super::{Button, Drag, Game, Input, Request, Screen};
use crate::ui::{Rect, Ui, text_width};

/// Инструмент редактора.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTool {
    /// Залить пол.
    Floor(FloorId),
    /// Положить руду.
    Ore(OreId),
    /// Убрать руду.
    ClearOre,
    /// Поставить или убрать точку появления врагов.
    Spawn,
    /// Поставить ядро.
    Core,
    /// Снести постройку.
    Erase,
}

/// Пресеты волн: выключены или по темпу сложности.
const WAVE_PRESETS: [Option<Difficulty>; 4] = [
    None,
    Some(Difficulty::Easy),
    Some(Difficulty::Normal),
    Some(Difficulty::Hard),
];

/// Варианты стартового запаса железных слитков.
const STARTER_STEPS: [u32; 6] = [0, 150, 300, 600, 1000, 2000];

/// Размеры кисти (сторона квадрата).
const BRUSHES: [i32; 4] = [1, 3, 5, 9];

/// Предел точек появления врагов.
const MAX_SPAWNS: usize = 16;

/// Глубина отмены (снимков мира).
pub const UNDO_DEPTH: usize = 64;

/// Действие, при котором несохранённые изменения пропадут.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leave {
    /// Новая случайная карта.
    Generate,
    /// Пустая карта.
    Blank,
    /// Список карт.
    Maps,
    /// Главное меню.
    Back,
}

/// Состояние редактора.
#[derive(Debug, Clone)]
pub struct Editor {
    tool: Option<EditTool>,
    brush: usize,
    waves: usize,
    starter: Vec<(ItemId, u32)>,
    /// Файл карты (`None` — новая, ещё не сохранённая).
    pub(super) path: Option<String>,
    undo: Vec<MapDef>,
    redo: Vec<MapDef>,
    /// Карта до начала текущего мазка.
    before: Option<MapDef>,
    dirty: bool,
    confirm: Option<Leave>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            tool: None,
            brush: 1,
            waves: 2,
            starter: Vec::new(),
            path: None,
            undo: Vec::new(),
            redo: Vec::new(),
            before: None,
            dirty: false,
            confirm: None,
        }
    }
}

impl Editor {
    /// Правила партии, которые получит карта.
    pub fn rules(&self) -> Rules {
        let tps = u64::from(TICKS_PER_SECOND);
        match WAVE_PRESETS[self.waves] {
            None => Rules {
                infinite_resources: false,
                waves: false,
                ..Rules::default()
            },
            Some(d) => {
                let (first, spacing) = d.timing_secs();
                Rules {
                    infinite_resources: false,
                    waves: true,
                    first_wave: first * tps,
                    wave_spacing: spacing * tps,
                }
            }
        }
    }

    fn set_rules(&mut self, r: &Rules) {
        let tps = u64::from(TICKS_PER_SECOND);
        self.waves = if !r.waves {
            0
        } else {
            WAVE_PRESETS
                .iter()
                .position(|p| {
                    p.is_some_and(|d| {
                        let (f, s) = d.timing_secs();
                        f * tps == r.first_wave && s * tps == r.wave_spacing
                    })
                })
                .unwrap_or(2)
        };
    }

    fn waves_label(&self) -> &'static str {
        match WAVE_PRESETS[self.waves] {
            None => "WAVES: OFF",
            Some(Difficulty::Easy) => "WAVES: EASY",
            Some(Difficulty::Normal) => "WAVES: NORMAL",
            Some(Difficulty::Hard) => "WAVES: HARD",
        }
    }
}

/// Раскладка экрана редактора.
#[derive(Debug, Clone)]
pub struct EditorLayout {
    /// Инструменты (в порядке [`Game::editor_tools`]).
    pub tools: Vec<Rect>,
    /// Размер кисти.
    pub brush: Rect,
    /// Волны.
    pub waves: Rect,
    /// Стартовый запас.
    pub starter: Rect,
    /// UNDO, REDO.
    pub history: [Rect; 2],
    /// GENERATE, BLANK, SAVE, MAPS, PLAY, BACK.
    pub actions: [Rect; 6],
}

impl Game {
    /// Инструменты редактора: все полы, все руды, затем остальные.
    pub fn editor_tools(&self) -> Vec<(EditTool, String)> {
        let mut v: Vec<(EditTool, String)> = Vec::new();
        for (i, f) in self.reg.floors().iter().enumerate() {
            v.push((
                EditTool::Floor(FloorId(i as u16)),
                f.name.replace('_', " ").to_uppercase(),
            ));
        }
        for (i, o) in self.reg.ores().iter().enumerate() {
            v.push((
                EditTool::Ore(OreId(i as u16)),
                o.name.replace('_', " ").to_uppercase(),
            ));
        }
        v.push((EditTool::ClearOre, "CLEAR ORE".into()));
        v.push((EditTool::Spawn, "ENEMY SPAWN".into()));
        v.push((EditTool::Core, "CORE".into()));
        v.push((EditTool::Erase, "REMOVE BUILDING".into()));
        v
    }

    /// Раскладка экрана редактора.
    pub fn editor_layout(&self) -> EditorLayout {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let (x, w, h, gap) = (12.0 * u, 230.0 * u, 30.0 * u, 5.0 * u);
        let n = self.editor_tools().len();
        let top = 64.0 * u;
        let tools: Vec<Rect> = (0..n)
            .map(|i| [x, top + i as f32 * (h + gap), w, h])
            .collect();
        let below = top + n as f32 * (h + gap) + 14.0 * u;
        let brush = [x, below, w, h];
        let waves = [x, below + h + gap, w, h];
        let starter = [x, below + 2.0 * (h + gap), w, h];
        let hy = below + 3.0 * (h + gap) + 9.0 * u;
        let hw = (w - gap) / 2.0;
        let history = [[x, hy, hw, h], [x + hw + gap, hy, hw, h]];
        let (bw, bh, bgap) = (150.0 * u, 40.0 * u, 8.0 * u);
        let x0 = (vw - (6.0 * bw + 5.0 * bgap)) / 2.0;
        let y = vh - bh - 16.0 * u;
        let actions = [0, 1, 2, 3, 4, 5].map(|i| [x0 + i as f32 * (bw + bgap), y, bw, bh]);
        EditorLayout {
            tools,
            brush,
            waves,
            starter,
            history,
            actions,
        }
    }

    /// Открыть редактор на карте, созданной генератором по настройкам новой игры.
    pub fn open_editor(&mut self) {
        let settings = MapSettings {
            size: self.map_size,
            difficulty: self.difficulty,
            seed: self.seed,
        };
        match mapgen::generate(&self.reg, &settings) {
            Ok(w) => {
                let def = MapDef::from_world(&w, &self.reg);
                self.edit_map(&def, None);
            }
            Err(e) => self.notify(format!("CANNOT CREATE MAP: {e}")),
        }
    }

    /// Открыть карту в редакторе: правила и стартовый запас переходят в настройки
    /// редактора, а сам мир правится в песочнице. История отмены начинается заново.
    pub(super) fn edit_map(&mut self, def: &MapDef, path: Option<String>) {
        let mut sandbox = def.clone();
        sandbox.rules = Rules::default();
        sandbox.starter.clear();
        match sandbox.instantiate(&self.reg, self.seed) {
            Ok(w) => {
                self.editor.set_rules(&def.rules);
                self.editor.starter = def.starter.clone();
                self.editor.path = path;
                self.editor.undo.clear();
                self.editor.redo.clear();
                self.editor.before = None;
                self.editor.dirty = false;
                self.editor.confirm = None;
                self.world = w;
                self.camera.center = [
                    self.world.grid().width() as f32 / 2.0,
                    self.world.grid().height() as f32 / 2.0,
                ];
                self.camera.zoom = 16.0;
                self.drag = None;
                self.tool = None;
                self.paste = None;
                self.copy_mode = false;
                self.inspected = None;
                self.pending.clear();
                self.screen = Screen::Editor;
            }
            Err(e) => self.notify(format!("CANNOT OPEN MAP: {e}").to_uppercase()),
        }
    }

    /// Пустая карта текущего размера, залитая первым полом.
    fn blank_map(&mut self) {
        let (w, h) = self.map_size.dims();
        let floor = FloorId(0);
        let world = World::new(&self.reg, w as u32, h as u32, floor, self.seed);
        let mut def = MapDef::from_world(&world, &self.reg);
        def.rules = self.editor.rules();
        def.starter = self.editor.starter.clone();
        self.edit_map(&def, None);
        self.notify("BLANK MAP: PLACE A CORE AND ENEMY SPAWNS");
    }

    /// Карта из редактора с его правилами и стартовым запасом.
    pub fn editor_map(&self) -> MapDef {
        let mut def = MapDef::from_world(&self.world, &self.reg);
        def.rules = self.editor.rules();
        def.starter = self.editor.starter.clone();
        def
    }

    /// Почему по карте нельзя начать партию (`None` — можно).
    pub fn editor_problem(&self) -> Option<String> {
        let cores: Vec<TilePos> = self
            .world
            .buildings()
            .iter()
            .filter(|(_, b)| b.tag == KindTag::Core)
            .map(|(_, b)| b.origin)
            .collect();
        if cores.is_empty() {
            return Some("PLACE A CORE FIRST".into());
        }
        let rules = self.editor.rules();
        if rules.waves && self.world.spawns().is_empty() {
            return Some("ADD AN ENEMY SPAWN OR TURN WAVES OFF".into());
        }
        if rules.waves {
            for (i, &sp) in self.world.spawns().iter().enumerate() {
                if !cores
                    .iter()
                    .any(|&c| mapgen::reachable(&self.world, &self.reg, sp, c))
                {
                    return Some(format!("ENEMY SPAWN {} HAS NO PATH TO A CORE", i + 1));
                }
            }
        }
        None
    }

    fn editor_play(&mut self) {
        if let Some(p) = self.editor_problem() {
            self.notify(p);
            return;
        }
        match self.editor_map().instantiate(&self.reg, self.seed) {
            Ok(w) => self.begin_play(w),
            Err(e) => self.notify(format!("CANNOT START: {e}").to_uppercase()),
        }
    }

    /// Результат записи карты: путь файла, куда она записана.
    pub fn map_saved(&mut self, res: Result<String, String>) {
        match res {
            Ok(path) => {
                self.notify(format!("MAP SAVED: {path}").to_uppercase());
                self.editor.path = Some(path);
                self.editor.dirty = false;
                self.editor.confirm = None;
            }
            Err(e) => self.notify(format!("MAP NOT SAVED: {e}").to_uppercase()),
        }
    }

    /// Есть ли несохранённые изменения.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn editor_dirty(&self) -> bool {
        self.editor.dirty
    }

    /// Запомнить карту перед изменением (начало мазка или клика).
    fn editor_begin(&mut self) {
        self.editor.before = Some(MapDef::from_world(&self.world, &self.reg));
    }

    /// Закончить изменение: если карта изменилась, это шаг отмены.
    fn editor_commit(&mut self) {
        let Some(before) = self.editor.before.take() else {
            return;
        };
        if before == MapDef::from_world(&self.world, &self.reg) {
            return;
        }
        if self.editor.undo.len() >= UNDO_DEPTH {
            self.editor.undo.remove(0);
        }
        self.editor.undo.push(before);
        self.editor.redo.clear();
        self.editor.dirty = true;
        self.editor.confirm = None;
    }

    /// Вернуть песочницу к снимку; возвращает снимок текущего состояния.
    fn editor_restore(&mut self, def: &MapDef) -> Option<MapDef> {
        let now = MapDef::from_world(&self.world, &self.reg);
        match def.instantiate(&self.reg, self.seed) {
            Ok(w) => {
                self.world = w;
                self.editor.dirty = true;
                Some(now)
            }
            Err(e) => {
                self.notify(format!("CANNOT RESTORE: {e}").to_uppercase());
                None
            }
        }
    }

    /// Отменить последний шаг.
    pub fn editor_undo(&mut self) {
        match self.editor.undo.pop() {
            Some(d) => {
                if let Some(now) = self.editor_restore(&d) {
                    self.editor.redo.push(now);
                }
            }
            None => self.notify("NOTHING TO UNDO"),
        }
    }

    /// Повторить отменённый шаг.
    pub fn editor_redo(&mut self) {
        match self.editor.redo.pop() {
            Some(d) => {
                if let Some(now) = self.editor_restore(&d) {
                    self.editor.undo.push(now);
                }
            }
            None => self.notify("NOTHING TO REDO"),
        }
    }

    /// Уйти с карты; при несохранённых изменениях — только со второго раза.
    pub fn editor_leave(&mut self, leave: Leave) {
        if self.editor.dirty && self.editor.confirm != Some(leave) {
            self.editor.confirm = Some(leave);
            self.notify("UNSAVED CHANGES: SAVE, OR CLICK AGAIN TO DISCARD");
            return;
        }
        self.editor.confirm = None;
        self.drag = None;
        match leave {
            Leave::Generate => {
                self.seed = self
                    .seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                self.open_editor();
                self.notify(format!("GENERATED MAP, SEED {}", self.seed));
            }
            Leave::Blank => self.blank_map(),
            Leave::Maps => self.open_maps(),
            Leave::Back => self.screen = Screen::MainMenu,
        }
    }

    /// Мышь в редакторе.
    pub(super) fn editor_mouse(&mut self, input: Input) {
        match input {
            Input::MouseDown(Button::Left) if self.ui.blocks(self.cursor) => {
                self.ui.pressed_at = Some(self.cursor);
            }
            Input::MouseUp(Button::Left) if self.ui.pressed_at.is_some() => {
                self.ui.released = true;
            }
            Input::Wheel(d) => self.camera.zoom_at(1.15f32.powf(d), self.cursor),
            Input::MouseDown(Button::Middle) => self.drag = Some(Drag::Pan(self.cursor)),
            Input::MouseDown(b @ (Button::Left | Button::Right)) => {
                let erase = b == Button::Right;
                match (self.editor.tool, erase) {
                    // одиночные действия — по нажатию
                    (Some(EditTool::Spawn | EditTool::Core), false) => {
                        self.editor_begin();
                        self.editor_click();
                        self.editor_commit();
                    }
                    (None, false) => self.notify("CHOOSE A TOOL ON THE LEFT"),
                    _ => {
                        self.editor_begin();
                        self.drag = Some(Drag::Paint(erase));
                        self.editor_paint(erase);
                    }
                }
            }
            Input::MouseUp(_) => match self.drag {
                Some(Drag::Paint(_)) => {
                    self.drag = None;
                    self.editor_commit();
                }
                Some(Drag::Pan(_)) => self.drag = None,
                _ => {}
            },
            _ => {}
        }
    }

    fn editor_click(&mut self) {
        let t = self.cursor_tile();
        if !self.world.grid().in_bounds(t) {
            return;
        }
        match self.editor.tool {
            Some(EditTool::Spawn) => {
                let mut spawns = self.world.spawns().to_vec();
                if let Some(i) = spawns
                    .iter()
                    .position(|s| (s.x - t.x).abs() <= 1 && (s.y - t.y).abs() <= 1)
                {
                    spawns.remove(i);
                } else if spawns.len() >= MAX_SPAWNS {
                    self.notify(format!("AT MOST {MAX_SPAWNS} ENEMY SPAWNS"));
                    return;
                } else if !self
                    .world
                    .grid()
                    .floor(t)
                    .is_some_and(|f| self.reg.floor(f).buildable)
                {
                    self.notify("ENEMIES CANNOT WALK HERE");
                    return;
                } else {
                    spawns.push(t);
                }
                self.world.set_spawns(spawns);
            }
            Some(EditTool::Core) => {
                let Some(core) = self.reg.block_by_name("core") else {
                    return;
                };
                let origin = self.origin_for(t, self.reg.block(core).size);
                let res = self.world.apply(
                    &self.reg,
                    &Command::Place {
                        block: core,
                        origin,
                        rot: Dir::Right,
                    },
                );
                self.report_failures(&[res]);
            }
            _ => {}
        }
    }

    /// Кисть под курсором: рисует инструментом или стирает (руду и постройки).
    pub(super) fn editor_paint(&mut self, erase: bool) {
        let c = self.cursor_tile();
        let tool = if erase {
            EditTool::Erase
        } else {
            match self.editor.tool {
                Some(t @ (EditTool::Floor(_) | EditTool::Ore(_) | EditTool::ClearOre)) => t,
                Some(EditTool::Erase) => EditTool::Erase,
                _ => return,
            }
        };
        let side = BRUSHES[self.editor.brush];
        let r0 = (side - 1) / 2;
        for y in c.y - r0..c.y - r0 + side {
            for x in c.x - r0..c.x - r0 + side {
                let p = TilePos::new(x, y);
                if !self.world.grid().in_bounds(p) {
                    continue;
                }
                let occupied = self.world.grid().building(p).is_some();
                match tool {
                    EditTool::Erase => {
                        if occupied {
                            // снос в песочнице бесплатен и не меняет запасы карты
                            let _ = self.world.apply(&self.reg, &Command::Remove { at: p });
                        } else if erase {
                            self.world.set_ore(p, None);
                        }
                    }
                    // под постройками рельеф не меняется: иначе карта станет непроходимой
                    // для их правил (бур без руды, ядро на шлаке)
                    _ if occupied => {}
                    EditTool::Floor(f) => {
                        self.world.set_floor(p, f);
                        if !self.reg.floor(f).buildable {
                            self.world.set_ore(p, None);
                        }
                    }
                    EditTool::Ore(o) => {
                        if self
                            .world
                            .grid()
                            .floor(p)
                            .is_some_and(|f| self.reg.floor(f).buildable)
                        {
                            self.world.set_ore(p, Some(o));
                        }
                    }
                    EditTool::ClearOre => self.world.set_ore(p, None),
                    EditTool::Spawn | EditTool::Core => {}
                }
            }
        }
    }

    /// Маркеры точек появления и рамка кисти поверх карты.
    pub(super) fn editor_overlay(&self, wb: &mut SpriteBatch, atlas: &Atlas, table: &SpriteTable) {
        let white = atlas.uv(table.white);
        for s in self.world.spawns() {
            let c = [s.x as f32 + 0.5, s.y as f32 + 0.5];
            wb.push(white, c, [3.0, 3.0], 0.0, [0.9, 0.15, 0.1, 0.35]);
            wb.push(white, c, [1.0, 1.0], 0.0, [1.0, 0.3, 0.2, 0.9]);
        }
        if self.ui.blocks(self.cursor) {
            return;
        }
        let t = self.cursor_tile();
        let side = match self.editor.tool {
            Some(EditTool::Spawn) => 1,
            Some(EditTool::Core) => self
                .reg
                .block_by_name("core")
                .map_or(1, |b| i32::from(self.reg.block(b).size)),
            _ => BRUSHES[self.editor.brush],
        };
        let r0 = (side - 1) / 2;
        let c = [
            (t.x - r0) as f32 + side as f32 / 2.0,
            (t.y - r0) as f32 + side as f32 / 2.0,
        ];
        let col = if matches!(self.drag, Some(Drag::Paint(true))) {
            [0.9, 0.15, 0.1, 0.3]
        } else {
            [0.95, 0.55, 0.15, 0.3]
        };
        wb.push(white, c, [side as f32, side as f32], 0.0, col);
    }

    /// Интерфейс редактора.
    pub(super) fn editor_hud(
        &mut self,
        ui: &mut Ui,
        atlas: &Atlas,
        table: &SpriteTable,
        hud: &mut Hud,
    ) {
        let u = self.u();
        let white = table.white;
        let panel = [0.02, 0.025, 0.04, 0.82];
        let (orange, steel, titanium) = (
            rgba(palette::FORGE_ORANGE),
            rgba(palette::STEEL_WHITE),
            rgba(palette::TITANIUM),
        );
        let l = self.editor_layout();
        let tools = self.editor_tools();
        let bottom = l.history[0][1] + l.history[0][3] + 10.0 * u;
        ui.panel(hud, atlas, white, [0.0, 0.0, 254.0 * u, bottom], panel);
        hud.text(atlas, 12.0 * u, 14.0 * u, 3.0 * u, "MAP EDITOR", orange);
        let g = self.world.grid();
        let name = self
            .editor
            .path
            .as_deref()
            .map_or_else(|| "NEW MAP".to_owned(), super::maps::map_name);
        let info = format!(
            "{name}{}  {}X{}  SPAWNS {}",
            if self.editor.dirty { "*" } else { "" },
            g.width(),
            g.height(),
            self.world.spawns().len()
        );
        hud.text(atlas, 12.0 * u, 42.0 * u, 2.0 * u, &info, titanium);
        for ((tool, name), r) in tools.iter().zip(&l.tools) {
            if self.editor.tool == Some(*tool) {
                hud.rect(atlas, white, [r[0] - 6.0 * u, r[1], 4.0 * u, r[3]], orange);
            }
            if ui.button(hud, atlas, white, *r, name, 2.0 * u, true) {
                self.editor.tool = if self.editor.tool == Some(*tool) {
                    None
                } else {
                    Some(*tool)
                };
            }
        }
        let side = BRUSHES[self.editor.brush];
        let brush = format!("BRUSH: {side}X{side}");
        if ui.button(hud, atlas, white, l.brush, &brush, 2.0 * u, true) {
            self.editor.brush = (self.editor.brush + 1) % BRUSHES.len();
        }
        let waves = self.editor.waves_label();
        if ui.button(hud, atlas, white, l.waves, waves, 2.0 * u, true) {
            self.editor.waves = (self.editor.waves + 1) % WAVE_PRESETS.len();
            self.editor.dirty = true;
        }
        let ingot = self.reg.item_by_name("iron_ingot");
        let have = ingot
            .and_then(|i| self.editor.starter.iter().find(|(it, _)| *it == i))
            .map_or(0, |&(_, n)| n);
        let start = format!("START: {have} INGOTS");
        if ui.button(hud, atlas, white, l.starter, &start, 2.0 * u, true)
            && let Some(i) = ingot
        {
            let next = STARTER_STEPS
                .iter()
                .copied()
                .find(|&s| s > have)
                .unwrap_or(0);
            self.editor.starter.retain(|(it, _)| *it != i);
            if next > 0 {
                self.editor.starter.push((i, next));
            }
            self.editor.dirty = true;
        }
        let [undo, redo] = l.history;
        let (can_undo, can_redo) = (!self.editor.undo.is_empty(), !self.editor.redo.is_empty());
        if ui.button(hud, atlas, white, undo, "UNDO", 2.0 * u, can_undo) {
            self.editor_undo();
        }
        if ui.button(hud, atlas, white, redo, "REDO", 2.0 * u, can_redo) {
            self.editor_redo();
        }

        let a = l.actions[0];
        let vw = self.camera.viewport[0];
        ui.panel(
            hud,
            atlas,
            white,
            [
                a[0] - 10.0 * u,
                a[1] - 34.0 * u,
                vw - 2.0 * a[0] + 20.0 * u,
                a[3] + 44.0 * u,
            ],
            panel,
        );
        let labels = ["GENERATE", "BLANK", "SAVE", "MAPS", "PLAY", "BACK"];
        let mut clicked = None;
        for (k, (r, name)) in l.actions.iter().zip(labels).enumerate() {
            if ui.button(hud, atlas, white, *r, name, 2.0 * u, true) {
                clicked = Some(k);
            }
        }
        match clicked {
            Some(0) => self.editor_leave(Leave::Generate),
            Some(1) => self.editor_leave(Leave::Blank),
            Some(2) => self.requests.push(Request::SaveMap {
                def: Box::new(self.editor_map()),
                path: self.editor.path.clone(),
            }),
            Some(3) => self.editor_leave(Leave::Maps),
            Some(4) => self.editor_play(),
            Some(5) => self.editor_leave(Leave::Back),
            _ => {}
        }
        let hint =
            "LMB PAINT   RMB ERASE   Z UNDO   Y REDO   MMB/WASD MOVE   WHEEL ZOOM   ESC BACK";
        hud.text(
            atlas,
            (vw - text_width(hint, 2.0 * u)) / 2.0,
            a[1] - 24.0 * u,
            2.0 * u,
            hint,
            steel,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ferrustry_content::ContentRegistry;
    use ferrustry_sim::scenario;

    fn game() -> Game {
        let reg = ContentRegistry::base().unwrap();
        let world = scenario::demo_map(&reg, 1).unwrap();
        let mut g = Game::new(reg, world, [1280.0, 720.0]);
        g.map_size = mapgen::MapSize::Small;
        g
    }

    fn at_tile(g: &mut Game, t: TilePos) {
        let p = g
            .camera
            .world_to_screen([t.x as f32 + 0.5, t.y as f32 + 0.5]);
        g.handle(Input::CursorMoved(p));
    }

    fn pick(g: &mut Game, tool: EditTool) {
        g.editor.tool = Some(tool);
    }

    #[test]
    fn editor_opens_on_a_generated_map_with_its_rules() {
        let mut g = game();
        g.open_editor();
        assert_eq!(g.screen(), Screen::Editor);
        assert!(g.world.rules().infinite_resources, "editing is a sandbox");
        assert!(!g.world.rules().waves, "no waves while editing");
        let def = g.editor_map();
        assert!(def.rules.waves && !def.rules.infinite_resources);
        assert_eq!(
            def.starter.len(),
            1,
            "starter stock comes from the generator"
        );
        assert_eq!(g.editor_problem(), None);
        // карта из редактора = карта генератора (бит в бит после старта)
        let generated = mapgen::generate(
            &g.reg,
            &MapSettings {
                size: g.map_size,
                difficulty: g.difficulty,
                seed: g.seed,
            },
        )
        .unwrap();
        let played = def.instantiate(&g.reg, g.seed).unwrap();
        assert_eq!(
            ferrustry_sim::state_hash(&generated),
            ferrustry_sim::state_hash(&played)
        );
    }

    #[test]
    fn painting_floor_ore_and_erasing() {
        let mut g = game();
        g.blank_map();
        let reg = g.reg.clone();
        let slag = reg.floor_by_name("slag_pool").unwrap();
        let iron = reg.ore_by_name("iron_vein").unwrap();
        let t = TilePos::new(20, 20);
        at_tile(&mut g, t);
        pick(&mut g, EditTool::Ore(iron));
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        // кисть 3×3 по умолчанию
        for p in [t, TilePos::new(19, 19), TilePos::new(21, 21)] {
            assert_eq!(g.world.grid().ore(p), Some(iron));
        }
        assert_eq!(g.world.grid().ore(TilePos::new(22, 20)), None);
        // протяжка рисует по пути
        pick(&mut g, EditTool::Floor(slag));
        at_tile(&mut g, TilePos::new(30, 10));
        g.handle(Input::MouseDown(Button::Left));
        at_tile(&mut g, TilePos::new(40, 10));
        g.handle(Input::MouseUp(Button::Left));
        assert_eq!(g.world.grid().floor(TilePos::new(30, 10)), Some(slag));
        assert_eq!(g.world.grid().floor(TilePos::new(40, 10)), Some(slag));
        // ПКМ стирает руду
        at_tile(&mut g, t);
        g.handle(Input::MouseDown(Button::Right));
        g.handle(Input::MouseUp(Button::Right));
        assert_eq!(g.world.grid().ore(t), None);
    }

    #[test]
    fn play_requires_a_core_and_reachable_spawns() {
        let mut g = game();
        g.blank_map();
        assert_eq!(g.editor_problem().as_deref(), Some("PLACE A CORE FIRST"));
        pick(&mut g, EditTool::Core);
        at_tile(&mut g, TilePos::new(30, 30));
        g.handle(Input::MouseDown(Button::Left));
        assert!(g.editor_problem().unwrap().contains("SPAWN"));
        pick(&mut g, EditTool::Spawn);
        at_tile(&mut g, TilePos::new(4, 30));
        g.handle(Input::MouseDown(Button::Left));
        assert_eq!(g.world.spawns(), &[TilePos::new(4, 30)]);
        assert_eq!(g.editor_problem(), None);
        // стена шлака отрезает точку появления
        let slag = g.reg.floor_by_name("slag_pool").unwrap();
        for y in 0..g.world.grid().height() as i32 {
            g.world.set_floor(TilePos::new(10, y), slag);
        }
        assert!(g.editor_problem().unwrap().contains("NO PATH"));
        let basalt = g.reg.floor_by_name("basalt").unwrap();
        for y in 0..g.world.grid().height() as i32 {
            g.world.set_floor(TilePos::new(10, y), basalt);
        }
        // повторный клик рядом убирает точку
        at_tile(&mut g, TilePos::new(5, 31));
        g.handle(Input::MouseDown(Button::Left));
        assert!(g.world.spawns().is_empty());
        at_tile(&mut g, TilePos::new(4, 30));
        g.handle(Input::MouseDown(Button::Left));
        // запуск: правила и запас редактора, без песочницы
        g.editor_play();
        assert_eq!(g.screen(), Screen::Playing);
        assert!(g.world.rules().waves);
        assert!(!g.world.rules().infinite_resources);
        assert_eq!(g.world.spawns().len(), 1);
    }

    #[test]
    fn floor_under_buildings_is_kept() {
        let mut g = game();
        g.open_editor();
        let slag = g.reg.floor_by_name("slag_pool").unwrap();
        let core = g
            .world
            .buildings()
            .iter()
            .find(|(_, b)| b.tag == KindTag::Core)
            .map(|(_, b)| b.origin)
            .unwrap();
        g.editor.brush = 3; // 9×9
        pick(&mut g, EditTool::Floor(slag));
        at_tile(&mut g, core);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        assert_ne!(g.world.grid().floor(core), Some(slag));
        assert_eq!(
            g.world.grid().floor(TilePos::new(core.x + 3, core.y)),
            Some(slag)
        );
        assert!(g.editor_map().instantiate(&g.reg, 1).is_ok());
    }

    #[test]
    fn undo_and_redo_strokes() {
        let mut g = game();
        g.blank_map();
        let iron = g.reg.ore_by_name("iron_vein").unwrap();
        let t = TilePos::new(20, 20);
        assert!(!g.editor_dirty());
        // клик без изменений (снос пустого места) — не шаг отмены
        pick(&mut g, EditTool::Erase);
        at_tile(&mut g, t);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        assert!(g.editor.undo.is_empty() && !g.editor_dirty());
        // мазок с протяжкой — один шаг
        pick(&mut g, EditTool::Ore(iron));
        g.handle(Input::MouseDown(Button::Left));
        at_tile(&mut g, TilePos::new(30, 20));
        g.handle(Input::MouseUp(Button::Left));
        pick(&mut g, EditTool::Spawn);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        assert_eq!(g.editor.undo.len(), 2);
        assert!(g.editor_dirty());
        let painted = MapDef::from_world(&g.world, &g.reg);
        g.handle(Input::Key(crate::game::Key::Undo, true));
        assert!(g.world.spawns().is_empty());
        assert_eq!(g.world.grid().ore(t), Some(iron));
        g.editor_undo();
        assert_eq!(g.world.grid().ore(t), None);
        assert_eq!(g.world.grid().ore(TilePos::new(30, 20)), None);
        g.handle(Input::Key(crate::game::Key::Redo, true));
        g.editor_redo();
        assert_eq!(MapDef::from_world(&g.world, &g.reg), painted);
        // новый шаг после отмены стирает повтор
        g.editor_undo();
        pick(&mut g, EditTool::ClearOre);
        g.handle(Input::MouseDown(Button::Left));
        g.handle(Input::MouseUp(Button::Left));
        assert!(g.editor.redo.is_empty());
    }

    #[test]
    fn unsaved_changes_need_a_second_click() {
        let mut g = game();
        g.blank_map();
        pick(&mut g, EditTool::Spawn);
        at_tile(&mut g, TilePos::new(10, 10));
        g.handle(Input::MouseDown(Button::Left));
        assert!(g.editor_dirty());
        g.editor_leave(Leave::Back);
        assert_eq!(g.screen(), Screen::Editor, "first click only warns");
        g.editor_leave(Leave::Maps);
        assert_eq!(g.screen(), Screen::Editor, "another action warns again");
        g.handle(Input::Key(crate::game::Key::Cancel, true));
        g.handle(Input::Key(crate::game::Key::Cancel, true));
        assert_eq!(g.screen(), Screen::MainMenu);
        // после сохранения уходим сразу
        g.blank_map();
        g.handle(Input::MouseDown(Button::Left));
        g.map_saved(Ok("maps/map-002.fmap".into()));
        assert!(!g.editor_dirty());
        g.editor_leave(Leave::Back);
        assert_eq!(g.screen(), Screen::MainMenu);
    }

    #[test]
    fn undo_depth_is_bounded() {
        let mut g = game();
        g.blank_map();
        let iron = g.reg.ore_by_name("iron_vein").unwrap();
        at_tile(&mut g, TilePos::new(10, 10));
        for i in 0..UNDO_DEPTH + 5 {
            // руда и её стирание по очереди: каждый мазок меняет карту
            pick(
                &mut g,
                if i % 2 == 0 {
                    EditTool::Ore(iron)
                } else {
                    EditTool::ClearOre
                },
            );
            g.handle(Input::MouseDown(Button::Left));
            g.handle(Input::MouseUp(Button::Left));
        }
        assert_eq!(g.editor.undo.len(), UNDO_DEPTH);
    }

    #[test]
    fn editor_rules_roundtrip_through_presets() {
        let mut e = Editor::default();
        for k in 0..WAVE_PRESETS.len() {
            e.waves = k;
            let r = e.rules();
            let mut f = Editor::default();
            f.set_rules(&r);
            assert_eq!(f.waves, k);
        }
    }
}
