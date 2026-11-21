# Performance

Status: the methodology is defined; there are **measurements of B1–B3, B5 and B8** on a cloud VM and on two of the owner's PCs (section 6). The numbers in section 4 are targets, not results.

## 1. Principles

- No speed-up claims without reproducible measurements.
- All benchmarks run from the repository with one command and save their result to
  `docs/benchmarks/results/<date>-<commit>-<machine>.json`.
- Recorded: commit, rustc version, build profile, OS, CPU, RAM, GPU, driver, tick rate.

## 2. Scenarios

Scenarios are Ferrustry maps generated deterministically from a seed (`ferrustry-headless bench <name>`).

| ID | Scenario | What it stresses | Stage |
|---|---|---|---|
| B1 | Empty 512×512 map | Tick overhead | 1 |
| B2 | 10,000 / 50,000 conveyors with items | Logistics | 1 (B2-10k), 3 |
| B3 | Production base: 2,000 machines, 20,000 conveyors, 5 power networks | Production + power + logistics | 3 |
| B4 | 500 / 2,000 units walk to the core | Flow fields, movement, collisions | 4 |
| B5 | Mass battle: 1,000 units, 300 turrets, thousands of projectiles | Combat, spatial grid | 4 |
| B6 | 2048×2048 map with active logistics | Memory, chunks, cache | 5 |
| B7 | Dedicated server, 8 and 32 bot clients (`ferrustry-headless net-bench`) | Networking, serialization | 6 |
| B8 | The B5 battle with a wall placed or removed on the enemies' path every 5 ticks | Local flow field repair | 4 |

## 3. Metrics

| Metric | How we measure |
|---|---|
| Tick time (mean, p50, p99, max) | `std::time::Instant` around the tick, a histogram over 10,000 ticks after warm-up |
| Stability | p99/p50, number of ticks over budget |
| FPS and frame time (client) | CPU frame time + GPU timestamp queries (where available) |
| RAM | peak RSS (Linux: `/proc/self/status`; Windows/macOS: system APIs) |
| CPU | user+sys process time per run |
| Allocations | a counter via a custom `GlobalAlloc` in the bench build; target: 0 allocations in a steady tick |
| System micro-benchmarks | `criterion` |
| Profiling | `tracy` (via `tracing-tracy`), `perf`/`cargo flamegraph` |
| Networking (B7) | bytes/s per client, snapshot/delta serialization time |

## 4. Preliminary targets (not promises)

| Scenario | Target on the reference machine (to be defined, D-11) |
|---|---|
| B1 | tick < 0.1 ms |
| B2-10k | tick < 1 ms, 0 allocations per tick |
| B3 | tick < 5 ms |
| B5 | tick < 10 ms at 30 Hz |

Targets are revised after the first measurements in stage 1.

## 5. Comparison with other games

A head-to-head comparison of Ferrustry with other games of the genre is not valid: different
mechanics and amounts of work per tick. If a comparison is ever published, it will only be with an
explicit methodology, identical hardware and a caveat about the differences in mechanics. The main
tool is **regression control of our own performance** in CI (comparison with the previous commit,
a degradation threshold).

## 6. Results

### 2026-11-15 — first slice (stage 1)

Conditions: Linux x86_64, cloud VM "Intel(R) Xeon(R) Processor", 4 logical CPUs,
rustc 1.97.0, release profile, 30 Hz, 300 warm-up ticks + 3000 measured.
This is **not the reference machine** (D-11): a cloud VM with neighbours, high max variance.
Raw reports: `docs/benchmarks/results/2026-11-15-*.json`.

| Scenario | Buildings | Items on belts | Tick mean | p99 | max | Allocations/tick | Peak RSS |
|---|---|---|---|---|---|---|---|
| B1 empty 512×512 | 0 | 0 | 0.02 µs | 0.03 µs | 0.05 µs | 0 | 7.3 MiB |
| B2 10k conveyors (52 loops) | 10,192 | 20,436 | 0.82 µs | 1.2 µs | 18 µs | 0 | 6.5 MiB |
| B2 50k conveyors (256 loops) | 50,176 | 100,608 | 3.9 µs | 6.0 µs | 27 µs | 0 | 18.8 MiB |

Criterion (`cargo bench -p ferrustry-sim`): `tick/b2_conveyor_loops_10k` ≈ 0.81 µs,
`build/conveyor_loop_196` ≈ 3.2 ms (includes creating the 512×512 map).

