//! Инструменты баланса: настоящие прогоны симуляции, из которых складывается кривая сложности.
//!
//! - [`defense_trial`] — волна номер N (состав из данных, настоящие точки появления и разброс)
//!   против K турелей вокруг ядра; боеприпасы пополняются каждый тик, расход считается.
//! - [`min_defense`] — наименьшее K, при котором ядро не получает урона.
//! - [`wave_arrival`] — когда придёт волна N при заданных правилах и постоянном шуме.
//! - [`economy_cell`] — производственная ячейка мегабазы (2 бура → плавильня): выход слитков,
//!   шум, стоимость.
//!
//! Всё детерминировано: один и тот же контент даёт один и тот же отчёт.

use ferrustry_content::{BlockId, BlockKind, ContentRegistry};

use crate::buildings::KindTag;
use crate::geom::{Dir, TilePos};
use crate::scenario::{self, BaseIds, ScenarioError};
use crate::world::{Command, Rules, TICKS_PER_SECOND, World};

/// Чем обороняться в испытании.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defense {
    /// Пушки (боеприпасы — железные слитки).
    Guns,
    /// Дуговые турели, каждая со своими ветряками (полное питание).
    Arcs,
}

/// Итог испытания обороны.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trial {
    /// Ядро не получило урона и все враги уничтожены.
    pub held: bool,
    /// Ядро разрушено.
    pub lost: bool,
    /// Урон ядру.
    pub core_damage: u32,
    /// Израсходовано боеприпасов, предметов (у дуговых турелей — 0).
    pub ammo_items: u32,
    /// Врагов в волне.
    pub enemies: u32,
    /// Тиков до конца испытания.
    pub ticks: u32,
    /// Сколько турелей удалось поставить.
    pub placed: u32,
}

const ARENA: i32 = 64;
const CORE_AT: i32 = 30;
const MAX_TICKS: u32 = 5000;

fn block(reg: &ContentRegistry, name: &'static str) -> Result<BlockId, ScenarioError> {
    reg.block_by_name(name)
        .ok_or(ScenarioError::MissingContent(name))
}

/// Места под турели: квадратные кольца вокруг ядра изнутри наружу.
fn ring_spots() -> Vec<TilePos> {
    let c = CORE_AT + 1;
    let mut out = Vec::new();
    let mut r = 4;
    while r < ARENA / 2 - 4 {
        for k in (-r..r).step_by(3) {
            for (x, y) in [
                (c + k, c - r),
                (c + r, c + k),
                (c - k, c + r),
                (c - r, c - k),
            ] {
                out.push(TilePos::new(x, y));
            }
        }
        r += 4;
    }
    out
}

/// Сколько раз повторять испытание с разным разбросом появления врагов.
pub const SEEDS: u64 = 3;

/// Испытание: волна `wave` против `count` турелей вида `defense` — на [`SEEDS`] вариантах
/// разброса появления. Держит, только если держит во всех; расход и урон — наибольшие.
pub fn defense_trial(
    reg: &ContentRegistry,
    wave: u32,
    defense: Defense,
    count: u32,
) -> Result<Trial, ScenarioError> {
    let mut out: Option<Trial> = None;
    for seed in 0..SEEDS {
        let t = trial_once(reg, wave, defense, count, u64::from(wave) * 1000 + seed)?;
        out = Some(match out {
            None => t,
            Some(o) => Trial {
                held: o.held && t.held,
                lost: o.lost || t.lost,
                core_damage: o.core_damage.max(t.core_damage),
                ammo_items: o.ammo_items.max(t.ammo_items),
                enemies: o.enemies.max(t.enemies),
                ticks: o.ticks.max(t.ticks),
                placed: o.placed.min(t.placed),
            },
        });
    }
    out.ok_or(ScenarioError::MissingContent("seeds"))
}

