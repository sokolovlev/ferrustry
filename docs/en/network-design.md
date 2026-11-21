# Multiplayer: design (stage 6)

Status: **design approved** (2026-11-20), **first playable version** (2026-11-21). Decisions D-18…D-22 are in [decisions.md](decisions.md).
Every number below is measured (`examples/net_estimates.rs`, cloud VM; the owner's PC is faster), not
promised.

## 1. The main question: what goes over the network

There are two ways to keep the world identical for all players.

| | **State replication** (snapshot + changes) | **Command relay** (deterministic simulation everywhere) |
|---|---|---|
| What is sent | Changes of the world every tick | Only player commands, ordered by the server |
| Who simulates | The server; clients display | The server and every client run the same simulation |
| Traffic grows with | Base size | Number of player actions |
| Requires | Change tracking in every system | Determinism on every platform |
| Pros | Clients don't need a strong CPU; the map can be hidden (fog of war in PvP) | Near-zero traffic; any base size; a client is the same single-player game |
| Cons | Traffic and the cost of finding changes grow with the base | Desync if determinism breaks; clients see the whole world; joining needs a snapshot |

We already have determinism and test it: an integer simulation, a state hash, and in CI a replay recorded
on Linux x86_64 plays back bit for bit on Windows x86_64 and macOS ARM64.

## 2. Measurements

Changes per tick are a lower bound: the byte difference of the serialized world before and after a tick,
compressed with zstd. A real delta protocol would send no less, and would also spend time finding changes.

| Scenario | State | Snapshot (compressed save) | Changes per tick | State traffic per client | Serializing the world | `state_hash` |
|---|---|---|---|---|---|---|
| B3-10k (14k buildings) | 682 KB | 164 KB, write 4 ms, read 6 ms | 3.4 KB (≥ 1.7 KB compressed) | **≥ 49 KB/s** | 1.4 ms | 2.4 ms |
| B3-100k (125k buildings) | 7.1 MB | 1.6 MB, write 47 ms, read 62 ms | 34 KB (≥ 15 KB compressed) | **≥ 447 KB/s** (3.6 Mbit/s) | 17 ms | 28 ms |
| B5 (1000 enemies, 300 turrets) | 127 KB | 10 KB | 3.7 KB (≥ 3 KB compressed) | ≥ 87 KB/s | 0.2 ms | 0.35 ms |

A build command (tick, player, command) is **10 bytes**. Eight players at 5 commands per second is 0.4 KB/s in total.

Conclusions:

- With state replication a megabase needs ≥ 3.6 Mbit/s for **each** client (8 players — ≥ 29 Mbit/s of server
  upload), and finding changes by serializing costs 17 ms — half the tick budget, while the simulation tick
  itself is 0.3 ms. Making it cheaper would mean building change tracking into every system — a large rework
  with a risk of bugs in each.
- Command relay doesn't depend on base size and fits in a fraction of a kilobyte per second.
- A snapshot for joining is acceptable even for a megabase (1.6 MB, read in 62 ms).
- `state_hash` is too expensive for frequent checks (28 ms on the megabase): we need a cheap checksum
  (task S6-04). **Solved** with a rolling checksum: 151 µs per tick on average on the megabase, 206 µs in the
  worst tick (~0.6% of the budget) instead of a 28 ms spike every 5 s (section 6).

## 3. The proposed model (D-18 — D-22)

**Authoritative server + command relay** (D-18):

1. The client sends a command to the server and immediately shows a building "ghost" (as single player does now).
2. The server assigns the command an execution tick — the nearest one not yet simulated — and sends everyone
   a **frame**: the tick number and that tick's commands (a frame goes out every tick, even an empty one — it is
   the "you may simulate on" signal).
3. The server and the clients simulate the same tick with the same commands. A client simulates a tick only
   once it has that tick's frame; a small buffer smooths uneven delivery (D-19).
