//! Контент v0.4: топливный генератор, склад, дуговая турель, дальнобойные и бронированные враги.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use ferrustry_content::{BlockId, ContentRegistry, ItemId};
use ferrustry_sim::combat::SUB;
use ferrustry_sim::scenario::{self, BaseIds};
use ferrustry_sim::{Command, Detail, Dir, TilePos, World, state_hash};

fn reg() -> ContentRegistry {
    ContentRegistry::base().unwrap()
}

fn b(reg: &ContentRegistry, n: &str) -> BlockId {
    reg.block_by_name(n).unwrap()
}

fn item(reg: &ContentRegistry, n: &str) -> ItemId {
    reg.item_by_name(n).unwrap()
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

fn empty(reg: &ContentRegistry, w: u32, h: u32) -> World {
    let ids = BaseIds::resolve(reg).unwrap();
    World::new(reg, w, h, ids.basalt, 7)
}

fn health(w: &World, reg: &ContentRegistry, x: i32, y: i32) -> u32 {
    w.inspect(reg, TilePos::new(x, y)).map_or(0, |i| i.health)
}

/// Все предметы на месте: произведено + создано = израсходовано + уничтожено + в мире.
fn assert_conserved(w: &World, reg: &ContentRegistry) {
    let s = w.stats();
    let (belts, bufs, cores) = (
        w.items_on_conveyors(),
        w.items_in_buffers(reg),
        w.total_core_items(),
    );
    for i in 0..reg.items().len() {
        assert_eq!(
            s.produced[i] + s.spawned[i],
            s.consumed[i] + s.destroyed[i] + belts[i] + bufs[i] + cores[i],
            "item {} not conserved",
            reg.items()[i].name
        );
    }
}

#[test]
fn burner_runs_only_while_it_has_fuel() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let carbon = item(&reg, "carbon");
    let mut w = empty(&reg, 20, 10);
    let mut cmds = vec![place(b(&reg, "burner"), 10, 4, Dir::Right)];
    cmds.extend((2..10).map(|x| place(ids.conveyor, x, 4, Dir::Right)));
    // потребитель, чтобы сеть существовала и было видно выработку
    cmds.push(place(ids.smelter, 12, 4, Dir::Right));
    scenario::build(&mut w, &reg, &cmds).unwrap();
    run(&mut w, &reg, 5);
    let gen_detail = |w: &World| w.inspect(&reg, TilePos::new(10, 4)).unwrap().detail;
    match gen_detail(&w) {
        Detail::Generator { running, fuel, .. } => {
            assert!(!running, "no fuel yet");
            assert_eq!(fuel, Some((carbon, 0)));
        }
        d => panic!("{d:?}"),
    }
    assert_eq!(
        w.inspect(&reg, TilePos::new(10, 4))
            .unwrap()
            .power
            .unwrap()
            .production,
        0
    );
    // лента угля
    w.debug_fill_conveyors(carbon, 64);
    run(&mut w, &reg, 60);
    match gen_detail(&w) {
        Detail::Generator { running, .. } => assert!(running),
        d => panic!("{d:?}"),
    }
    assert_eq!(
        w.inspect(&reg, TilePos::new(10, 4))
            .unwrap()
            .power
            .unwrap()
            .production,
        12
    );
    assert_conserved(&w, &reg);
    // весь уголь с ленты сгорает по 120 тиков за штуку
    let fuel = w.stats().spawned[usize::from(carbon.0)];
    run(&mut w, &reg, fuel as u32 * 120 + 200);
    match gen_detail(&w) {
        Detail::Generator { running, fuel, .. } => {
            assert!(!running, "fuel ran out");
            assert_eq!(fuel, Some((carbon, 0)));
        }
        d => panic!("{d:?}"),
    }
    assert_eq!(w.stats().consumed[usize::from(carbon.0)], fuel);
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn wind_turbine_still_refuses_items() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = empty(&reg, 12, 6);
    let mut cmds = vec![place(ids.wind_turbine, 8, 2, Dir::Right)];
    cmds.extend((2..8).map(|x| place(ids.conveyor, x, 2, Dir::Right)));
    scenario::build(&mut w, &reg, &cmds).unwrap();
    w.debug_fill_conveyors(ids.iron_ingot, 64);
    let before = w.items_on_conveyors();
    run(&mut w, &reg, 300);
    assert_eq!(w.items_on_conveyors(), before);
}

