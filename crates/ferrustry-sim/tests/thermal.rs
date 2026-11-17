//! Тепло и жидкости: перегрев, охлаждение, вода, тепловые машины, учёт при перестройке.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use ferrustry_content::{BlockId, ContentRegistry, ItemId};
use ferrustry_sim::scenario::{self, BaseIds};
use ferrustry_sim::{Command, Dir, TilePos, World};

fn reg() -> ContentRegistry {
    ContentRegistry::base().unwrap()
}

fn b(reg: &ContentRegistry, n: &str) -> BlockId {
    reg.block_by_name(n).unwrap()
}

fn place(block: BlockId, x: i32, y: i32) -> Command {
    Command::Place {
        block,
        origin: TilePos::new(x, y),
        rot: Dir::Right,
    }
}

fn item(reg: &ContentRegistry, n: &str) -> ItemId {
    reg.item_by_name(n).unwrap()
}

/// Кузница (10,10) с ядром слева и 5 ветряками справа (10 энергии) + `extra`.
fn forge_world(reg: &ContentRegistry, extra: &[Command]) -> World {
    let ids = BaseIds::resolve(reg).unwrap();
    let mut w = World::new(reg, 40, 40, ids.basalt, 1);
    let mut cmds = vec![place(b(reg, "steel_forge"), 10, 10), place(ids.core, 7, 10)];
    for x in 12..17 {
        cmds.push(place(ids.wind_turbine, x, 10));
    }
    cmds.extend_from_slice(extra);
    scenario::build(&mut w, reg, &cmds).unwrap();
    w
}

/// Гоняет мир, подкладывая кузнице входы; возвращает произведённую сталь.
fn run_forge(w: &mut World, reg: &ContentRegistry, ticks: u32) -> u64 {
    let (ingot, carbon, steel) = (
        item(reg, "iron_ingot"),
        item(reg, "carbon"),
        item(reg, "steel"),
    );
    let at = TilePos::new(10, 10);
    for _ in 0..ticks {
        w.debug_fill_crafter(reg, at, ingot, 10);
        w.debug_fill_crafter(reg, at, carbon, 10);
        w.step(reg);
    }
    w.stats().produced[usize::from(steel.0)]
}

fn forge_temperature(w: &World, reg: &ContentRegistry) -> u32 {
    w.inspect(reg, TilePos::new(10, 10))
        .unwrap()
        .heat
        .unwrap()
        .temperature
}

fn assert_fluid_conserved(w: &World, reg: &ContentRegistry) {
    let s = w.fluid_stats();
    let inside = w.fluid_in_nets(reg);
    for (i, f) in reg.fluids().iter().enumerate() {
        let get = |v: &Vec<u64>| v.get(i).copied().unwrap_or(0);
        assert_eq!(
            get(&s.produced),
            get(&s.consumed) + get(&s.destroyed) + inside[i],
            "fluid `{}` not conserved",
            f.name
        );
    }
}

#[test]
fn uncooled_forge_overheats_and_slows_down() {
    let reg = reg();
    let mut hot = forge_world(&reg, &[]);
    let radiator = b(&reg, "radiator");
    let mut cool = forge_world(
        &reg,
        &[
            place(radiator, 10, 12),
            place(radiator, 11, 12),
            place(radiator, 10, 9),
            place(radiator, 11, 9),
        ],
    );
    let hot_steel = run_forge(&mut hot, &reg, 3000);
    let cool_steel = run_forge(&mut cool, &reg, 3000);
    // 60 тиков на цикл → 50 циклов за 3000 тиков без перегрева
    assert!(
        cool_steel >= 49,
        "cooled forge must run at full speed: {cool_steel}"
    );
    assert!(forge_temperature(&cool, &reg) <= 700);
    // без охлаждения равновесие при скорости ≈1/6 (тепло 6, пассивное охлаждение 1)
    assert!(
        hot_steel * 10 < cool_steel * 3,
        "hot {hot_steel} vs cool {cool_steel}"
    );
    let t = forge_temperature(&hot, &reg);
    assert!((900..1000).contains(&t), "equilibrium temperature {t}");
    hot.validate(&reg).unwrap();
}

