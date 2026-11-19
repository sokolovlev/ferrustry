# Content reference

> Generated from `crates/ferrustry-content/data/base.ron` — do not edit by hand.
> Update: `FERRUSTRY_BLESS=1 cargo test -p ferrustry-content reference`.
> Times assume 30 ticks/s. Mechanics are explained in the [player guide](player-guide.md).

## Items and fluids

| Item | Mined from |
|---|---|
| `iron_ore` | `iron_vein` |
| `carbon` | `carbon_seam` |
| `iron_ingot` | produced |
| `steel` | produced |

Fluids: `water`.
Floors: `basalt`, `ash`, `slag_pool` (not buildable).

## Buildings

### Logistics

| Building | Size | Cost | Health | What it does |
|---|---|---|---|---|
| `conveyor` | 1×1 | 1 `iron_ingot` | 60 | 1.88 tiles/s, up to 7.5 items/s |
| `depot` | 2×2 | 20 `iron_ingot` | 240 | stores 300 items: accepts from three sides, outputs forward up to 30 items/s |
| `splitter` | 1×1 | 3 `iron_ingot` | 60 | hands out items to its three other sides in turn |
| `sorter` | 1×1 | 4 `iron_ingot` | 60 | filter items go straight, the rest alternate sideways |
| `bridge_conveyor` | 1×1 | 6 `iron_ingot` | 60 | throws items up to 4 tiles to the next bridge; 7.5 items/s; buffer 4 |

### Production

| Building | Size | Cost | Health | What it does |
|---|---|---|---|---|
| `bore` | 2×2 | 10 `iron_ingot` | 240 | ore under all tiles (4): 0.75 items/s; proportionally slower with less ore; buffer 10; **noise 1** while working |
| `smelter` | 2×2 | 25 `iron_ingot` | 240 | 2 `iron_ore` → 1 `iron_ingot` in 1 s; power 4/tick; buffer 10; **noise 2** while working |
| `steel_forge` | 2×2 | 40 `iron_ingot` | 240 | 2 `iron_ingot` + 1 `carbon` → 1 `steel` in 2 s; power 10/tick; buffer 10; heat +6/tick while working, passive cooling 1/tick; **noise 4** while working |
| `core` | 3×3 | 500 `iron_ingot` + 100 `steel` | 3000 | player storage: up to 4000 of each item; losing the last one loses the game |

### Power

| Building | Size | Cost | Health | What it does |
|---|---|---|---|---|
| `wind_turbine` | 1×1 | 10 `iron_ingot` | 60 | +2 power/tick, always |
| `burner` | 2×2 | 25 `iron_ingot` | 240 | +12 power/tick while `carbon` burns (1 item = 4 s); fuel buffer 10; **noise 3** while working |
| `power_node` | 1×1 | 4 `iron_ingot` | 60 | links power buildings within 6 tiles |
| `battery` | 1×1 | 15 `iron_ingot` + 2 `carbon` | 60 | stores 2000 power |
| `heat_engine` | 2×2 | 40 `iron_ingot` + 10 `steel` | 240 | takes up to 8 heat/tick and returns 50% of it as power (from the next tick); heat capacity 200 |

### Fluids and heat

| Building | Size | Cost | Health | What it does |
|---|---|---|---|---|
| `pipe` | 1×1 | 1 `iron_ingot` | 60 | volume 20; connects fluid buildings |
| `condenser` | 2×2 | 30 `iron_ingot` | 240 | makes `water` 2/tick, power 3/tick; volume 40 |
| `cooling_tower` | 2×2 | 30 `iron_ingot` | 240 | uses `water` 1/tick and removes 8 heat/tick; heat capacity 100 |
| `heat_pipe` | 1×1 | 2 `iron_ingot` | 60 | connects heat buildings; heat capacity 50 |
| `radiator` | 1×1 | 6 `iron_ingot` | 60 | dissipates 2 heat/tick; heat capacity 50 |

### Defense

| Building | Size | Cost | Health | What it does |
|---|---|---|---|---|
| `gun_turret` | 2×2 | 45 `iron_ingot` | 400 | range 9, 12 damage per shot, 2 shots/s (24 damage/s); ammo `iron_ingot` — 2 shots per item, buffer 10 items; projectile 11.25 tiles/s |
| `arc_tower` | 2×2 | 40 `iron_ingot` + 15 `steel` | 350 | range 7, instant 40 damage discharge, **ignores armor**; at full power 1.2 discharges/s (48 damage/s); power 8/tick while recharging |
| `iron_wall` | 1×1 | 4 `iron_ingot` | 400 | health only; enemies prefer to go around high-health buildings |

## Enemies

| Enemy | Health | Armor | Speed, tiles/s | Attack | Damage/s |
|---|---|---|---|---|---|
| `crawler` | 40 | 0 | 2.34 | melee 6, every 0.67 s | 9 |
| `brute` | 260 | 0 | 1.17 | melee 25, every 1 s | 25 |
| `spitter` | 50 | 0 | 1.64 | projectile 8 from 4 tiles, every 1.5 s | 5.33 |
| `shellback` | 320 | 8 | 0.94 | melee 30, every 1.33 s | 22.5 |

Armor is subtracted from projectile damage, but damage never drops below a third of the original. The arc tower's discharge ignores armor.

## Waves

| Enemy | From wave | At first | Growth | Wave 1 | Wave 5 | Wave 10 | Wave 20 |
|---|---|---|---|---|---|---|---|
| `crawler` | 1 | 5 | +3 each wave | 5 | 17 | 32 | 62 |
| `brute` | 2 | 1 | +1 each wave | 0 | 4 | 9 | 19 |
| `spitter` | 3 | 1 | +1 every 2 waves | 0 | 2 | 4 | 9 |
| `shellback` | 6 | 1 | +1 every 2 waves | 0 | 0 | 3 | 8 |
| **Total** | | | | **5** | **23** | **48** | **98** |

**Noise** of the base speeds up the wave timer: +2% per unit of noise of the buildings working this tick, at most +150% (reached at 75 noise). A quiet base gets waves on the schedule above.
