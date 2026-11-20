# Player guide

Ferrustry is a game about factory automation and base defense. You mine ore, process it
in factories connected by belts, and build defences: waves of enemies march on your core.

The exact numbers for every building and enemy are in the [content reference](content-reference.md)
(generated from the game data, always up to date). This guide explains how everything works and how to use it.

## Running the game

```bash
cargo run --release -p ferrustry-client
```

You need a CPU with AVX2 (x86_64) or Apple Silicon and a GPU with Vulkan, DirectX 12 or Metal.
On Linux, the `libvulkan1` and `libxkbcommon-x11-0` packages.

## New game

**NEW GAME** opens the settings:

| Setting | Options | What it changes |
|---|---|---|
| Map size | small 128×96, medium 192×128, large 320×200 | Area and number of enemy spawn points (2 / 3 / 4) |
| Difficulty | easy, normal, hard | Wave pace: the first wave after 8 / 5 / 3 min, then every 3 / 2 / 1.5 min |
| Seed | a number or RANDOM | The same seed gives the same map on any computer |

The map is generated so the start is always fair: the core is in the centre with a flat area around it,
iron ore 11–20 tiles away, carbon 14–24 tiles away, and every enemy spawn point has a path to the core.
You start with only the core and **300 iron ingots**.

## Map editor

**MAP EDITOR** in the main menu opens the editor on a map made by the generator with the new game
settings (size, difficulty, seed). The tools are on the left, the actions at the bottom.

| Tool | What it does |
|---|---|
| BASALT, ASH, SLAG POOL | Paints the floor with the brush. You can't build on slag and enemies can't walk on it; ore is removed from it |
| IRON VEIN, CARBON SEAM | Lays ore (only on floor you can build on) |
| CLEAR ORE | Removes ore |
| ENEMY SPAWN | A click places an enemy spawn point (a red square), a click next to one removes it. Up to 16 points |
| CORE | Places a core |
| REMOVE BUILDING | Removes buildings under the brush |
| BRUSH | Brush size: 1, 3, 5 or 9 tiles |
| WAVES | Wave pace: off, easy, normal, hard |
| START | Starting stock of iron ingots |

LMB paints (you can drag), RMB erases ore and buildings. Floor and ore under buildings are not
changed. The camera works as in the game (MMB, WASD, wheel).

**GENERATE** makes a new random map, **BLANK** an empty map of the same size, **SAVE** / **LOAD**
save to / load from `ferrustry-map.fmap`, **PLAY** starts a game on this map, **BACK** or **Esc**
returns to the main menu. Before starting, the editor checks that there is a core and, with waves on,
that there are spawn points and each of them has a path to a core.

The map file stores floor, ore and buildings by name, so it survives game updates; if something on
the map no longer exists in the game, loading tells you what.

## Controls

| Key / mouse | Action |
|---|---|
| **1–6** or click a slot | Choose a building in the tab |
| **Tab** or click a tab | Switch tab (LOGISTICS, PRODUCTION, DEFENSE, POWER, FLUIDS+HEAT) |
| LMB | Build |
| LMB drag | A line of buildings; belts turn the corner |
| **R** | Rotate |
| RMB | Drop the tool |
| RMB drag | Remove everything in the frame (the cost is refunded) |
| Click a building with no tool | Inspector: buffers, progress, power, heat; the sorter's filter choice |
| **C** + frame | Copy buildings into a schematic; then click to paste, **R** to rotate, RMB/Esc to cancel |
| **F6 / F7** | Save / load a schematic (`ferrustry-schematic.fschem`) |
| **WASD**, arrows, MMB | Camera |
| Wheel | Zoom |
| **Space** | Pause |
| **+ / −** | Simulation speed |
| **F5 / F9** | Quick save / load (`ferrustry-quicksave.fsave`) |
| **F1** | Help |
| **Esc** | Menu |

## Economy

Building is paid for from the core. Removing a building refunds its full cost; a building **destroyed by enemies**
refunds nothing, and its contents are lost too. The core stores up to 4000 of each item.

## Your first production chain

```
bore on iron ore → belt → smelter (+ power) → belt → core
```

1. A **bore** (2×2) goes on ore. Its speed is proportional to the number of ore tiles under it: on 4 tiles
   it makes 0.75 ore/s, on 2 tiles half as much. The bore outputs into neighbouring belts and factories.
2. A **conveyor** carries up to 7.5 items/s. Run it from the bore to the smelter.
3. The **smelter** turns 2 ore into 1 ingot per second and uses 4 power/tick. It needs ~2.7 bores
   on full ore to run without idling.
4. A **wind turbine** gives 2 power/tick — two turbines are enough for one smelter. Power buildings
   touching each other by sides form a network; a **power node** links everything within 6 tiles.
5. A belt from the smelter to the core — and ingots start piling up.

If a factory stands still, open the inspector: `WAITING FOR INPUT / OUTPUT SPACE` means it lacks
input or has nowhere to put its output.

## Logistics

