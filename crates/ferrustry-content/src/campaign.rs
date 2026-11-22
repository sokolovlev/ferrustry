//! Кампания (этап 7, вариант А): цепочка секторов. У сектора — карта (настройки генератора),
//! цели и постройки, которые открывает победа. Данные — `data/campaign.ron`, имена контента
//! проверяются при загрузке.

use std::collections::HashSet;

use serde::Deserialize;

use crate::{BlockId, ContentError, ContentRegistry, ItemId};

/// Кампания базовой игры.
pub const BASE_CAMPAIGN: &str = include_str!("../data/campaign.ron");

/// Размер карты сектора.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum SectorSize {
    /// Малая.
    Small,
    /// Средняя.
    Medium,
    /// Большая.
    Large,
}

/// Сложность сектора (темп волн).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum SectorDifficulty {
    /// Лёгкая.
    Easy,
    /// Обычная.
    Normal,
    /// Сложная.
    Hard,
}

/// Карта сектора: настройки генератора.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorMap {
    /// Размер.
    pub size: SectorSize,
    /// Сложность.
    pub difficulty: SectorDifficulty,
    /// Seed генератора.
    pub seed: u64,
}

/// Цель сектора.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    /// Пережить столько волн.
    SurviveWaves(u32),
    /// Добыть или сделать рецептами столько предмета.
    Make {
        /// Предмет.
        item: ItemId,
        /// Сколько.
        amount: u64,
    },
    /// Отремонтировать столько руин.
    Repair(u32),
}

/// Сектор.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sector {
    /// Идентификатор (для прогресса).
    pub id: String,
    /// Название (интерфейс).
    pub title: String,
    /// Задача одной строкой (интерфейс).
    pub brief: String,
    /// Карта.
    pub map: SectorMap,
    /// Цели (все обязательны).
    pub objectives: Vec<Objective>,
    /// Постройки, которые открывает победа.
    pub unlocks: Vec<BlockId>,
}

/// Кампания.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Campaign {
    /// Постройки, доступные с начала.
    pub starting: Vec<BlockId>,
    /// Сектора по порядку.
    pub sectors: Vec<Sector>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
