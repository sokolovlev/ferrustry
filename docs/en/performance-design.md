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
| 5. Parallelism | Independent islands on different cores, deterministically | — | Splitting logistics into islands + two-phase transfer (P-06) |
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
| P-03 | Segment arena: hot fields (head, len, compressed, tail, speed) in a separate dense array; items of all segments in one pool | Fewer cache misses per segment | cachegrind: D1 misses per awake segment ↓ |
| P-04 | Building tables in SoA, narrow buffer types | Denser pass over drills/crafters | B3: tick time ↓ |
| P-05 | Bounds check elimination in hot loops | −10% instructions in B2-comb | callgrind |
| P-06 | Parallel processing of logistics islands | Scaling across cores | B3 on 4/8 cores, determinism in CI |
| P-07 (**Done**: B3) | B3 "megabase" scenario: production + logistics + power | A realistic target for P-01…P-06 | Report in `docs/benchmarks/results/` |
| P-09 (**closed: false alarm**) | The suspected B2-comb regression turned out to be an error in the earlier measurement; re-measured: a1dd7ad 237,923, 93ae603 237,299 instructions/tick | — | — |
| P-10 | Turrets: don't shoot a target that projectiles in flight will already kill ("overkill"); incremental flow field instead of a full Dijkstra on every build | Defence efficiency; remove the ~1.5 ms spike | B5: max tick < 500 µs |
| P-08 | Performance regression control in CI (comparison with the previous commit by callgrind instructions — independent of VM noise) | Protection against degradation | +5% instructions threshold → warning |
