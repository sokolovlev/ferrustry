//! Интеграционные тесты симуляции.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use ferrustry_content::ContentRegistry;
use ferrustry_sim::scenario::{self, BaseIds};
use ferrustry_sim::{Command, CommandError, Dir, TilePos, World, state_hash};

fn reg() -> ContentRegistry {
    ContentRegistry::base().unwrap()
}

fn place(block: ferrustry_content::BlockId, x: i32, y: i32, rot: Dir) -> Command {
    Command::Place {
        block,
        origin: TilePos::new(x, y),
        rot,
    }
}

fn empty(reg: &ContentRegistry, w: u32, h: u32) -> (World, BaseIds) {
    let ids = BaseIds::resolve(reg).unwrap();
    (World::new(reg, w, h, ids.basalt, 7), ids)
}

fn run(w: &mut World, reg: &ContentRegistry, ticks: u32) {
    for _ in 0..ticks {
        w.step(reg);
    }
}

fn idx(item: ferrustry_content::ItemId) -> usize {
    usize::from(item.0)
}

/// Все предметы учтены: (добыто+создано) == (израсходовано+уничтожено+в ядрах+на лентах+в буферах).
fn assert_conserved(w: &World, reg: &ContentRegistry) {
    let s = w.stats();
    let core = w.total_core_items();
    let belts = w.items_on_conveyors();
    let bufs = w.items_in_buffers(reg);
    for i in 0..reg.items().len() {
        let inflow = s.produced[i] + s.spawned[i];
        let outflow = s.consumed[i] + s.destroyed[i] + core[i] + belts[i] + bufs[i];
        assert_eq!(
            inflow,
            outflow,
            "item `{}` not conserved",
            reg.items()[i].name
        );
    }
}

#[test]
fn placement_rules() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 16, 16);
    assert_eq!(w.apply(&reg, &place(ids.core, 0, 0, Dir::Right)), Ok(()));
    assert_eq!(
        w.apply(&reg, &place(ids.bore, 2, 2, Dir::Right)),
        Err(CommandError::Occupied)
    );
    assert_eq!(
        w.apply(&reg, &place(ids.bore, 15, 0, Dir::Right)),
        Err(CommandError::OutOfBounds)
    );
    assert_eq!(
        w.apply(&reg, &place(ids.bore, -1, 5, Dir::Right)),
        Err(CommandError::OutOfBounds)
    );
    let slag = reg.floor_by_name("slag_pool").unwrap();
    w.set_floor(TilePos::new(8, 8), slag);
    assert_eq!(
        w.apply(&reg, &place(ids.conveyor, 8, 8, Dir::Right)),
        Err(CommandError::NotBuildable)
    );
    assert_eq!(
        w.apply(
            &reg,
            &Command::Remove {
                at: TilePos::new(9, 9)
            }
        ),
        Err(CommandError::NothingThere)
    );
    // снос по любому тайлу освобождает все тайлы
    assert_eq!(
        w.apply(
            &reg,
            &Command::Remove {
                at: TilePos::new(2, 1)
            }
        ),
        Ok(())
    );
    for y in 0..3 {
        for x in 0..3 {
            assert!(w.grid().building(TilePos::new(x, y)).is_none());
        }
    }
    w.validate(&reg).unwrap();
}

#[test]
fn bore_rate_is_exact() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 16, 16);
    for y in 0..2 {
        for x in 0..2 {
            w.set_ore(TilePos::new(x, y), Some(ids.iron_vein));
        }
    }
    // половина бура на руде — вдвое медленнее
    w.set_ore(TilePos::new(10, 0), Some(ids.iron_vein));
    w.set_ore(TilePos::new(11, 0), Some(ids.iron_vein));
    scenario::build(
        &mut w,
        &reg,
        &[
            place(ids.bore, 0, 0, Dir::Right),
            place(ids.core, 2, 0, Dir::Right),
            place(ids.bore, 10, 0, Dir::Right),
            place(ids.core, 12, 0, Dir::Right),
        ],
    )
    .unwrap();
    run(&mut w, &reg, 400);
    // base_ticks 40 при полном покрытии 2×2
    assert_eq!(
        w.core_items(TilePos::new(2, 0)).unwrap()[idx(ids.iron_ore)],
        10
    );
    assert_eq!(
        w.core_items(TilePos::new(12, 0)).unwrap()[idx(ids.iron_ore)],
        5
    );
    assert_conserved(&w, &reg);
}