enum RawObjective {
    SurviveWaves(u32),
    Make { item: String, amount: u64 },
    Repair(u32),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSector {
    id: String,
    title: String,
    brief: String,
    map: SectorMap,
    objectives: Vec<RawObjective>,
    #[serde(default)]
    unlocks: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCampaign {
    starting: Vec<String>,
    sectors: Vec<RawSector>,
}

fn invalid(path: String, reason: &str) -> ContentError {
    ContentError::Invalid {
        path,
        reason: reason.into(),
    }
}

impl Campaign {
    /// Кампания базовой игры.
    pub fn base(reg: &ContentRegistry) -> Result<Self, ContentError> {
        Self::from_ron(reg, BASE_CAMPAIGN)
    }

    /// Загрузить и проверить кампанию: имена контента, непустые цели, уникальные id.
    pub fn from_ron(reg: &ContentRegistry, text: &str) -> Result<Self, ContentError> {
        let raw: RawCampaign = ron::from_str(text)?;
        let block = |name: &str, path: String| {
            reg.block_by_name(name)
                .filter(|b| !matches!(reg.block(*b).kind, crate::BlockKind::Ruin { .. }))
                .ok_or_else(|| invalid(path, &format!("unknown or unbuildable block `{name}`")))
        };
        let starting = raw
            .starting
            .iter()
            .enumerate()
            .map(|(i, n)| block(n, format!("campaign.starting[{i}]")))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ids = HashSet::new();
        let mut sectors = Vec::with_capacity(raw.sectors.len());
        for (i, s) in raw.sectors.into_iter().enumerate() {
            let p = |f: &str| format!("campaign.sectors[{i}] `{}`.{f}", s.id);
            if !ids.insert(s.id.clone()) {
                return Err(invalid(p("id"), "duplicate sector id"));
            }
            if s.objectives.is_empty() {
                return Err(invalid(
                    p("objectives"),
                    "a sector needs at least one objective",
                ));
            }
            let mut objectives = Vec::with_capacity(s.objectives.len());
            for (k, o) in s.objectives.iter().enumerate() {
                let path = p(&format!("objectives[{k}]"));
                objectives.push(match o {
                    RawObjective::SurviveWaves(0) | RawObjective::Repair(0) => {
                        return Err(invalid(path, "must be at least 1"));
                    }
                    RawObjective::Make { amount: 0, .. } => {
                        return Err(invalid(path, "amount must be at least 1"));
                    }
                    RawObjective::SurviveWaves(n) => Objective::SurviveWaves(*n),
                    RawObjective::Repair(n) => Objective::Repair(*n),
                    RawObjective::Make { item, amount } => Objective::Make {
                        item: reg
                            .item_by_name(item)
                            .ok_or_else(|| invalid(path, &format!("unknown item `{item}`")))?,
                        amount: *amount,
                    },
                });
            }
            let unlocks = s
                .unlocks
                .iter()
                .enumerate()
                .map(|(k, n)| block(n, p(&format!("unlocks[{k}]"))))
                .collect::<Result<Vec<_>, _>>()?;
            sectors.push(Sector {
                id: s.id,
                title: s.title,
                brief: s.brief,
                map: s.map,
                objectives,
                unlocks,
            });
        }
        Ok(Self { starting, sectors })
    }

    /// Постройки, доступные после победы в первых `completed` секторах.
    pub fn unlocked(&self, completed: usize) -> Vec<BlockId> {
        let mut v = self.starting.clone();
        for s in self.sectors.iter().take(completed) {
            for b in &s.unlocks {
                if !v.contains(b) {
                    v.push(*b);
                }
            }
        }
        v
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::BlockKind;

    #[test]
    fn base_campaign_loads_and_unlocks_everything_buildable() {
        let reg = ContentRegistry::base().unwrap();
        let c = Campaign::base(&reg).unwrap();
        assert!(c.sectors.len() >= 3);
        let all = c.unlocked(c.sectors.len());
        for (i, b) in reg.blocks().iter().enumerate() {
            let id = BlockId(i as u16);
            let skip = matches!(b.kind, BlockKind::Ruin { .. } | BlockKind::Core { .. });
            assert!(
                skip || all.contains(&id),
                "block `{}` is never unlocked by the campaign",
                b.name
            );
        }
        // тексты влезают в строку экрана кампании (шрифт 6 px на символ при масштабе 2)
        for s in &c.sectors {
            assert!(s.brief.len() <= 52, "brief of `{}` is too long", s.id);
            assert!(s.title.len() <= 30, "title of `{}` is too long", s.id);
        }
        // открытия по шагам: всё начальное — сразу, остальное — после побед
        assert_eq!(c.unlocked(0), c.starting);
        assert!(c.unlocked(1).len() > c.unlocked(0).len());
    }

    #[test]
    fn bad_campaigns_are_rejected_with_a_path() {
        let reg = ContentRegistry::base().unwrap();
        let bad = |from: &str, to: &str| {
            let text = BASE_CAMPAIGN.replacen(from, to, 1);
            assert_ne!(text, BASE_CAMPAIGN, "{from}");
            Campaign::from_ron(&reg, &text).unwrap_err().to_string()
        };
        assert!(bad("\"steel_forge\"", "\"no_such\"").contains("no_such"));
        assert!(bad("\"conveyor\"", "\"ruined_bore\"").contains("ruined_bore"));
        assert!(bad("SurviveWaves(3)", "SurviveWaves(0)").contains("at least 1"));
        assert!(bad("\"iron_ingot\"", "\"gold\"").contains("gold"));
        assert!(bad("id: \"foundry\"", "id: \"landing\"").contains("duplicate"));
    }
}
