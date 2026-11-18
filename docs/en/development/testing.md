# Testing

Before every commit: `scripts/check.sh` (fmt, check, clippy `-D warnings`, tests, release build).
Run it **as a separate command** and look at the exit code: not through `| tail`, otherwise the failure
gets lost (that's how commit 18442a2 with a failing clippy got into the repository).

## What the tests check

| Where | What | Technique |
|---|---|---|
| `ferrustry-sim/tests/sim.rs` | Belts, drills, smelter, power, saves | Resource conservation (`assert_conserved`), `World::validate`, random building (`RandomBuilder`) checking invariants and reproducibility |
| `ferrustry-sim/tests/stage3.rs` | Economy, splitter, sorter, bridge, batteries | Regression: the test for stale links after removal fails without the fix |
| `ferrustry-sim/tests/thermal.rs` | Fluids and heat | Exact number of cycles when overheated (9 vs 50) |
| `ferrustry-sim/tests/combat.rs` | Flow field, turrets, waves, defeat | Combat determinism and continuation after save/load |
| `ferrustry-sim/tests/content.rs` | Fuel, depot, arc tower, armor, ranged enemies, overkill | Exact damage per tick; a mixed battle across save/load; exactly 4 shots from five turrets at one crawler; damage-in-flight bookkeeping holds every battle tick |
| `ferrustry-sim/src/*` (unit tests) | Segments, PRNG, map generator, schematics, flow field | Generator guarantees on 36 maps; 4 schematic rotations = the original; flow field repair ≡ full rebuild on random building |
| `ferrustry-format` | Saves, replay, schematics | Round-trip and continuation with the same hash; corrupted and truncated files; different content; wrong version or file kind; locating a tampered command in a replay; schematics with name remapping |
| `ferrustry-content` | Content validation, reference | The reference and guides don't fall behind the data; English docs mirror the Russian ones (see below) |
| `ferrustry-render/tests/offscreen.rs` | Rendering a frame on the GPU | Brand colour pixels, the core emblem, the discharge glow; without a GPU — `SKIPPED` |
| `ferrustry-client` (unit tests) | UI logic without a window | Clicks on menus and the hotbar, building, copy and paste |

### Differential tests

Sleeping belts and drills is an optimization that must not change the result. The tests build two worlds:
a normal one and one with `set_sleep_enabled(false)`, run them identically and compare `state_hash` along
the way. Any new wake-up logic must pass them.

### Documentation under test

- The content references are generated from `base.ron`; `reference_doc_is_up_to_date` fails
  if either the Russian or the English file is stale. Update: `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`.
- `guides_cover_all_content`: every enemy is described in the player guide and every building kind
  in the content format description, in both languages.
- `english_docs_mirror_russian`: every Russian document has an English counterpart with the same
  number of sections, and vice versa.

## End-to-end checks (in CI on Linux)

| Check | Command |
|---|---|
| Save → load → same hash | `cargo run --release -p ferrustry-headless -- save-check` |
| Replay on another OS | `record` on Linux, `replay` on Windows and macOS (ARM) — hashes match |
| Client screenshot | `ferrustry --screenshot client.png --ticks 600 --zoom 40 --center 96,61` |
| The window opens | `xvfb-run ... ferrustry --frames 120` |
| Mouse-driven UI | `scripts/e2e-ui.sh target/release/ferrustry out/` — Xvfb + xdotool |

`scripts/e2e-ui.sh` clicks at screen coordinates of a 1280×720 window. If the layout of the menu,
the new game screen or the number of slots in the LOGISTICS tab changes, the coordinates in the script
must be updated (the comments next to them explain the calculation).

## Performance

- Time: `ferrustry-headless bench <b1|b2|b2-blocked|b2-comb|b3|b5|b8>` (`--full-flow` is the reference
  without flow field repair; the state hash must match), the full suite with medians:
  `scripts/bench-suite.sh` (Windows: `scripts/bench-suite.ps1`). On a VM, time is noisy by ±10%.
- **Instructions per tick** is a stable metric for comparing versions on one machine:

  ```bash
  for t in 300 600; do
    valgrind --tool=callgrind --callgrind-out-file=cg.$t \
      target/release/ferrustry-headless bench b3 --conveyors 100000 --warmup 0 --ticks $t >/dev/null
  done
  # (total of cg.600 − total of cg.300) / 300 = instructions per tick without setup cost
  ```

  Compare against the previous commit built in a separate worktree. Results go into
  [docs/en/performance.md](../performance.md).
- Allocations per tick and peak memory are in the `bench` output (`allocations_per_tick`, `peak_rss_kib`).
