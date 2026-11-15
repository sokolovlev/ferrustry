//! Микробенчмарки систем симуляции (criterion).
#![allow(missing_docs, clippy::unwrap_used)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use ferrustry_content::ContentRegistry;
use ferrustry_sim::scenario;

fn ticks(c: &mut Criterion) {
    let reg = ContentRegistry::base().unwrap();

    let (mut slice, _) = scenario::vertical_slice(&reg).unwrap();
    c.bench_function("tick/vertical_slice", |b| {
        b.iter(|| slice.step(black_box(&reg)))
    });

    let mut loops = scenario::bench_conveyor_loops(&reg, 10_000).unwrap();
    c.bench_function("tick/b2_conveyor_loops_10k", |b| {
        b.iter(|| loops.step(black_box(&reg)))
    });
}

fn construction(c: &mut Criterion) {
    let reg = ContentRegistry::base().unwrap();
    c.bench_function("build/conveyor_loop_196", |b| {
        b.iter(|| {
            let mut w = scenario::bench_empty(&reg).unwrap();
            let ids = scenario::BaseIds::resolve(&reg).unwrap();
            scenario::build(
                &mut w,
                &reg,
                &scenario::loop_commands(ids.conveyor, 1, 1, 50, 50),
            )
            .unwrap();
            black_box(w.segments().count())
        })
    });
}

criterion_group!(benches, ticks, construction);
criterion_main!(benches);
