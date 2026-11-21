//! Цена любой стройки на большой базе: тики, в которых ставится и сносится стена (не
//! энергопостройка, не лента), против тиков без стройки. Для профилирования (callgrind).
//!
//! `cargo run --release -p ferrustry-headless --example build_cost -- 100000 [ticks] [--plain] [--block=NAME]`
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use ferrustry_content::ContentRegistry;
use ferrustry_sim::{Command, Dir, TilePos, scenario};

fn main() {
    let reg = ContentRegistry::base().unwrap();
    let args: Vec<String> = std::env::args().collect();
    let n: u32 = args.get(1).map_or(100_000, |s| s.parse().expect("size"));
    let ticks: u32 = args.get(2).map_or(200, |s| s.parse().expect("ticks"));
    let build = !args.iter().any(|a| a == "--plain");
    let mut w = scenario::bench_megabase(&reg, n).unwrap();
    for _ in 0..600 {
        w.step(&reg);
    }
    let name = args
        .iter()
        .find_map(|a| a.strip_prefix("--block="))
        .unwrap_or("iron_wall");
    let wall = reg.block_by_name(name).unwrap();
    let (gw, gh) = (w.grid().width() as i32, w.grid().height() as i32);
    let spot = (0..gh)
        .flat_map(|y| (0..gw).map(move |x| TilePos::new(x, y)))
        .find(|&p| w.can_place(&reg, wall, p).is_ok())
        .unwrap();
    // --paced: пауза 33 мс между тиками, как у сервера в реальном времени (кэши остывают)
    let paced = args.iter().any(|a| a == "--paced");
    let mut busy = std::time::Duration::ZERO;
    for i in 0..ticks {
        if paced {
            std::thread::sleep(std::time::Duration::from_millis(33));
        }
        let t = Instant::now();
        let cmds = match (build, i % 2) {
            (false, _) => vec![],
            (true, 0) => vec![Command::Place {
                block: wall,
                origin: spot,
                rot: Dir::Right,
            }],
            (true, _) => vec![Command::Remove { at: spot }],
        };
        w.tick(&reg, &cmds);
        busy += t.elapsed();
    }
    let us = busy.as_secs_f64() * 1e6 / f64::from(ticks);
    println!(
        "B3 {n}: {} buildings, {} tick: {us:.1} us",
        w.buildings().len(),
        if build { "building" } else { "plain" }
    );
}