fn trial_once(
    reg: &ContentRegistry,
    wave: u32,
    defense: Defense,
    count: u32,
    seed: u64,
) -> Result<Trial, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let mut w = World::new(reg, ARENA as u32, ARENA as u32, ids.basalt, seed);
    scenario::build(
        &mut w,
        reg,
        &[Command::Place {
            block: ids.core,
            origin: TilePos::new(CORE_AT, CORE_AT),
            rot: Dir::Right,
        }],
    )?;
    let (gun, arc) = (block(reg, "gun_turret")?, block(reg, "arc_tower")?);
    let mut placed = 0;
    for p in ring_spots() {
        if placed >= count {
            break;
        }
        let ok = match defense {
            Defense::Guns => w
                .apply(
                    reg,
                    &Command::Place {
                        block: gun,
                        origin: p,
                        rot: Dir::Right,
                    },
                )
                .is_ok(),
            Defense::Arcs => {
                // турель и 4 ветряка (8 энергии — полное питание) со стороны ядра: снаружи
                // ветряки стали бы целью и препятствием вне досягаемости турели
                let c = CORE_AT + 1;
                let (dx, dy) = (p.x - c, p.y - c);
                let turbines: Vec<TilePos> = if dx.abs() >= dy.abs() {
                    let x = if dx > 0 { p.x - 1 } else { p.x + 2 };
                    (-1..3).map(|k| TilePos::new(x, p.y + k)).collect()
                } else {
                    let y = if dy > 0 { p.y - 1 } else { p.y + 2 };
                    (-1..3).map(|k| TilePos::new(p.x + k, y)).collect()
                };
                let fits = w.can_place(reg, arc, p).is_ok()
                    && turbines
                        .iter()
                        .all(|&t| w.can_place(reg, ids.wind_turbine, t).is_ok());
                if fits {
                    let mut cmds = vec![Command::Place {
                        block: arc,
                        origin: p,
                        rot: Dir::Right,
                    }];
                    cmds.extend(turbines.iter().map(|&t| Command::Place {
                        block: ids.wind_turbine,
                        origin: t,
                        rot: Dir::Right,
                    }));
                    scenario::build(&mut w, reg, &cmds)?;
                }
                fits
            }
        };
        placed += u32::from(ok);
    }
    // три точки появления, как на средней карте; волна N — сразу
    let e = ARENA - 2;
    w.set_spawns(vec![
        TilePos::new(1, ARENA / 2),
        TilePos::new(e, ARENA / 2),
        TilePos::new(ARENA / 2, e),
    ]);
    w.set_rules(Rules {
        waves: true,
        first_wave: 0,
        wave_spacing: u64::MAX / 4000,
        ..*w.rules()
    });
    w.waves.next_number = wave;
    w.waves.remaining = 0;
    w.debug_fill_turrets(reg);
    w.prepare_flow(reg);
    let core_full = core_health(&w).unwrap_or(0);
    let (mut shots, mut per_item) = (0u64, 1u32);
    let mut ticks = 0;
    let mut enemies = 0;
    while ticks < MAX_TICKS {
        w.step(reg);
        ticks += 1;
        enemies = enemies.max(w.units().count() as u32);
        // пополнить боеприпасы (как бесперебойная лента) и учесть расход
        for (row, owner) in w.turrets.rows.iter_mut().zip(&w.turrets.owners) {
            let Some(b) = w.buildings.get(*owner) else {
                continue;
            };
            if let BlockKind::Turret {
                shots_per_item,
                buffer,
                ..
            } = reg.block(b.block).kind
            {
                let full = shots_per_item * buffer;
                shots += u64::from(full - row.ammo.min(full));
                row.ammo = full;
                per_item = shots_per_item;
            }
        }
        if w.units().count() == 0 && enemies > 0 {
            break;
        }
        if core_health(&w).is_none() {
            break;
        }
    }
    let health = core_health(&w);
    let core_damage = core_full - health.unwrap_or(0);
    Ok(Trial {
        held: health.is_some() && core_damage == 0 && w.units().count() == 0,
        lost: health.is_none(),
        core_damage,
        ammo_items: u32::try_from(shots.div_ceil(u64::from(per_item.max(1)))).unwrap_or(u32::MAX),
        enemies,
        ticks,
        placed,
    })
}

fn core_health(w: &World) -> Option<u32> {
    w.buildings()
        .iter()
        .find(|(_, b)| b.tag == KindTag::Core)
        .map(|(_, b)| b.health())
}

/// Наименьшее число турелей (до `max`), которое удерживает волну без урона ядру, и итог
/// этого испытания. Двоичный поиск с проверкой соседа снизу.
pub fn min_defense(
    reg: &ContentRegistry,
    wave: u32,
    defense: Defense,
    max: u32,
) -> Result<Option<(u32, Trial)>, ScenarioError> {
    let top = defense_trial(reg, wave, defense, max)?;
    if !top.held {
        return Ok(None);
    }
    let (mut lo, mut hi, mut best) = (0u32, top.placed.min(max), top);
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        let t = defense_trial(reg, wave, defense, mid)?;
        if t.held {
            hi = mid;
            best = t;
        } else {
            lo = mid;
        }
    }
    Ok(Some((hi, best)))
}

