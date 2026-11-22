//! Генератор карт: рельеф (шум значений на целых числах), руды, точки появления врагов, старт.
//!
//! Всё детерминировано seed'ом и считается в целых числах (одинаковая карта на любой платформе). Гарантии генератора (проверяются тестами):
//! - у старта есть железо (2 месторождения в 11–20 тайлах) и уголь (14–24 тайла);
//! - вокруг ядра свободная площадка;
//! - от каждой точки появления врагов есть проходимый путь до ядра (иначе прорубается коридор).

use std::collections::VecDeque;

use ferrustry_content::{ContentRegistry, ItemId};

use crate::geom::{Dir, TilePos};
use crate::scenario::{BaseIds, ScenarioError};
use crate::world::{Command, Rules, World};

/// Размер карты.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapSize {
    /// 128×96.
    Small,
    /// 192×128.
    Medium,
    /// 320×200.
    Large,
}

impl MapSize {
    /// Размеры в тайлах.
    pub fn dims(self) -> (i32, i32) {
        match self {
            MapSize::Small => (128, 96),
            MapSize::Medium => (192, 128),
            MapSize::Large => (320, 200),
        }
    }
}

/// Сложность: темп волн.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    /// Первая волна через 8 мин, далее каждые 3 мин.
    Easy,
    /// 5 мин / 2 мин.
    Normal,
    /// 3 мин / 1,5 мин.
    Hard,
}

impl Difficulty {
    /// (первая волна, интервал) в секундах.
    pub fn timing_secs(self) -> (u64, u64) {
        match self {
            Difficulty::Easy => (480, 180),
            Difficulty::Normal => (300, 120),
            Difficulty::Hard => (180, 90),
        }
    }
}

/// Настройки генерации.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapSettings {
    /// Размер.
    pub size: MapSize,
    /// Сложность.
    pub difficulty: Difficulty,
    /// Seed.
    pub seed: u64,
}

/// Стартовый запас слитков.
pub const STARTER_INGOTS: u32 = 300;

fn hash(x: i32, y: i32, seed: u64) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ (seed as u32)
        ^ ((seed >> 32) as u32).wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

/// Шум значений 0..=1023 с шагом решётки `cell` (билинейная интерполяция, только целые числа).
fn value_noise(x: i32, y: i32, cell: i32, seed: u64) -> i32 {
    let (gx, gy) = (x.div_euclid(cell), y.div_euclid(cell));
    let (fx, fy) = (x.rem_euclid(cell), y.rem_euclid(cell));
    let v = |dx: i32, dy: i32| (hash(gx + dx, gy + dy, seed) & 1023) as i32;
    let top = v(0, 0) * (cell - fx) + v(1, 0) * fx;
    let bot = v(0, 1) * (cell - fx) + v(1, 1) * fx;
    (top * (cell - fy) + bot * fy) / (cell * cell)
}

fn blob(
    world: &mut World,
    cx: i32,
    cy: i32,
    r: i32,
    seed: u64,
    mut f: impl FnMut(&mut World, TilePos),
) {
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            let d2 = (x - cx).pow(2) + (y - cy).pow(2);
            let jitter = (hash(x, y, seed) % 3) as i32;
            if d2 <= (r - 1 + jitter).pow(2) && world.grid().in_bounds(TilePos::new(x, y)) {
                f(world, TilePos::new(x, y));
            }
        }
    }
}

