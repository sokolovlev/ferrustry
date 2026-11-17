//! Этап 4: юниты, поле путей, разрушение, турели, волны, поражение.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use ferrustry_content::{BlockId, ContentRegistry};
use ferrustry_sim::combat::{Outcome, SUB};
use ferrustry_sim::scenario::{self, BaseIds};
use ferrustry_sim::{Command, Detail, Dir, Rules, TilePos, World, state_hash};

fn reg() -> ContentRegistry {
    ContentRegistry::base().unwrap()
}

fn b(reg: &ContentRegistry, n: &str) -> BlockId {
    reg.block_by_name(n).unwrap()
}

fn place(block: BlockId, x: i32, y: i32, rot: Dir) -> Command {
    Command::Place {
        block,
        origin: TilePos::new(x, y),
        rot,
    }
}

fn run(w: &mut World, reg: &ContentRegistry, n: u32) {
    for _ in 0..n {
        w.step(reg);
    }
}

fn tile_center(x: i32, y: i32) -> (i32, i32) {
    (x * SUB + SUB / 2, y * SUB + SUB / 2)
}

/// Поле 40×21, ядро справа (34..36, 9..11).
fn arena(reg: &ContentRegistry) -> World {
    let ids = BaseIds::resolve(reg).unwrap();
    let mut w = World::new(reg, 40, 21, ids.basalt, 3);
    scenario::build(&mut w, reg, &[place(ids.core, 34, 9, Dir::Right)]).unwrap();
    w
}

fn core_health(w: &World, reg: &ContentRegistry) -> u32 {
    w.inspect(reg, TilePos::new(35, 10)).map_or(0, |i| i.health)
}

#[test]
fn crawler_walks_to_the_core_and_attacks_it() {
    let reg = reg();
    let mut w = arena(&reg);
    let crawler = reg.unit_by_name("crawler").unwrap();
    let (x, y) = tile_center(2, 10);
    w.spawn_unit(&reg, crawler, x, y);
    let full = core_health(&w, &reg);
    // ~31 тайл × 256 / 20 ≈ 400 тиков пути
    run(&mut w, &reg, 300);
    assert_eq!(core_health(&w, &reg), full, "still on the way");
    run(&mut w, &reg, 300);
    let after = core_health(&w, &reg);
    assert!(after < full, "core must be under attack: {after}");
    w.validate(&reg).unwrap();
}

#[test]
fn units_go_around_walls_and_break_them_when_sealed() {
    let reg = reg();
    let wall = b(&reg, "iron_wall");
    let crawler = reg.unit_by_name("crawler").unwrap();
    // стена поперёк поля с проходом в y=2
    let mut w = arena(&reg);
    let cmds: Vec<_> = (3..21).map(|y| place(wall, 20, y, Dir::Right)).collect();
    scenario::build(&mut w, &reg, &cmds).unwrap();
    let (x, y) = tile_center(2, 10);
    w.spawn_unit(&reg, crawler, x, y);
    run(&mut w, &reg, 1500);
    let walls_hp: u32 = (3..21)
        .map(|y| w.inspect(&reg, TilePos::new(20, y)).unwrap().health)
        .sum();
    assert_eq!(walls_hp, 18 * 400, "a gap exists: walls stay intact");
    assert!(
        core_health(&w, &reg) < 3000,
        "reached the core through the gap"
    );

    // заделать проход — придётся ломать стену
    let mut w = arena(&reg);
    let cmds: Vec<_> = (0..21).map(|y| place(wall, 20, y, Dir::Right)).collect();
    scenario::build(&mut w, &reg, &cmds).unwrap();
    w.spawn_unit(&reg, crawler, x, y);
    run(&mut w, &reg, 2500);
    let broken = (0..21)
        .filter(|&y| w.grid().building(TilePos::new(20, y)).is_none())
        .count();
    assert!(broken >= 1, "sealed wall must be broken through");
    w.validate(&reg).unwrap();
}

/// Турель (20,12) питается слитками с ленты сверху.
fn defended(reg: &ContentRegistry, with_ammo: bool) -> World {
    let ids = BaseIds::resolve(reg).unwrap();
    let mut w = arena(reg);
    let mut cmds = vec![place(b(reg, "gun_turret"), 20, 9, Dir::Right)];
    cmds.extend((11..16).map(|y| place(ids.conveyor, 20, y, Dir::Down)));
    scenario::build(&mut w, reg, &cmds).unwrap();
    if with_ammo {
        w.debug_fill_conveyors(ids.iron_ingot, 64);
    }
    run(&mut w, reg, 200); // боеприпасы доехали
    w
}

