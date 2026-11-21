# Roadmap and technical backlog

Every stage ends with a verifiable result. Task IDs are used in commits and issues.

## Stages

| Stage | Result | Status |
|---|---|---|
| 0. Research and architecture | Documents in `docs/`, decisions in decisions.md | **Done** (decisions approved 2026-11-15) |
| 1. Simulation core (headless) | Vertical slice "drill → conveyor → furnace → core", save/load, replay, B1/B2-10k | **Done** 2026-11-16 |
| 2. First graphical client | Window, camera, rendering of tiles and buildings, placing blocks with the mouse | **Done (v0)** 2026-11-16 — see limitations below |
| 3. Production, logistics, power | Chains of 3+ links, splitters/sorters/bridges, power networks, fluids | **Done** (2026-11-20): economy, splitter, sorter, bridge, depot, overflow gate, batteries, burner, inspector, fluids, heat network, local power network rebuilds |
| 4. Units and combat | Flow fields, movement, turrets, projectiles, damage, waves — the first playable match | **Done (v0)** 2026-11-17: two enemy types (four in v0.4), flow field, health and destruction, turret, wall, waves, defeat, B5 |
| 5. Content and map format | Map editor/generator, full content set v0.1, schematics | **In progress** (2026-11-17): map generator with guarantees, new game screen (size, difficulty, seed), schematics (copy, rotate, paste, `.fschem` file); content v0.4: burner, depot, arc tower, ranged and armored enemies; wave composition in data (S5-02); waves reacting to the base's noise (D-02); map editor and the `.fmap` file (2026-11-20); missing: campaign |
| 6. Multiplayer | Dedicated server, client, synchronization, B7 | **Design** (2026-11-20): [network-design.md](network-design.md), decisions D-18…D-22 approved |
| 7. Completeness | Campaign/progression, data mods, menus and UI per the brand guidelines, sound | Not started |
| 8. Optimization and release | Profiling, builds for 3 OSes, installers, public release | Not started |

## Minimal vertical slice (stage 1)

Headless, no graphics. A 64×64 map from a seed; on it:
iron ore → **drill** → **conveyors** (with turns) → **furnace** (ore + power → ingot) →
**core** (storage). One generator and a power node. The result is checked by a test:
"after N ticks the core holds exactly K ingots", and the same scenario is saved, loaded and
continued with the same state hash.

## Stage 1 backlog

| ID | Task | Depends on | Acceptance criteria | Status |
|---|---|---|---|---|
| S1-01 | Workspace, shared lints, profiles, CI (Linux/Windows/macOS), `cargo deny` | — | All 5 check commands from the brief pass in CI on 3 OSes | Done |
| S1-02 | `ferrustry-content`: content schema (resources, blocks, recipes) in RON, reference validator, `ContentRegistry` with dense IDs | S1-01 | Broken references → an error with the field path; tests for valid/invalid content | Done |
| S1-03 | `ferrustry-sim`: world of 32×32 chunks, floor/ore, `BuildingId`, placing/removing multi-tile buildings | S1-02 | Tests: overlaps, map bounds, removal frees all tiles | Done |
| S1-04 | Loop: fixed tick, command queue, deterministic PRNG, state hash | S1-03 | Two runs with the same seed and commands → the same hash; property test on random commands | Done (property test — random building on 3 seeds) |
| S1-05 | Drills and furnaces: mining by the ore under the drill, recipes, internal buffers | S1-04 | Mining/crafting speed test over N ticks is exact to the item | Done |
| S1-06 | Conveyors: segments with gaps, turns, merging at joints, machine input/output, incremental rebuild | S1-04 | Tests: throughput, no lost/duplicated items, rebuild when removing in the middle | Done; the rebuild is local but O(segment length) |
| S1-07 | Power network v0: generator, node, consumer, satisfaction factor, union-find + rebuild on removal | S1-04 | Test: with a power shortage the furnace works proportionally slower | Done; the network rebuild is global (v0 limitation) |
| S1-08 | Core storage and player resource accounting | S1-05 | End-to-end test of the vertical slice | Done |
| S1-09 | `ferrustry-format`: container v1, world save/load, content name table | S1-08 | Round-trip: save → load → hash matches; continuing the simulation matches an uninterrupted run | Done; no ID remapping |
| S1-10 | `ferrustry-headless`: `run <scenario>`, `bench <B1|B2>`, `replay <file>`, JSON report | S1-09 | Benchmarks are reproducible, the report contains the fields from performance.md | Done: run/resume/save-check/bench/suite/record/replay |
| S1-11 | Allocation counter in the bench build | S1-10 | B2-10k: 0 allocations in a steady tick (or a documented reason) | Done: 0 allocations/tick in B2 |
| S1-12 | Public API documentation (`#![warn(missing_docs)]`), `docs/development/` | S1-01 | `cargo doc` without warnings | Done (`cargo doc` without warnings); `docs/development/` created |
| S1-13 | Realistic logistics benchmarks | S1-10 | Reports in `docs/benchmarks/results/` | Done: B2-blocked, B2-comb |
| S1-14 | Replay: initial state + commands + checkpoint hashes, `headless record/replay` | S1-10 | Replay reproduces the state hash; a desync is localized to a checkpoint | **Done**; in CI a replay from Linux x86_64 is checked on Windows and macOS ARM |