/// Тик прихода волны `wave` (с 1) при правилах `rules` и постоянном шуме `noise`.
pub fn wave_arrival(reg: &ContentRegistry, rules: &Rules, wave: u32, noise: u32) -> u64 {
    let quiet = rules.first_wave + u64::from(wave.saturating_sub(1)) * rules.wave_spacing;
    quiet * 1000 / u64::from(reg.noise_rule().timer_rate(noise))
}

/// Производственная ячейка мегабазы B3: 2 бура на руде → плавильня на 2 ветряках → лента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// Слитков в ядро за 10 минут (установившийся режим).
    pub ingots_per_10min: u32,
    /// Шум ячейки за тик.
    pub noise: u32,
    /// Стоимость постройки ячейки в слитках (с её долей общей шины).
    pub cost: u32,
}

/// Измерить ячейку: ряд из 12 ячеек, прогрев 6000 тиков, затем 10 минут.
pub fn economy_cell(reg: &ContentRegistry) -> Result<Cell, ScenarioError> {
    const CELLS: u32 = 12;
    let ids = BaseIds::resolve(reg)?;
    let mut w = scenario::bench_megabase(reg, 240)?;
    for _ in 0..6000 {
        w.step(reg);
    }
    let ingot = usize::from(ids.iron_ingot.0);
    let before = w.total_core_items()[ingot];
    let ten_min = 10 * 60 * u64::from(TICKS_PER_SECOND);
    let mut noise = 0u64;
    for _ in 0..ten_min {
        w.step(reg);
        noise += u64::from(w.noise());
    }
    let made = w.total_core_items()[ingot] - before;
    let cost: u32 = w
        .buildings()
        .iter()
        .filter(|(_, b)| b.tag != KindTag::Core)
        .flat_map(|(_, b)| reg.block(b.block).cost.iter())
        .filter(|s| s.item == ids.iron_ingot)
        .map(|s| s.count)
        .sum();
    Ok(Cell {
        ingots_per_10min: u32::try_from(made / u64::from(CELLS)).unwrap_or(u32::MAX),
        noise: u32::try_from(noise / ten_min / u64::from(CELLS)).unwrap_or(u32::MAX),
        cost: cost / CELLS,
    })
}

/// Наименьшее число ячеек (до `max`), которое к приходу волны `wave` окупает `needed` слитков
/// сверх стоимости самих ячеек — с учётом того, что шум ячеек ускоряет волны.
pub fn cells_needed(
    reg: &ContentRegistry,
    rules: &Rules,
    cell: &Cell,
    wave: u32,
    needed: u64,
    max: u32,
) -> Option<u32> {
    (1..=max).find(|&c| {
        let t = wave_arrival(reg, rules, wave, c * cell.noise);
        let minutes_x10 = t * 10 / (60 * u64::from(TICKS_PER_SECOND));
        let made = u64::from(c) * u64::from(cell.ingots_per_10min) * minutes_x10 / 100;
        made >= needed + u64::from(c) * u64::from(cell.cost)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn more_guns_never_hurt_the_first_wave() {
        let reg = ContentRegistry::base().unwrap();
        let none = defense_trial(&reg, 1, Defense::Guns, 0).unwrap();
        assert!(!none.held, "no defence: the core takes damage");
        let (k, t) = min_defense(&reg, 1, Defense::Guns, 12).unwrap().unwrap();
        assert!(t.held && k >= 1);
        assert!(defense_trial(&reg, 1, Defense::Guns, k + 2).unwrap().held);
    }

    #[test]
    fn quiet_arrival_matches_the_rules() {
        let reg = ContentRegistry::base().unwrap();
        let rules = Rules {
            waves: true,
            first_wave: 9000,
            wave_spacing: 3600,
            ..Rules::default()
        };
        assert_eq!(wave_arrival(&reg, &rules, 1, 0), 9000);
        assert_eq!(wave_arrival(&reg, &rules, 3, 0), 9000 + 2 * 3600);
        assert!(wave_arrival(&reg, &rules, 3, 50) < 9000 + 2 * 3600);
    }
}
