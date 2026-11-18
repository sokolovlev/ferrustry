//! Отпечаток наблюдаемого поведения мегабазы B3 (для рефакторингов раскладки данных).
//!
//! Хеш состояния (`state_hash`) зависит от формата сериализации и меняется при любой правке
//! раскладки, даже если игра ведёт себя так же. Этот отпечаток строится только через публичный
//! API — учёт ресурсов, содержимое ядер, лент и буферов, позиции предметов на каждом сегменте,
//! состояние каждой постройки в инспекторе — поэтому собирается одинаково на старой и новой
//! версии. Совпадение вывода двух версий = поведение не изменилось (так проверены P-04 и P-01).
//!
//! `cargo run --release -p ferrustry-headless --example behaviour_fingerprint`
#![allow(missing_docs, clippy::unwrap_used)]

use std::hash::{DefaultHasher, Hash, Hasher};

use ferrustry_content::ContentRegistry;
use ferrustry_sim::scenario;

fn main() {
    let reg = ContentRegistry::base().unwrap();
    for n in [10_000u32, 100_000] {
        let mut w = scenario::bench_megabase(&reg, n).unwrap();
        for t in 1..=9000 {
            w.step(&reg);
            if t % 1500 == 0 {
                let mut h = DefaultHasher::new();
                format!("{:?}", w.stats()).hash(&mut h);
                w.total_core_items().hash(&mut h);
                w.items_on_conveyors().hash(&mut h);
                w.items_in_buffers(&reg).hash(&mut h);
                for s in w.segments() {
                    for p in s.item_positions() {
                        format!("{p:?}").hash(&mut h);
                    }
                }
                for (_, b) in w.buildings().iter() {
                    format!(
                        "{:?}",
                        w.inspect(&reg, b.origin).map(|i| (i.detail, i.health))
                    )
                    .hash(&mut h);
                }
                println!("B3 {n} tick {t}: {:016x}", h.finish());
            }
        }
    }
}