## Rendering backlog (after stage 2)

| ID | Task | Why |
|---|---|---|
| R-01 | GPU cache of static layers per chunk (floor, ore) | The floor is currently assembled on the CPU every frame; zoomed far out that is up to ~260k sprites |
| R-02 | Mip levels / LOD for zooming out | Ore "shimmers" at small scale (aliasing without mips) |
| R-03 | Interpolating item movement between ticks | Items are currently drawn at their last-tick position (30 Hz) |
| R-04 | Brand fonts (Inter, JetBrains Mono, Exo 2; OFL) | Currently a built-in 5×7 pixel font |
| R-05 | Main menu, settings screen, map selection | **Partial (2026-11-20)**: main menu, pause menu, F1 help, clickable hotbar, DPI scaling, new game screen (size, difficulty, seed), map editor, map list (the `maps/` folder); no settings screen |

## Performance backlog

See `docs/en/performance-design.md`, tasks P-01…P-10.

## Backlog of later stages (high level)

| ID | Task | Depends on |
|---|---|---|
| S2-01 (**done** except the chunk cache) | `ferrustry-render`: wgpu context, atlas, instanced sprites, chunked floor meshes | S1-03 |
| S2-02 (**done** except interpolation) | `ferrustry-client`: winit loop, interpolation, camera (pan/zoom), placing/removing with the mouse | S2-01, S1-04 |
| S2-03 (**partial**: HUD, building panel, brand palette; brand fonts and menus — no) | UI layer (decision D-07), menus, palette and fonts per the brand guidelines | S2-02 |
| S3-01 (**done** 2026-11-20: splitter, sorter, overflow gate, bridge, depot) | Splitter, sorter, overflow, bridge, storage | S1-06 |
| S3-02 (**done** 2026-11-20: batteries, burner, diagnostics in the inspector, local network rebuild) | Power: batteries, several generator types, network diagnostics in the UI | S1-07 |
| S3-05 (**done**) | Economy: building costs from cores, refund on removal, starting stock | S1-08 |
| S3-03 (**done**: water, pipes, condenser, cooling tower; compartment model) | Fluids (D-09) | S1-07 |
| S3-04 (**done**: forge heat output, slowdown above 70%, heat pipes, radiator, cooling tower, heat engine) | Heat as a network (if D-02 is approved) | S3-03 |
| S4-01 (**done**) | Spatial grid, units (SoA/hecs, D-05), movement, separation | S1-04 |
| S4-02 (**partial**: one field for ground units with local repair; movement classes — no) | Flow fields per movement class with incremental recompute | S4-01 |
| S4-03 (**done**: gun, arc tower, armor, enemy projectiles) | Turrets, projectiles, damage, armor, building destruction | S4-01 |
| S4-04 (**done**) | Enemy waves | S4-02, S4-03 |
| S5-01 (**done** 2026-11-20: generator, schematics, `.fmap`, map editor) | Map generator, `.fmap` format, `.fschem` schematics | S1-09 |
| S5-02 (**done** 2026-11-18) | Wave composition in content data | S4-04 |
| S6-00 (**done** 2026-11-20: [design](network-design.md), measurements, decisions D-18…D-22 approved) | Multiplayer model | — |
| S6-01 (**done** 2026-11-20, except the chunked snapshot) | `ferrustry-net`: protocol (version, handshake, command frames, chunked snapshot, checksums), an in-memory transport for tests | D-18 |
| S6-02 (**done** 2026-11-20) | Session: the server orders commands by tick, clients simulate by frames; a "server + 3 clients, thousands of ticks, all hashes match" test | S6-01 |
| S6-03 | Joining a running game: a background snapshot, catching up through frames (D-21) | S6-02 |
| S6-04 (**done** 2026-11-21) | A fast state checksum (not through serialization) and recovery on a mismatch (D-20) | S6-02 |
| S6-05 | renet transport (D-08), `ferrustry-server` (headless, config), connecting from the client, B7 | S6-02 |

## Major risks

| Risk | Mitigation |
|---|---|
| Logistics doesn't fit the budget | Segment model from stage 1, B2 benchmark in CI |
| Desync/non-determinism | State hash from stage 1, replay tests, ban on non-deterministic APIs in sim |
| UI complexity on wgpu | Decision D-07 before S2-03 starts |
| Volume of content and balance | Data-driven content, a dedicated game design owner |
