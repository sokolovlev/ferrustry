# Performance design: aiming at the Factorio class

Status: **accepted** (2026-11-15). Decisions D-14 (AVX2 baseline), D-15 (`unsafe` for SIMD in an isolated module), D-16 (tuning on the owner's PC) are approved.

The goal is not "N times faster than X" but an engineering property: **tick cost is proportional
to activity, not to base size**, and every unit of activity is processed with the minimum
number of memory accesses and instructions. Public material from the Factorio developers (Friday
Facts: transport lines, sleeping entities, electric networks) is a source of ideas; we take no code
or numbers from it and publish no comparisons without measurements.

## 1. Hierarchy of levers

From the largest effect to the smallest. The order matters: SIMD will not save an algorithm that
visits things that don't change.

| Level | Principle | Done | Next |
|---|---|---|---|
| 1. Algorithm | Don't touch what doesn't change | Segment model; compressed-prefix cache; **segment sleeping**; **predictive wake-up** (P-01); **sleeping drills** (P-02) | Sleeping crafters and belts blocked by crafters/cores |
| 2. Memory access | One cache miss per unit of work | **Link cache** segment→receiver and drill→receivers by topology epoch; bitset of active segments | Segment arena + hot fields in SoA (P-03) |
| 3. Data layout | Dense arrays of narrow types | **Item+gap in one `u32`**, a linear window of exact capacity, no ring indices | `u16`/`u8` for building buffers; SoA for building tables (P-04) |
| 4. Registers and SIMD | Hot loop without calls, branches or bounds checks | Position search in blocks of 16 gaps (auto-vectorization) | Bounds check elimination (P-05); explicit SIMD (D-15) |
| 5. Parallelism | Each thread owns its part of the world, one synchronization per tick | Measurements: the ceiling and the pool cost (section 8) | A world of regions (D-17) |
| 6. Build | LTO, PGO, target CPU | **AVX2 baseline** (`.cargo/config.toml`, CPU check at startup); LTO on the VM is below noise | LTO/PGO — measure on the owner's PC |

## 2. "Processing in registers" — what it means for Rust

LLVM allocates registers; you cannot and need not control them directly in safe Rust.
Our job is to write hot loops so that the compiler **can** keep state in registers
and vectorize the work:

1. **Dense slices instead of pointers.** A loop over `&[u32]` without indirect jumps — data
   arrives in 64-byte cache lines, the prefetcher predicts the stream.
2. **Narrow types.** A gap is a `u16`: a 512-bit AVX-512 register holds 32 gaps, a 256-bit
   AVX2 one 16, a 128-bit NEON one (Apple Silicon, Android) 8.
3. **Local copies of fields.** In `Segment::advance` the counters (`compressed`, `wide`) are copied
   into locals and written back once: the compiler keeps them in registers without
   worrying about aliasing.
4. **Block processing.** Position search sums blocks of 16 gaps
   (`chunks_exact(16).map(sum)`), and only the found block is scanned element by element. LLVM
   turns such a loop into vector additions.
5. **No branches in counters.** `wide += u32::from(cond)` instead of `if`.
6. **Bounds checks.** Currently ~12% of B2-comb tick instructions are checked indexing.
   We remove them via iterators and bounds the compiler can prove (P-05); `get_unchecked` only by
   decision D-15.

Explicit SIMD (`core::arch` with runtime implementation selection) requires `unsafe`.
`std::simd` is only available on nightly, and the project is on stable. Options: decision D-15.

## 3. Determinism and parallelism

Parallelism is allowed only if the result is bit-identical to sequential
execution. Plan (P-06):
- the logistics graph is split into **islands** (connected components of segments, drills, crafters);
- islands are processed in parallel; transfers between islands are deferred into a buffer and
  applied after the phase in a fixed order;
- verification: the same differential test that currently proves sleeping is transparent.

## 4. How we prove an optimization didn't break the game

| Mechanism | What it catches |
|---|---|
| `assert_conserved` in all scenario tests | Lost and duplicated items |
| `World::validate` (including segment counters and link caches) | Broken internal invariants |
| Differential test: world with sleeping vs the reference without sleeping, including random building | Any behaviour change caused by the scheduler |
| Behaviour fingerprint before/after a layout refactor | Behaviour change caused by the refactor |
| Randomized test of segment operations (20,000 operations × 5 lengths) | Bugs in the packed layout |

The differential test has already paid off: it caught a real bug in the processing order of
woken segments (a segment was processed twice in one tick).

## 5. Measurements

Methodology: `docs/en/performance.md`. Tools:
- `ferrustry-headless bench` — tick time, p99, allocations, RSS;
- `cargo bench -p ferrustry-sim` — criterion;
- `valgrind --tool=callgrind --toggle-collect='*World*step*'` — a profile of the tick only;
- `valgrind --tool=cachegrind` — cache model;
- on the reference machine (D-11/D-16): `perf stat` (IPC, L1/LLC misses, branch-misses), Tracy.

## 6. Predictive wake-up (P-01) — how it works

A branch blocked by a moving belt T does not poll it every tick. Instead it:
1. adds itself to T's waiter list together with the insertion position `pos`;
2. computes a **lower bound** of ticks after which space may appear at `pos` if
   T's uncompressed items keep moving uniformly (`Segment::earliest_insert`): the nearest
   "wide" gap behind `pos` or the free tail passes through `pos`;
3. sets a timer in a 256-slot wheel (no allocations) and goes to sleep.

Only events that can open space **earlier** than predicted wake it up early: an item leaving
a belt where some items were standing (movement changes — wake everyone), space freed at the
output (wake those whose position falls into it), a rebuild. Insertions into T and its items
stopping can only reduce space, and the estimate is a lower bound, so an early wake-up is harmless:
the waiter simply tries again.

The guarantees are checked by a property test (the estimate is a lower bound on thousands of random
belts) and by differential tests (world with sleeping ≡ world without, including B3 and random building).

Limitation: belts and drills blocked by crafters and cores don't sleep yet — that needs
"crafter consumed an input" events (next iteration).

## 7. Tasks

| ID | Task | Expected effect | Criterion |
|---|---|---|---|
| P-01 (**Done**) | Precise wake-up on side insertion: a branch sleeps until a gap appears at the insertion point on the target belt | The main remainder of B2-comb: ~12k segments awake doing nothing | B2-comb-100k: awake segments ≤ 10%, differential test green |
| P-02 (**Partial**: drills; crafters — no) | Sleeping drills and crafters with a full buffer and all receivers refusing | Jammed drills stop ticking | Same |
| P-03 (**Done** 2026-11-19: shared item pool; a separate hot-field array was tried and rejected) | Segment arena: hot fields (head, len, compressed, tail, speed) in a separate dense array; items of all segments in one pool | Fewer cache misses per segment | cachegrind: D1 misses per awake segment ↓ |
| P-04 (**Partial** 2026-11-18: crafters and drills) | Building tables in SoA, narrow buffer types | Denser pass over drills/crafters | B3: tick time ↓ — −14% (10k) and −22% (100k), L1 misses −38% |
| P-05 | Bounds check elimination in hot loops | −10% instructions in B2-comb | callgrind |
| P-06 (**Investigated** 2026-11-18, see section 8; implementation awaits D-17) | Parallel processing | Scaling across cores | B3 on 4/8 cores, determinism in CI |
| P-07 (**Done**: B3) | B3 "megabase" scenario: production + logistics + power | A realistic target for P-01…P-06 | Report in `docs/benchmarks/results/` |
| P-09 (**closed: false alarm**) | The suspected B2-comb regression turned out to be an error in the earlier measurement; re-measured: a1dd7ad 237,923, 93ae603 237,299 instructions/tick | — | — |
| P-10 (**Done** 2026-11-18: B5 max 172–410 µs, +7.3% kills) | Turrets: don't shoot a target that projectiles in flight will already kill ("overkill"); incremental flow field instead of a full Dijkstra on every build | Defence efficiency; remove the ~1.5 ms spike | B5: max tick < 500 µs |
| P-08 | Performance regression control in CI (comparison with the previous commit by callgrind instructions — independent of VM noise) | Protection against degradation | +5% instructions threshold → warning |

## 8. Parallelism: what the measurements showed (P-06, 2026-11-18)

**Where the tick goes** (B3, share of time per system, 10k / 100k): belts 44% / 36%, drills 23% / 24%,
crafters and heat 19% / 22%, power and timers 15% / 18%. Parallelizing belts alone gives at most
1.4× on 4 cores; every system has to be split.

**The ceiling** (`examples/parallel_ceiling.rs`): B3-100k split into 4 independent bases of 25k,
one per thread with one synchronization per tick — **3.1–4.1×** on the cloud VM (4 vCPU). Memory
did not become the bottleneck: smaller pieces fit the cache better.

**The fine-grained approach was tried and rejected.** The plan was "two phases inside each system":
each element's own work in parallel, then the exchange with neighbours sequentially. As a first step,
the crafters' power demand was parallelized (5,000 crafters; bit-identical result). Outcome: the phase
became **slower** — 115–149 µs on one thread versus 165–173 µs on four. The causes were measured
(`examples/fork_join_cost.rs`):

| One parallel pool call (8192 simple operations) | Right after the previous one | After 100–600 µs of sequential work |
|---|---|---|
| Pool of 4 threads | 14–20 µs | **63–92 µs** |
| Pool of 1 thread | 13 µs | 25–32 µs |

Between tick phases the pool's threads fall asleep, and every parallel call pays for waking them
(tens of µs on the VM) — as much as the phase's entire work. On top of that, the next sequential phase
pulls rows processed on another core back from that core's cache. The code was rolled back.

**What works** is what the ceiling showed: a thread permanently **owns** its part of the world, with
one synchronization per tick. For the engine this means **a world of regions**:

- the map is split into regions (rectangular blocks of chunks); a building belongs to the region of its
  corner; a region has its own tables, belt segments, buffers;
- the tick: all regions in parallel (thread ↔ region pinned, data does not migrate), then
  sequentially in a fixed order — exchange across borders: an item that left a belt into a
  neighbouring region sits in the border's "mailbox" and is accepted at the start of the next tick
  (a one-tick delay only at borders; deterministic for any number of threads);
- power and heat networks that cross borders: regions compute partial sums, the network is
  reduced after the phase (integer sums — order does not matter);
- the core accepts items from other regions through the same mailbox;
- combat stays global and sequential for now; small worlds are one region, without threads.

This is a large rework of world storage (building identifiers, commands, saves, rendering,
the future network), so it is put up as decision D-17. Expectation from the measurements: up to ~3× on large
bases on 4 cores; no change on bases under ~10k belts.

**Measurements on the owner's PC** (i3-8350K, 4 cores / 4 threads, 8 MB L3, Windows 10):

| Measurement | Cloud VM | Owner's PC |
|---|---|---|
| Ceiling, 4 threads with their own data | 3.1–4.1× | **2.6×** (2 threads — 1.7×) |
| Pool call right after the previous one | 14–20 µs | **7 µs** |
| Pool call after 100 µs of work | 63–92 µs | **13 µs** |
| Pool call after 600 µs of work | 63–92 µs | **43 µs** |

On real hardware the pool wakes 2–7 times cheaper, but the ceiling is lower too (all cores share an
8 MB L3). Fine-grained phase parallelism is still bounded by Amdahl's law here (~1.5× with a ~50%
parallel share) — not recommended. The options for D-17 are in decisions.md.

