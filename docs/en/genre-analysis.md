# Genre analysis

Replaces `original-analysis.md` from the original brief: Ferrustry is an original game, so we
analyse **the genre and engineering techniques**, not a single game as a model to reproduce.

Project rule (see licensing.md): we **do not copy** code, assets, texts, content names,
numeric balance or file formats of other games. Studying published ideas and well-known
algorithms is fine.

## 1. The genre

"Factory + tower defense": the player mines resources, builds production chains and logistics,
and the output feeds defences against enemy waves and an army. Well-known games of the genre and
adjacent genres: Mindustry (factory + TD + RTS), Factorio (factory + survival), Satisfactory (3D factory),
Shapez (abstract automation), They Are Billions / Creeper World (wave defence).

What the genre has in common and what the Ferrustry engine must handle:

| Mechanic | Engineering problem | Our approach (see architecture.md) |
|---|---|---|
| Thousands of conveyors with items | O(items) updates eat the tick | Line segments with gaps, O(segments) |
| Large power networks | Recomputing connectivity on every change | Union-find + local rebuild on removal |
| Fluids | Numerical instability, expensive iterations | Compartment model by segment |
| Hundreds/thousands of units walking to the base | A* per unit is expensive | Flow fields per movement class and goal |
| Turrets and projectiles | O(n²) target and hit search | Uniform spatial grid |
| Multiplayer on large bases | State volume | Authoritative server, snapshot + deltas/events |
| Saving large worlds | Size and speed | Chunks, dense arrays, zstd |

## 2. Public sources on technique

- The Factorio developer blog (Friday Facts): descriptions of optimizing transport lines,
  power networks and fluids — the general idea "a line = one object" is widely known in the genre.
- Flow fields for mass movement in RTS (Elijah Emerson, "Crowd Pathfinding and Steering
  Using Flow Field Tiles", Game AI Pro, 2013).
- Gaffer On Games (Glenn Fiedler): "Fix Your Timestep!", articles on network synchronization and
  reliable UDP.
- Data-oriented design (Richard Fabian, "Data-Oriented Design").

We use these sources as ideas; the implementation is our own.

## 3. What makes Ferrustry its own game

Game design is the product owner's responsibility. Below are **proposals** for discussion
(decisions.md, D-01, D-02), not approved decisions.

1. **The "Reforge" theme**: colonizing a dead industrial world; the player restores
   abandoned ruined factories (repair instead of building from scratch, as a separate mechanic).
2. **Heat as a third network** alongside power and fluids: production emits heat that
   must be removed or used — this creates its own layout puzzles.
3. **Waves that react to the base**: the enemy "hears" the noise/heat of production instead of just
   arriving on a timer — this ties the economy to defence.
4. An original resource tree (iron, carbon, steel, silica, …) with its own names,
   recipes and balance.

## 4. Genre risks for us

- **Logistics performance** is the central risk; the first vertical slice is built around it.
- **Multiplayer on large bases** — synchronization volume; we lay down an authoritative model from stage 1
  (commands as the only input into the simulation).
- **Balance and content** — the amount of game design work is large; a dedicated content owner is needed.
- **Similarity to existing games** — we keep the differences in theme, content, visuals and mechanics
  deliberate and documented.