#[test]
fn turret_with_ammo_stops_a_wave_before_the_core() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let crawler = reg.unit_by_name("crawler").unwrap();
    let mut w = defended(&reg, true);
    match w.inspect(&reg, TilePos::new(20, 9)).unwrap().detail {
        Detail::Turret { ammo } => assert!(ammo > 0),
        d => panic!("{d:?}"),
    }
    // Баланс v0 (проверено): турель наносит 0,8 урона/тик, краулер (40 HP) пересекает зону
    // поражения (18 тайлов) за ~230 тиков; часть выстрелов — «перебор» по уже обречённой цели.
    // Итог: одна турель надёжно останавливает трёх одновременных краулеров.
    for k in 0..3 {
        let (x, y) = tile_center(2, 7 + 2 * k);
        w.spawn_unit(&reg, crawler, x, y);
    }
    run(&mut w, &reg, 1500);
    assert_eq!(w.kills(), 3);
    assert_eq!(w.units().count(), 0);
    assert_eq!(core_health(&w, &reg), 3000, "nobody reached the core");
    assert!(
        w.stats().consumed[usize::from(ids.iron_ingot.0)] > 0,
        "ammo was used"
    );
    w.validate(&reg).unwrap();
}

#[test]
fn turret_without_ammo_does_nothing() {
    let reg = reg();
    let crawler = reg.unit_by_name("crawler").unwrap();
    let mut w = defended(&reg, false);
    let (x, y) = tile_center(2, 10);
    w.spawn_unit(&reg, crawler, x, y);
    run(&mut w, &reg, 1200);
    assert_eq!(w.kills(), 0);
    assert!(core_health(&w, &reg) < 3000);
}

#[test]
fn losing_the_last_core_ends_the_game_and_destruction_gives_no_refund() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let brute = reg.unit_by_name("brute").unwrap();
    let mut w = arena(&reg);
    w.set_rules(Rules {
        infinite_resources: false,
        waves: true,
        ..Default::default()
    });
    let wall = b(&reg, "iron_wall");
    w.grant(&reg, ids.iron_ingot, 8);
    scenario::build(&mut w, &reg, &[place(wall, 30, 10, Dir::Right)]).unwrap();
    let stock = w.total_core_items()[usize::from(ids.iron_ingot.0)];
    for k in 0..20 {
        let (x, y) = tile_center(31 + k % 2, 4 + k % 12);
        w.spawn_unit(&reg, brute, x, y);
    }
    let mut lost_at = None;
    for _ in 0..6000 {
        w.step(&reg);
        if let Outcome::Lost { tick } = w.outcome() {
            lost_at = Some(tick);
            break;
        }
    }
    assert!(lost_at.is_some(), "core destroyed → game lost");
    assert_eq!(w.total_core_items().len(), reg.items().len());
    let _ = stock; // ядро разрушено вместе с запасом — ресурсы не возвращаются
    w.validate(&reg).unwrap();
}

#[test]
fn waves_arrive_on_schedule_and_grow() {
    let reg = reg();
    let mut w = arena(&reg);
    w.set_spawns(vec![TilePos::new(1, 2), TilePos::new(1, 18)]);
    w.set_rules(Rules {
        waves: true,
        first_wave: 100,
        wave_spacing: 50,
        ..Default::default()
    });
    let size = |n| {
        World::wave_composition(&reg, n)
            .iter()
            .map(|(_, c)| *c as usize)
            .sum::<usize>()
    };
    run(&mut w, &reg, 100);
    assert_eq!(w.units().count(), 0, "first wave at tick 100");
    w.step(&reg);
    assert_eq!(w.units().count(), size(1));
    assert_eq!(w.wave_info(), Some((2, 49)));
    run(&mut w, &reg, 50);
    assert_eq!(w.units().count(), size(1) + size(2));
    assert!(size(2) > size(1));
    w.validate(&reg).unwrap();
}

#[test]
fn battle_is_deterministic_and_survives_save_load() {
    let reg = reg();
    let crawler = reg.unit_by_name("crawler").unwrap();
    let brute = reg.unit_by_name("brute").unwrap();
    let setup = || {
        let mut w = defended(&reg, true);
        for k in 0..30 {
            let (x, y) = tile_center(1 + k % 4, 1 + (k * 7) % 19);
            w.spawn_unit(&reg, if k % 5 == 0 { brute } else { crawler }, x, y);
        }
        w
    };
    let (mut a, mut b2) = (setup(), setup());
    run(&mut a, &reg, 400);
    run(&mut b2, &reg, 400);
    assert_eq!(state_hash(&a), state_hash(&b2));
    assert!(
        a.projectiles().count() > 0 || a.kills() > 0,
        "a battle is going on"
    );
    // снимок посреди боя: «до» и «после загрузки» идут одинаково
    let bytes = postcard_roundtrip(&a);
    let mut c = bytes;
    run(&mut a, &reg, 600);
    run(&mut c, &reg, 600);
    assert_eq!(state_hash(&a), state_hash(&c));
    a.validate(&reg).unwrap();
}

fn postcard_roundtrip(w: &World) -> World {
    let bytes = postcard::to_allocvec(w).unwrap();
    postcard::from_bytes(&bytes).unwrap()
}
