# Changelog

[Русский](CHANGELOG.md) · English

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Before 1.0,
saves are incompatible between minor versions (the game reports a format version mismatch).

## Unreleased

### Added
- An English version of all documentation (`docs/en/`, `README.en.md`, `CHANGELOG.en.md`); a test
  makes sure every document exists in both versions with the same sections, and both content
  references are generated from the data.

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
