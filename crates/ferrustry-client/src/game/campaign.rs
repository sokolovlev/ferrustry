//! Кампания (этап 7): экран секторов, запуск сектора, цели в углу экрана, победа и открытия.
//! Прогресс — число пройденных по порядку секторов; хранит его окружение (файл), а здесь —
//! только логика и интерфейс.

use ferrustry_content::BlockId;
use ferrustry_content::campaign::{Campaign, Objective, Sector};
use ferrustry_render::atlas::palette;
use ferrustry_render::{Atlas, Hud, SpriteTable, rgba};
use ferrustry_sim::campaign::{progress, sector_won, sector_world};

use super::{CATEGORIES, Game, Request, Screen};
use crate::ui::{Rect, Ui, text_width};

/// Файл прогресса кампании.
pub const CAMPAIGN_FILE: &str = "ferrustry-campaign.txt";

/// Идущий сектор.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mission {
    /// Номер сектора.
    pub index: usize,
    /// Цели выполнены.
    pub won: bool,
    /// Показан экран победы.
    pub victory_open: bool,
}

/// Раскладка экрана кампании.
#[derive(Debug, Clone)]
pub struct CampaignLayout {
    /// Строки секторов: панель и кнопка PLAY.
    pub rows: Vec<(Rect, Rect)>,
    /// BACK.
    pub back: Rect,
}

/// Описание цели для интерфейса.
fn objective_name(g: &Game, o: &Objective) -> String {
    match *o {
        Objective::SurviveWaves(_) => "SURVIVE WAVES".into(),
        Objective::Make { item, .. } => format!("MAKE {}", g.item_name(item)),
        Objective::Repair(_) => "REPAIR RUINS".into(),
    }
}

fn objective_target(o: &Objective) -> u64 {
    match *o {
        Objective::SurviveWaves(n) | Objective::Repair(n) => u64::from(n),
        Objective::Make { amount, .. } => amount,
    }
}

