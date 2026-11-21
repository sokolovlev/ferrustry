//! Оценки для выбора модели сетевой игры (этап 6, D-18): сколько стоит рассылать состояние
//! мира и сколько — только команды игроков.
//!
//! - снимок для подключения: размер сжатого сохранения, время записи и чтения;
//! - изменения состояния за тик: мир сериализуется до и после тика, побайтная разница (XOR)
//!   сжимается zstd — оценка снизу для любой дельты состояния (настоящий протокол дельт ещё и
//!   тратит время на поиск изменений). Тики, где длина сериализации изменилась, пропускаются;
//! - поток команд: размер типичной команды стройки;
//! - контрольная сумма: время `state_hash` (как часто можно сверять клиентов).
//!
//! `cargo run --release -p ferrustry-headless --example net_estimates`
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use ferrustry_content::ContentRegistry;
use ferrustry_sim::{Command, Dir, TilePos, World, scenario, state_hash};

fn measure(name: &str, reg: &ContentRegistry, mut w: World) {
    for _ in 0..600 {
        w.step(reg);
    }
    let t = Instant::now();
    let save = ferrustry_format::write_save(&w, reg).unwrap();
    let write_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let back = ferrustry_format::read_save(&save, reg).unwrap();
    let read_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(state_hash(&back), state_hash(&w));

    let t = Instant::now();
    let copy = w.clone();
    let clone_us = t.elapsed().as_secs_f64() * 1e6;
    drop(copy);
    // скользящая контрольная сумма: 150 частей (5 с при 30 Hz), цена одной части за тик
    let mut worst = 0.0f64;
    let mut total = 0.0;
    for k in 0..150 {
        let t = Instant::now();
        std::hint::black_box(w.checksum_part(k, 150));
        let us = t.elapsed().as_secs_f64() * 1e6;
        worst = worst.max(us);
        total += us;
    }
    println!(
        "{name:<8} world.clone(): {clone_us:6.0} us; rolling checksum part: mean {:5.0} us, worst {worst:5.0} us (all 150: {total:6.0} us)",
        total / 150.0
    );
    let mut prev = postcard::to_allocvec(&w).unwrap();
    let (mut ticks, mut changed, mut delta) = (0usize, 0usize, 0usize);
    let (mut ser_us, mut hash_us) = (0.0, 0.0);
    for _ in 0..30 {
        w.step(reg);
        let t = Instant::now();
        let cur = postcard::to_allocvec(&w).unwrap();
        ser_us += t.elapsed().as_secs_f64() * 1e6;
        let t = Instant::now();
        std::hint::black_box(state_hash(&w));
        hash_us += t.elapsed().as_secs_f64() * 1e6;
        if cur.len() == prev.len() {
            let diff: Vec<u8> = cur.iter().zip(&prev).map(|(a, b)| a ^ b).collect();
            changed += diff.iter().filter(|&&d| d != 0).count();
            delta += zstd::bulk::compress(&diff, 3).unwrap().len();
            ticks += 1;
        }
        prev = cur;
    }
    let n = ticks.max(1);
    println!(
        "{name:<8} state {:>5} KB; snapshot {:>5} KB (write {write_ms:5.1} ms, read {read_ms:5.1} ms); \
         per tick: {:>6} bytes change, delta >= {:>6} B = {:>6.0} KB/s per client; \
         serialize {:6.0} us, state_hash {:6.0} us ({ticks}/30 ticks)",
        prev.len() / 1024,
        save.len() / 1024,
        changed / n,
        delta / n,
        (delta / n) as f64 * 30.0 / 1024.0,
        ser_us / 30.0,
        hash_us / 30.0
    );
}

fn main() {
    let reg = ContentRegistry::base().unwrap();
    measure(
        "B3-10k",
        &reg,
        scenario::bench_megabase(&reg, 10_000).unwrap(),
    );
    measure(
        "B3-100k",
        &reg,
        scenario::bench_megabase(&reg, 100_000).unwrap(),
    );
    measure(
        "B5-1000",
        &reg,
        scenario::bench_battle(&reg, 1000, 300).unwrap(),
    );
    let cmd = Command::Place {
        block: reg.block_by_name("conveyor").unwrap(),
        origin: TilePos::new(1234, 567),
        rot: Dir::Up,
    };
    let bytes = postcard::to_allocvec(&(12_345u64, 3u8, &cmd)).unwrap();
    println!(
        "command (tick, player, Place): {} B; 8 players x 5 commands/s = {:.2} KB/s",
        bytes.len(),
        bytes.len() as f64 * 40.0 / 1024.0
    );
}