/// Склад 2×2 в (10,4), выход вправо; лента слева; с выходом — лента вправо в ядро.
fn depot_line(reg: &ContentRegistry, with_output: bool) -> World {
    let ids = BaseIds::resolve(reg).unwrap();
    let mut w = empty(reg, 24, 10);
    let mut cmds = vec![place(b(reg, "depot"), 10, 4, Dir::Right)];
    cmds.extend((2..10).map(|x| place(ids.conveyor, x, 4, Dir::Right)));
    if with_output {
        cmds.extend((12..20).map(|x| place(ids.conveyor, x, 5, Dir::Right)));
        cmds.push(place(ids.core, 20, 4, Dir::Right));
    }
    scenario::build(&mut w, reg, &cmds).unwrap();
    w
}

#[test]
fn depot_buffers_items_and_outputs_forward() {
    let reg = reg();
    let carbon = item(&reg, "carbon");
    let mut w = depot_line(&reg, false);
    w.debug_fill_conveyors(carbon, 64);
    run(&mut w, &reg, 200);
    match w.inspect(&reg, TilePos::new(10, 4)).unwrap().detail {
        Detail::Depot { total, items, .. } => {
            let spawned = w.stats().spawned[usize::from(carbon.0)] as u32;
            assert_eq!(total, spawned, "the whole belt went into the depot");
            assert_eq!(items, vec![(carbon, spawned)]);
        }
        d => panic!("{d:?}"),
    }
    assert_eq!(w.items_on_conveyors()[usize::from(carbon.0)], 0);
    assert_conserved(&w, &reg);

    // с выходной лентой в ядро: склад выдаёт вперёд, пока не опустеет
    let mut w = depot_line(&reg, true);
    w.debug_fill_conveyors(carbon, 64);
    run(&mut w, &reg, 600);
    match w.inspect(&reg, TilePos::new(10, 4)).unwrap().detail {
        Detail::Depot { total, .. } => assert_eq!(total, 0, "depot emptied into the core"),
        d => panic!("{d:?}"),
    }
    assert_eq!(
        w.total_core_items()[usize::from(carbon.0)],
        w.stats().spawned[usize::from(carbon.0)]
    );
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn depot_refuses_items_from_its_front() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = empty(&reg, 24, 10);
    // лента справа смотрит в переднюю (правую) сторону склада
    let mut cmds = vec![place(b(&reg, "depot"), 10, 4, Dir::Right)];
    cmds.extend((12..20).map(|x| place(ids.conveyor, x, 4, Dir::Left)));
    scenario::build(&mut w, &reg, &cmds).unwrap();
    w.debug_fill_conveyors(ids.iron_ingot, 64);
    run(&mut w, &reg, 300);
    match w.inspect(&reg, TilePos::new(10, 4)).unwrap().detail {
        Detail::Depot { total, .. } => assert_eq!(total, 0),
        d => panic!("{d:?}"),
    }
}

/// Дуговая турель (4,4) без ядра (враги стоят на месте: пути нет) и цель в 3 тайлах.
fn arc_range(reg: &ContentRegistry, turbines: i32, unit: &str) -> World {
    let ids = BaseIds::resolve(reg).unwrap();
    let mut w = empty(reg, 16, 12);
    let mut cmds = vec![place(b(reg, "arc_tower"), 4, 4, Dir::Right)];
    // ветряки цепочкой вниз от левого края турели (касаются сторонами)
    cmds.extend((0..turbines).map(|k| place(ids.wind_turbine, 3, 5 - k, Dir::Right)));
    scenario::build(&mut w, reg, &cmds).unwrap();
    let (x, y) = tile_center(8, 5);
    w.spawn_unit(reg, reg.unit_by_name(unit).unwrap(), x, y);
    w
}

fn unit_health(w: &World) -> Option<u32> {
    w.units().next().map(|(_, _, _, hp)| hp)
}

#[test]
fn arc_tower_needs_power_and_ignores_armor() {
    let reg = reg();
    // 4 ветряка = 8 энергии/тик = полное питание: 320 / 30 → 11 разрядов × 25 тиков
    let mut w = arc_range(&reg, 4, "shellback");
    run(&mut w, &reg, 11 * 25 + 10);
    assert_eq!(w.kills(), 1, "armor does not help against the arc");
    w.validate(&reg).unwrap();

    // без питания: первый разряд (турель построена заряженной), дальше — тишина
    let mut w = arc_range(&reg, 0, "shellback");
    run(&mut w, &reg, 400);
    assert_eq!(unit_health(&w), Some(320 - 30));

    // половина питания — перезарядка вдвое дольше
    let mut full = arc_range(&reg, 4, "shellback");
    let mut half = arc_range(&reg, 2, "shellback");
    run(&mut full, &reg, 150);
    run(&mut half, &reg, 150);
    let (f, h) = (unit_health(&full).unwrap(), unit_health(&half).unwrap());
    assert_eq!(320 - f, 30 * 6, "full power: shots at 0, 25, ..., 125");
    assert_eq!(320 - h, 30 * 3, "half power: shots at 0, 50, 100");
}

