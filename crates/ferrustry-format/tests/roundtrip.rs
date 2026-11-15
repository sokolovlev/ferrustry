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
