//! Потолок ускорения от потоков на этом железе (P-06).
//!
//! Мегабаза B3 размера N делится на K **независимых** баз по N/K; каждая живёт в своём потоке
//! и синхронизируется с остальными раз в тик (как будущая симуляция по регионам). Время тика
//! «всей» базы при K = 1, 2, 4 показывает, сколько дают ядра, когда каждый поток владеет
//! своими данными, — без накладных расходов на обмен между регионами. Реальная реализация
//! этого числа не превзойдёт.
//!
//! `cargo run --release -p ferrustry-headless --example parallel_ceiling -- 100000`
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Barrier};
use std::time::Instant;

use ferrustry_content::ContentRegistry;
use ferrustry_sim::scenario;

fn main() {
    let reg = Arc::new(ContentRegistry::base().unwrap());
    let total: u32 = std::env::args()
        .nth(1)
        .map_or(100_000, |s| s.parse().expect("size"));
    let (warmup, ticks) = (6000, 2000);
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!(
        "B3 {total} conveyors, {cpus} logical CPUs, {warmup} warm-up + {ticks} measured ticks"
    );
    let mut single = None;
    for threads in [1usize, 2, 4, 8].into_iter().filter(|&t| t <= cpus.max(1)) {
        let per = total / threads as u32;
        let tick = Arc::new(Barrier::new(threads));
        let phase = Arc::new(Barrier::new(threads + 1));
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let (reg, tick, phase) = (reg.clone(), tick.clone(), phase.clone());
                std::thread::spawn(move || {
                    let mut w = scenario::bench_megabase(&reg, per).unwrap();
                    for _ in 0..warmup {
                        w.step(&reg);
                    }
                    phase.wait();
                    for _ in 0..ticks {
                        w.step(&reg);
                        tick.wait();
                    }
                    phase.wait();
                })
            })
            .collect();
        phase.wait();
        let t = Instant::now();
        phase.wait();
        let us = t.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);
        for h in workers {
            h.join().unwrap();
        }
        let base = *single.get_or_insert(us);
        println!(
            "{threads} thread(s) x {per:>6}: {us:8.1} us per tick of the whole base  ({:.2}x)",
            base / us
        );
    }
}
