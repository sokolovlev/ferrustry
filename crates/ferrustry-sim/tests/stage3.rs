//! Этап 3: экономика, разветвители, сортировщики, мосты, батареи.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use ferrustry_content::{BlockId, ContentRegistry, ItemId};
use ferrustry_sim::scenario::{self, BaseIds};
use ferrustry_sim::{Command, CommandError, Detail, Dir, Rules, TilePos, World, state_hash};

fn reg() -> ContentRegistry {
    ContentRegistry::base().unwrap()
}

fn block(reg: &ContentRegistry, n: &str) -> BlockId {
    reg.block_by_name(n).unwrap()
}

fn place(b: BlockId, x: i32, y: i32, rot: Dir) -> Command {
    Command::Place {
        block: b,
        origin: TilePos::new(x, y),
        rot,
    }
}

fn run(w: &mut World, reg: &ContentRegistry, n: u32) {
    for _ in 0..n {
        w.step(reg);
    }
}

fn idx(i: ItemId) -> usize {
    usize::from(i.0)
}

fn assert_conserved(w: &World, reg: &ContentRegistry) {
    let s = w.stats();
    let (core, belts, bufs) = (
        w.total_core_items(),
        w.items_on_conveyors(),
        w.items_in_buffers(reg),
    );
    for i in 0..reg.items().len() {
        assert_eq!(
            s.produced[i] + s.spawned[i],
            s.consumed[i] + s.destroyed[i] + core[i] + belts[i] + bufs[i],
            "item `{}` not conserved",
            reg.items()[i].name
        );
    }
}

#[test]
fn building_costs_are_paid_refunded_and_enforced() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = World::new(&reg, 32, 32, ids.basalt, 1);
    scenario::build(&mut w, &reg, &[place(ids.core, 20, 20, Dir::Right)]).unwrap();
    w.set_rules(Rules {
        infinite_resources: false,
        ..Default::default()
    });
    w.grant(&reg, ids.iron_ingot, 12);

    // бур стоит 10 слитков
    w.apply(&reg, &place(ids.bore, 2, 2, Dir::Right)).unwrap();
    assert_eq!(w.total_core_items()[idx(ids.iron_ingot)], 2);
    // второй бур — не хватает; мир не меняется
    let before = state_hash(&w);
    assert_eq!(
        w.apply(&reg, &place(ids.bore, 6, 2, Dir::Right)),
        Err(CommandError::NotEnough(ids.iron_ingot))
    );
    assert_eq!(state_hash(&w), before);
    assert!(w.can_place(&reg, ids.bore, TilePos::new(6, 2)).is_err());
    // снос возвращает стоимость
    w.apply(
        &reg,
        &Command::Remove {
            at: TilePos::new(3, 3),
        },
    )
    .unwrap();
    assert_eq!(w.total_core_items()[idx(ids.iron_ingot)], 12);
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn demo_map_starts_with_economy_and_starter_stock() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let w = scenario::demo_map(&reg, 1).unwrap();
    assert!(!w.rules().infinite_resources);
    assert_eq!(
        w.total_core_items()[idx(ids.iron_ingot)],
        u64::from(scenario::STARTER_INGOTS)
    );
}

/// Лента слева в тайл (10,10) и три ядра вокруг: сверху, снизу, впереди.
fn junction(reg: &ContentRegistry, center: &str) -> World {
    let ids = BaseIds::resolve(reg).unwrap();
    let mut w = World::new(reg, 32, 32, ids.basalt, 1);
    let mut cmds: Vec<Command> = (0..10)
        .map(|x| place(ids.conveyor, x, 10, Dir::Right))
        .collect();
    cmds.push(place(block(reg, center), 10, 10, Dir::Right));
    cmds.push(place(ids.core, 8, 11, Dir::Right)); // сверху: (10,11)
    cmds.push(place(ids.core, 8, 7, Dir::Right)); // снизу: (10,9)
    cmds.push(place(ids.core, 11, 8, Dir::Right)); // впереди: (11,10)
    scenario::build(&mut w, reg, &cmds).unwrap();
    w
}

fn core_at(w: &World, x: i32, y: i32, item: ItemId) -> u32 {
    w.core_items(TilePos::new(x, y)).unwrap()[idx(item)]
}