**How to read these numbers:**
- B2 in its current form is the **best case** for the segment model: each loop is one long
  segment, items are not blocked, so tick cost ≈ O(segments) (52 and 256).
  This confirms that cost does not grow with item count, but says nothing about real bases
  with short segments, branches and blockages.
- The target "B2-10k: 0 allocations in a steady tick" is met.
- B1 is almost empty: no system visits tiles yet, which is expected.
- **Building** a long belt is quadratic (each placement rebuilds the neighbouring segment
  entirely): ~3 ms per 196-tile loop. Acceptable for stage 1; optimization is S3.


### 2026-11-15 — first logistics optimization iteration

Realistic scenarios added:
- **B2-blocked** — straight 100-tile belts into a dead end, completely full;
- **B2-comb** — "combs": a 99-tile main belt into the core and 33 branches of 6 tiles from drills
  merging from the side. The main belt saturates, the branches stall — a mix of movement and jams.

Same conditions (cloud VM, 4 vCPU, rustc 1.97.0, release). VM noise is high (run-to-run spread
up to 40%), so the final row is the median of 3 runs; intermediate steps are single runs.
Time is the mean tick, µs.

| Step | Change | B2 10k | B2 100k | B2-blocked 100k | B2-comb 10k | B2-comb 100k |
|---|---|---|---|---|---|---|
| 0 | Starting point | 1.1 | 8.3 | 15.8 | 171 | 2,267 |
| 1 | "Wide" gap counter, fast head/tail insertion paths | 0.8 | 8.9 | 16.6 | 149 | 2,508 |
| 2 | Segment length ≤ 32 tiles | 6.1 | 106 | 184 | 60 | 1,123 |
| 3–4 | Segment sleeping + in-place processing (no copying) | 4.5 | 99 | **0.1** | 58 | 1,101 |
| 5 | Link cache by topology epoch (segments and building output) | 3.4 | 36 | 0.1 | 29 | 667 |
| 6 | Packed linear layout + block search | 3.6 | 41 | 0.1 | 25 | 554 |
| **Result** (median of 3) | | **3.6** | **36** | **0.1** | **30** | **577** |

Conclusions:
- **B2-comb (realistic): 3.9× faster** (2,267 → 577 µs for 100k conveyors and 11k drills).
- **B2-blocked: 158× (15.8 → 0.1 µs)** — jams no longer cost anything.
- **B2 (best case — long free loops): 4.3× slower** (8.3 → 36 µs).
  This is the deliberate price of the segment length cap: there are 7× more segments. Per unit,
  ≈ 10 ns per awake segment; task P-03 targets exactly this.
- Allocations in a steady tick are still 0.
- LTO + `codegen-units=1` and `target-cpu=native` were tried: the effect is within VM noise
  (−10% to +38% depending on the scenario). Not enabled; to be decided on the reference machine (D-14, D-16).

Behaviour did not change with any of the optimizations: steps 3–5 were verified by a differential
test against the reference without sleeping, step 6 by matching a fingerprint of the observable
state before/after on 4 scenarios of 3000 ticks each. Details in `docs/en/performance-design.md`.

### 2026-11-15 — second iteration: precise wake-ups (P-01), sleeping drills (P-02), megabase (P-07)

New:
- **B3 "megabase"** — rows of 12 production cells (2 drills → belt → smelter on wind turbines →
  belt) on a shared bus into the core; 100k conveyors ≈ 10k drills, 5k smelters, 10k wind turbines.
- The base CPU level is **AVX2** (x86-64-v3, D-14) for all measurements below.
- **Steady state**: 6000 warm-up ticks. At 2000 ticks B3 is still filling up
  (1198 vs 627 µs) — these are different phases of the game; we compare the steady state.

A metric independent of VM noise is **instructions per tick** (callgrind, the difference between
600- and 300-tick runs after a 6000-tick warm-up), size 10k:

| Scenario | Before P-01/P-02 | After | Change |
|---|---|---|---|
| B2-comb 10k | 436k | ~~192k~~ **238k** | ~~−56%~~ **−45%** |
| B3 10k | 471k | ~~398k~~ **400k** | **−15%** |

