//! Цели секторов кампании: прогресс считается по состоянию мира — детерминированно и одинаково
//! у всех, кто считает этот мир.

use ferrustry_content::ContentRegistry;
use ferrustry_content::campaign::{Objective, Sector, SectorDifficulty, SectorSize};

use crate::mapgen::{self, Difficulty, MapSettings, MapSize};
use crate::scenario::ScenarioError;
use crate::world::World;

/// Прогресс одной цели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Сделано.
    pub current: u64,
    /// Нужно.
    pub target: u64,
    /// Цель выполнена.
    pub done: bool,
}

/// Пережито волн: пришедшие волны, кроме текущей, пока её враги живы.
pub fn waves_survived(world: &World, reg: &ContentRegistry) -> u64 {
    let arrived = world
        .wave_info(reg)
        .map_or(0, |(next, _)| u64::from(next.saturating_sub(1)));
    if world.units().next().is_some() {
        arrived.saturating_sub(1)
    } else {
        arrived
    }
}

/// Прогресс цели.
pub fn progress(world: &World, reg: &ContentRegistry, obj: &Objective) -> Progress {
    let (current, target) = match *obj {
        Objective::SurviveWaves(n) => (waves_survived(world, reg), u64::from(n)),
        Objective::Make { item, amount } => (
            world
                .stats()
                .made
                .get(usize::from(item.0))
                .copied()
                .unwrap_or(0),
            amount,
        ),
        Objective::Repair(n) => (world.stats().repaired, u64::from(n)),
    };
    Progress {
        current: current.min(target),
        target,
        done: current >= target,
    }
}

/// Все цели сектора выполнены (и ядро цело).
pub fn sector_won(world: &World, reg: &ContentRegistry, sector: &Sector) -> bool {
    world.outcome() == crate::combat::Outcome::Running
        && sector
            .objectives
            .iter()
            .all(|o| progress(world, reg, o).done)
}

/// Настройки генератора для карты сектора.
pub fn map_settings(sector: &Sector) -> MapSettings {
    MapSettings {
        size: match sector.map.size {
            SectorSize::Small => MapSize::Small,
            SectorSize::Medium => MapSize::Medium,
            SectorSize::Large => MapSize::Large,
        },
        difficulty: match sector.map.difficulty {
            SectorDifficulty::Easy => Difficulty::Easy,
            SectorDifficulty::Normal => Difficulty::Normal,
            SectorDifficulty::Hard => Difficulty::Hard,
        },
        seed: sector.map.seed,
    }
}

/// Мир сектора.
pub fn sector_world(reg: &ContentRegistry, sector: &Sector) -> Result<World, ScenarioError> {
    mapgen::generate(reg, &map_settings(sector))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::KindTag;
    use crate::geom::{Dir, TilePos};
    use crate::world::Command;
    use ferrustry_content::campaign::Campaign;

    #[test]
    fn every_sector_is_playable() {
        let reg = ContentRegistry::base().unwrap();
        let c = Campaign::base(&reg).unwrap();
        for s in &c.sectors {
            let w = sector_world(&reg, s).unwrap();
            w.validate(&reg).unwrap();
            assert!(w.rules().waves, "{}", s.id);
            // целей «отремонтировать N» не больше, чем руин на карте
            let ruins = w
                .buildings()
                .iter()
                .filter(|(_, b)| b.tag == KindTag::Ruin)
                .count() as u64;
            for o in &s.objectives {
                if let Objective::Repair(n) = o {
                    assert!(
                        ruins >= u64::from(*n),
                        "{}: {ruins} ruins for Repair({n})",
                        s.id
                    );
                }
                assert!(!progress(&w, &reg, o).done, "{}: done at start", s.id);
            }
        }
    }

    #[test]
    fn make_counts_crafting_not_refunds_and_repair_counts_repairs() {
        let reg = ContentRegistry::base().unwrap();
        let c = Campaign::base(&reg).unwrap();
        let mut w = sector_world(&reg, &c.sectors[1]).unwrap();
        let ingot = reg.item_by_name("iron_ingot").unwrap();
        let make = Objective::Make {
            item: ingot,
            amount: 10,
        };
        // строим и сносим плавильню: возврат слитков — не «сделано»
        let smelter = reg.block_by_name("smelter").unwrap();
        let (cx, cy) = (w.grid().width() as i32 / 2, w.grid().height() as i32 / 2);
        let at = TilePos::new(cx + 4, cy);
        w.apply(
            &reg,
            &Command::Place {
                block: smelter,
                origin: at,
                rot: Dir::Right,
            },
        )
        .unwrap();
        w.apply(&reg, &Command::Remove { at }).unwrap();
        assert_eq!(progress(&w, &reg, &make).current, 0);
        // ремонт засчитывается
        let ruin = w
            .buildings()
            .iter()
            .find(|(_, b)| b.tag == KindTag::Ruin)
            .map(|(_, b)| b.origin)
            .unwrap();
        w.apply(&reg, &Command::Repair { at: ruin }).unwrap();
        let p = progress(&w, &reg, &Objective::Repair(3));
        assert_eq!((p.current, p.target, p.done), (1, 3, false));
    }

    #[test]
    fn surviving_waves_counts_cleared_and_passed_waves() {
        let reg = ContentRegistry::base().unwrap();
        let c = Campaign::base(&reg).unwrap();
        let _ = c;
        // маленький мир: ядро, точка появления, волны каждую секунду
        let basalt = reg.floor_by_name("basalt").unwrap();
        let mut w = World::new(&reg, 40, 40, basalt, 5);
        w.apply(
            &reg,
            &Command::Place {
                block: reg.block_by_name("core").unwrap(),
                origin: TilePos::new(30, 18),
                rot: Dir::Right,
            },
        )
        .unwrap();
        w.set_spawns(vec![TilePos::new(2, 20)]);
        w.set_rules(crate::world::Rules {
            infinite_resources: false,
            waves: true,
            first_wave: 30,
            wave_spacing: 30,
        });
        w.prepare_flow(&reg);
        assert_eq!(waves_survived(&w, &reg), 0);
        // идёт время: пришедшие волны засчитываются, когда враги перебиты или пришла следующая
        let mut last = 0;
        for _ in 0..200 {
            w.step(&reg);
            let s = waves_survived(&w, &reg);
            assert!(s >= last, "never goes down");
            last = s;
        }
        assert!(last >= 1, "{last}");
    }
}
