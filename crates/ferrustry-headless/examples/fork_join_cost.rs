//! Цена одного параллельного вызова пула потоков (rayon) в ритме тика (P-06).
//!
//! Работа в вызове ничтожна (8192 сложения), поэтому измеряется почти чистая цена «раздать
//! работу потокам и дождаться». Пауза перед вызовом имитирует последовательную часть тика:
//! после неё потоки пула успевают уснуть, и вызов платит за их пробуждение. Если эта цена
//! сравнима с работой фазы тика (100–150 мкс на B3-100k), мелкозернистый параллелизм фаз
//! не окупается — так и вышло на облачной VM.
//!
//! `cargo run --release -p ferrustry-headless --example fork_join_cost`
//! (`RAYON_NUM_THREADS=1` — для сравнения с пулом из одного потока)
#![allow(missing_docs)]

use std::time::{Duration, Instant};

use rayon::prelude::*;

fn main() {
    let mut v = vec![0u64; 8192];
    let calls = 2000u32;
    for gap_us in [0u64, 100, 600] {
        let mut total = Duration::ZERO;
        for _ in 0..calls {
            let idle = Instant::now();
            while idle.elapsed() < Duration::from_micros(gap_us) {}
            let t = Instant::now();
            v.par_iter_mut().with_min_len(512).for_each(|x| *x += 1);
            total += t.elapsed();
        }
        println!(
            "pool of {} thread(s), {gap_us:>3} us of sequential work before the call: {:6.1} us per parallel call",
            rayon::current_num_threads(),
            total.as_secs_f64() * 1e6 / f64::from(calls)
        );
    }
}
