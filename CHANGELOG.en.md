# Changelog

[Русский](CHANGELOG.md) · English

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Before 1.0,
saves are incompatible between minor versions (the game reports a format version mismatch).

## Unreleased

### Added
- **Overflow gate** `overflow_gate`: items go straight while there is room ahead, the surplus
  alternates sideways. The LOGISTICS tab has 6 buildings (keys 1–6).

### Changed
- **Balance by the report** (`docs/en/balance/2026-11-19-normal-tuned.md`): the base has to grow — by
  wave 5 it takes ~2 production cells, by wave 10 ~6, by 15 ~10, by 20 ~16 (was 1 / 2 / 3 / 5). Waves:
  crawlers +3 per wave (was +2), brutes +1 every wave (was every other). The gun turret costs 45 ingots
  (was 30), 1 ingot = 2 shots (was 4). Arc tower: range 7 (was 5), discharge 40 (was 30) — in later
  waves it is more efficient than guns.

### Added
- **Balance tool** `ferrustry-headless balance`: the difficulty curve from real runs — how many guns
  and arc towers hold each wave, ammo use, arrival time at different noise levels and how many
  production cells are needed to pay for the defence. First report:
  `docs/en/balance/2026-11-19-normal.md`.

## 0.7.0 — 2026-11-19

Save format 7.0.

### Added
- **Noisy waves (D-02)**: working drills, crafters and fueled generators make noise, and the base's
  noise speeds up the wave timer — by 2% per unit of noise, at most 2.5 times. A quiet base gets
  waves on the previous schedule. Noise is shown in the corner of the screen and in building
  tooltips; the values are data (`noise` on buildings and the `noise` section).

### Changed
- **Belts are 19–20% faster** in the realistic B2-comb scenario, the B3-100k megabase by 5%: the items
  of all belts live in one contiguous pool (P-03). Behaviour and the save format did not change.
- The power balance walks the networks twice per tick instead of four times (same behaviour).
- `World::validate` checks that the live link caches and the drill row table match a fresh
  calculation.

## 0.6.0 — 2026-11-18

Save format 6.0.

### Changed
- **The simulation is 14–22% faster** (megabase 10k / 100k) with the same behaviour: crafters and
  drills keep everything the tick needs in a dense table row (P-04). A recipe has at most 4 different
  items on each side (checked by the content validator).

### Added
- Measurement tools: `parallel_ceiling`, `fork_join_cost` (multithreading) and
  `behaviour_fingerprint` (behaviour unchanged after a refactor) — examples in
  `ferrustry-headless`. The P-06 investigation and the options for decision D-17 are in the docs.

## 0.5.0 — 2026-11-18

Save format 5.0.

### Added
- **Turrets spread their fire**: they don't shoot an enemy that shells in flight will already kill.
  In a battle of 1000 enemies against 300 turrets, 7.3% more kills with the same ammo.
- **The enemy flow field is repaired locally** on building and removal instead of a full rebuild:
  a battle with building (new benchmark B8) has a 3.2× faster mean tick and 6–8× faster p99; the field
  is built when the map is created, so the first wave causes no spike.
- **Wave composition is defined by data** (`waves` in `base.ron`), not code; the content reference
  shows the number of enemies in waves 1, 5, 10 and 20. The game did not change (verified by a test).
- An English version of all documentation (`docs/en/`, `README.en.md`, `CHANGELOG.en.md`); a test
  makes sure every document exists in both versions with the same sections, and both content
  references are generated from the data.

### Changed
- The cost of going around a building is based on its full health, not its current health.

## 0.4.0 — 2026-11-17

Save format 4.0.

### Added
- **Map generator**: terrain from integer noise (the same map on any platform), ores
  with guarantees near the start, a cleared area for the core, enemy spawn points on the edges with a guaranteed path.
- **New game screen**: map size, difficulty (wave pace), seed. You start with a core and 300 ingots.
- **Schematics**: copy with a frame (C), rotate (R), paste, `.fschem` files (F6/F7) storing buildings by
  name.
- **Burner** `burner`: 12 power/tick while carbon burns.
- **Depot** `depot`: a buffer for 300 items, outputs only forward.
- **Arc tower** `arc_tower`: powered by the grid, an instant discharge that ignores armor.
- **Enemies**: `spitter` (from wave 3) shoots buildings from 4 tiles; `shellback` (from wave 6) with armor 8.
- Documentation: [player guide](docs/en/player-guide.md),
  [content reference](docs/en/content-reference.md) (generated from data and checked by a test),
  [content format](docs/en/content-format.md), [testing](docs/en/development/testing.md).

### Changed
- Turrets reload between waves too.
- Wind turbine output is computed once per power network rebuild: −6.4% instructions per tick in
  the B3 megabase.

## 0.3.0 — 2026-11-17

Save format 3.0.

### Added
- **Combat**: `crawler` and `brute` enemies, a flow field (enemies go around strong buildings and break
  through them if the detour is too long), building health and destruction, a gun turret on ingots with
  target leading, a wall, waves on a timer, defeat when the last core is lost.
- **Fluids and heat**: pipes, condenser, cooling tower, heat pipes, radiator, heat engine;
  overheating slows down the steel forge.
- Building tabs, the B5 benchmark (mass battle), `scripts/check.sh`.

### Fixed
- An overheated forge ran twice as fast as the model (truncated fractional heat).

## 0.2.0 — 2026-11-17

Save format 2.0.

### Added
- **Economy**: building costs paid from the core, refunds on removal.
- Splitter, sorter with a filter, bridge, battery; the building inspector.
- Main menu and pause menu, a clickable building panel, help (F1), HiDPI scaling.

### Fixed
- Removing a building did not reset the conveyor link cache: items could leave through a stale link.

## 0.1.0 — 2026-11-15…16

Save format 1.0.

### Added
- A headless simulation with a fixed 30 Hz step: belts (segments with sleeping), drills, smelter,
  power networks, core; determinism through integer math and a custom PRNG.
- A headless runner, B1–B3 benchmarks, CI on Windows, Linux and macOS.
- Replay: recording commands with checkpoint hashes and verifying playback on another OS in CI.
- A wgpu graphical client: a procedural atlas in brand colours, camera, building with the mouse, F5/F9
  saves.
- Reference measurements on the owner's PC.
- Stage 0 documentation: architecture, genre analysis, compatibility, roadmap, licensing, decisions.