- The **splitter** hands out items to its three other sides in turn.
- The **sorter**: items matching the filter chosen in the inspector go straight, the rest alternate sideways.
- The **overflow gate**: items go straight while there is room ahead; the surplus alternates sideways.
  Handy for feeding the main line first and sending the rest to a depot or to turrets.
- The **bridge conveyor** throws items up to 4 tiles over obstacles to the next bridge facing the same way.
- The **depot** (2×2) is a buffer for 300 items. It accepts from three sides and outputs only **forward**
  (the arrow on the sprite; rotate with R), one item per tick. If the next item isn't accepted,
  the depot tries the next kind, so a mix doesn't jam in front of a factory that needs only one of the items.

## Power

| Source | Output | Condition |
|---|---|---|
| Wind turbine 1×1 | 2/tick | Always |
| Burner 2×2 | 12/tick | While carbon burns: 1 carbon = 4 s, i.e. 0.25 carbon/s. Burns even when nobody needs the power |
| Heat engine 2×2 | up to 4/tick | Takes up to 8 heat/tick from the heat network and returns 50% as power |

A **battery** stores surplus and releases it during shortages. During a deficit all consumers on the network
slow down equally: at 50% power a smelter works half as fast.

## Steel and heat

The **steel forge**: 2 ingots + 1 carbon → 1 steel in 2 s, 10 power/tick. While working it heats up by 6
heat/tick and cools itself by only 1. Above 70% of its heat capacity the forge slows down
(turns red) and stops at 100%. Without cooling that happens within a couple of seconds of work.

What to cool it with (heat travels through **heat pipes** and between buildings touching by sides):

| Cooler | Removes heat/tick | Needs |
|---|---|---|
| Radiator 1×1 | 2 | Nothing — 3 radiators keep one forge going |
| Cooling tower 2×2 | 8 | Water 1/tick: a **condenser** (2 water/tick, 3 power/tick) and **pipes** |
| Heat engine 2×2 | up to 8 | Nothing; it also produces power |

## Defence

### Waves

Enemies appear at the map edges on a timer (in the corner of the screen: `WAVE N IN m:ss`) and walk to the
nearest core along the cheapest path. Each building on the way "costs" more the stronger it is, so enemies
**go around** walls if the detour isn't too long and **break through** them if there's no other way.

Each wave brings more enemies, and later waves bring new kinds. How many enemies of which kind come
in each wave is in the "Waves" section of the [content reference](content-reference.md#waves); the
composition is defined by the game data and may change as the balance is tuned.

### Base noise

Working drills, crafters and fueled generators make **noise**, and the base's noise hurries the enemy:
the wave timer runs faster — by 2% per unit of noise, but at most 2.5 times as fast. Idle buildings make
no noise. The noise is shown in the corner of the screen (`NOISE 12 (+24%)`; a quiet base shows
`QUIET`), and each building's noise is in its tooltip on the panel (`NOISE 2`). The choice is yours:
grow production fast and meet the waves sooner — or grow quietly and get time to build defences.

### Enemies

- The **crawler** (`crawler`) is fast and weak, and attacks in melee.
- The **brute** (`brute`) is slow and tough, with a heavy hit.
- The **spitter** (`spitter`) stops 4 tiles from the nearest building and shoots at it. Walls don't
  hold it back — you need a turret that reaches further than 4 tiles (the gun turret has a range of 9).
- The **shellback** (`shellback`) has armor 8: a gun turret shell (12 damage) deals it only 4. Armor never reduces
  damage below a third of the original.

### Turrets and walls

| Building | Strengths | Needs |
|---|---|---|
| **Gun turret** 2×2 | Range 9, 24 damage/s, leads its targets | Iron ingots by belt: 1 ingot = 2 shots |
| **Arc tower** 2×2 | Range 7, instant discharge, **ignores armor**, 48 damage/s | Power: 8/tick while recharging; at 50% power it fires half as often. Costs steel |
| **Iron wall** 1×1 | 400 health, cheap | — |

Turrets spread their fire: they don't shoot an enemy that shells already in flight will kill, and
pick the next target instead — several turrets don't waste ammo on a single crawler.

How difficulty grows is in the [balance report](balance/2026-11-19-normal-tuned.md): how many turrets
hold each wave and how much production it takes to pay for them.

Tip: run an ingot belt to your gun turrets in advance — without ammo a turret stays silent. Against later
waves, put arc towers behind the line of guns and don't forget the power.

If the last core is destroyed, the game is lost.

## Schematics

**C** + frame copies buildings (with rotations and sorter filters) into a schematic. Click to paste,
**R** rotates the schematic by 90°. Pasting is ordinary building: the cost is deducted, and buildings whose tiles
are occupied are simply not placed (the rest of the schematic is built). **F6** saves the schematic to a file,
**F7** loads it. The schematic file stores buildings by name, so it survives game updates; if a building
no longer exists in the game, loading tells you so.

## Saves

**F5** quick-saves, **F9** loads, **CONTINUE** in the menu resumes the last one. The simulation
is deterministic: a loaded game continues bit for bit as it would have without saving.
Saves from older game versions (before release) don't load — the game reports a version mismatch.