#[test]
fn splitter_distributes_evenly_and_never_backwards() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = junction(&reg, "splitter");
    w.debug_fill_conveyors(ids.iron_ore, 64);
    let n = w.items_on_conveyors()[idx(ids.iron_ore)];
    assert!(n >= 30);
    run(&mut w, &reg, 800);
    let (up, down, fwd) = (
        core_at(&w, 8, 11, ids.iron_ore),
        core_at(&w, 8, 7, ids.iron_ore),
        core_at(&w, 11, 8, ids.iron_ore),
    );
    assert_eq!(u64::from(up + down + fwd), n, "all items delivered");
    let (mx, mn) = (up.max(down).max(fwd), up.min(down).min(fwd));
    assert!(mx - mn <= 1, "uneven split: {up} {down} {fwd}");
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn sorter_sends_filter_straight_and_rest_sideways() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = junction(&reg, "sorter");
    w.apply(
        &reg,
        &Command::Configure {
            at: TilePos::new(10, 10),
            item: Some(ids.iron_ingot),
        },
    )
    .unwrap();
    w.debug_fill_conveyors(ids.iron_ore, 64);
    let n = w.items_on_conveyors()[idx(ids.iron_ore)];
    run(&mut w, &reg, 800);
    let (up, down, fwd) = (
        core_at(&w, 8, 11, ids.iron_ore),
        core_at(&w, 8, 7, ids.iron_ore),
        core_at(&w, 11, 8, ids.iron_ore),
    );
    assert_eq!(fwd, 0, "ore is not the filter");
    assert_eq!(u64::from(up + down), n);
    assert!(up.abs_diff(down) <= 1, "sides alternate: {up} {down}");

    // фильтр = руда → всё прямо
    w.apply(
        &reg,
        &Command::Configure {
            at: TilePos::new(10, 10),
            item: Some(ids.iron_ore),
        },
    )
    .unwrap();
    w.debug_fill_conveyors(ids.iron_ore, 64);
    run(&mut w, &reg, 800);
    assert_eq!(core_at(&w, 11, 8, ids.iron_ore), n as u32);
    assert_eq!(
        w.inspect(&reg, TilePos::new(10, 10)).unwrap().detail,
        Detail::Sorter {
            item: None,
            filter: Some(ids.iron_ore)
        }
    );
    // настраивать можно только сортировщик
    assert_eq!(
        w.apply(
            &reg,
            &Command::Configure {
                at: TilePos::new(9, 12),
                item: None
            }
        ),
        Err(CommandError::NotConfigurable)
    );
    assert_conserved(&w, &reg);
}

