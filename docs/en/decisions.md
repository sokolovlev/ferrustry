# Decisions for approval

Format: **my proposal** + alternatives. The status changes to "Approved/Rejected" after the product
owner's answer.

## Product

| ID | Question | Proposal | Alternatives | Status |
|---|---|---|---|---|
| D-01 | Setting and theme | "Reforge": restoring a dead industrial world, repairing ruins as a mechanic | Your own option | Approved 2026-11-15 |
| D-02 | Distinctive mechanics | Heat as a third network; waves that react to the base's "noise" | Only the classic genre set | Approved 2026-11-15 |
| D-12 | License | Code MIT OR Apache-2.0; assets CC BY-SA 4.0; brand under a trademark policy | MIT only; assets CC BY 4.0 | Approved 2026-11-15 |
| D-13 | Descriptor in the brand guidelines | "Industrial strategy, reengineered in Rust" instead of "reimplementation of Mindustry" | Your own option | Approved 2026-11-15 |

## Technical (I can decide these myself, but they affect everything after — please confirm)

| ID | Question | Proposal | Why | Status |
|---|---|---|---|---|
| D-03 | Tick rate | 30 Hz + render interpolation | Half the server cost of 60 Hz; enough for the genre | Approved 2026-11-15 |
| D-04 | Numbers in the simulation | Integers for grid/resources/power; `f32` without FMA/std transcendentals for units | Simpler than fixed point; determinism is checked in CI | Approved 2026-11-15 |
| D-05 | ECS | Own SoA tables for buildings and projectiles; `hecs` for units, replaceable after profiling | Buildings are the hottest spot, better stored densely by type | Approved 2026-11-15 |
| D-06 | Content format | RON | Native to Rust/serde, supports enums; TOML is worse for nested structures | Approved 2026-11-15 |
| D-07 | UI | `egui` for debugging and tools; the game UI is our own light layer on the renderer | egui is awkward for a stylized game UI | Approved 2026-11-15 |
| D-08 | Network transport | Prototype on `renet` (UDP); our own reliable UDP if it is limiting | Quick start, keeps the option to replace it | Approved 2026-11-15 |
| D-09 | Fluid model | Compartment model by segment | Stable and cheap; exact fluid dynamics is not needed | Approved 2026-11-15 |
| D-10 | Scripted mods | Postpone to stage 6; candidates WASM (wasmtime) / Lua (mlua) | Data-only mods first | Approved 2026-11-15 |
| D-11 | Reference machine for benchmarks | For now the development machine (recorded in each report); a permanent one chosen before stage 3 | Needed for target numbers | Partially approved |

## Performance (added 2026-11-15)

| ID | Question | Proposal | Why | Status |
|---|---|---|---|---|
| D-14 | Minimum CPU | **x86-64-v3 (AVX2, FMA, BMI2; Intel Haswell 2013+, AMD Excavator/Zen)** — the owner's decision; AVX-512 only with runtime selection; ARM — NEON | An AVX2 baseline lets the compiler vectorize without dispatch | Approved 2026-11-15 |
| D-15 | `unsafe` for SIMD and hot paths | Allow it in a dedicated `ferrustry-sim::simd` module with SAFETY comments, reference tests against a scalar version and Miri in CI | Explicit SIMD (`core::arch`) is impossible without `unsafe`; `std::simd` is nightly only | Approved 2026-11-15 |
| D-16 | Reference machine | **The project owner's PC**; runs via `scripts/bench-suite.*`, results in `docs/benchmarks/results/` | Cloud VM noise (up to 40%) is larger than the effect of many optimizations | Approved 2026-11-15 |
| D-17 | Multithreaded simulation (P-06) | See the options below; recommendation — **A now, B at the start of stage 6** (if the trigger fires), C and D in their stages | Measurements (performance-design.md, section 8): the ceiling on the owner's PC is 2.6×, fine-grained parallelism does not pay off; P-04 already sped up a single thread by 14–22% | Awaiting approval |

### Options for D-17

| Option | What | Pros | Cons |
|---|---|---|---|
| **A. Postpone** (recommended now) | The simulation stays single-threaded; continue P-04/P-03 (data density). Trigger to revisit: the target base size takes > 25% of the tick budget (8 ms) on the reference PC — or the start of stage 6 | Zero risk; the 100k megabase is now ~1.5% of the budget on the owner's PC; every single-thread speed-up also helps future regions | Cores sit idle on very large bases |
| **B. A world of regions** | Regions with their own data, exchange across borders once per tick (one-tick delay at a border) | Up to 2.6× on the owner's PC for large bases; deterministic for any number of threads | A large rework of world storage; to be done together with the network and save design (stage 6), otherwise the work is done twice |
| **C. Rendering on a separate thread** (client) | Simulation and drawing on different threads | FPS does not drop because of a heavy tick and vice versa; determinism is not affected | Needs a state snapshot for rendering; the benefit shows on large bases together with the layer cache (R-01) |
| **D. Matches per core** (server) | The dedicated server runs one match per core | Server scaling without changes to the simulation | Does not speed up a single match; part of stage 6 |