#[test]
fn armor_reduces_gun_damage_but_not_below_a_third() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let hp_after = |unit: &str| {
        let mut w = empty(&reg, 20, 12);
        scenario::build(
            &mut w,
            &reg,
            &[place(b(&reg, "gun_turret"), 4, 4, Dir::Right)],
        )
        .unwrap();
        w.debug_fill_turrets(&reg);
        let (x, y) = tile_center(9, 5);
        w.spawn_unit(&reg, reg.unit_by_name(unit).unwrap(), x, y);
        let _ = ids;
        run(&mut w, &reg, 31);
        unit_health(&w)
    };
    // за 31 тик — выстрелы в тиках 0, 15, 30; снаряд летит ~4 тайла / 96 ≈ 10 тиков: 2 попадания
    assert_eq!(hp_after("shellback"), Some(320 - 2 * 4), "12 - 8 armor = 4");
    assert_eq!(hp_after("brute"), Some(260 - 2 * 12), "no armor");
}

#[test]
fn spitter_stops_at_range_and_shoots_buildings() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = empty(&reg, 40, 21);
    scenario::build(
        &mut w,
        &reg,
        &[
            place(ids.core, 34, 9, Dir::Right),
            place(b(&reg, "iron_wall"), 20, 10, Dir::Right),
        ],
    )
    .unwrap();
    let (x, y) = tile_center(4, 10);
    w.spawn_unit(&reg, reg.unit_by_name("spitter").unwrap(), x, y);
    run(&mut w, &reg, 600);
    let (_, ux, _, _) = w.units().next().unwrap();
    assert!(
        ux <= 16 * SUB + SUB / 2 + 8,
        "stopped about 4 tiles before the wall: x = {}",
        ux as f32 / SUB as f32
    );
    assert!(health(&w, &reg, 20, 10) < 400, "the wall is under fire");
    assert_eq!(health(&w, &reg, 35, 10), 3000, "the core is untouched");
    w.validate(&reg).unwrap();
}

#[test]
fn waves_bring_new_enemy_types_later() {
    let reg = reg();
    let names = |n: u32| -> Vec<String> {
        World::wave_composition(&reg, n)
            .into_iter()
            .map(|(k, _)| reg.unit(k).name.clone())
            .collect()
    };
    assert!(!names(2).contains(&"spitter".to_owned()));
    assert!(names(3).contains(&"spitter".to_owned()));
    assert!(!names(5).contains(&"shellback".to_owned()));
    assert!(names(6).contains(&"shellback".to_owned()));
}

#[test]
fn mixed_battle_is_deterministic_and_survives_save_load() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let build = || {
        let mut w = empty(&reg, 40, 21);
        let mut cmds = vec![
            place(ids.core, 34, 9, Dir::Right),
            place(b(&reg, "gun_turret"), 26, 12, Dir::Right),
            place(b(&reg, "arc_tower"), 26, 7, Dir::Right),
        ];
        cmds.extend((5..9).map(|y| place(ids.wind_turbine, 28, y, Dir::Right)));
        cmds.extend((3..18).map(|y| place(b(&reg, "iron_wall"), 22, y, Dir::Right)));
        scenario::build(&mut w, &reg, &cmds).unwrap();
        w.debug_fill_turrets(&reg);
        for (k, name) in ["spitter", "shellback", "crawler", "spitter", "brute"]
            .iter()
            .enumerate()
        {
            let (x, y) = tile_center(3, 4 + 3 * k as i32);
            w.spawn_unit(&reg, reg.unit_by_name(name).unwrap(), x, y);
        }
        w
    };
    let mut a = build();
    run(&mut a, &reg, 400);
    let bytes = postcard::to_allocvec(&a).unwrap();
    let mut c: World = postcard::from_bytes(&bytes).unwrap();
    let mut b2 = build();
    run(&mut b2, &reg, 400);
    assert_eq!(state_hash(&a), state_hash(&b2));
    run(&mut a, &reg, 1200);
    run(&mut b2, &reg, 1200);
    run(&mut c, &reg, 1200);
    assert_eq!(state_hash(&a), state_hash(&b2));
    assert_eq!(state_hash(&a), state_hash(&c), "save/load mid-battle");
    assert!(a.kills() > 0);
    a.validate(&reg).unwrap();
}
