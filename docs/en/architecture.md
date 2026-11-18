# Ferrustry architecture

Status: **approved 2026-11-15**. Implemented crates: `ferrustry-content`, `ferrustry-sim`,
`ferrustry-format`, `ferrustry-headless`, `ferrustry-render`, `ferrustry-client`; audio, networking and
the server will arrive in their stages.

Ferrustry is an original factory automation + base defense game written in Rust.
The key requirement: the game simulation is fully separated from graphics and runs headless —
one core for the client, the dedicated server, tests and benchmarks.

## 1. Principles

1. **Headless-first.** The simulation crate does not depend on wgpu, winit, audio or networking.
2. **Fixed timestep.** The simulation ticks at a constant rate; rendering interpolates.
3. **Determinism.** Same binary + same input commands + same seed ⇒
   bit-identical state. Verified by a state hash in tests.
4. **Data separate from code.** Content (resources, blocks, units, recipes) is described as data
   and loaded into a registry; systems work with numeric IDs.
5. **Data-oriented.** Dense arrays (SoA) per object type, incremental structures
   (conveyor segments, power networks), minimal allocation per tick.
6. **Measure, then optimize.** Parallelism only after profiling and only
   with a deterministic merge order of results.

## 2. Workspace layout

Crates are created as real code appears — no empty placeholders.

```text
ferrustry/
├── Cargo.toml                 # workspace, shared lints and profiles
├── crates/
│   ├── ferrustry-sim/         # Stage 1: world, content registry, systems, tick, state hash
│   ├── ferrustry-content/     # Stage 1: content schemas (serde) + base data set
│   ├── ferrustry-format/      # Stage 1: map/save/schematic format, versioning
│   ├── ferrustry-headless/    # Stage 1: binary — scenario runs, benchmarks, replay
│   ├── ferrustry-render/      # Stage 2: wgpu renderer (sprites, tiles, camera)
│   ├── ferrustry-client/      # Stage 2: winit application, input, UI, ties sim+render together
│   ├── ferrustry-audio/       # Stage 4+: sound (candidate: kira)
│   ├── ferrustry-net/         # Stage 6: protocol, packet serialization, transport
│   └── ferrustry-server/      # Stage 6: dedicated server (sim + net, no graphics)
├── assets/                    # original Ferrustry assets (own license, see licensing.md)
├── benches/ / tests/          # benchmark scenarios and integration tests
└── docs/
```

Dependency graph (arrow = "depends on"):

```text
content ← sim ← format
            ↑      ↑
   headless ┘      │
   render ← client ┤ (client → sim, format, render, audio, net)
   net ← server ───┘ (server → sim, format, net)
```

A separate `ferrustry-core` crate is **not created** until sim and net actually share code
(IDs, fixed-point math). If that happens, we will extract it.

## 3. Game loop

- **Simulation rate: 30 ticks/s** (proposal; see decisions.md, D-03). Tick budget is 33.3 ms,
  the target is far lower so the server can handle large maps and several games per core.
- Client: an `accumulator` loop — N simulation ticks per frame (with a "spiral of death" cap),
  rendering interpolates unit/projectile positions between the last two states.
- The order of systems in a tick is fixed and documented:

```text
1. apply player input commands (building, orders)   — ordered by (tick, player_id, seq)
2. building/removal → incremental rebuilds (segments, networks, flow fields)
3. power networks: production/consumption balance → satisfaction factor
4. mining and production (use the power factor from step 3)
5. logistics: conveyor segments, sorters, storage
6. fluids
7. units: AI → movement (flow field + local avoidance) → collisions
8. turrets: target selection (spatial grid) → firing
9. projectiles: movement, hits, damage
10. death/destruction, cleanup, events for the client
11. enemy waves
12. (debug/test) state hash
```

## 4. World and data storage

### Grid
- The world is split into **32×32-tile chunks**. A tile = floor (terrain) + ore (overlay) + building reference.
- Floor/ore are dense `u16` arrays inside a chunk (cache friendly, easy to serialize and render).
- Multi-tile buildings: a tile stores a `BuildingId`, the building stores its origin and size.

