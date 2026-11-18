# Licensing

## 1. Current state

- The code is distributed under **MIT OR Apache-2.0** (decision D-12): `LICENSE-MIT`, `LICENSE-APACHE`.
- Ferrustry is an original game. We do **not** use code, assets, sounds, texts, content names,
  balance tables or file formats of other games. Therefore the GPL or other copyleft licenses
  of other projects impose no requirements on Ferrustry.

## 2. Rules for contributors

1. Do not copy or "translate to Rust" code of other games, including open-source games of the genre.
   Ideas and well-known algorithms are fine, with a link to the public source in a comment or in the docs.
2. Do not add assets without stating the author and license (`assets/ATTRIBUTION.md`).
3. Do not use names, logos or recognizable elements of other games in content.
4. Dependencies only with compatible licenses (section 4); checked in CI.

## 3. Licenses (decisions.md, D-12 — approved)

| What | Proposal | Why |
|---|---|---|
| Code | MIT OR Apache-2.0 (dual) — **approved** | The Rust ecosystem standard; Apache-2.0 adds an explicit patent grant |
| Assets (graphics, sound) | CC BY-SA 4.0 — **approved** | BY-SA keeps derived assets open; BY would be simpler for modders |
| The Ferrustry name and logo | Not under an open license (trademark policy) | So that forks cannot pass themselves off as the official build |
| Fonts (Exo 2, Inter, JetBrains Mono) | SIL Open Font License 1.1 | Allows embedding in the game; requires including the OFL text |

## 4. Dependencies

Allowed: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode-3.0, MPL-2.0 (as a dependency), CC0.
Need approval: LGPL, GPL, AGPL, any non-standard licenses.

Check: `cargo deny check licenses bans sources` in CI (`deny.toml` in the root).

Preliminary check of candidates (licenses per crates.io as of 2026-11-15):

| Crate | Version | License | Purpose |
|---|---|---|---|
| wgpu | 30.x | MIT OR Apache-2.0 | Graphics |
| winit | 0.30.x (stable) | Apache-2.0 | Windows, input |
| glam | 0.34.x | MIT OR Apache-2.0 | Math |
| hecs | 0.11.x | MIT OR Apache-2.0 | ECS (candidate) |
| serde | 1.x | MIT OR Apache-2.0 | Serialization |
| postcard | 1.x | MIT OR Apache-2.0 | Binary packet serialization |
| tokio | 1.x | MIT | Async networking |
| tracing | 0.1.x | MIT | Logging/tracing |
| criterion | 0.8.x | MIT OR Apache-2.0 | Micro-benchmarks |
| rayon | 1.x | MIT OR Apache-2.0 | Parallelism (after profiling) |
| kira | 0.12.x | MIT OR Apache-2.0 | Audio (candidate) |
| renet | 2.x | MIT OR Apache-2.0 | Networking (candidate) |

The licenses in the table must be re-checked with `cargo deny` when a dependency is added — this is
part of the acceptance criteria of the task that adds it.

## 5. `unsafe`

All crates have `unsafe_code = "deny"`. The only exception is `crates/ferrustry-headless/src/alloc.rs`
(an allocation counter for benchmarks: `GlobalAlloc` requires an `unsafe impl`), with SAFETY comments.
