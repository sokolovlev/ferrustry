//! Экран «MAPS»: список карт из папки `maps/`, запуск партии на карте и открытие её в
//! редакторе. Файловую систему трогает окружение (`app.rs`) по запросам, а здесь — только
//! логика и интерфейс, поэтому всё проверяется тестами без окна.

use ferrustry_render::atlas::palette;
use ferrustry_render::{Atlas, Hud, SpriteTable, rgba};
use ferrustry_sim::buildings::KindTag;
use ferrustry_sim::map::MapDef;

use super::{Game, Request, Screen};
use crate::ui::{Rect, Ui, text_width};

/// Папка карт (рядом с местом запуска игры).
pub const MAPS_DIR: &str = "maps";
/// Файл карты первой версии редактора: тоже показывается в списке.
pub const LEGACY_MAP_FILE: &str = "ferrustry-map.fmap";
/// Карт на странице списка.
pub const MAPS_PER_PAGE: usize = 6;

/// Зачем открывается карта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapPurpose {
    /// Начать партию.
    Play,
    /// Открыть в редакторе.
    Edit,
}

/// Карта в списке.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    /// Путь к файлу.
    pub path: String,
    /// Имя для показа (имя файла без расширения).
    pub name: String,
    /// Сводка («192X128  3 SPAWNS») или причина, по которой карта не читается.
    pub info: String,
    /// Карта читается.
    pub ok: bool,
}

impl MapEntry {
    /// Запись списка по результату чтения файла.
    pub fn new(path: &str, res: &Result<MapDef, String>) -> Self {
        let (info, ok) = match res {
            Ok(d) => (
                format!("{}X{}  {} SPAWNS", d.width, d.height, d.spawns.len()),
                true,
            ),
            Err(e) => (format!("CANNOT READ: {e}").to_uppercase(), false),
        };
        Self {
            path: path.to_owned(),
            name: map_name(path),
            info,
            ok,
        }
    }
}

/// Имя карты по пути: имя файла без расширения, заглавными.
pub fn map_name(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.trim_end_matches(".fmap").to_uppercase()
}

/// Первый свободный путь вида `maps/map-001.fmap`.
pub fn next_map_path(exists: impl Fn(&str) -> bool) -> String {
    (1..)
        .map(|n| format!("{MAPS_DIR}/map-{n:03}.fmap"))
        .find(|p| !exists(p))
        .unwrap_or_else(|| format!("{MAPS_DIR}/map.fmap"))
}

/// Раскладка экрана карт.
#[derive(Debug, Clone)]
pub struct MapsLayout {
    /// Строки страницы: подпись, PLAY, EDIT.
    pub rows: Vec<[Rect; 3]>,
    /// NEW MAP, PREV, NEXT, BACK.
    pub actions: [Rect; 4],
}

impl Game {
    /// Открыть список карт (запрашивает чтение папки).
    pub fn open_maps(&mut self) {
        self.drag = None;
        self.maps_page = 0;
        self.screen = Screen::Maps;
        self.requests.push(Request::ListMaps);
    }

    /// Список карт прочитан.
    pub fn maps_listed(&mut self, maps: Vec<MapEntry>) {
        self.maps = maps;
        let pages = self.maps.len().div_ceil(MAPS_PER_PAGE).max(1);
        self.maps_page = self.maps_page.min(pages - 1);
    }