### Buildings
- Not "an object per tile" but **tables per behaviour kind** (drill, crafter, conveyor-segment,
  storage, turret, power-node…): dense SoA arrays + generational `BuildingId → (kind, index)`.
- Removal is swap-remove with an index update. This gives a linear pass over buildings of one
  kind with no virtual calls.

### Dynamic entities (units, projectiles)
- Candidates: `hecs` or custom SoA pools. Proposal: **custom pools** for projectiles
  (the most numerous, short-lived) and `hecs` for units in stage 4, possibly replaced
  after profiling (decisions.md, D-05).
- Spatial index: a uniform grid (cell = chunk or 8×8 tiles), rebuilt per tick
  in O(n) — for target search, collisions and area damage.

## 5. Key subsystems

### Logistics (conveyors)
The hottest spot of the genre. Model:
- A chain of connected conveyors of one direction is merged into a **segment** (transport line).
- A segment stores items as an array of `(item_id, gap_to_next)`; movement = decrementing one
  "head" gap instead of visiting every item. Tick cost is ~O(segments), not O(items).
- Segments are rebuilt **incrementally** only when neighbouring tiles are built or removed.
- Nodes (splitters, sorters, bridges, machine inputs) are separate tables with buffers that
  interact with segment ends.

### Power
- A network = a connected component of the graph (nodes + buildings).
- Adding is union-find (almost O(1)); removal rebuilds only the affected component (BFS).
- Each network caches the sum of production/consumption/battery capacity; the balance tick is O(networks).
- A power shortage → a satisfaction factor `0..1` that consumers apply to their speed.

### Fluids
- The same graph approach as power, but with volume and throughput per segment
  (compartment model). The exact model is a game design decision (decisions.md, D-09).

### Units and pathfinding
- **Flow fields** per movement class (ground / heavy / flying) towards goals (player core,
  rally point). Recomputed incrementally per chunk when passability changes.
- A* only for individual player orders; a hierarchical (per chunk) variant if needed.
- Local avoidance: separation via the spatial grid; physics is circles, no full physics engine.

### Combat
- Turrets: target selection via the spatial grid with a limited retargeting rate.
- Projectiles: an SoA pool, hit tests via the grid; damage, armor, building destruction → events
  to rebuild logistics/power.

## 6. Numbers and determinism

- Grid, item, power and fluid logic use **integers** (power in integer "micro units").
- Unit/projectile positions are `f32` without FMA-dependent and transcendental std functions in
  simulation logic (`libm` equivalents instead), so results don't depend on the platform.
  The alternative is fixed point (decisions.md, D-04).
- A custom PRNG (e.g. PCG/xoshiro) with an explicit seed in the state; no `thread_rng` in sim.
- Iteration order only over stable indices/IDs, no `HashMap` with a random hasher in
  logic that affects state.

## 7. Content and modding

- Content is described in **RON** (or TOML) and loaded into a `ContentRegistry` with reference checks
  (a recipe refers to an existing resource, etc.). Content errors are clear messages, not panics.
- Block behaviour is a fixed set of "behaviour kinds" parameterized by data.
  This is the first step of modding (data-only mods).
- Scripted mods (WASM via wasmtime or Lua via mlua) come after stage 6, as a separate decision (D-10).

## 8. Rendering (stage 2)

- `wgpu` + `winit` (stable winit 0.30 branch; 0.31 is currently in beta).
- A texture atlas, **instanced sprite batching** (one draw call per layer/atlas).
- Static layers (floor, ore) are per-chunk meshes, cached and rebuilt on change.
- Visible chunks via frustum culling against the camera.
- The renderer reads a **snapshot** of the state (or an interpolation of two snapshots) and does not modify the simulation.
- UI: see decisions.md, D-07.

## 9. Networking (stage 6)

- **Authoritative server**: the server is the single source of truth.
- The client sends commands (build, unit orders, interaction), the server sends:
  a full snapshot on connect (via the save format) + per-tick deltas/events.
- Client-side prediction for the player's own unit/build cursor.
- Transport: UDP with custom reliable/unreliable channels or an existing solution (renet / QUIC via quinn).
  Decided after a prototype (D-08). `tokio` is used only in net/server, sim stays synchronous.