#[test]
fn conveyor_line_is_one_segment_and_delivers() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 32, 8);
    for y in 0..2 {
        for x in 0..2 {
            w.set_ore(TilePos::new(x, y + 1), Some(ids.iron_vein));
        }
    }
    let mut cmds = vec![place(ids.bore, 0, 1, Dir::Right)];
    for x in 0..20 {
        cmds.push(place(ids.conveyor, x, 0, Dir::Right));
    }
    cmds.push(place(ids.core, 20, 0, Dir::Right));
    scenario::build(&mut w, &reg, &cmds).unwrap();
    assert_eq!(w.segments().count(), 1);
    run(&mut w, &reg, 2000);
    let delivered = w.core_items(TilePos::new(20, 0)).unwrap()[idx(ids.iron_ore)];
    // 50 предметов добыто за 2000 тиков; в пути не больше, чем проходит за время пути (~320 тиков)
    assert_eq!(w.stats().produced[idx(ids.iron_ore)], 50);
    assert!(delivered >= 41, "delivered {delivered}");
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn corner_keeps_single_segment() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 16, 16);
    let mut cmds = Vec::new();
    for x in 0..5 {
        cmds.push(place(
            ids.conveyor,
            x,
            0,
            if x == 4 { Dir::Up } else { Dir::Right },
        ));
    }
    for y in 1..5 {
        cmds.push(place(ids.conveyor, 4, y, Dir::Up));
    }
    scenario::build(&mut w, &reg, &cmds).unwrap();
    assert_eq!(w.segments().count(), 1);
    assert_eq!(w.segments().next().unwrap().tiles().len(), 9);
    w.validate(&reg).unwrap();
}

#[test]
fn filling_gap_merges_segments_and_keeps_items() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 32, 4);
    let mut cmds = Vec::new();
    for x in (0..10).chain(11..20) {
        cmds.push(place(ids.conveyor, x, 0, Dir::Right));
    }
    scenario::build(&mut w, &reg, &cmds).unwrap();
    assert_eq!(w.segments().count(), 2);
    w.debug_fill_conveyors(ids.iron_ore, 128);
    run(&mut w, &reg, 10);
    let before: u64 = w.items_on_conveyors().iter().sum();
    w.apply(&reg, &place(ids.conveyor, 10, 0, Dir::Right))
        .unwrap();
    assert_eq!(w.segments().count(), 1);
    let after: u64 = w.items_on_conveyors().iter().sum();
    assert_eq!(before, after + w.stats().destroyed.iter().sum::<u64>());
    w.validate(&reg).unwrap();
    assert_conserved(&w, &reg);
}

#[test]
fn removing_middle_conveyor_splits_and_destroys_only_its_items() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 32, 4);
    let cmds: Vec<_> = (0..20)
        .map(|x| place(ids.conveyor, x, 0, Dir::Right))
        .collect();
    scenario::build(&mut w, &reg, &cmds).unwrap();
    w.debug_fill_conveyors(ids.iron_ore, 64);
    let total: u64 = w.items_on_conveyors().iter().sum();
    w.apply(
        &reg,
        &Command::Remove {
            at: TilePos::new(10, 0),
        },
    )
    .unwrap();
    assert_eq!(w.segments().count(), 2);
    let destroyed: u64 = w.stats().destroyed.iter().sum();
    assert!(
        (4..=5).contains(&destroyed),
        "destroyed {destroyed} of {total}"
    );
    w.validate(&reg).unwrap();
    assert_conserved(&w, &reg);
}