#[test]
fn bridge_carries_items_over_obstacles() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let bridge = block(&reg, "bridge_conveyor");
    let mut w = World::new(&reg, 32, 16, ids.basalt, 1);
    let mut cmds: Vec<Command> = (0..5)
        .map(|x| place(ids.conveyor, x, 5, Dir::Right))
        .collect();
    cmds.push(place(bridge, 5, 5, Dir::Right));
    for x in 6..9 {
        cmds.push(place(ids.wind_turbine, x, 5, Dir::Right)); // препятствия
    }
    cmds.push(place(bridge, 9, 5, Dir::Right));
    cmds.push(place(ids.core, 10, 4, Dir::Right));
    scenario::build(&mut w, &reg, &cmds).unwrap();
    w.debug_fill_conveyors(ids.iron_ore, 64);
    let n = w.items_on_conveyors()[idx(ids.iron_ore)];
    run(&mut w, &reg, 30);
    let early = core_at(&w, 10, 4, ids.iron_ore);
    assert!(
        early <= 30 / 4 + 1,
        "throughput limited by interval: {early}"
    );
    run(&mut w, &reg, 600);
    assert_eq!(u64::from(core_at(&w, 10, 4, ids.iron_ore)), n);
    match w.inspect(&reg, TilePos::new(5, 5)).unwrap().detail {
        Detail::Bridge { linked, .. } => assert!(linked),
        d => panic!("{d:?}"),
    }
    // без выходного моста вход становится выходом и упирается в турбину — предметы ждут
    w.apply(
        &reg,
        &Command::Remove {
            at: TilePos::new(9, 5),
        },
    )
    .unwrap();
    w.debug_fill_conveyors(ids.iron_ore, 64);
    run(&mut w, &reg, 300);
    match w.inspect(&reg, TilePos::new(5, 5)).unwrap().detail {
        Detail::Bridge { linked, items } => {
            assert!(!linked);
            assert_eq!(items.len(), 4, "buffer full, nowhere to go");
        }
        d => panic!("{d:?}"),
    }
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn battery_charges_from_surplus_and_powers_crafting() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let battery = block(&reg, "battery");
    let mut w = World::new(&reg, 32, 32, ids.basalt, 1);
    for (bx, by) in [(8, 10), (12, 10), (10, 12)] {
        for dy in 0..2 {
            for dx in 0..2 {
                w.set_ore(TilePos::new(bx + dx, by + dy), Some(ids.iron_vein));
            }
        }
    }
    // фаза 1: плавильня без входов, ветряк заряжает батарею (+2/тик)
    let mut cmds = vec![
        place(ids.smelter, 10, 10, Dir::Right),
        place(battery, 11, 9, Dir::Right),
        place(ids.wind_turbine, 12, 9, Dir::Right),
        place(ids.conveyor, 10, 9, Dir::Down),
        place(ids.conveyor, 10, 8, Dir::Down),
        place(ids.conveyor, 10, 7, Dir::Down),
        place(ids.core, 9, 4, Dir::Right),
    ];
    scenario::build(&mut w, &reg, &cmds).unwrap();
    run(&mut w, &reg, 600);
    let stored = |w: &World| match w.inspect(&reg, TilePos::new(11, 9)).unwrap().detail {
        Detail::Battery { stored, .. } => stored,
        d => panic!("{d:?}"),
    };
    assert_eq!(stored(&w), 1200);

    // фаза 2: генератор убран, буры подают руду — плавильня работает от батареи
    w.apply(
        &reg,
        &Command::Remove {
            at: TilePos::new(12, 9),
        },
    )
    .unwrap();
    cmds.clear();
    cmds.extend([
        place(ids.bore, 8, 10, Dir::Right),
        place(ids.bore, 12, 10, Dir::Right),
        place(ids.bore, 10, 12, Dir::Right),
    ]);
    scenario::build(&mut w, &reg, &cmds).unwrap();
    run(&mut w, &reg, 200);
    let p = w
        .inspect(&reg, TilePos::new(10, 10))
        .unwrap()
        .power
        .unwrap();
    assert_eq!(p.production, 0);
    assert_eq!(p.satisfaction, 1000, "battery covers the whole demand");
    assert!(p.battery_flow < 0);
    run(&mut w, &reg, 1000);
    // 1200 энергии / 4 за тик = 300 тиков работы = 10 слитков, дальше — тишина
    let ingots = w.stats().produced[idx(ids.iron_ingot)];
    assert_eq!(ingots, 10, "battery energy converted to exactly 10 crafts");
    assert_eq!(stored(&w), 0);
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn random_construction_with_new_blocks_is_conserved_and_sleep_transparent() {
    let reg = reg();
    for seed in [11, 12] {
        let (mut w, mut b) = scenario::RandomBuilder::new(&reg, 24, seed).unwrap();
        let mut twin = w.clone();
        twin.set_sleep_enabled(false);
        for t in 0..3000u64 {
            let cmds = b.commands(t);
            assert_eq!(w.tick(&reg, &cmds), twin.tick(&reg, &cmds));
            if t % 250 == 0 {
                w.validate(&reg).unwrap();
                assert_conserved(&w, &reg);
                assert_eq!(w.total_core_items(), twin.total_core_items());
                assert_eq!(w.stats(), twin.stats());
            }
        }
        assert_conserved(&w, &reg);
    }
}

/// Регрессия: снос должен сбрасывать кэши связей. Лента A вливается сбоку в тайл x=8 ленты B;
/// снос тайла x=4 делит B, и номер старого сегмента достаётся левой части (x=0..3). Без сброса
/// кэша A продолжала бы отдавать по устаревшей ссылке и предметы не попадали бы в правую часть.
#[test]
fn removal_invalidates_cached_links() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = World::new(&reg, 32, 32, ids.basalt, 1);
    let mut cmds: Vec<Command> = (0..10)
        .map(|x| place(ids.conveyor, x, 5, Dir::Right))
        .collect();
    cmds.extend((0..5).map(|y| place(ids.conveyor, 8, y, Dir::Up)));
    cmds.push(place(ids.core, 10, 4, Dir::Right)); // B уходит в ядро
    scenario::build(&mut w, &reg, &cmds).unwrap();
    // поток A → B → ядро идёт, связь A закэширована
    w.debug_fill_conveyors(ids.iron_ore, 64);
    run(&mut w, &reg, 60);
    w.apply(
        &reg,
        &Command::Remove {
            at: TilePos::new(4, 5),
        },
    )
    .unwrap();
    let on_right: u32 = w
        .segments()
        .filter(|s| s.tiles().first() == Some(&TilePos::new(5, 5)))
        .map(|s| s.item_count() as u32)
        .sum();
    let core_before = core_at(&w, 10, 4, ids.iron_ore);
    run(&mut w, &reg, 400);
    let delivered = core_at(&w, 10, 4, ids.iron_ore) - core_before;
    assert!(
        delivered > on_right,
        "after the split A must keep feeding the right part: delivered {delivered}, right part had {on_right}"
    );
    w.validate(&reg).unwrap();
    assert_conserved(&w, &reg);
}