- The protocol is versioned; a version mismatch is a clear error on connect.

## 10. Formats

See compatibility.md. In short: one binary container format (header + version + sections,
zstd compression) for maps, saves and schematics; network packets are `postcard` over serde.

## 11. Error handling and quality

- Library crates: `thiserror` error types; binaries: `anyhow`/`eyre` at the top level.
- No `unwrap/expect` on runtime paths without a justifying comment; `unsafe` is forbidden with
  `#![forbid(unsafe_code)]` in all crates except explicitly agreed places in the renderer.
- CI: `fmt --check`, `clippy -D warnings`, `test`, `build --release` on Linux/Windows/macOS.

## 12. Stage 1 implementation (what was actually built)

| Module | File | Essence |
|---|---|---|
| Content | `ferrustry-content/src/lib.rs`, `data/base.ron` | RON → reference/value validation → `ContentRegistry` with `u16` IDs |
| Grid | `ferrustry-sim/src/grid.rs` | 32×32 chunks: floor, ore, building reference |
| Buildings | `ferrustry-sim/src/buildings.rs` | Generational arena + dense tables per behaviour kind (swap-remove) |
| Conveyors | `ferrustry-sim/src/segment.rs`, `world.rs` | Segments ≤ 32 tiles with gaps, compressed-prefix cache, turns, loops, local rebuild that keeps items; packed `item<<16 \| gap` layout; sleeping (active bitset + waiter lists); link cache keyed by topology epoch |
| Power | `ferrustry-sim/src/power.rs` | Union-find over touching buildings and node radius, satisfaction factor in thousandths |
| Production | `world.rs` | Drills (share of ore under the drill), recipes, round-robin output around the perimeter |
| Determinism | `hash.rs`, `rng.rs` | FNV-1a over the serialized state, xoshiro256** in the world state |
| Format | `ferrustry-format` | Container v1.0, zstd, CRC32, name table, integrity checks |
| Headless | `ferrustry-headless` | `run`, `resume`, `save-check`, `bench` with a JSON report and an allocation counter |

Deviations from the plan worth knowing:
- The tick order is currently: power → drills → crafters → conveyors (no units, fluids or
  combat yet). Commands are applied before the systems, in the order given.
- The power network is rebuilt entirely on any change of power buildings (local rebuild is S3-02).
- Building is instant and free (sandbox mode); build cost and time come in stage 3.
- An item leaving a segment into a segment with a higher index may move once more in the same
  tick. The behaviour is deterministic; if it becomes noticeable, we will add a two-phase update.

### Logistics scheduler (since 2026-11-15)