#[test]
fn closed_loop_circulates_forever() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 16, 16);
    scenario::build(
        &mut w,
        &reg,
        &scenario::loop_commands(ids.conveyor, 1, 1, 10, 10),
    )
    .unwrap();
    // 36 тайлов режутся на сегменты не длиннее MAX_SEGMENT_TILES
    assert_eq!(
        w.segments().count(),
        36usize.div_ceil(ferrustry_sim::MAX_SEGMENT_TILES)
    );
    w.debug_fill_conveyors(ids.iron_ore, 128);
    let n: u64 = w.items_on_conveyors().iter().sum();
    assert!(n > 0);
    let snapshot = |w: &World| {
        w.segments()
            .next()
            .unwrap()
            .item_positions()
            .collect::<Vec<_>>()
    };
    let before = snapshot(&w);
    run(&mut w, &reg, 7);
    assert_ne!(snapshot(&w), before, "items must move");
    run(&mut w, &reg, 1000);
    assert_eq!(w.items_on_conveyors().iter().sum::<u64>(), n);
    w.validate(&reg).unwrap();
    assert_conserved(&w, &reg);
}

#[test]
fn long_line_is_split_and_items_flow_across_pieces() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 128, 8);
    for y in 0..2 {
        for x in 0..2 {
            w.set_ore(TilePos::new(x, y + 1), Some(ids.iron_vein));
        }
    }
    let mut cmds = vec![place(ids.bore, 0, 1, Dir::Right)];
    for x in 0..100 {
        cmds.push(place(ids.conveyor, x, 0, Dir::Right));
    }
    cmds.push(place(ids.core, 100, 0, Dir::Right));
    scenario::build(&mut w, &reg, &cmds).unwrap();
    assert_eq!(w.segments().count(), 4);
    assert!(
        w.segments()
            .all(|s| s.tiles().len() <= ferrustry_sim::MAX_SEGMENT_TILES)
    );
    run(&mut w, &reg, 4000);
    // путь 100 тайлов = 1600 тиков; добыто 100 предметов
    let delivered = w.core_items(TilePos::new(100, 0)).unwrap()[idx(ids.iron_ore)];
    assert!(delivered >= 55, "delivered {delivered}");
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn full_belt_does_not_accept_head_on_or_overflow() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 16, 4);
    let cmds: Vec<_> = (0..4)
        .map(|x| place(ids.conveyor, x, 0, Dir::Right))
        .collect();
    scenario::build(&mut w, &reg, &cmds).unwrap();
    // лента упирается в пустоту: заполнить и прогнать — предметы не исчезают и не дублируются
    w.debug_fill_conveyors(ids.iron_ore, 64);
    run(&mut w, &reg, 500);
    let n: u64 = w.items_on_conveyors().iter().sum();
    assert_eq!(n, w.stats().spawned[idx(ids.iron_ore)]);
    assert_eq!(n, 17); // 4 тайла × 4 + один на самом входе
    w.validate(&reg).unwrap();
}

/// Плавильня, питаемая тремя бурами напрямую; выход — лента вниз в ядро.
fn smelter_setup(reg: &ContentRegistry, turbines: u32) -> World {
    let (mut w, ids) = empty(reg, 32, 32);
    for (bx, by) in [(8, 10), (12, 10), (10, 12)] {
        for dy in 0..2 {
            for dx in 0..2 {
                w.set_ore(TilePos::new(bx + dx, by + dy), Some(ids.iron_vein));
            }
        }
    }
    let mut cmds = vec![
        place(ids.smelter, 10, 10, Dir::Right),
        place(ids.bore, 8, 10, Dir::Right),
        place(ids.bore, 12, 10, Dir::Right),
        place(ids.bore, 10, 12, Dir::Right),
        place(ids.conveyor, 10, 9, Dir::Down),
        place(ids.conveyor, 10, 8, Dir::Down),
        place(ids.conveyor, 10, 7, Dir::Down),
        place(ids.core, 9, 4, Dir::Right),
    ];
    for i in 0..turbines {
        cmds.push(place(ids.wind_turbine, 11 + i as i32, 9, Dir::Right));
    }
    scenario::build(&mut w, reg, &cmds).unwrap();
    w
}