    /// Карты, известные экрану.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn maps(&self) -> &[MapEntry] {
        &self.maps
    }

    /// Раскладка экрана карт.
    pub fn maps_layout(&self) -> MapsLayout {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let (lw, bw, h, gap) = (520.0 * u, 110.0 * u, 60.0 * u, 10.0 * u);
        let x0 = (vw - (lw + 2.0 * bw + 2.0 * gap)) / 2.0;
        let top = 120.0 * u;
        let rows = (0..MAPS_PER_PAGE)
            .map(|i| {
                let y = top + i as f32 * (h + 8.0 * u);
                [
                    [x0, y, lw, h],
                    [x0 + lw + gap, y, bw, h],
                    [x0 + lw + bw + 2.0 * gap, y, bw, h],
                ]
            })
            .collect();
        let (aw, ah, agap) = (200.0 * u, 52.0 * u, 12.0 * u);
        let ax = (vw - (4.0 * aw + 3.0 * agap)) / 2.0;
        let ay = vh - ah - 24.0 * u;
        let actions = [0, 1, 2, 3].map(|i| [ax + i as f32 * (aw + agap), ay, aw, ah]);
        MapsLayout { rows, actions }
    }

    /// Карта прочитана: открыть в редакторе или начать партию.
    pub fn map_loaded(&mut self, path: &str, purpose: MapPurpose, res: Result<MapDef, String>) {
        let def = match res {
            Ok(d) => d,
            Err(e) => {
                self.notify(format!("MAP NOT LOADED: {e}").to_uppercase());
                return;
            }
        };
        match purpose {
            MapPurpose::Edit => self.edit_map(&def, Some(path.to_owned())),
            MapPurpose::Play => self.play_map(&def),
        }
    }

    /// Начать партию на карте.
    pub fn play_map(&mut self, def: &MapDef) {
        match def.instantiate(&self.reg, self.seed) {
            Ok(w) => {
                if !w.buildings().iter().any(|(_, b)| b.tag == KindTag::Core) {
                    self.notify("THIS MAP HAS NO CORE: OPEN IT IN THE EDITOR");
                    return;
                }
                self.begin_play(w);
            }
            Err(e) => self.notify(format!("CANNOT START: {e}").to_uppercase()),
        }
    }

    pub(super) fn maps_screen(
        &mut self,
        ui: &mut Ui,
        atlas: &Atlas,
        table: &SpriteTable,
        hud: &mut Hud,
    ) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        let (orange, steel, titanium) = (
            rgba(palette::FORGE_ORANGE),
            rgba(palette::STEEL_WHITE),
            rgba(palette::TITANIUM),
        );
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, 0.8]);
        let t = "MAPS";
        hud.text(
            atlas,
            (vw - text_width(t, 5.0 * u)) / 2.0,
            40.0 * u,
            5.0 * u,
            t,
            orange,
        );
        let l = self.maps_layout();
        let pages = self.maps.len().div_ceil(MAPS_PER_PAGE).max(1);
        let sub = if self.maps.is_empty() {
            format!("NO MAPS IN {MAPS_DIR}/ YET: CREATE ONE WITH NEW MAP")
        } else {
            format!(
                "{} MAPS IN {MAPS_DIR}/   PAGE {} OF {pages}",
                self.maps.len(),
                self.maps_page + 1
            )
        };
        hud.text(
            atlas,
            (vw - text_width(&sub, 2.0 * u)) / 2.0,
            90.0 * u,
            2.0 * u,
            &sub,
            titanium,
        );
        let mut chosen: Option<(String, MapPurpose)> = None;
        let first = self.maps_page * MAPS_PER_PAGE;
        for (row, m) in l.rows.iter().zip(self.maps.iter().skip(first)) {
            let [label, play, edit] = *row;
            ui.panel(hud, atlas, white, label, [0.06, 0.065, 0.09, 0.97]);
            hud.text(
                atlas,
                label[0] + 12.0 * u,
                label[1] + 9.0 * u,
                3.0 * u,
                &m.name,
                steel,
            );
            let col = if m.ok { titanium } else { orange };
            hud.text(
                atlas,
                label[0] + 12.0 * u,
                label[1] + 38.0 * u,
                2.0 * u,
                &m.info,
                col,
            );
            if ui.button(hud, atlas, white, play, "PLAY", 2.0 * u, m.ok) {
                chosen = Some((m.path.clone(), MapPurpose::Play));
            }
            if ui.button(hud, atlas, white, edit, "EDIT", 2.0 * u, m.ok) {
                chosen = Some((m.path.clone(), MapPurpose::Edit));
            }
        }
        if let Some((path, purpose)) = chosen {
            self.requests.push(Request::LoadMap { path, purpose });
        }
        let [new_map, prev, next, back] = l.actions;
        if ui.button(hud, atlas, white, new_map, "NEW MAP", 3.0 * u, true) {
            self.open_editor();
        }
        if ui.button(hud, atlas, white, prev, "PREV", 3.0 * u, self.maps_page > 0) {
            self.maps_page -= 1;
        }
        if ui.button(
            hud,
            atlas,
            white,
            next,
            "NEXT",
            3.0 * u,
            self.maps_page + 1 < pages,
        ) {
            self.maps_page += 1;
        }
        if ui.button(hud, atlas, white, back, "BACK", 3.0 * u, true) {
            self.screen = Screen::MainMenu;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ferrustry_content::ContentRegistry;
    use ferrustry_sim::mapgen::{self, Difficulty, MapSettings, MapSize};
    use ferrustry_sim::scenario;

    fn game() -> Game {
        let reg = ContentRegistry::base().unwrap();
        let world = scenario::demo_map(&reg, 1).unwrap();
        Game::new(reg, world, [1280.0, 720.0])
    }

    fn small_map(g: &Game) -> MapDef {
        let w = mapgen::generate(
            &g.reg,
            &MapSettings {
                size: MapSize::Small,
                difficulty: Difficulty::Easy,
                seed: 4,
            },
        )
        .unwrap();
        MapDef::from_world(&w, &g.reg)
    }

    #[test]
    fn names_and_free_paths() {
        assert_eq!(map_name("maps/map-007.fmap"), "MAP-007");
        assert_eq!(map_name("C:\\games\\maps\\Island.fmap"), "ISLAND");
        assert_eq!(map_name(LEGACY_MAP_FILE), "FERRUSTRY-MAP");
        let taken = ["maps/map-001.fmap", "maps/map-002.fmap"];
        assert_eq!(next_map_path(|p| taken.contains(&p)), "maps/map-003.fmap");
        assert_eq!(next_map_path(|_| false), "maps/map-001.fmap");
    }

    #[test]
    fn list_play_and_edit() {
        let mut g = game();
        g.open_maps();
        assert_eq!(g.screen(), Screen::Maps);
        assert_eq!(g.take_requests(), vec![Request::ListMaps]);
        let def = small_map(&g);
        let broken: Result<MapDef, String> = Err("file is truncated".into());
        g.maps_listed(vec![
            MapEntry::new("maps/map-001.fmap", &Ok(def.clone())),
            MapEntry::new("maps/bad.fmap", &broken),
        ]);
        assert!(g.maps()[0].ok && g.maps()[0].info.contains("128X96"));
        assert!(!g.maps()[1].ok && g.maps()[1].info.contains("TRUNCATED"));

        g.map_loaded("maps/map-001.fmap", MapPurpose::Edit, Ok(def.clone()));
        assert_eq!(g.screen(), Screen::Editor);
        assert_eq!(g.editor.path.as_deref(), Some("maps/map-001.fmap"));

        g.map_loaded("maps/map-001.fmap", MapPurpose::Play, Ok(def.clone()));
        assert_eq!(g.screen(), Screen::Playing);
        // карта без ядра не запускается
        let mut no_core = def;
        no_core.buildings.clear();
        g.screen = Screen::Maps;
        g.play_map(&no_core);
        assert_eq!(g.screen(), Screen::Maps);
    }
}
