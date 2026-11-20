//! Цена перестройки энергосетей при стройке (S3-02).
//!
//! Мегабаза B3 размера N; по желанию поверх неё — решётка энергоузлов, объединяющая всё в одну
//! большую сеть (как у игрока, который тянет общую сеть по базе). Сравнивается время тика без
//! стройки и тика, в котором ставится или сносится ветряк (энергопостройка → перестройка сетей).
//!
//! `cargo run --release -p ferrustry-headless --example power_rebuild_cost -- 100000 [--grid]`
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use ferrustry_content::ContentRegistry;
use ferrustry_sim::{Command, Dir, TilePos, scenario};

fn main() {
    let reg = ContentRegistry::base().unwrap();
    let args: Vec<String> = std::env::args().collect();
    let n: u32 = args.get(1).map_or(100_000, |s| s.parse().expect("size"));
    let grid = args.iter().any(|a| a == "--grid");
    let mut w = scenario::bench_megabase(&reg, n).unwrap();
    let node = reg.block_by_name("power_node").unwrap();
    let turbine = reg.block_by_name("wind_turbine").unwrap();
    let (gw, gh) = (w.grid().width() as i32, w.grid().height() as i32);
    if grid {
        // узлы с шагом 5 по свободным тайлам: дальность 6 связывает соседей
        let mut placed = 0;
        for y in (0..gh).step_by(5) {
            for x in (0..gw).step_by(5) {
                let c = Command::Place {
                    block: node,
                    origin: TilePos::new(x, y),
                    rot: Dir::Right,
                };
                if w.apply(&reg, &c).is_ok() {
                    placed += 1;
                }
            }
        }
        println!("placed {placed} power nodes");
    }
    for _ in 0..600 {
        w.step(&reg);
    }
    println!(
        "B3 {n}: {} buildings, {} power nets",
        w.buildings().len(),
        w.live_power_nets()
    );
    let wall = reg.block_by_name("iron_wall").unwrap();
    // свободный тайл для ветряка
    let spot = (0..gh)
        .flat_map(|y| (0..gw).map(move |x| TilePos::new(x, y)))
        .find(|&p| w.can_place(&reg, turbine, p).is_ok())
        .expect("free tile");
    let ticks = 200;
    let measure = |w: &mut ferrustry_sim::World, block: Option<ferrustry_content::BlockId>| {
        let t = Instant::now();
        for i in 0..ticks {
            let cmds = match block {
                None => Vec::new(),
                Some(b) if i % 2 == 0 => vec![Command::Place {
                    block: b,
                    origin: spot,
                    rot: Dir::Right,
                }],
                Some(_) => vec![Command::Remove { at: spot }],
            };
            let r = w.tick(&reg, &cmds);
            assert!(r.iter().all(Result::is_ok));
        }
        t.elapsed().as_secs_f64() * 1e6 / f64::from(ticks)
    };
    let plain = measure(&mut w, None);
    let walls = measure(&mut w, Some(wall));
    let power = measure(&mut w, Some(turbine));
    println!("tick without building:                {plain:8.1} us");
    println!("tick with a wall placed/removed:      {walls:8.1} us");
    println!(
        "tick with a turbine placed/removed:   {power:8.1} us  (power part ~{:.1} us)",
        power - walls
    );
}