4. Every few seconds the server sends a checksum of its state; the client compares it with its own. On a
   mismatch the client gets a fresh snapshot and continues from it (D-20); a mismatch report with the tick
   number is kept for debugging (as with replays).
5. Joining a running game: the server takes a snapshot (copying the world takes milliseconds, serialization
   runs on another thread so the tick isn't delayed), the client loads it and catches up through the frames,
   simulating faster than real time (a megabase tick is 0.3 ms, i.e. ~3000 ticks per second) (D-21).

A player's action delay = the round trip to the server + the buffer. For building in a factory game this is
acceptable, and the ghost hides the feeling of delay. Command rejections (no resources, tile occupied) are
decided by the simulation identically everywhere.

The server is the same `ferrustry-sim` without graphics; the transport follows D-08 (renet). For tests — an
in-memory transport: a server and several clients in one process, checking "all state hashes match" over
thousands of ticks.

## 4. Not in the first version

- PvP with fog of war: with command relay a client has the whole world. The first version is co-op (D-22).
- Client-side anti-cheat: the server checks a player's right to a command, but the client sees everything.
- Reconnecting while keeping one's place: after a disconnect the client joins again through a snapshot.
- Multithreaded simulation by regions (D-17, option B): the command model doesn't depend on it — regions can
  be added later without changing the protocol, as long as determinism is kept.

## 5. Risks

| Risk | What we do |
|---|---|
| A determinism bug → desync | Already: integers, a ban on non-deterministic APIs, cross-OS replays in CI. To add: a "server + 3 clients" test over thousands of ticks, a checksum with the tick number, automatic recovery through a snapshot |
| A slow client falls behind | The client simulates several ticks per frame (as with speed-up now); if it falls behind by more than N seconds — a warning and a rejoin |
| A server hitch while taking a snapshot | Copy the world on the main thread, serialize and compress in the background |
| An expensive checksum | Task S6-04: a fast hash (not through serialization) or checking parts of the world in turn |

## 6. Implementation status

| Task | Status |
|---|---|
| S6-01 protocol | **Done** (2026-11-20) without a chunked snapshot: `ferrustry-net` — messages `Hello`/`Command` and `Welcome`/`Reject`/`Frame`/`Checksum` (postcard), the protocol version, the simulation version and a **content fingerprint** (`ContentRegistry::fingerprint` — names and numbers: mods with different numbers won't join), id checks in commands, a message size limit, an in-memory channel |
| S6-02 session | **Done** (2026-11-20): `Server` (joining, frames, checksums, dropping misbehaving clients), `Client` (snapshot, simulating by frames, checking). Test: a server and 3 clients over 2000 ticks with building and combat, the third joins a running game and catches up in batches — the state matches bit for bit for everyone; incompatible clients and a full server are rejected; tampering with a client's state is caught at the first check |
| S6-04 checks | **Done** (2026-11-21): a rolling checksum — after every tick part `t % 150` of the world is hashed (`World::checksum_part`), the whole state every 5 s. The canonical representation (as in a save) is hashed, so a client that joined through a snapshot matches the server even though its internal memory layout differs (the test fails if the layout is hashed). On a mismatch the client requests a snapshot itself, doesn't simulate until it arrives, and continues from it |
| S6-05 network | **Done** (2026-11-21): UDP via renet 2 + `renet_netcode` (D-08) — a reliable ordered channel, large messages (the snapshot) are split into packets; unencrypted mode (direct address). `ferrustry-server` — a 30 Hz dedicated server (a `.fmap` map, a save or the generator; paused without players); the client — the JOIN button (address and name fields; the end-to-end test `scripts/e2e-net.sh`: a real server process, a window, keyboard input, joining and leaving through the menu) and `--connect`. Tests: 2 clients and a 20k megabase over real UDP on 127.0.0.1 match the server; the game connects to a server over UDP, builds and matches it. Live: the game window and the server process — `ONLINE PLAYER 1 PING 16 MS`, 30 TPS |
| S6-03 | Ahead: a background snapshot (a player joining currently delays a megabase tick by ~50 ms) |