> **Correction of 2026-11-17.** The "after" values in this table were measured incorrectly (probably
> on a different build). Re-measured with the same method on commits 4cefe75 (before) and a1dd7ad (after):
> B2-comb 436,412 → 237,923, B3 470,778 → 400,061. Because of the wrong number a false
> "regression" P-09 was filed — there was none: 93ae603 gives 237,299, the same as a1dd7ad.

Tick time, median of 3 runs, steady state, 100k:

| Scenario | Before | After | Awake (after) |
|---|---|---|---|
| B2-comb | 551 µs | **196 µs** (2.8×) | 4.7k of 12.5k segments; 3.4k of 11k drills |
| B3 | 1,244 µs | 1,090 µs (within noise: runs 689–1257) | 10.4k of 11.3k segments; 8.3k of 10k drills |

In B3 almost all segments and drills do real work (items move, smelters are not clogged),
so sleeping helps little there — next steps are P-03/P-05 (cheaper unit of work) and P-06
(threads). B3 is also clearly memory bound: at 10× the size there are 10× more instructions,
but 25–50× more time.

Found and fixed along the way: a leak in the waiter lists (a waiter woken by a timer
re-registered itself) caused allocations in the tick and a slowdown; registration is now
idempotent, and a steady tick is back to 0 allocations.

Full suite on the VM (`scripts/bench-suite.sh`, 37 s): `docs/benchmarks/results/2026-11-15-vm-suite.json`.

## 7. Running on your PC

```bash
scripts/bench-suite.sh            # Linux / macOS, ~1 min
```
```powershell
powershell -ExecutionPolicy Bypass -File scripts\bench-suite.ps1   # Windows
```
The result is a `bench-results/<date>-<commit>/` folder (`suite.json`, `machine.txt`, and on Linux with
`perf` — `perf.txt` with IPC/cache counters). Its contents go into
`docs/benchmarks/results/` or are sent in chat. Before running: close heavy programs,
plug in power (laptops), don't touch the machine during the run.

### 2026-11-16 — the owner's reference machine (D-16)

Intel Core i3-8350K (4C/4T, 4.0 GHz, 256 KB L2/core, 8 MB L3), 8 GB, Windows 10, rustc 1.100.0,
AVX2, commit 70734c7. Files: `docs/benchmarks/results/2026-11-16-owner-pc-*`.
Run-to-run spread < 2% (up to 40% on the cloud VM): fine tuning is now possible.

| Scenario | Size | mean, µs | p99, µs | VM mean, µs |
|---|---|---|---|---|
| B1 | 512² | 0.04 | 0.10 | 0.02 |
| B2 loops | 10k / 100k | 4.6 / 51.9 | 8.3 / 94.4 | 3.5 / 33.3 |
| B2-blocked | 10k / 100k | 0.05 / 0.10 | 0.10 / 0.20 | 0.03 / 0.09 |
| B2-comb | 10k / 100k | 21.0 / 220 | 72.5 / 775 | 12.0 / 260 |
| B3 megabase | 10k / 100k | 45.4 / **671** | 79.6 / 1,232 | 27.3 / 703 |

- B3-100k (≈ 125k buildings) is **2% of the tick budget** (33.3 ms at 30 Hz).
- On small scenarios the PC is slower than the VM (the VM's server CPU has several times larger
  caches); on large ones they are on par: both machines are memory bound. For this class of
  hardware P-03 (data density) matters more than for the VM.
- Peak RSS on Windows was not measured in this run; it has been added (current RSS at the end of the run).

### 2026-11-17 — heat and fluids

Instructions per tick (callgrind, 10k, steady state) before/after heat and fluids:
B3 404,864 → 412,049 (+1.8%), B2-comb 237,299 → 236,729 (no change).

The first version of the heat system visited **all** buildings in the world every tick (B3-100k: 651 → 1652 µs);
the benchmark caught it before the commit and it was fixed — only heat network members are visited,
and with no heat networks the system does not run at all.

~~Found during comparison: B2-comb grew from 192k to 237k instructions/tick (P-09).~~
False alarm: the original 192k was a measurement error (see the correction above).

### 2026-11-17 — combat (stage 4)

**B5 "mass battle"** (`bench b5`): a 160×160 map, the core in the centre, 300 turrets with full
ammo, 1000 enemies (every fifth is heavy) from the map edges; the first 1500 ticks with no warm-up
(march + battle; by the end 626 enemies are destroyed — the same in every run).

