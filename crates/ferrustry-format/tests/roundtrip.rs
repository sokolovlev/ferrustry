//! Сохранение → загрузка → продолжение совпадает с непрерывным прогоном.
#![allow(clippy::unwrap_used, missing_docs)]

use ferrustry_content::ContentRegistry;
use ferrustry_format::{read_save, write_save};
use ferrustry_sim::{scenario, state_hash};

#[test]
fn save_load_continue_matches_uninterrupted_run() {
    let reg = ContentRegistry::base().unwrap();
    let (mut a, _) = scenario::vertical_slice(&reg).unwrap();
    for _ in 0..777 {
        a.step(&reg);
    }
    let bytes = write_save(&a, &reg).unwrap();
    let mut b = read_save(&bytes, &reg).unwrap();
    assert_eq!(state_hash(&a), state_hash(&b));
    for _ in 0..1500 {
        a.step(&reg);
        b.step(&reg);
    }
    assert_eq!(state_hash(&a), state_hash(&b));
    assert_eq!(a, b);
}

#[test]
fn large_loops_roundtrip() {
    let reg = ContentRegistry::base().unwrap();
    let mut w = scenario::bench_conveyor_loops(&reg, 2_000).unwrap();
    for _ in 0..100 {
        w.step(&reg);
    }
    let bytes = write_save(&w, &reg).unwrap();
    let loaded = read_save(&bytes, &reg).unwrap();
    assert_eq!(loaded, w);
}

#[test]
fn loaded_world_has_the_same_rolling_checksum() {
    // клиент, вошедший по снимку, должен совпасть с сервером по каждой части суммы (D-20):
    // внутренняя раскладка (пул лент) после загрузки другая, хешируется только каноническое
    let reg = ContentRegistry::base().unwrap();
    let mut w = ferrustry_sim::scenario::bench_megabase(&reg, 3000).unwrap();
    for _ in 0..500 {
        w.step(&reg);
    }
    // перестройки лент оставляют дыры в пуле — раскладка отличается от загруженной
    let belts: Vec<_> = w
        .buildings()
        .iter()
        .filter(|(_, b)| b.tag == ferrustry_sim::KindTag::Conveyor)
        .map(|(_, b)| (b.block, b.origin, b.rot))
        .step_by(7)
        .take(40)
        .collect();
    for &(_, at, _) in &belts {
        w.apply(&reg, &ferrustry_sim::Command::Remove { at })
            .unwrap();
    }
    w.step(&reg);
    for &(block, origin, rot) in &belts {
        w.apply(&reg, &ferrustry_sim::Command::Place { block, origin, rot })
            .unwrap();
    }
    for _ in 0..50 {
        w.step(&reg);
    }
    let bytes = ferrustry_format::write_save(&w, &reg).unwrap();
    let mut back = ferrustry_format::read_save(&bytes, &reg).unwrap();
    let n = 150;
    for k in 0..n {
        assert_eq!(w.checksum_part(k, n), back.checksum_part(k, n), "part {k}");
    }
    for _ in 0..200 {
        w.step(&reg);
        back.step(&reg);
        let k = w.tick_count() % n;
        assert_eq!(w.checksum_part(k, n), back.checksum_part(k, n));
    }
}
