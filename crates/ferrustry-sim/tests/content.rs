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
    // числа — из данных: тест проверяет механику, а не конкретный баланс
    let ferrustry_content::BlockKind::ArcTurret {
        damage,
        reload,
        power_use,
        ..
    } = reg.block(b(&reg, "arc_tower")).kind
    else {
        panic!("arc_tower must be an arc turret");
    };
    let hp = reg.unit(reg.unit_by_name("shellback").unwrap()).health;
    assert_eq!(power_use, 8, "4 wind turbines (2 each) give full power");
    // полное питание: hp / damage разрядов (вверх), по одному за перезарядку
    let shots = hp.div_ceil(damage);
    let mut w = arc_range(&reg, 4, "shellback");
    run(&mut w, &reg, (shots - 1) * reload + 10);
    assert_eq!(w.kills(), 1, "armor does not help against the arc");
    w.validate(&reg).unwrap();

    // без питания: первый разряд (турель построена заряженной), дальше — тишина
    let mut w = arc_range(&reg, 0, "shellback");
    run(&mut w, &reg, 400);
    assert_eq!(unit_health(&w), Some(hp - damage));

    // половина питания — перезарядка вдвое дольше; окно короче, чем нужно на убийство
    let window = 6 * reload;
    assert!(hp > damage * window.div_ceil(reload));
    let mut full = arc_range(&reg, 4, "shellback");
    let mut half = arc_range(&reg, 2, "shellback");
    run(&mut full, &reg, window);
    run(&mut half, &reg, window);
    let (f, h) = (unit_health(&full).unwrap(), unit_health(&half).unwrap());
    assert_eq!(hp - f, damage * window.div_ceil(reload), "full power");
    assert_eq!(hp - h, damage * window.div_ceil(2 * reload), "half power");
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

/// P-10: турели не тратят выстрелы на юнита, которого уже добьют снаряды в полёте.
#[test]
fn turrets_do_not_overkill() {
    let reg = reg();
    let gun = b(&reg, "gun_turret");
    // пять пушек вокруг неподвижного ползуна (ядра нет — пути нет, юнит стоит)
    let mut w = empty(&reg, 30, 20);
    let spots = [(4, 8), (8, 3), (8, 14), (13, 4), (13, 13)];
    let cmds: Vec<_> = spots
        .iter()
        .map(|&(x, y)| place(gun, x, y, Dir::Right))
        .collect();
    scenario::build(&mut w, &reg, &cmds).unwrap();
    w.debug_fill_turrets(&reg);
    let ammo = |w: &World| -> u32 {
        spots
            .iter()
            .map(
                |&(x, y)| match w.inspect(&reg, TilePos::new(x, y)).unwrap().detail {
                    Detail::Turret { ammo } => ammo,
                    d => panic!("{d:?}"),
                },
            )
            .sum()
    };
    let full = ammo(&w);
    let (x, y) = tile_center(10, 9);
    w.spawn_unit(&reg, reg.unit_by_name("crawler").unwrap(), x, y);
    for _ in 0..120 {
        w.step(&reg);
        w.validate(&reg).unwrap(); // учёт урона в полёте сходится каждый тик
    }
    assert_eq!(w.kills(), 1);
    // 40 прочности / 12 урона = 4 снаряда; пятый не нужен
    assert_eq!(full - ammo(&w), 4, "shots fired");
}

/// Учёт урона в полёте сходится каждый тик в настоящем бою с промахами и смертями.
#[test]
fn incoming_damage_bookkeeping_holds_in_battle() {
    let reg = reg();
    let mut w = scenario::bench_battle(&reg, 300, 60).unwrap();
    for _ in 0..1500 {
        w.step(&reg);
        w.validate(&reg).unwrap();
    }
    assert!(w.kills() > 100, "kills {}", w.kills());
}

/// Состав волн из данных: каждая следующая волна не меньше предыдущей ни по одному виду врагов,
/// и состав совпадает с правилами `waves` (count + grow × ⌊(N − from) / every⌋).
#[test]
fn waves_grow_by_the_data_rules() {
    let reg = reg();
    let mut prev: Vec<u32> = vec![0; reg.units().len()];
    for n in 1..=60u32 {
        let mut cur = vec![0u32; reg.units().len()];
        for (kind, count) in World::wave_composition(&reg, n) {
            cur[usize::from(kind.0)] += count;
        }
        for r in reg.waves() {
            let want = if n < r.from_wave {
                0
            } else {
                r.count + r.grow * ((n - r.from_wave) / r.every)
            };
            assert_eq!(cur[usize::from(r.unit.0)], want, "wave {n}");
        }
        assert!(
            cur.iter().zip(&prev).all(|(c, p)| c >= p),
            "wave {n} shrank"
        );
        prev = cur;
    }
}