| Metric | Value (cloud VM) |
|---|---|
| Tick mean / p50 / p99 | 111 / 113 / 142 µs |
| Tick max | ~1.5 ms — the first flow field build (Dijkstra over 25,600 tiles) |
| Allocations per tick | 0.04 (projectile buffers growing at the start of the battle) |

Combat did not change instructions per tick for B2-comb and B3 (with no units the system does not run).

### 2026-11-17 — content v0.4 (fuel, depot, arc tower, new enemies)

Instructions per tick (callgrind, the difference between 600 and 300 ticks), before → after:

| Scenario | Before | After | Change |
|---|---|---|---|
| B3 megabase 100k | 4,022,822 | 3,763,741 | **−6.4%** |
| B3 megabase 10k | 405,627 | 379,643 | **−6.4%** |
| B5 battle (10,000 enemies, the `bench b5` default; 1000 in `suite`) | 24,290,016 | 24,022,291 | −1.1% |

The first version of the burner gave **+1.3%** on B3: the generator loop became heavier.
The fix went further than "put it back": the output of fuel-less generators is constant, so
it is computed once per power network rebuild (`PowerNet::steady`), and only fueled generators
are visited each tick. As a result B3 is 6.4% faster than before. The B5 kill count (2397) did not change.

### 2026-11-18 — P-10: overkill and local flow field repair

Cloud VM, 1000 enemies against 300 turrets, the first 1500 ticks with no warm-up, 3 runs.

| Scenario | Metric | Before | After |
|---|---|---|---|
| B5 | killed in 1500 ticks | 626 | **672** (+7.3%) — same ammo |
| B5 | tick mean / max | 114 / ~1,500 µs | 112–129 / **172–410 µs** |
| B8 | tick mean | 397–417 µs (full rebuild) | **127 µs** (3.2×) |
| B8 | tick p99 | 1,576–2,129 µs | **256–262 µs** (6–8×) |
| B8 | tick max | 1,795–3,687 µs | **284–301 µs** |

- "Before" for B8 is the same binary with `--full-flow` (the field is rebuilt entirely on every
  change). The state hash and the kill count **match** in both modes — the repair does not change the game.
- The max spike in B5 was the first field build inside a tick. Now the field is built when the
  map is created (`World::prepare_flow`; 1.6 ms once, outside the tick) and only repaired afterwards.
- The economy is unaffected: B3 10k — 379,643 → 379,658 instructions/tick.

### 2026-11-18 — P-06: the multithreading ceiling and the pool cost

Cloud VM, 4 vCPU. Detailed analysis and conclusion: `docs/en/performance-design.md`, section 8.

| Measurement | Result |
|---|---|
| B3-100k as 1 / 2 / 4 independent bases in their own threads (`parallel_ceiling`) | 587 / 415 / 187 µs per tick of the whole base — **3.1×** on 4 threads (up to 4.1× in other runs) |
| One rayon pool call after 100–600 µs of sequential work (`fork_join_cost`) | 63–92 µs (right after the previous one — 14–20 µs) |
| Crafters' power demand, B3-100k: 1 thread → 4 threads | 115–149 → 165–173 µs — **slower**, code rolled back |

Running on your PC:

```bash
cargo run --release -p ferrustry-headless --example parallel_ceiling -- 100000
cargo run --release -p ferrustry-headless --example fork_join_cost
```

### 2026-11-18 — measurements on the owner's PC and P-04

Owner's PC (i3-8350K, Windows 10): the `parallel_ceiling` B3-100k ceiling is 637 / 373 / 245 µs on
1 / 2 / 4 threads (**2.6×**); `fork_join_cost` — 7 / 13 / 43 µs after 0 / 100 / 600 µs of work.
Analysis: `docs/en/performance-design.md`, section 8.

P-04 (dense crafter and drill rows), cloud VM, behaviour unchanged (fingerprint):

| B3 | Before | After |
|---|---|---|
| Instructions/tick, 10k / 100k | 379,670 / 3,763,883 | 334,806 / 3,319,853 (−11.8%) |
| L1 data misses/tick, 100k | 137,548 | 85,459 (−37.9%) |
| Tick time (median of 5 alternating runs), 10k / 100k | 26.7 / 613 µs | 23.0 / 480 µs (−14% / −22%) |

### 2026-11-18 — P-04, second step

Cloud VM, B3, behaviour and state hash unchanged. Details: `docs/en/performance-design.md`.