Before deciding D-17, **P-04** was done (2026-11-18). Before it: the crafters' demand
calculation is bound by memory latency (recipe inputs and outputs are separate heap `Vec`s, ~30 ns per
crafter), and dense pointer-free rows speed up both a single thread and future regions.

**P-04 result (crafters and drills):** the hot loops read only a dense table row. Recipe buffers
are arrays in the row instead of a heap `Vec` (a recipe has up to 4 items per side, checked by the
content validator). Rows hold copies of what used to be read from the large building record and the
registry: building kind, power network, heat network, drill constants; `World::validate` checks the
copies against the source. Drills find their row through a "slot → row" scratch table rebuilt when
the topology changes. Behaviour did not change — the fingerprint
(`examples/behaviour_fingerprint.rs`) matches on 10k and 100k up to 9000 ticks.

| B3 | Before | After | Change |
|---|---|---|---|
| Instructions/tick, 10k / 100k | 379,670 / 3,763,883 | 334,806 / 3,319,853 | −11.8% |
| L1 data misses/tick, 100k | 137,548 | 85,459 | **−37.9%** |
| Tick time, median of 5 runs, 10k / 100k | 26.7 / 613 µs | 23.0 / 480 µs | **−14% / −22%** |

Next for P-04: the "belt → building" link can carry the receiver's row number (it changes only on
removal, which already resets the link cache), so handing an item to a crafter or a core will not
read the building record either.