impl Game {
    /// Файл прогресса кампании.
    pub fn campaign_file() -> &'static str {
        CAMPAIGN_FILE
    }

    /// Сколько секторов пройдено (из файла прогресса).
    pub fn set_campaign_progress(&mut self, done: usize) {
        self.campaign_done = done;
    }

    /// Кампания загружена.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn campaign(&self) -> Option<&Campaign> {
        self.campaign.as_ref()
    }

    /// Ограничить панель построек открытыми (`None` — все).
    pub(super) fn set_unlocked(&mut self, allowed: Option<&[BlockId]>) {
        self.categories = CATEGORIES
            .iter()
            .map(|(name, blocks)| {
                (
                    *name,
                    blocks
                        .iter()
                        .filter_map(|n| self.reg.block_by_name(n))
                        .filter(|b| allowed.is_none_or(|a| a.contains(b)))
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|(_, v)| !v.is_empty())
            .collect();
        self.category = 0;
        self.hotbar = self
            .categories
            .first()
            .map(|c| c.1.clone())
            .unwrap_or_default();
        self.tool = None;
    }

    /// Открыть экран кампании.
    pub fn open_campaign(&mut self) {
        if self.campaign.is_none() {
            self.notify("CAMPAIGN DATA IS MISSING");
            return;
        }
        self.screen = Screen::Campaign;
    }

    /// Раскладка экрана кампании.
    pub fn campaign_layout(&self) -> CampaignLayout {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let n = self.campaign.as_ref().map_or(0, |c| c.sectors.len());
        let (w, h, gap) = (820.0 * u, 104.0 * u, 12.0 * u);
        let x = (vw - w) / 2.0;
        let top = 110.0 * u;
        let rows = (0..n)
            .map(|i| {
                let y = top + i as f32 * (h + gap);
                (
                    [x, y, w, h],
                    [
                        x + w - 150.0 * u,
                        y + (h - 44.0 * u) / 2.0,
                        136.0 * u,
                        44.0 * u,
                    ],
                )
            })
            .collect();
        let back = [
            (vw - 340.0 * u) / 2.0,
            vh - 52.0 * u - 24.0 * u,
            340.0 * u,
            52.0 * u,
        ];
        CampaignLayout { rows, back }
    }

    /// Начать сектор `index` (если он открыт).
    pub fn start_sector(&mut self, index: usize) {
        let Some(sector) = self
            .campaign
            .as_ref()
            .and_then(|c| c.sectors.get(index))
            .cloned()
        else {
            return;
        };
        if index > self.campaign_done {
            self.notify("FINISH THE PREVIOUS SECTOR FIRST");
            return;
        }
        match sector_world(&self.reg, &sector) {
            Ok(w) => {
                self.begin_play(w);
                let unlocked = self
                    .campaign
                    .as_ref()
                    .map(|c| c.unlocked(index))
                    .unwrap_or_default();
                self.set_unlocked(Some(&unlocked));
                self.mission = Some(Mission {
                    index,
                    won: false,
                    victory_open: false,
                });
                self.message = Some((sector.brief.clone(), 8.0));
            }
            Err(e) => self.notify(format!("CANNOT CREATE SECTOR: {e}").to_uppercase()),
        }
    }

    fn sector(&self) -> Option<&Sector> {
        let m = self.mission?;
        self.campaign.as_ref()?.sectors.get(m.index)
    }

    /// После тиков: проверить победу в секторе.
    pub(super) fn campaign_tick(&mut self) {
        let Some(m) = self.mission else { return };
        if m.won {
            return;
        }
        let Some(sector) = self.sector() else { return };
        if sector_won(&self.world, &self.reg, sector) {
            let done = self.campaign_done.max(m.index + 1);
            self.campaign_done = done;
            self.requests.push(Request::SaveCampaign(done));
            self.paused = true;
            self.mission = Some(Mission {
                won: true,
                victory_open: true,
                ..m
            });
        }
    }

    /// Цели сектора в левом нижнем углу.
    pub(super) fn objectives_panel(
        &mut self,
        ui: &mut Ui,
        atlas: &Atlas,
        table: &SpriteTable,
        hud: &mut Hud,
    ) {
        let Some(sector) = self.sector().cloned() else {
            return;
        };
        let u = self.u();
        let vh = self.camera.viewport[1];
        let row = 18.0 * u;
        let lines: Vec<(String, bool)> = sector
            .objectives
            .iter()
            .map(|o| {
                let p = progress(&self.world, &self.reg, o);
                let mark = if p.done { "[X]" } else { "[ ]" };
                (
                    format!(
                        "{mark} {} {}/{}",
                        objective_name(self, o),
                        p.current,
                        objective_target(o)
                    ),
                    p.done,
                )
            })
            .collect();
        let h = 30.0 * u + lines.len() as f32 * row;
        let r = [8.0 * u, vh - 40.0 * u - h, 330.0 * u, h];
        ui.panel(hud, atlas, table.white, r, [0.02, 0.025, 0.04, 0.93]);
        hud.text(
            atlas,
            r[0] + 10.0 * u,
            r[1] + 8.0 * u,
            2.0 * u,
            &sector.title,
            rgba(palette::FORGE_ORANGE),
        );
        for (i, (l, done)) in lines.iter().enumerate() {
            let c = if *done {
                rgba(palette::MOLTEN_GOLD)
            } else {
                rgba(palette::STEEL_WHITE)
            };
            hud.text(
                atlas,
                r[0] + 10.0 * u,
                r[1] + 28.0 * u + i as f32 * row,
                2.0 * u,
                l,
                c,
            );
        }
    }

    /// Прямоугольники кнопок экрана победы: к кампании, играть дальше.
    pub fn victory_buttons(&self) -> [Rect; 2] {
        let r = self.menu_rects(2, self.camera.viewport[1] * 0.55);
        [r[0], r[1]]
    }

    /// Экран победы в секторе.
    pub(super) fn victory_screen(
        &mut self,
        ui: &mut Ui,
        atlas: &Atlas,
        table: &SpriteTable,
        hud: &mut Hud,
    ) {
        let Some(m) = self.mission.filter(|m| m.victory_open) else {
            return;
        };
        let Some(sector) = self.sector().cloned() else {
            return;
        };
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.03, 0.02, 0.7]);
        let t = "SECTOR SECURED";
        hud.text(
            atlas,
            (vw - text_width(t, 6.0 * u)) / 2.0,
            vh * 0.25,
            6.0 * u,
            t,
            rgba(palette::MOLTEN_GOLD),
        );
        let names: Vec<String> = sector
            .unlocks
            .iter()
            .map(|b| self.reg.block(*b).name.replace('_', " ").to_uppercase())
            .collect();
        let sub = if names.is_empty() {
            "CAMPAIGN COMPLETE".to_owned()
        } else {
            format!("UNLOCKED: {}", names.join(", "))
        };
        hud.text(
            atlas,
            (vw - text_width(&sub, 2.0 * u)) / 2.0,
            vh * 0.25 + 70.0 * u,
            2.0 * u,
            &sub,
            rgba(palette::STEEL_WHITE),
        );
        let [to_campaign, keep] = self.victory_buttons();
        if ui.button(hud, atlas, white, to_campaign, "CAMPAIGN", 3.0 * u, true) {
            self.mission = None;
            self.set_unlocked(None);
            self.screen = Screen::Campaign;
        }
        if ui.button(hud, atlas, white, keep, "KEEP PLAYING", 3.0 * u, true) {
            self.mission = Some(Mission {
                victory_open: false,
                ..m
            });
            self.paused = false;
        }
    }

    /// Экран выбора сектора.
    pub(super) fn campaign_screen(
        &mut self,
        ui: &mut Ui,
        atlas: &Atlas,
        table: &SpriteTable,
        hud: &mut Hud,
    ) {
        let Some(c) = self.campaign.clone() else {
            self.screen = Screen::MainMenu;
            return;
        };
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        let (orange, steel, titanium, gold) = (
            rgba(palette::FORGE_ORANGE),
            rgba(palette::STEEL_WHITE),
            rgba(palette::TITANIUM),
            rgba(palette::MOLTEN_GOLD),
        );
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, 0.9]);
        let t = "CAMPAIGN";
        hud.text(
            atlas,
            (vw - text_width(t, 5.0 * u)) / 2.0,
            40.0 * u,
            5.0 * u,
            t,
            orange,
        );
        let l = self.campaign_layout();
        let mut start = None;
        for (i, (s, (panel, play))) in c.sectors.iter().zip(&l.rows).enumerate() {
            let open = i <= self.campaign_done;
            let done = i < self.campaign_done;
            ui.panel(hud, atlas, white, *panel, [0.06, 0.065, 0.09, 0.97]);
            let (x, y) = (panel[0] + 14.0 * u, panel[1] + 10.0 * u);
            let status = if done {
                "SECURED"
            } else if open {
                "AVAILABLE"
            } else {
                "LOCKED"
            };
            hud.text(
                atlas,
                x,
                y,
                3.0 * u,
                &format!("{}. {}", i + 1, s.title),
                if open { steel } else { titanium },
            );
            hud.text(
                atlas,
                panel[0] + panel[2] - 160.0 * u - text_width(status, 2.0 * u),
                y + 4.0 * u,
                2.0 * u,
                status,
                if done { gold } else { titanium },
            );
            hud.text(atlas, x, y + 30.0 * u, 2.0 * u, &s.brief, titanium);
            let goals: Vec<String> = s
                .objectives
                .iter()
                .map(|o| format!("{} {}", objective_name(self, o), objective_target(o)))
                .collect();
            hud.text(
                atlas,
                x,
                y + 52.0 * u,
                2.0 * u,
                &format!("GOALS: {}", goals.join(", ")),
                steel,
            );
            let label = if done { "REPLAY" } else { "PLAY" };
            if ui.button(hud, atlas, white, *play, label, 2.0 * u, open) {
                start = Some(i);
            }
        }
        if let Some(i) = start {
            self.start_sector(i);
        }
        if ui.button(hud, atlas, white, l.back, "BACK", 3.0 * u, true) {
            self.screen = Screen::MainMenu;
        }
    }
}