| Change | L1 misses/tick, 100k | Tick time |
|---|---|---|
| Link carrying the receiver's row (**rolled back**) | 85,464 → 84,785 (−0.8%) | unchanged |
| Merging the power network passes (4 → 2) | 85,464 → 75,451 (−11.7%) | 10k: 22.6 → 22.0 µs; 100k: within noise |

### 2026-11-19 — P-03: a shared item pool for belts

Cloud VM, medians of 5 alternating runs; state hash and behaviour unchanged.

| Scenario | Before | After |
|---|---|---|
| B2-comb 10k / 100k | 13.7 / 249 µs | 11.1 / 200 µs (−19% / −20%) |
| B3 10k / 100k | 21.8 / 512 µs | 21.8 / 489 µs (0 / −5%) |
| B2 100k | 33.5 µs | 33.5 µs |
| Instructions/tick B3-100k / B2-comb-100k | 3,304,181 / 2,912,672 | 3,309,027 / 2,927,852 |

### 2026-11-19 — P-03: segment hot fields (rejected)

| Experiment | B3-100k | B2-comb-100k | B3-10k |
|---|---|---|---|
| Segment record `repr(C, align(64))`, hot fields first (**rolled back**) | 392 → 519 µs | 206 → 226 µs | 21.9 → 22.0 µs |
| Last tile read only when the link is recomputed (kept) | within noise | within noise | instructions −0.7% |

### 2026-11-20 — the owner's second PC: "before/after" on the same hardware

Intel Core i5-12400F (6C/12T, L2 1.25 MB/core, L3 18 MB), 16 GB, Windows 11, rustc 1.100.0, AVX2.
The version from 16.11 (`70734c7`, v0.1.0) and the current one (`c558c20`, v0.7.0) were run on the same machine,
`bench-suite.ps1`, median of 3 runs. Files: `docs/benchmarks/results/2026-11-20-pc2-*`.

| Scenario | Size | 70734c7, µs | c558c20, µs | Change |
|---|---|---|---|---|
| B1 | 512² | 0.03 | 0.03 | — |
| B2 loops | 10k / 100k | 3.0 / 32.6 | 3.0 / 34.3 | within noise (runs spread 32–36) |
| B2-blocked | 10k / 100k | 0.03 / 0.08 | 0.03 / 0.09 | at the timer's resolution |
| B2-comb | 10k / 100k | 13.6 / 173 | 11.1 / **128** | −18% / **−26%** |
| B3 megabase | 10k / 100k | 25.1 / 399 | 20.4 / **277** | −19% / **−31%** |
| B5 combat | 1000 enemies | — | 104 | the scenario didn't exist yet |
| B8 combat + building | 1000 enemies | — | 117 | the scenario didn't exist yet |

- B3 is the same world in both versions: 125,517 buildings, exactly 848,595 items reached the cores during the run.
  The new version does more every tick (heat, fluids, noise, combat) and is still 31% faster.
  The gain comes from P-04 (dense crafter and drill rows) and P-03 (a shared belt item pool).
- B3-100k is **0.8% of the tick budget** (33.3 ms at 30 Hz); p99 is 0.8 ms.
- No allocations in a steady tick (B5/B8: 0.03 / 0.26 per tick — waves and building).
- Hardware: the same old version is 1.3–1.7× faster on the i5-12400F than on the i3-8350K (B3-100k 671 → 399 µs):
  large scenarios are memory-bound, and a bigger cache helps more than clock speed.
- Caveat: both versions came from archives without `.git`, so the result folders record the commit as `unknown`;
  the runs were a few minutes apart, and the spread within each is up to 5%.

### 2026-11-20 — the multithreading ceiling and the client on the i5-12400F

`parallel_ceiling` (B3-100k split into K independent bases, each on its own thread, synchronized once per tick):

| Threads | 1 | 2 | 4 | 6 | 8 | 12 |
|---|---|---|---|---|---|---|
| Tick of the whole base, µs | 270.0 | 156.6 | 105.5 | 81.3 | **77.0** | 95.8 |
| Speed-up | 1.00× | 1.72× | 2.56× | 3.32× | **3.50×** | 2.82× |

- On 6 physical cores the ceiling is **3.3×**; the cores' second threads (SMT) add a little more (3.5× at 8),
  and at 12 threads it gets worse: the cores share cache and memory, and the once-per-tick synchronization waits
  for the slowest thread. As on the i3-8350K (2.6× on 4 cores), memory sets the limit, not the core count.
