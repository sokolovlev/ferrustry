# Compatibility

Ferrustry is an original game, so compatibility means **Ferrustry's compatibility with
itself** across versions and platforms. Compatibility with formats, servers and mods of other games
is **not a goal**.

## 1. Format matrix

| Format | Purpose | Status | Versioning policy |
|---|---|---|---|
| Map (`.fmap`) | Terrain, ore, starting buildings, rules | Not implemented | Read all later versions of the format ≥ 1.0 |
| Save (`.fsave`) | Full simulation state | **Partial** (format 5.0 since 2026-11-18 — damage in flight and projectile targets, the flow field change log; 4.x and older files are rejected with a version error — no migrations before release): container v1.0, round-trip and continuation with the same hash are tested; content ID remapping and migrations are not implemented (a file with different content is rejected) | Read saves of the previous minor game version; migrations |
| Schematic (`.fschem`, + a base64 text string) | A build fragment to copy | **Implemented** (without the text string): buildings by content **name**, bounds checking | Always backward compatible; an unknown building is a clear error |
| Content (RON) | Resources, blocks, units, recipes | **Implemented**: items, fluids, floors, ores, 20 building kinds, enemies ([format](content-format.md)); wave composition is still in code | The schema is versioned, validator |
| Network protocol | Client ↔ server | Not implemented | Exact `protocol_version` match |
| Replay (`.freplay`) | Initial world + command stream + checkpoint hashes | **Implemented**: container v1.0, sections NAMES/META/WORLD/INPUTS/CHECKPOINTS | Only the same simulation version (`SIM_VERSION`) and the same content |

## 2. Container format (draft)

```text
magic      : 8 bytes  "FERRUSTR"
kind       : u8       (map / save / schematic / replay)
version    : u16 major, u16 minor
flags      : u32      (compression etc.)
sections   : [ { id: u32, len: u64, crc32: u32, data } ]   # section data is zstd compressed
```

- Implemented sections: `1` — content name table, `2` — world state (postcard).
  The file is checked by the CRC32 of each section, a 512 MiB decompression limit and the world's structural integrity.
- v1.0 limitation: the world section is a direct serialization of simulation structures; changing their
  layout requires a version bump and a migration.
- Unknown sections are skipped (forward compatibility for minor versions).
- Content is saved **by string names** with a "name → ID" table in the file, so that changing the
  content order does not break saves.
- Each format gets a set of reference files in `tests/fixtures/` and "load → save →
  compare" and "old version → load" tests.

## 3. Platform matrix

| Platform | Target | Status |
|---|---|---|
| Linux x86_64 | Client + server | CI green: headless, offscreen rendering (lavapipe), window and mouse-driven UI (Xvfb) |
| Windows x86_64 | Client + server | CI green (build, tests, replay from Linux); the client was checked by the owner on Windows 10; reference machine |
| macOS (Apple Silicon, x86_64) | Client + server | CI green on ARM64 (build, tests, replay from Linux); the client window has not been checked |
| Android | Client (later) | Not started |

## 4. Determinism

| Property | Target |
|---|---|
| Same binary, same platform | Bit-identical state hash — **checked by tests** |
| Different platforms | **Checked in CI**: a replay recorded on Linux x86_64 plays back on Windows x86_64 and macOS ARM64 |
| Different game versions | Not guaranteed |

Multiplayer does not rely on cross-platform determinism (authoritative server), but it is
needed for replays and regression tests.
