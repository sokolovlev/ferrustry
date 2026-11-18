# Development

## Requirements
- Rust stable (tested on 1.97.0), `rustfmt`, `clippy`.

What the tests, e2e, rendering and performance measurements check and how: [testing.md](testing.md).
Content data format: [content-format.md](../content-format.md).

## Checks before committing

One command (stops at the first error): `scripts/check.sh`. Run it after the
last edit, right before committing — CI on 7bf08cd failed on formatting
because of an edit made after the checks.

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

## Headless
```bash
cargo run --release -p ferrustry-headless -- run --ticks 3000 --save slice.fsave
cargo run --release -p ferrustry-headless -- resume slice.fsave --ticks 1000
cargo run --release -p ferrustry-headless -- save-check
FERRUSTRY_COMMIT=$(git rev-parse --short HEAD) cargo run --release -p ferrustry-headless -- bench b2 --conveyors 10000 --out bench.json
cargo bench -p ferrustry-sim
scripts/bench-suite.sh          # full suite with medians (Windows: scripts/bench-suite.ps1)
```

## CPU requirements
On x86_64 the build targets AVX2 (x86-64-v3, `.cargo/config.toml`, decision D-14). On ARM
(Apple Silicon) the baseline aarch64 level is used (NEON is always present).

## Code rules
- The simulation (`ferrustry-sim`) does not depend on graphics, networking, time or threads; it changes only
  through `Command` and `World::tick`/`step`.
- No `HashMap` iteration, `thread_rng`, system time or float transcendental functions
  in logic that affects state.
- Every new system comes with a resource conservation test (`assert_conserved`) and `World::validate`.
- `unwrap`/`expect` in runtime code is forbidden by a lint; allowed in tests.
- Documentation is part of the change: a new mechanic is described in the player guide, a data change
  means regenerating the content reference, a notable change gets a line in the changelog,
  measurements go into the performance doc. **Both languages** are updated: Russian in `docs/`,
  English in `docs/en/` (see "Two languages" below). Tests in `ferrustry-content` catch the
  reference and guides falling behind the data.

## Two languages

The documentation exists in Russian (`docs/`, `README.md`, `CHANGELOG.md`) and English (`docs/en/`,
`README.en.md`, `CHANGELOG.en.md`), with the same file names and sections. The test
`english_docs_mirror_russian` in `ferrustry-content` fails if a document exists in only one language
or if the two versions have a different number of sections. It cannot check that the text says the
same thing — a reviewer does that. Both content references are generated from the same data.

## Client

```bash
cargo run --release -p ferrustry-client                         # window
cargo run --release -p ferrustry-client -- --screenshot shot.png --ticks 600 --zoom 40 --center 96,61
cargo run --release -p ferrustry-client -- --frames 120         # window smoke test
```

Linux: `libvulkan1` (or Mesa) and `libxkbcommon-x11-0` are needed. Without a GPU, software
Vulkan works (`mesa-vulkan-drivers`, lavapipe) — that's how rendering is checked in CI.

The test `ferrustry-render/tests/offscreen.rs` renders a frame on a real GPU and checks pixels
(brand colours, the core emblem); without an adapter it is skipped with a `SKIPPED` message.