/// D-02: шумят только работающие постройки; шум — сумма их шума за тик.
#[test]
fn working_buildings_make_noise_idle_ones_do_not() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let carbon = item(&reg, "carbon");
    let mut w = empty(&reg, 30, 12);
    // три топливных генератора (шум 3) с лентами угля и плавильня без руды (простаивает)
    let mut cmds = vec![place(ids.smelter, 24, 8, Dir::Right)];
    for k in 0..3 {
        let y = 1 + 3 * k;
        cmds.push(place(b(&reg, "burner"), 12, y, Dir::Right));
        cmds.extend((2..12).map(|x| place(ids.conveyor, x, y, Dir::Right)));
    }
    scenario::build(&mut w, &reg, &cmds).unwrap();
    run(&mut w, &reg, 5);
    assert_eq!(w.noise(), 0, "nothing works yet");
    w.debug_fill_conveyors(carbon, 64);
    run(&mut w, &reg, 60);
    assert_eq!(
        w.noise(),
        3 * 3,
        "three burning burners, the idle smelter is silent"
    );
}

/// D-02: шумная база получает волну раньше; тихая — ровно по расписанию.
#[test]
fn noisy_base_brings_waves_sooner() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let carbon = item(&reg, "carbon");
    let first = 900u64;
    let arrival = |noisy: bool| -> (u64, u64) {
        let mut w = empty(&reg, 40, 20);
        let mut cmds = vec![place(ids.core, 34, 8, Dir::Right)];
        if noisy {
            for k in 0..5 {
                let y = 1 + 3 * k;
                cmds.push(place(b(&reg, "burner"), 12, y, Dir::Right));
                cmds.extend((2..12).map(|x| place(ids.conveyor, x, y, Dir::Right)));
            }
        }
        scenario::build(&mut w, &reg, &cmds).unwrap();
        w.debug_fill_conveyors(carbon, 64);
        w.set_spawns(vec![TilePos::new(1, 18)]);
        w.set_rules(ferrustry_sim::Rules {
            waves: true,
            first_wave: first,
            ..*w.rules()
        });
        // ожидание по правилу из данных, по шуму каждого тика
        let rule = reg.noise_rule();
        let mut rem = first * 1000;
        let mut expected = None;
        for t in 1..=first + 10 {
            w.step(&reg);
            if expected.is_none() {
                if rem > 0 {
                    rem = rem.saturating_sub(u64::from(rule.timer_rate(w.noise())));
                } else {
                    expected = Some(t);
                }
            }
            if w.units().count() > 0 {
                return (t, expected.unwrap_or(0));
            }
        }
        panic!("no wave");
    };
    let (quiet, quiet_expected) = arrival(false);
    assert_eq!(quiet, first + 1, "a quiet base keeps the schedule");
    assert_eq!(quiet, quiet_expected);
    let (noisy, noisy_expected) = arrival(true);
    assert_eq!(
        noisy, noisy_expected,
        "timer follows the data rule tick by tick"
    );
    // 5 горящих генераторов = шум 15 → таймер +30%
    assert!(noisy * 100 < quiet * 80, "noisy {noisy} vs quiet {quiet}");
}

#[test]
fn noise_speedup_is_capped() {
    let rule = reg().noise_rule();
    assert_eq!(rule.timer_rate(0), 1000);
    assert_eq!(rule.timer_rate(10), 1000 + 10 * rule.speedup_per_unit);
    assert_eq!(rule.timer_rate(1_000_000), 1000 + rule.max_speedup);
}

/// Переполнитель: прямо, пока впереди есть место; излишек — вбок, поочерёдно; ничего не теряется.
#[test]
fn overflow_gate_feeds_straight_first_then_sideways() {
    let reg = reg();
    let ids = BaseIds::resolve(&reg).unwrap();
    let gate = b(&reg, "overflow_gate");
    let mut w = empty(&reg, 16, 12);
    // вход слева: лента 1..5 → переполнитель (5,5)
    let mut cmds: Vec<Command> = (1..5)
        .map(|x| place(ids.conveyor, x, 5, Dir::Right))
        .collect();
    cmds.push(place(gate, 5, 5, Dir::Right));
    scenario::build(&mut w, &reg, &cmds).unwrap();
    w.debug_fill_conveyors(ids.iron_ore, 64);
    let spawned = w.stats().spawned[usize::from(ids.iron_ore.0)];
    // выходы (пустые, в тупик): прямо (6..9), вверх (5, 6..9), вниз (5, 4..1)
    let mut outs: Vec<Command> = (6..9)
        .map(|x| place(ids.conveyor, x, 5, Dir::Right))
        .collect();
    outs.extend((6..9).map(|y| place(ids.conveyor, 5, y, Dir::Up)));
    outs.extend((2..5).rev().map(|y| place(ids.conveyor, 5, y, Dir::Down)));
    scenario::build(&mut w, &reg, &outs).unwrap();
    let on = |w: &World, tiles: &[TilePos]| -> usize {
        let wanted: Vec<TilePos> = tiles.to_vec();
        w.segments()
            .filter(|s| s.tiles().iter().any(|t| wanted.contains(t)))
            .map(|s| s.item_count())
            .sum()
    };
    let front: Vec<TilePos> = (6..9).map(|x| TilePos::new(x, 5)).collect();
    let up: Vec<TilePos> = (6..9).map(|y| TilePos::new(5, y)).collect();
    let down: Vec<TilePos> = (2..5).map(|y| TilePos::new(5, y)).collect();
    run(&mut w, &reg, 30);
    assert!(on(&w, &front) > 0);
    assert_eq!(
        on(&w, &up) + on(&w, &down),
        0,
        "room ahead: nothing goes sideways"
    );
    run(&mut w, &reg, 600);
    let (f, u, d) = (on(&w, &front), on(&w, &up), on(&w, &down));
    assert!(f >= 12, "the front line filled up first: {f}");
    assert!(u > 0 && d > 0, "surplus goes both ways: up {u}, down {d}");
    assert!(u.abs_diff(d) <= 1, "sides alternate: up {u}, down {d}");
    let total = w.items_on_conveyors()[usize::from(ids.iron_ore.0)]
        + w.items_in_buffers(&reg)[usize::from(ids.iron_ore.0)];
    assert_eq!(total, spawned, "no item lost or created");
    w.validate(&reg).unwrap();
}