#[test]
fn power_shortage_slows_crafting_proportionally() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let ticks = 3000;
    let mut ingots = Vec::new();
    for turbines in [0, 1, 2] {
        let mut w = smelter_setup(&reg, turbines);
        run(&mut w, &reg, ticks);
        assert_conserved(&w, &reg);
        w.validate(&reg).unwrap();
        ingots.push(w.stats().produced[idx(ids.iron_ingot)]);
    }
    let full_expected = u64::from(ticks) / 30;
    assert_eq!(ingots[0], 0, "no power — no crafting");
    assert!(
        ingots[2] + 3 >= full_expected && ingots[2] <= full_expected,
        "full: {ingots:?}"
    );
    assert!(
        ingots[1] + 3 >= full_expected / 2 && ingots[1] <= full_expected / 2,
        "half: {ingots:?}"
    );
}

#[test]
fn power_node_links_distant_buildings() {
    let reg = reg();
    let (mut w, ids) = empty(&reg, 32, 32);
    scenario::build(
        &mut w,
        &reg,
        &[
            place(ids.wind_turbine, 0, 0, Dir::Right),
            place(ids.power_node, 5, 0, Dir::Right),
            place(ids.smelter, 10, 0, Dir::Right),
        ],
    )
    .unwrap();
    w.step(&reg);
    let a = w
        .buildings()
        .iter()
        .map(|(id, _)| w.power_net_of(id).cloned())
        .collect::<Vec<_>>();
    assert!(a.iter().all(|n| n.is_some()));
    assert_eq!(w.power_nets().len(), 1, "node range 6 reaches both");
    w.apply(
        &reg,
        &Command::Remove {
            at: TilePos::new(5, 0),
        },
    )
    .unwrap();
    w.step(&reg);
    assert_eq!(w.power_nets().len(), 2);
}