- Only **awake** segments are processed, strictly in increasing index order (bitset).
- A segment goes to sleep if nothing changed and either: it is empty; or its items are fully
  compressed and its output faces nothing / a building that does not accept items / a sleeping
  segment (in which case it is added to that segment's waiter list).
- Woken by: a successful insertion; movement or output of a segment it waits on; building nearby.
- The receiver at a segment's output and a building's output receiver list are cached and
  invalidated by a growing **topology epoch** (any build/removal).
- `World::set_sleep_enabled(false)` is the reference mode for differential tests.

## 13. Stage 2: client (2026-11-16)

| Module | File | Essence |
|---|---|---|
| Atlas | `ferrustry-render/src/atlas.rs` | Procedural graphics in the brand palette (no external assets), shelf packing, bleed padding |
| Batches | `batch.rs` | Flat instance array (13 `f32`), HUD in pixels, text with a built-in 5×7 font |
| Scene | `scene.rs` | Culling by the visible area, a sprite table by numeric ID (no strings per frame), items on belts along the tile direction |
| GPU | `gpu.rs` | wgpu 30: one pipeline, instanced quads with no geometry vertex buffer; world and HUD are two draw calls; offscreen render → PNG |
| Client logic | `ferrustry-client/src/game.rs` | Platform-independent input, drag building, area removal, ghosts validated by `World::can_place`, fixed 30 Hz step (≤ 8 ticks per frame) |
| Window | `ferrustry-client/src/app.rs` | winit 0.30, sRGB surface, AutoVsync, lost/outdated surface handling |

Colours: the atlas is loaded as an sRGB texture, tints are converted to linear space
(`batch::rgba`), the target is sRGB. A pixel test of the renderer caught a double-gamma bug.

Player commands from the client go through the same `Command`/`World::tick` as replay and the
future network. Exception in v0: while paused, building is applied immediately (`World::apply`).

## 14. Stage 3, first iteration (2026-11-17)

| What | Where | Essence |
|---|---|---|
| Economy | `world.rs` (`check_cost`/`pay_cost`/`refund_cost`, `Rules`) | Cost from content (`cost`), deducted from cores in order, refunded on removal; `Rules::infinite_resources` is a sandbox for tests/benchmarks |
| Splitter, sorter | `logistics.rs` | One-item buffer; the input remembers its side (`Link::Directed`); the splitter cycles over 3 sides, the sorter sends the filter straight and the rest alternately sideways |
| Bridge | `logistics.rs` | The partner is the first bridge of the same direction within `range` (cached by topology epoch); 1 item per `interval` ticks; a bridge without a partner is an output |
| Batteries | `power.rs` | Network surplus charges, shortage discharges; demand satisfaction accounts for discharge; distribution across batteries is deterministic |
| Configuration | `Command::Configure` | Sorter filter; goes through replay like any command |
| Inspector | `World::inspect` → `Detail` | Structured building state for the UI |

Found and fixed along the way: removal did not bump the topology epoch, so link caches
(segments, output) stayed stale until the next placement — items could leave through a wrong
link. The regression test `removal_invalidates_cached_links` fails without the fix.

## 15. Fluids and heat (2026-11-17, decisions D-02, D-09)

| What | Essence |
|---|---|
| Networks | `thermal.rs`: buildings touching by sides form a fluid network (volume, one fluid type) or a heat network (heat stock, heat capacity); union-find on change |
| Rebuild | The old network's contents are split between surviving members in proportion to capacity, the remainder goes to the first; the share of removed buildings is lost and accounted for (`FluidStats.destroyed`) |
| Heat | A crafter with `heat > 0` emits heat in proportion to its speed (the fraction accumulates without loss); above 70% network fill the speed drops linearly to 0 at 100% |
| Heat removal | Passive crafter cooling, radiator, cooling tower (water → heat), heat engine (heat → power on the next tick, 50% efficiency) |
| Water | The condenser makes water from power; pipes connect condensers and cooling towers |
| Tick order | network rebuild → power → condensers → drills → crafters (heat) → heat → routers → bridges → belts |

Verified by tests with exact expectations: an overheated forge settles at exactly 950‰
and 1/6 speed (9 cycles vs 50 for a cooled one over 3000 ticks); water is accounted to the unit
across network rebuilds.

## 16. Combat (stage 4, 2026-11-17)

| What | Essence |
|---|---|
| Numbers | Positions in 1/256 tile (`combat::SUB`), distances via `i64::isqrt`; no `f32` in the simulation → combat is deterministic between x86 and ARM |
| Units, projectiles | SoA pools (`Units`, `Projectiles`), swap-remove from the end |
| Flow field | Dijkstra from all core tiles; entering an empty tile = 1, a building tile = 1 + health/8 (a wall ≈ a 51-tile detour); recomputed only when the topology epoch changes and units exist |
| Enemy behaviour | Step to the neighbour with a lower cost; if a building is there, hit it with a cooldown; at the core, hit the core |
| Grid | 2×2-tile cells, allocation-free "head + next" lists; the `Scratch` buffer is not saved and not compared between worlds |
| Turret | Nearest target in range via the grid, leading by the target's per-tick displacement, projectiles with flight time; ammo is items from a belt |
| Destruction | `World::destroy_at`: like removal but with no refund; the last core → `Outcome::Lost` |
| Waves | On a timer from the map's spawn points; composition grows with the number (`World::wave_composition`) |
| Tick order | … belts → waves → units → grid → separation → grid → turrets → projectiles → removal of the dead |

Balance v0 (verified by a test): one turret reliably stops three simultaneous crawlers.

## 17. Maps and schematics (stage 5, 2026-11-17)

| What | Where | Essence |
|---|---|---|
| Generator | `ferrustry-sim/src/mapgen.rs` | Integer value noise (the same map on any platform); ash, slag pools away from the start; ores: random first, then guaranteed ones near the start (cannot be overwritten); a cleared area near the core; spawn points on the edges with a guaranteed path (otherwise a corridor is carved) |
| Difficulty | `Difficulty` | Wave pace: easy 8/3 min, normal 5/2, hard 3/1.5 |
| Schematics | `ferrustry-sim/src/schematic.rs` | Copy of buildings in a frame (rotation, sorter filter), 90° rotation respecting sizes, paste as ordinary commands (`Place` + `Configure`) — economy, replay and networking work automatically |
| Schematic file | `ferrustry-format/src/schematic.rs` | Buildings by content name; an unknown name is an error, not a guess |

The generator guarantees are checked on 36 maps (3 sizes × 12 seeds). The test caught a real flaw:
the guaranteed carbon near the start was overwritten by a random deposit generated later.

## 18. Content v0.4 (2026-11-17)

| What | Mechanics | Why |
|---|---|---|
| Burner (`Generator` with `fuel`) | A fuel buffer, an item burns for `burn_ticks` ticks | The same building kind as the wind turbine: no new table. Burns regardless of demand (v0) |
| Steady output | The sum of a network's wind turbines is computed on network rebuild | Only fueled generators are visited each tick (−6.4% instructions on B3) |
| Depot (`Depot`) | Accepts from three sides, outputs from the front, 1 item per tick; if an item is not accepted it tries the next kind | A mixed depot does not stall in front of a crafter that needs a different item |
| Arc tower (`ArcTurret`) | Recharge speeds up with network power (500‰ → twice as long), instant damage, ignores armor | Shares the turret table with the gun; power demand only while charging |
| Enemy armor | Projectile damage minus armor, but no less than a third | The gun is weak against the shellback but not useless |
| Ranged enemies | Find the nearest building in range (a square of tiles, row by row — deterministic), stand and shoot until the cooldown ends | Enemy projectiles share the same SoA with a `hostile` flag; they hit any building on their tile |

Turret reload now runs every tick, even without enemies (it used to freeze together with the combat step).

## 19. Combat: overkill and flow field repair (P-10, 2026-11-18)

| What | Mechanics | Why |
|---|---|---|
| Damage in flight | `Units::incoming` — damage of projectiles flying at the unit (after armor); `Projectiles::target` — the projectile's target. Turrets pick the nearest unit whose health exceeds its damage in flight | Without it several turrets waste shots on an already doomed target. A miss (the projectile hit someone else) makes the target selectable again |
| Indices after deaths | Removing the dead keeps an "old index → new" table in a scratch buffer and remaps projectile targets in one pass | Units are SoA with swap-remove; stable IDs would cost more |
| Invariant | `World::validate`: each unit's `incoming` equals the sum of damage of projectiles aimed at it | A bookkeeping error shows up in any test that calls `validate` (in battle — every tick) |
| Change log | Building and removal record tiles in `FlowField::dirty`; a core or > 4096 tiles means a full rebuild | The field is repaired only where something changed |
| Repair | Region D: the changed tiles and every tile whose distance was obtained through them (`dist[q] == dist[p] + cost(q)`); D is reset, filled from its border, then Dijkstra. D > ¼ of the map — full rebuild | The result is bit-identical to a full rebuild (differential test on random building) |
| Tile cost | 1 + the building kind's **full** health / 8 (previously current health) | The field is a function of what is built, not of damage: otherwise the repair would not match a rebuild. Previously damage was only picked up at the next, arbitrary, build |
| In advance | `World::prepare_flow` at map generation and in scenarios | The first wave does not pay for a full build inside a tick |

## 20. Waves in data (S5-02, 2026-11-18)

Wave composition is the `waves` section of `base.ron`: a rule "enemy kind, from which wave, how many at
first, growth every N waves" (`WaveRule::count_in`). `World::wave_composition` only applies the rules.
The move did not change the game: a test compares waves 1–60 with the former formulas from code. The content
reference shows how many enemies arrive in waves 1, 5, 10 and 20 — wave balance is tuned by
editing data, with the result visible immediately.