/// Генерирует карту по настройкам: рельеф, руды, точки появления, ядро со стартовым запасом.
pub fn generate(reg: &ContentRegistry, s: &MapSettings) -> Result<World, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let ash = reg
        .floor_by_name("ash")
        .ok_or(ScenarioError::MissingContent("ash"))?;
    let slag = reg
        .floor_by_name("slag_pool")
        .ok_or(ScenarioError::MissingContent("slag_pool"))?;
    let carbon = reg
        .ore_by_name("carbon_seam")
        .ok_or(ScenarioError::MissingContent("carbon_seam"))?;
    let (w, h) = s.size.dims();
    let mut world = World::new(reg, w as u32, h as u32, ids.basalt, s.seed);
    let (cx, cy) = (w / 2, h / 2);

    // рельеф: пепел — крупные пятна, шлаковые озёра — редкие и не у старта
    for y in 0..h {
        for x in 0..w {
            let p = TilePos::new(x, y);
            let far = (x - cx).pow(2) + (y - cy).pow(2) > 18 * 18;
            if value_noise(x, y, 14, s.seed ^ 0xA5) > 640 {
                world.set_floor(p, ash);
            }
            if far && value_noise(x, y, 9, s.seed ^ 0x51) > 800 {
                world.set_floor(p, slag);
            }
        }
    }

    // руды: гарантированные у старта + случайные по карте
    let mut rng = crate::rng::Rng::new(s.seed ^ 0x0E5);
    // точка в кольце rmin..=rmax вокруг центра — только целые числа (одинаково на всех платформах)
    let ring = |rng: &mut crate::rng::Rng, rmin: u32, rmax: u32| loop {
        let r = rmax as i32;
        let dx = rng.below(2 * rmax + 1) as i32 - r;
        let dy = rng.below(2 * rmax + 1) as i32 - r;
        let d2 = dx * dx + dy * dy;
        if d2 >= (rmin * rmin) as i32 && d2 <= r * r {
            break (cx + dx, cy + dy);
        }
    };
    // сначала случайные месторождения, затем гарантированные — их ничто не перекроет
    let mut ore_spots = Vec::new();
    let area = (w * h) as u32;
    for _ in 0..area / 1500 {
        let (x, y) = (rng.below(w as u32) as i32, rng.below(h as u32) as i32);
        let r = 2 + rng.below(3) as i32;
        let ore = if rng.below(3) == 0 {
            carbon
        } else {
            ids.iron_vein
        };
        ore_spots.push((x, y, r, ore));
    }
    for _ in 0..2 {
        let (x, y) = ring(&mut rng, 11, 20);
        ore_spots.push((x, y, 4, ids.iron_vein));
    }
    let (x, y) = ring(&mut rng, 14, 24);
    ore_spots.push((x, y, 3, carbon));
    for (i, &(x, y, r, ore)) in ore_spots.iter().enumerate() {
        blob(&mut world, x, y, r, s.seed ^ i as u64, |wd, p| {
            wd.set_floor(p, ids.basalt); // руда лежит на твёрдом полу
            wd.set_ore(p, Some(ore));
        });
    }

    // свободная площадка у ядра
    for y in cy - 6..=cy + 6 {
        for x in cx - 6..=cx + 6 {
            let p = TilePos::new(x, y);
            world.set_floor(p, ids.basalt);
            world.set_ore(p, None);
        }
    }

    // точки появления на краях и гарантированный путь до ядра
    let mut spawns = vec![TilePos::new(3, cy), TilePos::new(w - 4, cy)];
    if s.size != MapSize::Small {
        spawns.push(TilePos::new(cx, h - 4));
    }
    if s.size == MapSize::Large {
        spawns.push(TilePos::new(cx, 3));
    }
    for &sp in &spawns {
        for dy in -2..=2 {
            for dx in -2..=2 {
                world.set_floor(TilePos::new(sp.x + dx, sp.y + dy), ids.basalt);
            }
        }
        if !reachable(&world, reg, sp, TilePos::new(cx, cy)) {
            carve(&mut world, ids.basalt, sp, TilePos::new(cx, cy));
        }
    }
    world.set_spawns(spawns.clone());

    // разрушенные аванпосты (D-01 «Reforge»): ремонт за долю цены или разбор на слитки
    place_ruins(&mut world, reg, s, &spawns, &ore_spots);

    crate::scenario::build(
        &mut world,
        reg,
        &[Command::Place {
            block: ids.core,
            origin: TilePos::new(cx - 1, cy - 1),
            rot: Dir::Right,
        }],
    )?;
    let (first, spacing) = s.difficulty.timing_secs();
    let tps = u64::from(crate::world::TICKS_PER_SECOND);
    world.set_rules(Rules {
        infinite_resources: false,
        waves: true,
        first_wave: first * tps,
        wave_spacing: spacing * tps,
    });
    world.grant(reg, ids.iron_ingot, STARTER_INGOTS);
    world.prepare_flow(reg);
    Ok(world)
}

/// Число разрушенных аванпостов по размеру карты.
pub fn outposts(size: MapSize) -> u32 {
    match size {
        MapSize::Small => 3,
        MapSize::Medium => 5,
        MapSize::Large => 8,
    }
}