#[test]
fn vertical_slice_produces_ingots_in_core() {
    let reg = reg();
    let (mut w, ids) = scenario::vertical_slice(&reg).unwrap();
    run(&mut w, &reg, 3000);
    let core = w.core_items(TilePos::new(18, 2)).unwrap();
    // руда: 2 бура × 1/40 тика = 1 слиток / 40 тиков
    assert!(
        core[idx(ids.iron_ingot)] >= 65,
        "ingots {}",
        core[idx(ids.iron_ingot)]
    );
    assert!(core[idx(ids.iron_ingot)] <= 75);
    assert_eq!(
        core[idx(ids.iron_ore)],
        0,
        "ore must go only to the smelter"
    );
    assert_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn simulation_is_deterministic() {
    let reg = reg();
    let (mut a, _) = scenario::vertical_slice(&reg).unwrap();
    let (mut b, _) = scenario::vertical_slice(&reg).unwrap();
    for _ in 0..500 {
        a.step(&reg);
        b.step(&reg);
    }
    assert_eq!(state_hash(&a), state_hash(&b));
    a.step(&reg);
    assert_ne!(state_hash(&a), state_hash(&b));
}

/// Наблюдаемое состояние: всё, кроме служебных структур планировщика.
type Observable = (Vec<u64>, Vec<u64>, Vec<u64>, Vec<(TilePos, u16, u32)>);

fn observable(w: &World) -> Observable {
    let mut items = Vec::new();
    for seg in w.segments() {
        let tail = *seg.tiles().last().unwrap();
        for (item, pos) in seg.item_positions() {
            items.push((tail, item.0, pos));
        }
    }
    items.sort();
    (
        w.stats().produced.clone(),
        w.stats().destroyed.clone(),
        w.total_core_items(),
        items,
    )
}

fn assert_sleep_is_transparent(mut a: World, reg: &ContentRegistry, ticks: u32) {
    let mut b = a.clone();
    b.set_sleep_enabled(false);
    for t in 0..ticks {
        a.step(reg);
        b.step(reg);
        if t % 50 == 0 {
            assert_eq!(observable(&a), observable(&b), "diverged at tick {t}");
        }
    }
    assert_eq!(observable(&a), observable(&b));
    a.validate(reg).unwrap();
}

#[test]
fn sleeping_does_not_change_behavior() {
    let reg = reg();
    assert_sleep_is_transparent(scenario::bench_combs(&reg, 2_000).unwrap(), &reg, 3000);
    assert_sleep_is_transparent(
        scenario::bench_blocked_lines(&reg, 1_000).unwrap(),
        &reg,
        500,
    );
    assert_sleep_is_transparent(scenario::vertical_slice(&reg).unwrap().0, &reg, 3000);
    assert_sleep_is_transparent(smelter_setup(&reg, 1), &reg, 2000);
}

#[test]
fn sleeping_is_transparent_on_megabase_and_large_combs() {
    let reg = reg();
    assert_sleep_is_transparent(scenario::bench_megabase(&reg, 3_000).unwrap(), &reg, 4000);
    assert_sleep_is_transparent(scenario::bench_combs(&reg, 6_000).unwrap(), &reg, 4000);
}

#[test]
fn jammed_bores_and_branches_sleep() {
    let reg = reg();
    let mut w = scenario::bench_combs(&reg, 6_000).unwrap();
    for _ in 0..4000 {
        w.step(&reg);
    }
    let segs = w.segments().count();
    let drills = w
        .buildings()
        .iter()
        .filter(|(_, b)| b.tag == ferrustry_sim::KindTag::Drill)
        .count();
    assert!(
        w.awake_segments() * 2 < segs,
        "awake {} of {segs} segments",
        w.awake_segments()
    );
    assert!(
        w.awake_drills() * 2 < drills,
        "awake {} of {drills} drills",
        w.awake_drills()
    );
}

#[test]
fn blocked_belts_fall_asleep_and_wake_on_unblock() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let mut w = scenario::bench_blocked_lines(&reg, 1_000).unwrap();
    for _ in 0..10 {
        w.step(&reg);
    }
    assert_eq!(w.awake_segments(), 0, "all blocked belts must sleep");
    // поставить ядро в тупике первой ленты — она и её хвост просыпаются и едут
    w.apply(&reg, &place(ids.core, 100, 0, Dir::Right)).unwrap();
    for _ in 0..200 {
        w.step(&reg);
    }
    assert!(w.core_items(TilePos::new(100, 0)).unwrap()[idx(ids.iron_ore)] > 0);
    assert_conserved(&w, &reg);
}

/// Случайная стройка и снос: инварианты держатся, результат воспроизводим.
fn random_build(seed: u64) -> (u64, World) {
    let reg = reg();
    let (mut w, mut builder) = scenario::RandomBuilder::new(&reg, 24, seed).unwrap();
    let mut twin = w.clone();
    twin.set_sleep_enabled(false);
    for t in 0..3000u32 {
        let cmds = builder.commands(u64::from(t));
        let ra = w.tick(&reg, &cmds);
        let rb = twin.tick(&reg, &cmds);
        assert_eq!(ra, rb);
        if t % 100 == 0 {
            w.validate(&reg).unwrap();
            assert_conserved(&w, &reg);
            assert_eq!(
                observable(&w),
                observable(&twin),
                "sleep changed behavior, seed {seed} tick {t}"
            );
        }
    }
    w.validate(&reg).unwrap();
    assert_conserved(&w, &reg);
    (state_hash(&w), w)
}

#[test]
fn random_construction_keeps_invariants_and_is_reproducible() {
    for seed in [1, 2, 3] {
        let (h1, w) = random_build(seed);
        let (h2, _) = random_build(seed);
        assert_eq!(h1, h2, "seed {seed}");
        assert!(w.buildings().len() > 10);
    }
}
