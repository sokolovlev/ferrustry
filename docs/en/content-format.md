# Content format (`base.ron`)

All game content — items, ores, buildings, enemies — is described as data in
`crates/ferrustry-content/data/base.ron` (RON format, decision D-06). The simulation code knows only
**behaviour kinds** (`kind`); specific buildings are their parameters. A new building with an existing
behaviour is an edit of one file, with no code changes.

The file is embedded into the binary at build time (`ContentRegistry::base()`), so an edit needs
a rebuild. After changing numbers, update the references:

```bash
FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference   # regenerate both content references
scripts/check.sh                                                # all checks
```

## General rules

- References between definitions are by **name** (`ammo: "iron_ingot"`); numeric IDs are assigned at
  load time in file order. Saves store a name table, schematics store building names.
- Names are unique within their section. A repeat is a `duplicate block `...`` error.
- Unknown fields are forbidden (a typo in a field name is a load error, not a silently ignored field).
- All numbers are integers. Time is in **ticks** (30 ticks = 1 s), movement distances are in 1/256 tile,
  building ranges are in tiles.
- Parameters marked "> 0" are checked: zero is an error with the field path, for example
  `blocks[5] `smelter`.craft_ticks: must be greater than zero`.

## File sections

```ron
(
    floors: [ (name: "basalt", buildable: true), ... ],
    items:  [ (name: "iron_ore"), ... ],
    fluids: [ (name: "water") ],                 // optional
    units:  [ (name: "crawler", ...), ... ],     // optional
    ores:   [ (name: "iron_vein", item: "iron_ore"), ... ],
    blocks: [ (name: "conveyor", size: 1, cost: [("iron_ingot", 1)], kind: Conveyor(speed: 16)), ... ],
)
```

| Section | Fields |
|---|---|
| `floors` | `name`; `buildable` — whether you can build on it and enemies can walk on it |
| `items` | `name` |
| `fluids` | `name` |
| `ores` | `name`; `item` — what a drill mines from it |
| `units` | see [Enemies](#enemies) |
| `blocks` | `name`; `size` 1..=8 (side in tiles); `cost` — a list of `("item", amount > 0)`, free by default; `health` — 0 or omitted = 60 × size²; `kind` — the behaviour |

## Building kinds (`kind`)

| Kind | Parameters | Constraints |
|---|---|---|
| `Conveyor` | `speed` — path units per tick (tile = 256, item spacing = 64) | size 1; speed > 0 |
| `Splitter` | — | size 1 |
| `Sorter` | — (the filter is set by the player) | size 1 |
| `Bridge` | `range` in tiles, `interval` ticks per item, `buffer` | size 1; all > 0 |
| `Depot` | `capacity` — items in total | capacity > 0; size ≤ 8 |
| `Drill` | `base_ticks` — ticks per item with ore under every tile; `buffer` | > 0 |
| `Crafter` | `inputs`, `outputs` — lists of `("item", n)`; `craft_ticks`; `power_use` per tick (0 — no power); `buffer`; heat: `heat` per working tick (default 0), passive `cooling` (0), `heat_capacity` (100) | outputs not empty; at most 4 different items per side; buffer ≥ any n in the recipe |
| `Core` | `capacity` — of each item | > 0 |
| `Generator` | `power` per tick; optional: `fuel: Some("item")`, `burn_ticks`, `buffer` | power > 0; with fuel, burn_ticks and buffer > 0 |
| `PowerNode` | `range` in tiles | > 0 |
| `Battery` | `capacity` of power | > 0 |
| `HeatEngine` | `max_heat` per tick, `power_per_heat_permille` (500 = 50%), `heat_capacity` | max_heat, heat_capacity > 0 |
| `Pipe` | `capacity` — volume | > 0 |
| `Condenser` | `fluid`, `rate` per tick, `power_use`, `capacity` | rate, capacity > 0 |
| `CoolingTower` | `fluid`, `rate` per tick, `cooling` heat per unit of fluid, `heat_capacity` | rate, heat_capacity > 0 |
| `HeatPipe` | `heat_capacity` | > 0 |
| `Radiator` | `dissipation` per tick, `heat_capacity` | heat_capacity > 0 |
| `Turret` | `range` in tiles, `damage`, `reload` ticks, `ammo` item, `shots_per_item`, `projectile_speed` (1/256 tile per tick), `buffer` items | all except damage > 0 |
| `ArcTurret` | `range`, `damage`, `reload` (at full power), `power_use` per recharge tick | all > 0 |
| `Wall` | — | — |

Example — a new building with no code changes:

```ron
(
    name: "steel_wall",
    cost: [("steel", 6)],
    size: 1,
    health: 1200,
    kind: Wall,
),
```

## Enemies

```ron
(name: "spitter", health: 50, speed: 14, damage: 8, cooldown: 45, radius: 80, range: 4, projectile_speed: 40),
```

| Field | Meaning | Constraints |
|---|---|---|
| `health` | Health | > 0 |
| `speed` | 1/256 tile per tick | > 0 |
| `damage` | Damage per hit or projectile | — |
| `cooldown` | Ticks between attacks | > 0 |
| `radius` | Size (1/256 tile): separation and hits | > 0 |
| `armor` | Subtracted from projectile damage, damage no lower than a third. Default 0 | — |
| `range` | Tiles; 0 (default) — melee | — |
| `projectile_speed` | Projectile speed of a ranged enemy | > 0 if range > 0 |

## Waves

```ron
waves: [
    (unit: "crawler", from_wave: 1, count: 5, grow: 2, every: 1),
    (unit: "brute", from_wave: 2, count: 1, grow: 1, every: 2),
],
```

Each rule: from wave `from_wave` on, `count` enemies of kind `unit` arrive, and every `every` waves there
are `grow` more: in wave N — `count + grow × ⌊(N − from_wave) / every⌋`. One kind may have several
rules (for example, a second "reinforcement" rule from wave 10). The resulting numbers
for waves 1, 5, 10 and 20 are shown in the content reference.

| Field | Meaning | Constraints |
|---|---|---|
| `unit` | Enemy name from the `units` section | Exists |
| `from_wave` | First wave (from 1) | > 0 |
| `count` | How many in the first wave | Not 0 together with `grow` |
| `grow` | Growth, default 0 | — |
| `every` | Growth every how many waves, default 1 | > 0 |

A new enemy joins the waves as soon as it has a rule — no code changes needed.

## What needs code

A new behaviour **kind** (a new `kind` variant) means changes to `ferrustry-content` (schema and validation)
and `ferrustry-sim` (state table, tick system, item acceptance, removal, `validate`, `inspect`),
as well as the renderer (a sprite in `atlas.rs`) and the client (a tab in `CATEGORIES`, tooltip, inspector).
The compiler points to almost every place: matches on `BlockKind` and `KindTag` are exhaustive.
A building without its own drawing in `atlas.rs` gets a placeholder — a frame with a diagonal cross;
an enemy gets the crawler silhouette.