**P-04, second step (2026-11-18): what worked and what did not.**

- *A "belt → building" link carrying the receiver's row number* — built, measured, **rolled back**:
  L1 misses −0.8%, time unchanged. Handing an item to a building is rare in B3 (a crafter receives
  an input once every few dozen ticks), so reading the building record there is not hot. The
  expectation was wrong.
- *Merging the passes over power networks*: B3-100k has about 5,000 networks of 64 bytes, and the
  balance walked them 4 times per tick — now 2. Same behaviour and state hash. L1 misses (cachegrind)
  −11.7%, time: −3% at 10k, within VM noise at 100k. The difference is that cachegrind does not
  model hardware prefetching, and the CPU prefetches a sequential pass over an array well.
- *Checks in `World::validate`*: the live link caches (building output, belt output) and the
  "slot → drill row" table must match a fresh calculation. They catch a whole class of "stale cache"
  bugs; a test corrupts a cache and expects an error.

**Where the misses are now (cachegrind, B3-100k):** belts — two memory accesses per segment per
tick: the segment record (`len`) and a **separate heap buffer of items** (`slots[head]`). Segment
records lie contiguously and are prefetched; the buffers sit at random heap addresses where
prefetching does not help. The next step is **P-03**: the items of all segments in one contiguous
pool in segment order, the segment's hot fields in a dense array. A separate rework of
`segment.rs` with its own randomized tests.