/// Аванпосты из руин: не ближе 16 тайлов к центру (там будет ядро и стартовая площадка) и не на
/// площадках появления врагов. Что не встало (занято, шлак) — пропускается; набор детерминирован
/// seed. Ставятся в песочнице, до правил партии: командой стройки руину не поставить.
fn place_ruins(
    world: &mut World,
    reg: &ContentRegistry,
    s: &MapSettings,
    spawns: &[TilePos],
    ore_spots: &[(i32, i32, i32, ferrustry_content::OreId)],
) {
    let ruin = |n: &str| reg.block_by_name(n);
    let (w, h) = s.size.dims();
    let (cx, cy) = (w / 2, h / 2);
    let mut rng = crate::rng::Rng::new(s.seed ^ 0x2B1);
    let far = |x: i32, y: i32| {
        (x - cx).pow(2) + (y - cy).pow(2) >= 16 * 16
            && spawns
                .iter()
                .all(|sp| (x - sp.x).abs() > 6 || (y - sp.y).abs() > 6)
            && x >= 2
            && y >= 2
            && x < w - 4
            && y < h - 4
    };
    let put = |world: &mut World, name: &str, x: i32, y: i32| {
        if let Some(block) = ruin(name)
            && far(x, y)
        {
            let _ = world.apply(
                reg,
                &Command::Place {
                    block,
                    origin: TilePos::new(x, y),
                    rot: Dir::Right,
                },
            );
        }
    };
    // буры — на рудных пятнах вдали от старта
    let far_ores: Vec<(i32, i32)> = ore_spots
        .iter()
        .filter(|&&(x, y, _, _)| far(x - 1, y - 1))
        .map(|&(x, y, _, _)| (x - 1, y - 1))
        .collect();
    for k in 0..outposts(s.size) {
        let (x, y) = (
            rng.below((w - 8) as u32) as i32 + 4,
            rng.below((h - 8) as u32) as i32 + 4,
        );
        match k % 4 {
            0 => {
                put(world, "ruined_smelter", x, y);
                put(world, "ruined_wind_turbine", x + 2, y);
                put(world, "ruined_wind_turbine", x + 2, y + 1);
            }
            1 if !far_ores.is_empty() => {
                let (ox, oy) = far_ores[rng.below(far_ores.len() as u32) as usize];
                put(world, "ruined_bore", ox, oy);
            }
            2 => {
                put(world, "ruined_steel_forge", x, y);
                put(world, "ruined_smelter", x + 3, y);
            }
            _ => {
                put(world, "ruined_gun_turret", x, y);
                put(world, "ruined_gun_turret", x + 3, y + 1);
            }
        }
    }
}

/// Проходим ли путь по полу между двумя тайлами (BFS по 4 соседям).
pub fn reachable(world: &World, reg: &ContentRegistry, a: TilePos, b: TilePos) -> bool {
    let (w, h) = (world.grid().width() as i32, world.grid().height() as i32);
    let passable = |p: TilePos| {
        world
            .grid()
            .floor(p)
            .is_some_and(|f| reg.floor(f).buildable)
    };
    if !passable(a) {
        return false;
    }
    let mut seen = vec![false; (w * h) as usize];
    let mut q = VecDeque::from([a]);
    seen[(a.y * w + a.x) as usize] = true;
    while let Some(p) = q.pop_front() {
        if p == b {
            return true;
        }
        for n in p.neighbors() {
            if world.grid().in_bounds(n) && !seen[(n.y * w + n.x) as usize] && passable(n) {
                seen[(n.y * w + n.x) as usize] = true;
                q.push_back(n);
            }
        }
    }
    false
}

/// Прорубает коридор шириной 3 по прямой.
fn carve(world: &mut World, floor: ferrustry_content::FloorId, a: TilePos, b: TilePos) {
    let steps = (b.x - a.x).abs().max((b.y - a.y).abs()).max(1);
    for i in 0..=steps {
        let x = a.x + (b.x - a.x) * i / steps;
        let y = a.y + (b.y - a.y) * i / steps;
        for d in -1..=1 {
            world.set_floor(TilePos::new(x + d, y), floor);
            world.set_floor(TilePos::new(x, y + d), floor);
        }
    }
}