- `fork_join_cost` (a pool of 12 threads): 11.7 µs per call without a pause, **35.7 µs** after 100 µs of
  sequential work, **60.3 µs** after 600 µs — the threads fall asleep, and waking them costs a third to a half
  of a B3-100k tick phase. Fine-grained phase parallelism won't pay off here either.
- Conclusion for D-17: multithreading by regions would give up to ~3.3× on this PC, but a single thread already
  spends only 0.8% of the budget on the 100k megabase. The recommendation "A now, B at stage 6" stands.

Client (large 320×200 map): **280 FPS**, zoomed all the way out **230 FPS** (the floor is assembled on the CPU
every frame, R-01). That is almost four times the 60 FPS target, so the GPU floor cache (R-01) is not urgent.

### 2026-11-20 — local power network rebuilds (S3-02)

`examples/power_rebuild_cost.rs`: a tick without building and a tick in which a wind turbine is placed/removed (VM, slower than the owner's PC).

| Scenario | Before (full rebuild) | After |
|---|---|---|
| B3-10k + a grid of 1682 power nodes (one shared network) | 12,604 µs per tick with building (38% of the budget) | 31 µs (power ~11 µs) |
| B3-100k (5004 networks, no nodes) | 2,514 µs, of which power ~2,170 | 591 µs; power ~15 µs — within noise |
| B3-100k + a grid of 16,682 nodes | seconds per tick (quadratic cost) | 564 µs; power within noise |

- The old rebuild: every node scanned all power buildings — a cost quadratic in their number.
- The remaining ~230 µs on B3-100k is the cost of **any** building, even a wall (link caches reset by the
  topology epoch, etc.) — a separate task; it is 0.7% of the tick budget and only in ticks with building.

### 2026-11-21 — checksums for multiplayer (S6-04)

`examples/net_estimates.rs`, cloud VM:

| | B3-10k | B3-100k | B5-1000 |
|---|---|---|---|
| `state_hash` (full): was FNV over the bytes of a buffer → now words straight from serialization | 2.4 → 2.1 ms | 28 → 19 ms | 0.35 → 0.31 ms |
| Copying the world (`clone`) | 0.8 ms | 11.8 ms | 0.06 ms |
| Rolling checksum: part per tick, mean / worst | 15 / 61 µs | **151 / 206 µs** | 3 / 19 µs |

- The main cost of a full hash is walking the structures during serialization, not the hash itself; so neither a
  fast hash nor hashing a copy in the background (a megabase copy takes 11.8 ms) removes the spike. The rolling
  checksum splits the work over 150 ticks: no spike, the mean cost is ~0.5% of the tick budget.

### 2026-11-21 — B7: a dedicated server under bot load (S6-05)

`ferrustry-headless net-bench`: bots connect over real UDP (renet, 127.0.0.1), join through a snapshot, each send
5 build commands per second and read everything the server sends (they don't simulate the world). The server runs
at 30 Hz in real time, 15 s measured. Cloud VM, 4 cores; the bots share the CPU with the server. Files:
`docs/benchmarks/results/2026-11-21-vm-b7-*.json`.

| Clients | Base | Server frame: mean / p99 / worst | Network in the frame | Payload → client | With headers → / ← | All joined |
|---|---|---|---|---|---|---|
| 8 | B3-10k (12.6k buildings) | 0.21 / 0.57 / 0.63 ms | 0.08 ms | 0.39 KB/s | 0.86 / 0.48 KB/s | 0.25 s |
| 8 | B3-100k (125k) | 1.18 / 4.4 / 5.4 ms | 0.11 ms | 0.39 KB/s | 0.86 / 0.48 KB/s | 0.66 s |
| 32 | B3-10k | 0.47 / 0.95 / 1.05 ms | 0.28 ms | 1.24 KB/s | 1.74 / 0.49 KB/s | 0.26 s |
| 32 | B3-100k | 1.47 / 5.0 / 7.2 ms | 0.31 ms | 1.36 KB/s | 1.75 / 0.49 KB/s | **1.76 s** |

- **Traffic** is as the design predicted (D-18): ~1–2 KB/s per client for any base. It grows with the number of
  players because a frame carries everyone's commands: 32 players — 56 KB/s of server upload in total.
- **Networking** (receiving, broadcasting, acks) — 0.1–0.3 ms per frame.
- **The main cost is building**: a megabase tick under 40–160 commands per second is ~1.1–1.2 ms versus 0.29 ms
  without building. That is the cost of any building on a large base (~230 µs, link caches reset by the topology
  epoch, see 2026-11-20) — in a multiplayer game with active players it is already noticeable. p99 up to 5 ms —
  15% of the tick budget.
- **32 players joining a megabase take 1.76 s**: the server writes a snapshot **for each** joining player (~47 ms)
  and stands still while writing. The fix is S6-03: one snapshot per tick for all joining players, written in the
  background.

### 2026-11-21 — the cost of any building and the "cold" tick

**Building.** On B3-100k a tick in which any building is placed or removed (even a wall) cost ~300 µs more than a
normal one. Profile (callgrind, `World::tick` only): +0.95 M instructions per such tick, of which **0.5 M was
clearing the "slot → drill row" table** (125k entries, ~500 KB): it was rebuilt on any topology change, although it
only changes when drills are built or removed. Now the table is maintained in place (when a drill is removed the
moved row is updated) and built in full once after a world is created or loaded.

| B3-100k, a tick with a wall placed | Before | After |
|---|---|---|
| Extra instructions per tick (callgrind) | +0.95 M | **+0.37 M (−61%)** |
| Extra time per tick (VM) | ~300 µs | ~100–120 µs |

What remains is recomputing the buildings' dump caches (`dump`/`resolve_link`) after the topology epoch changes.
Check: random building and removal of drills (with rows moving) mixed with walls, `validate` every tick and a
bit-for-bit state match with a reference that rebuilds the table every tick; without updating the moved row the
test fails on tick 2.

**The "cold" tick.** B7 showed a megabase server tick of ~0.85 ms **even without commands**, versus 0.28 ms in
the benchmark. The cause is the conditions, not the network: benchmarks run ticks back to back, while the server
(and the game) ticks every 33 ms, and during the pause the world's data leaves the cache (on a VM the clock
probably drops too). The same world without networking:

| B3-100k | Tick |
|---|---|
| Ticks back to back (`build_cost --plain`) | 0.29–0.32 ms |
| With a 33 ms pause between ticks (`build_cost --plain --paced`) | **0.74–0.79 ms** |

Conclusion: for large bases in a real game, what matters is not only the tick's work but also how much memory it
touches (data density, P-03/P-04, pays off twice here). Benchmark numbers are a lower bound. A measurement on the
owner's PC will show how much of this is a VM property.

### 2026-11-21 — the owner's PC: the "cold" tick, building, B7 (commit 053aad9)

The owner's PC — the same as on 2026-11-20: Intel Core i5-12400F, 16 GB, Windows 11. The benchmark suite matches the previous measurement within noise
(B3-100k 269 vs 277 µs, B2-comb-100k 126 vs 128 µs). B7 results —
`docs/benchmarks/results/2026-11-21-pc-b7-*.json`.

| B3-100k | Owner's PC | VM |
|---|---|---|
| Ticks back to back | 222 µs | 290–320 µs |
| **With a 33 ms pause between ticks** | **442 µs (×2.0)** | 740–790 µs (×2.5) |
| A tick with a wall placed (back to back) | 337 µs (+115 µs) | ~400 µs (+100–120 µs) |

| B7, B3-100k | Server frame: mean / p99 / worst | Network in the frame | Traffic per client | All joined |
|---|---|---|---|---|
| 8 clients | 0.71 / 2.25 / 3.43 ms | 0.15 ms | 0.86 KB/s | 0.90 s |
| 32 clients | 1.09 / 2.82 / 4.62 ms | 0.48 ms | 1.75 KB/s | 1.89 s |

- **The "cold" tick is not just a VM property**: on a desktop PC a megabase tick every 33 ms costs twice as much as
  ticks back to back. In a real game the megabase is ~0.45 ms (1.3% of the budget), not 0.22 ms. Back-to-back
  benchmarks remain handy for comparing versions, but the `--paced` mode shows the real cost.
- The server frame with 8 players on the megabase is 0.7 ms, with 32 — 1.1 ms (3% of the budget); the worst
  frame is 4.6 ms (14%).
- Traffic is the same as on the VM: it doesn't depend on the hardware.
- 32 players joining take 1.9 s, as on the VM: a snapshot per joining player (S6-03).