**P-03: a shared item pool for belts (2026-11-19).**

The hypothesis was checked before the rework: if the item buffers of all segments are re-allocated
contiguously in index order, the B3-100k tick is ~11% faster (585 → 520 µs). Then the pool:

- `Segments` in `segment.rs`: segment records by index and **one item array**; each segment has its own
  range of exact capacity. A new segment gets a range at the end of the pool; when freed ranges
  exceed a quarter of the pool, it is compacted in index order. The tick visits segments in
  increasing index order, so items are read almost sequentially and the CPU prefetches them.
- The segment record keeps only metadata; items are handled through the `SegRef`/`SegMut` views
  (record + its slice of the pool). The belt logic was moved unchanged.
- The save format did not change: a segment is written as the same list of `(item, gap)`. The state
  hash **matches bit for bit** with the previous version on B2, B2-comb, B3, B5, B8; so does the
  behaviour fingerprint.
- The first version slowed down a small base (B3-10k +6.5%): after switching to the pool, a belt
  update created a segment view three times and always read the fields for the sleep decision.
  Two views instead of three and a lazy check brought instructions back to the previous level.

| Scenario (median of 5 alternating runs) | Before | After |
|---|---|---|
| B2-comb 10k | 13.7 µs | **11.1 µs (−19%)** |
| B2-comb 100k | 249 µs | **200 µs (−20%)** |
| B3 100k | 512 µs | 489 µs (−5%) |
| B3 10k, B2 100k | 21.8 / 33.5 µs | unchanged |

Instructions are almost the same (+0.2…0.5%), misses in the cachegrind model −2…3% — yet the time
is 5–20% lower: the gain comes from hardware prefetching of the sequential pool, which cachegrind
does not model. Lesson for the methodology: for layout changes, time on hardware decides, and
cachegrind shows only a lower bound of the effect.

Tests: pool compaction over 4,000 random steps does not change contents and lays ranges out by
index; 400 rebuilds of belts carrying items — no item appeared or vanished, the pool was compacted.

**Hot fields of the segment record — tried and rejected.** A record is 104 bytes; the compiler
already groups the hot fields (offsets 56–100), but with a 104-byte stride that block often crosses a
cache line boundary. Cheap experiment: `#[repr(C, align(64))]` with the hot fields first — the block
always fits one line, the record is 128 bytes. Result: worse — B3-100k median 392 → 519 µs,
B2-comb-100k 206 → 226, B3-10k 21.9 → 22.0. Records are already read sequentially and prefetched; the
extra 23% of memory costs more than the saved lines. A full split into hot and cold arrays would give a
gain of the same nature, so it is not done.

One small improvement is kept: a segment's last tile (a separate heap array) is read only when the
link is recomputed, not on every item hand-off — instructions −1.3% (B2-comb-10k) and −0.7% (B3-10k),
time within noise.