#[test]
fn condenser_water_feeds_cooling_tower_and_keeps_forge_cool() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let extra = [
        place(b(&reg, "heat_pipe"), 10, 9),
        place(b(&reg, "heat_pipe"), 10, 8),
        place(b(&reg, "cooling_tower"), 10, 6),
        place(b(&reg, "pipe"), 9, 6),
        place(b(&reg, "pipe"), 8, 6),
        place(b(&reg, "condenser"), 6, 6),
        place(ids.wind_turbine, 6, 8),
        place(ids.wind_turbine, 7, 8),
    ];
    let mut w = forge_world(&reg, &extra);
    let steel = run_forge(&mut w, &reg, 3000);
    assert!(
        steel >= 49,
        "water cooling must keep the forge at full speed: {steel}"
    );
    assert!(forge_temperature(&w, &reg) <= 700);
    let water = reg.fluid_by_name("water").unwrap();
    let used = w.fluid_stats().consumed[usize::from(water.0)];
    assert!(used > 1000, "cooling tower used water: {used}");
    let net = w.inspect(&reg, TilePos::new(8, 6)).unwrap().fluid.unwrap();
    assert_eq!(net.fluid, Some(water));
    assert_fluid_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn heat_engine_turns_forge_heat_into_power() {
    let reg = reg();
    let mut w = forge_world(&reg, &[place(b(&reg, "heat_engine"), 10, 12)]);
    let steel = run_forge(&mut w, &reg, 3000);
    assert!(steel >= 49, "engine also cools the forge: {steel}");
    let ins = w.inspect(&reg, TilePos::new(10, 12)).unwrap();
    match ins.detail {
        ferrustry_sim::Detail::HeatEngine { output } => assert_eq!(output, 3, "6 heat x 50%"),
        d => panic!("{d:?}"),
    }
    // 5 ветряков (10) + машина (3)
    assert_eq!(ins.power.unwrap().production, 13);
}

#[test]
fn splitting_a_pipe_network_accounts_for_every_unit() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let pipe = b(&reg, "pipe");
    let mut w = World::new(&reg, 40, 20, ids.basalt, 1);
    let mut cmds = vec![
        place(b(&reg, "condenser"), 0, 0),
        place(ids.wind_turbine, 0, 2),
        place(ids.wind_turbine, 1, 2),
    ];
    for x in 2..12 {
        cmds.push(place(pipe, x, 0));
    }
    scenario::build(&mut w, &reg, &cmds).unwrap();
    for _ in 0..400 {
        w.step(&reg);
    }
    let full = w.inspect(&reg, TilePos::new(5, 0)).unwrap().fluid.unwrap();
    assert_eq!(full.amount, full.capacity, "network filled: {full:?}");
    // снос трубы посередине делит сеть; доля снесённой трубы теряется и учитывается
    w.apply(
        &reg,
        &Command::Remove {
            at: TilePos::new(6, 0),
        },
    )
    .unwrap();
    w.step(&reg);
    let right = w.inspect(&reg, TilePos::new(9, 0)).unwrap().fluid.unwrap();
    assert_eq!(right.capacity, 5 * 20);
    assert_eq!(right.amount, right.capacity, "right part keeps its share");
    let water = reg.fluid_by_name("water").unwrap();
    assert_eq!(w.fluid_stats().destroyed[usize::from(water.0)], 20);
    assert_fluid_conserved(&w, &reg);
    w.validate(&reg).unwrap();
}

#[test]
fn random_construction_with_thermal_blocks_conserves_fluid_and_is_reproducible() {
    let reg = reg();
    for seed in [21, 22] {
        let run = || {
            let (mut w, mut bld) = scenario::RandomBuilder::new(&reg, 24, seed).unwrap();
            for t in 0..3000u64 {
                let cmds = bld.commands(t);
                w.tick(&reg, &cmds);
                if t % 300 == 0 {
                    w.validate(&reg).unwrap();
                    assert_fluid_conserved(&w, &reg);
                }
            }
            ferrustry_sim::state_hash(&w)
        };
        assert_eq!(run(), run(), "seed {seed}");
    }
}