/// Мир с ядром в обычных правилах (стройка платная) и запасом слитков.
fn paid_world(reg: &ContentRegistry, ingots: u32) -> World {
    let mut w = empty(reg, 40, 40);
    w.apply(reg, &place(b(reg, "core"), 30, 30, Dir::Right))
        .unwrap();
    // руины ставит генератор карты в песочнице — так же и здесь, до включения правил партии
    w.apply(reg, &place(b(reg, "ruined_smelter"), 10, 10, Dir::Right))
        .unwrap();
    w.apply(reg, &place(b(reg, "ruined_gun_turret"), 14, 10, Dir::Right))
        .unwrap();
    w.set_rules(ferrustry_sim::Rules {
        infinite_resources: false,
        waves: false,
        ..Default::default()
    });
    w.grant(reg, item(reg, "iron_ingot"), ingots);
    w
}

fn ingots(w: &World, reg: &ContentRegistry) -> u64 {
    w.total_core_items()[usize::from(item(reg, "iron_ingot").0)]
}

#[test]
fn ruins_are_repaired_for_a_share_of_the_cost() {
    let reg = reg();
    let mut w = paid_world(&reg, 100);
    let at = TilePos::new(10, 10);
    let ins = w.inspect(&reg, at).unwrap();
    let Detail::Ruin {
        of,
        repair,
        salvage,
    } = ins.detail
    else {
        panic!("{:?}", ins.detail)
    };
    assert_eq!(of, b(&reg, "smelter"));
    assert_eq!((repair[0].count, salvage[0].count), (8, 12));
    // ремонт: 8 слитков вместо 25, на месте руины — плавильня с полной прочностью
    w.apply(&reg, &Command::Repair { at }).unwrap();
    assert_eq!(ingots(&w, &reg), 92);
    let ins = w.inspect(&reg, TilePos::new(11, 11)).unwrap();
    assert_eq!(ins.block, b(&reg, "smelter"));
    assert_eq!(ins.origin, at);
    assert_eq!(ins.health, reg.block(b(&reg, "smelter")).health);
    // плавильня — не руина
    assert_eq!(
        w.apply(&reg, &Command::Repair { at }),
        Err(ferrustry_sim::CommandError::NotARuin)
    );
    // снос отремонтированной — полная стоимость плавильни, как у любой постройки
    w.apply(&reg, &Command::Remove { at }).unwrap();
    assert_eq!(ingots(&w, &reg), 117);
    w.validate(&reg).unwrap();
    assert_conserved(&w, &reg);
}

#[test]
fn removing_a_ruin_salvages_and_poverty_blocks_repair() {
    let reg = reg();
    let mut w = paid_world(&reg, 10);
    let turret = TilePos::new(14, 10);
    // ремонт пушки — 14 слитков, есть 10: отказ, руина на месте
    assert_eq!(
        w.apply(&reg, &Command::Repair { at: turret }),
        Err(ferrustry_sim::CommandError::NotEnough(item(
            &reg,
            "iron_ingot"
        )))
    );
    assert!(matches!(
        w.inspect(&reg, turret).unwrap().detail,
        Detail::Ruin { .. }
    ));
    // разбор: половина стоимости пушки (45) — 22 слитка
    w.apply(&reg, &Command::Remove { at: turret }).unwrap();
    assert_eq!(ingots(&w, &reg), 32);
    assert!(w.inspect(&reg, turret).is_none());
    // в обычной партии руину не поставить
    assert_eq!(
        w.apply(&reg, &place(b(&reg, "ruined_smelter"), 20, 20, Dir::Right)),
        Err(ferrustry_sim::CommandError::NotPlaceable)
    );
    // руина не работает и не шумит
    run(&mut w, &reg, 60);
    assert_eq!(w.noise(), 0);
    w.validate(&reg).unwrap();
    assert_conserved(&w, &reg);
}