/// Тайлы руды ресурса `item` в радиусе `r` от центра карты (для тестов и подсказок).
pub fn ore_near_center(world: &World, reg: &ContentRegistry, item: ItemId, r: i32) -> usize {
    let (w, h) = (world.grid().width() as i32, world.grid().height() as i32);
    let (cx, cy) = (w / 2, h / 2);
    let mut n = 0;
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            if let Some(o) = world.grid().ore(TilePos::new(x, y))
                && reg.ore(o).item == item
            {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(size: MapSize, seed: u64) -> MapSettings {
        MapSettings {
            size,
            difficulty: Difficulty::Normal,
            seed,
        }
    }

    #[test]
    fn generator_guarantees_hold_for_many_seeds() {
        let reg = ContentRegistry::base().unwrap();
        let iron = reg.item_by_name("iron_ore").unwrap();
        let carbon = reg.item_by_name("carbon").unwrap();
        for size in [MapSize::Small, MapSize::Medium, MapSize::Large] {
            for seed in 0..12 {
                let w = generate(&reg, &settings(size, seed)).unwrap();
                w.validate(&reg).unwrap();
                let (mw, mh) = size.dims();
                let core = TilePos::new(mw / 2, mh / 2);
                assert!(
                    ore_near_center(&w, &reg, iron, 22) >= 20,
                    "{size:?} {seed}: iron near start"
                );
                assert!(
                    ore_near_center(&w, &reg, carbon, 28) >= 8,
                    "{size:?} {seed}: carbon near start"
                );
                assert!(w.spawns().len() >= 2);
                for &sp in w.spawns() {
                    // ядро занимает центр: проверяем путь до соседнего с ядром тайла
                    assert!(
                        reachable(&w, &reg, sp, TilePos::new(core.x - 2, core.y)),
                        "{size:?} {seed}: spawn {sp:?}"
                    );
                }
                assert_eq!(
                    w.total_core_items()[usize::from(reg.item_by_name("iron_ingot").unwrap().0)],
                    u64::from(STARTER_INGOTS)
                );
            }
        }
    }

    #[test]
    fn maps_have_ruined_outposts_away_from_the_start() {
        let reg = ContentRegistry::base().unwrap();
        for size in [MapSize::Small, MapSize::Medium, MapSize::Large] {
            let mut total = 0;
            for seed in 0..12 {
                let w = generate(&reg, &settings(size, seed)).unwrap();
                let (mw, mh) = size.dims();
                let (cx, cy) = (mw / 2, mh / 2);
                let ruins: Vec<TilePos> = w
                    .buildings()
                    .iter()
                    .filter(|(_, b)| b.tag == crate::KindTag::Ruin)
                    .map(|(_, b)| b.origin)
                    .collect();
                for r in &ruins {
                    assert!(
                        (r.x - cx).pow(2) + (r.y - cy).pow(2) >= 16 * 16,
                        "{size:?} {seed}: ruin at {r:?} too close to the start"
                    );
                    for sp in w.spawns() {
                        assert!(
                            (r.x - sp.x).abs() > 6 || (r.y - sp.y).abs() > 6,
                            "{size:?} {seed}: ruin on a spawn clearing"
                        );
                    }
                }
                total += ruins.len();
            }
            // в среднем хотя бы по постройке на аванпост
            assert!(
                total >= 12 * outposts(size) as usize,
                "{size:?}: {total} ruins over 12 maps"
            );
        }
    }

    #[test]
    fn same_seed_same_map_different_seed_different_map() {
        let reg = ContentRegistry::base().unwrap();
        let a = generate(&reg, &settings(MapSize::Small, 7)).unwrap();
        let b = generate(&reg, &settings(MapSize::Small, 7)).unwrap();
        let c = generate(&reg, &settings(MapSize::Small, 8)).unwrap();
        assert_eq!(crate::state_hash(&a), crate::state_hash(&b));
        assert_ne!(crate::state_hash(&a), crate::state_hash(&c));
    }

    #[test]
    fn difficulty_sets_wave_timing() {
        let reg = ContentRegistry::base().unwrap();
        let mut s = settings(MapSize::Small, 1);
        s.difficulty = Difficulty::Hard;
        let w = generate(&reg, &s).unwrap();
        assert_eq!(w.wave_info(&reg), Some((1, 180 * 30)));
    }
}
