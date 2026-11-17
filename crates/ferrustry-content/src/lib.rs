//! Схема контента Ferrustry, валидация и реестр с плотными числовыми ID.
//!
//! Контент описывается в RON (решение D-06). Ссылки между определениями задаются строковыми
//! именами и разрешаются при построении [`ContentRegistry`]; системы симуляции работают только
//! с числовыми ID.

use std::collections::HashMap;

use serde::Deserialize;

/// Встроенный базовый набор контента.
pub const BASE_CONTENT: &str = include_str!("../data/base.ron");

/// ID типа пола.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, Deserialize,
)]
pub struct FloorId(pub u16);

/// ID ресурса (предмета).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, Deserialize,
)]
pub struct ItemId(pub u16);

/// ID рудного месторождения (оверлей поверх пола).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, Deserialize,
)]
pub struct OreId(pub u16);

/// ID жидкости.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, Deserialize,
)]
pub struct FluidId(pub u16);

/// ID типа постройки.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, Deserialize,
)]
pub struct BlockId(pub u16);

/// Ошибка загрузки или валидации контента.
#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    /// Синтаксическая ошибка RON.
    #[error("content parse error: {0}")]
    Parse(#[from] ron::error::SpannedError),
    /// Ссылка на несуществующее определение.
    #[error("{path}: unknown {kind} `{name}`")]
    UnknownRef {
        /// Путь к полю, содержащему ссылку.
        path: String,
        /// Вид определения, на которое ссылаются.
        kind: &'static str,
        /// Имя, которое не удалось найти.
        name: String,
    },
    /// Два определения одного вида с одинаковым именем.
    #[error("duplicate {kind} `{name}`")]
    Duplicate {
        /// Вид определения.
        kind: &'static str,
        /// Повторяющееся имя.
        name: String,
    },
    /// Недопустимое значение поля.
    #[error("{path}: {reason}")]
    Invalid {
        /// Путь к полю.
        path: String,
        /// Описание проблемы.
        reason: String,
    },
    /// Слишком много определений одного вида для 16-битных ID.
    #[error("too many {0} definitions")]
    TooMany(&'static str),
}

// ---------------------------------------------------------------------------
// Сырая схема (как в файле)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContent {
    floors: Vec<RawFloor>,
    items: Vec<RawItem>,
    #[serde(default)]
    fluids: Vec<RawItem>,
    #[serde(default)]
    units: Vec<RawUnit>,
    ores: Vec<RawOre>,
    blocks: Vec<RawBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFloor {
    name: String,
    buildable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOre {
    name: String,
    item: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlock {
    name: String,
    size: u8,
    #[serde(default)]
    cost: Vec<(String, u32)>,
    /// 0 — по умолчанию (60 × площадь).
    #[serde(default)]
    health: u32,
    kind: RawKind,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUnit {
    name: String,
    health: u32,
    speed: u32,
    damage: u32,
    cooldown: u32,
    radius: u32,
}

#[derive(Debug, Clone, Deserialize)]
enum RawKind {
    Conveyor {
        speed: u32,
    },
    Drill {
        base_ticks: u32,
        buffer: u32,
    },
    Crafter {
        inputs: Vec<(String, u32)>,
        outputs: Vec<(String, u32)>,
        craft_ticks: u32,
        power_use: u32,
        buffer: u32,
        #[serde(default)]
        heat: u32,
        #[serde(default)]
        cooling: u32,
        #[serde(default = "default_heat_capacity")]
        heat_capacity: u32,
    },
    Pipe {
        capacity: u32,
    },
    Turret {
        range: u32,
        damage: u32,
        reload: u32,
        ammo: String,
        shots_per_item: u32,
        projectile_speed: u32,
        buffer: u32,
    },
    Wall,
    Condenser {
        fluid: String,
        rate: u32,
        power_use: u32,
        capacity: u32,
    },
    CoolingTower {
        fluid: String,
        rate: u32,
        cooling: u32,
        heat_capacity: u32,
    },
    HeatPipe {
        heat_capacity: u32,
    },
    Radiator {
        dissipation: u32,
        heat_capacity: u32,
    },
    HeatEngine {
        max_heat: u32,
        power_per_heat_permille: u32,
        heat_capacity: u32,
    },
    Generator {
        power: u32,
    },
    PowerNode {
        range: u32,
    },
    Core {
        capacity: u32,
    },
    Splitter,
    Sorter,
    Bridge {
        range: u32,
        interval: u32,
        buffer: u32,
    },
    Battery {
        capacity: u32,
    },
}

fn default_heat_capacity() -> u32 {
    100
}

// ---------------------------------------------------------------------------
// Разрешённый контент
// ---------------------------------------------------------------------------

/// Тип пола.
#[derive(Debug, Clone)]
pub struct FloorDef {
    /// Уникальное имя.
    pub name: String,
    /// Можно ли строить на этом полу.
    pub buildable: bool,
}

/// Ресурс.
#[derive(Debug, Clone)]
pub struct ItemDef {
    /// Уникальное имя.
    pub name: String,
}

/// Жидкость.
#[derive(Debug, Clone)]
pub struct FluidDef {
    /// Уникальное имя.
    pub name: String,
}

/// Рудное месторождение.
#[derive(Debug, Clone)]
pub struct OreDef {
    /// Уникальное имя.
    pub name: String,
    /// Ресурс, добываемый из месторождения.
    pub item: ItemId,
}

/// Количество ресурса в рецепте.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStack {
    /// Ресурс.
    pub item: ItemId,
    /// Количество.
    pub count: u32,
}

/// Поведение постройки, параметризованное данными.
#[derive(Debug, Clone)]
pub enum BlockKind {
    /// Конвейер.
    Conveyor {
        /// Скорость в единицах пути за тик (тайл = 256 единиц).
        speed: u32,
    },
    /// Бур: добывает руду под собой.
    Drill {
        /// Тиков на предмет при полном покрытии рудой.
        base_ticks: u32,
        /// Ёмкость выходного буфера.
        buffer: u32,
    },
    /// Производство по рецепту.
    Crafter {
        /// Входы рецепта.
        inputs: Vec<ItemStack>,
        /// Выходы рецепта.
        outputs: Vec<ItemStack>,
        /// Длительность цикла в тиках при полной энергии.
        craft_ticks: u32,
        /// Потребление энергии за тик (0 — не требует энергии).
        power_use: u32,
        /// Ёмкость буфера каждого ресурса.
        buffer: u32,
        /// Тепло за тик работы (0 — не греется).
        heat: u32,
        /// Пассивное охлаждение за тик.
        cooling: u32,
        /// Теплоёмкость (вклад в тепловую сеть).
        heat_capacity: u32,
    },
    /// Турель: стреляет по врагам в радиусе, расходуя боеприпасы.
    Turret {
        /// Радиус в тайлах.
        range: u32,
        /// Урон снаряда.
        damage: u32,
        /// Тиков между выстрелами.
        reload: u32,
        /// Ресурс-боеприпас.
        ammo: ItemId,
        /// Выстрелов на один предмет.
        shots_per_item: u32,
        /// Скорость снаряда, 1/256 тайла за тик.
        projectile_speed: u32,
        /// Ёмкость в предметах.
        buffer: u32,
    },
    /// Стена: только прочность.
    Wall,
    /// Труба: соединяет жидкостные постройки в сеть.
    Pipe {
        /// Объём.
        capacity: u32,
    },
    /// Конденсатор: производит жидкость из воздуха, потребляя энергию.
    Condenser {
        /// Производимая жидкость.
        fluid: FluidId,
        /// Единиц за тик при полной энергии.
        rate: u32,
        /// Потребление энергии за тик.
        power_use: u32,
        /// Собственный объём (вклад в сеть).
        capacity: u32,
    },
    /// Градирня: тратит жидкость, снимая тепло с тепловой сети.
    CoolingTower {
        /// Потребляемая жидкость.
        fluid: FluidId,
        /// Единиц за тик.
        rate: u32,
        /// Тепла снимается на единицу жидкости.
        cooling: u32,
        /// Теплоёмкость.
        heat_capacity: u32,
    },
    /// Тепловая труба: соединяет тепловые постройки.
    HeatPipe {
        /// Теплоёмкость.
        heat_capacity: u32,
    },
    /// Радиатор: пассивно рассеивает тепло.
    Radiator {
        /// Рассеивание за тик.
        dissipation: u32,
        /// Теплоёмкость.
        heat_capacity: u32,
    },
    /// Тепловая машина: превращает тепло сети в энергию.
    HeatEngine {
        /// Максимум тепла за тик.
        max_heat: u32,
        /// Энергии на единицу тепла, тысячные.
        power_per_heat_permille: u32,
        /// Теплоёмкость.
        heat_capacity: u32,
    },
    /// Генератор с постоянной выработкой.
    Generator {
        /// Выработка энергии за тик.
        power: u32,
    },
    /// Узел энергосети.
    PowerNode {
        /// Радиус соединения в тайлах.
        range: u32,
    },
    /// Ядро — хранилище ресурсов игрока.
    Core {
        /// Ёмкость по каждому ресурсу.
        capacity: u32,
    },
    /// Разветвитель 1×1: принимает предмет с любой стороны и раздаёт по кругу на остальные три.
    Splitter,
    /// Сортировщик 1×1: предметы фильтра идут прямо, остальные — вбок (поочерёдно влево/вправо).
    Sorter,
    /// Мост: переносит предметы над тайлами к следующему мосту того же направления.
    Bridge {
        /// Дальность поиска партнёра в тайлах.
        range: u32,
        /// Тиков на передачу одного предмета.
        interval: u32,
        /// Ёмкость буфера.
        buffer: u32,
    },
    /// Батарея: накапливает избыток энергии сети и отдаёт при нехватке.
    Battery {
        /// Ёмкость в единицах энергии.
        capacity: u32,
    },
}

/// Тип постройки.
#[derive(Debug, Clone)]
pub struct BlockDef {
    /// Уникальное имя.
    pub name: String,
    /// Размер стороны в тайлах.
    pub size: u8,
    /// Стоимость постройки (списывается из ядер, возвращается при сносе).
    pub cost: Vec<ItemStack>,
    /// Прочность.
    pub health: u32,
    /// Поведение.
    pub kind: BlockKind,
}

/// ID типа юнита.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, Deserialize,
)]
pub struct UnitTypeId(pub u16);

/// Тип юнита (сейчас — враги волн).
#[derive(Debug, Clone)]
pub struct UnitDef {
    /// Уникальное имя.
    pub name: String,
    /// Прочность.
    pub health: u32,
    /// Скорость, 1/256 тайла за тик.
    pub speed: u32,
    /// Урон за удар.
    pub damage: u32,
    /// Тиков между ударами.
    pub cooldown: u32,
    /// Радиус, 1/256 тайла.
    pub radius: u32,
}

impl BlockDef {
    /// Участвует ли постройка в энергосети.
    pub fn is_power(&self) -> bool {
        match &self.kind {
            BlockKind::Generator { .. }
            | BlockKind::PowerNode { .. }
            | BlockKind::Battery { .. }
            | BlockKind::HeatEngine { .. } => true,
            BlockKind::Crafter { power_use, .. } | BlockKind::Condenser { power_use, .. } => {
                *power_use > 0
            }
            _ => false,
        }
    }

    /// Объём в жидкостной сети (`None` — не участвует).
    pub fn fluid_capacity(&self) -> Option<u32> {
        match &self.kind {
            BlockKind::Pipe { capacity } | BlockKind::Condenser { capacity, .. } => Some(*capacity),
            BlockKind::CoolingTower { rate, .. } => Some(*rate * 4),
            _ => None,
        }
    }

    /// Теплоёмкость в тепловой сети (`None` — не участвует).
    pub fn heat_capacity(&self) -> Option<u32> {
        match &self.kind {
            BlockKind::Crafter {
                heat,
                heat_capacity,
                ..
            } if *heat > 0 => Some(*heat_capacity),
            BlockKind::CoolingTower { heat_capacity, .. }
            | BlockKind::HeatPipe { heat_capacity }
            | BlockKind::Radiator { heat_capacity, .. }
            | BlockKind::HeatEngine { heat_capacity, .. } => Some(*heat_capacity),
            _ => None,
        }
    }
}

/// Реестр загруженного контента.
#[derive(Debug, Clone)]
pub struct ContentRegistry {
    floors: Vec<FloorDef>,
    items: Vec<ItemDef>,
    ores: Vec<OreDef>,
    blocks: Vec<BlockDef>,
    units: Vec<UnitDef>,
    unit_names: HashMap<String, UnitTypeId>,
    fluids: Vec<FluidDef>,
    fluid_names: HashMap<String, FluidId>,
    floor_names: HashMap<String, FloorId>,
    item_names: HashMap<String, ItemId>,
    ore_names: HashMap<String, OreId>,
    block_names: HashMap<String, BlockId>,
}

fn index_names<I: Copy>(
    names: impl Iterator<Item = String>,
    kind: &'static str,
    make: impl Fn(u16) -> I,
) -> Result<HashMap<String, I>, ContentError> {
    let mut map = HashMap::new();
    for (i, name) in names.enumerate() {
        let id = u16::try_from(i).map_err(|_| ContentError::TooMany(kind))?;
        if map.insert(name.clone(), make(id)).is_some() {
            return Err(ContentError::Duplicate { kind, name });
        }
    }
    Ok(map)
}

fn resolve<I: Copy>(
    map: &HashMap<String, I>,
    name: &str,
    kind: &'static str,
    path: impl FnOnce() -> String,
) -> Result<I, ContentError> {
    map.get(name)
        .copied()
        .ok_or_else(|| ContentError::UnknownRef {
            path: path(),
            kind,
            name: name.to_owned(),
        })
}

fn positive(value: u32, path: impl FnOnce() -> String) -> Result<(), ContentError> {
    if value == 0 {
        Err(ContentError::Invalid {
            path: path(),
            reason: "must be greater than zero".into(),
        })
    } else {
        Ok(())
    }
}

impl ContentRegistry {
    /// Загружает встроенный базовый контент.
    pub fn base() -> Result<Self, ContentError> {
        Self::from_ron(BASE_CONTENT)
    }

    /// Разбирает и валидирует контент из RON-строки.
    pub fn from_ron(text: &str) -> Result<Self, ContentError> {
        let raw: RawContent = ron::from_str(text)?;
        Self::from_raw(raw)
    }

    fn from_raw(raw: RawContent) -> Result<Self, ContentError> {
        let floor_names = index_names(raw.floors.iter().map(|f| f.name.clone()), "floor", FloorId)?;
        let item_names = index_names(raw.items.iter().map(|f| f.name.clone()), "item", ItemId)?;
        let fluid_names = index_names(raw.fluids.iter().map(|f| f.name.clone()), "fluid", FluidId)?;
        let fluids: Vec<FluidDef> = raw
            .fluids
            .iter()
            .map(|f| FluidDef {
                name: f.name.clone(),
            })
            .collect();
        let unit_names = index_names(raw.units.iter().map(|u| u.name.clone()), "unit", UnitTypeId)?;
        let mut units = Vec::with_capacity(raw.units.len());
        for (i, u) in raw.units.iter().enumerate() {
            for (v, f) in [
                (u.health, "health"),
                (u.speed, "speed"),
                (u.cooldown, "cooldown"),
                (u.radius, "radius"),
            ] {
                positive(v, || format!("units[{i}] `{}`.{f}", u.name))?;
            }
            units.push(UnitDef {
                name: u.name.clone(),
                health: u.health,
                speed: u.speed,
                damage: u.damage,
                cooldown: u.cooldown,
                radius: u.radius,
            });
        }
        let ore_names = index_names(raw.ores.iter().map(|f| f.name.clone()), "ore", OreId)?;
        let block_names = index_names(raw.blocks.iter().map(|f| f.name.clone()), "block", BlockId)?;

        let floors = raw
            .floors
            .into_iter()
            .map(|f| FloorDef {
                name: f.name,
                buildable: f.buildable,
            })
            .collect();
        let items = raw
            .items
            .into_iter()
            .map(|i| ItemDef { name: i.name })
            .collect();

        let mut ores = Vec::with_capacity(raw.ores.len());
        for (i, ore) in raw.ores.into_iter().enumerate() {
            let item = resolve(&item_names, &ore.item, "item", || format!("ores[{i}].item"))?;
            ores.push(OreDef {
                name: ore.name,
                item,
            });
        }

        let mut blocks = Vec::with_capacity(raw.blocks.len());
        for (i, block) in raw.blocks.into_iter().enumerate() {
            let p = |field: &str| format!("blocks[{i}] `{}`.{field}", block.name);
            if !(1..=8).contains(&block.size) {
                return Err(ContentError::Invalid {
                    path: p("size"),
                    reason: "must be in 1..=8".into(),
                });
            }
            let kind = match block.kind.clone() {
                RawKind::Conveyor { speed } => {
                    positive(speed, || p("speed"))?;
                    if block.size != 1 {
                        return Err(ContentError::Invalid {
                            path: p("size"),
                            reason: "conveyors must have size 1".into(),
                        });
                    }
                    BlockKind::Conveyor { speed }
                }
                RawKind::Drill { base_ticks, buffer } => {
                    positive(base_ticks, || p("base_ticks"))?;
                    positive(buffer, || p("buffer"))?;
                    BlockKind::Drill { base_ticks, buffer }
                }
                RawKind::Crafter {
                    inputs,
                    outputs,
                    craft_ticks,
                    power_use,
                    buffer,
                    heat,
                    cooling,
                    heat_capacity,
                } => {
                    positive(heat_capacity, || p("heat_capacity"))?;
                    positive(craft_ticks, || p("craft_ticks"))?;
                    let stacks = |list: Vec<(String, u32)>, field: &str| {
                        list.into_iter()
                            .enumerate()
                            .map(|(j, (name, count))| {
                                let path = || format!("{}[{j}]", p(field));
                                positive(count, path)?;
                                let item = resolve(&item_names, &name, "item", path)?;
                                Ok(ItemStack { item, count })
                            })
                            .collect::<Result<Vec<_>, ContentError>>()
                    };
                    let inputs = stacks(inputs, "inputs")?;
                    let outputs = stacks(outputs, "outputs")?;
                    if outputs.is_empty() {
                        return Err(ContentError::Invalid {
                            path: p("outputs"),
                            reason: "recipe must produce something".into(),
                        });
                    }
                    for stack in inputs.iter().chain(&outputs) {
                        if stack.count > buffer {
                            return Err(ContentError::Invalid {
                                path: p("buffer"),
                                reason: "buffer is smaller than a recipe stack".into(),
                            });
                        }
                    }
                    BlockKind::Crafter {
                        inputs,
                        outputs,
                        craft_ticks,
                        power_use,
                        buffer,
                        heat,
                        cooling,
                        heat_capacity,
                    }
                }
                RawKind::Generator { power } => {
                    positive(power, || p("power"))?;
                    BlockKind::Generator { power }
                }
                RawKind::PowerNode { range } => {
                    positive(range, || p("range"))?;
                    BlockKind::PowerNode { range }
                }
                RawKind::Core { capacity } => {
                    positive(capacity, || p("capacity"))?;
                    BlockKind::Core { capacity }
                }
                RawKind::Splitter | RawKind::Sorter => {
                    if block.size != 1 {
                        return Err(ContentError::Invalid {
                            path: p("size"),
                            reason: "splitters and sorters must have size 1".into(),
                        });
                    }
                    if matches!(&block.kind, RawKind::Splitter) {
                        BlockKind::Splitter
                    } else {
                        BlockKind::Sorter
                    }
                }
                RawKind::Bridge {
                    range,
                    interval,
                    buffer,
                } => {
                    positive(range, || p("range"))?;
                    positive(interval, || p("interval"))?;
                    positive(buffer, || p("buffer"))?;
                    if block.size != 1 {
                        return Err(ContentError::Invalid {
                            path: p("size"),
                            reason: "bridges must have size 1".into(),
                        });
                    }
                    BlockKind::Bridge {
                        range,
                        interval,
                        buffer,
                    }
                }
                RawKind::Battery { capacity } => {
                    positive(capacity, || p("capacity"))?;
                    BlockKind::Battery { capacity }
                }
                RawKind::Turret {
                    range,
                    damage,
                    reload,
                    ammo,
                    shots_per_item,
                    projectile_speed,
                    buffer,
                } => {
                    for (v, f) in [
                        (range, "range"),
                        (reload, "reload"),
                        (shots_per_item, "shots_per_item"),
                        (projectile_speed, "projectile_speed"),
                        (buffer, "buffer"),
                    ] {
                        positive(v, || p(f))?;
                    }
                    BlockKind::Turret {
                        range,
                        damage,
                        reload,
                        ammo: resolve(&item_names, &ammo, "item", || p("ammo"))?,
                        shots_per_item,
                        projectile_speed,
                        buffer,
                    }
                }
                RawKind::Wall => BlockKind::Wall,
                RawKind::Pipe { capacity } => {
                    positive(capacity, || p("capacity"))?;
                    BlockKind::Pipe { capacity }
                }
                RawKind::Condenser {
                    fluid,
                    rate,
                    power_use,
                    capacity,
                } => {
                    positive(rate, || p("rate"))?;
                    positive(capacity, || p("capacity"))?;
                    BlockKind::Condenser {
                        fluid: resolve(&fluid_names, &fluid, "fluid", || p("fluid"))?,
                        rate,
                        power_use,
                        capacity,
                    }
                }
                RawKind::CoolingTower {
                    fluid,
                    rate,
                    cooling,
                    heat_capacity,
                } => {
                    positive(rate, || p("rate"))?;
                    positive(heat_capacity, || p("heat_capacity"))?;
                    BlockKind::CoolingTower {
                        fluid: resolve(&fluid_names, &fluid, "fluid", || p("fluid"))?,
                        rate,
                        cooling,
                        heat_capacity,
                    }
                }
                RawKind::HeatPipe { heat_capacity } => {
                    positive(heat_capacity, || p("heat_capacity"))?;
                    BlockKind::HeatPipe { heat_capacity }
                }
                RawKind::Radiator {
                    dissipation,
                    heat_capacity,
                } => {
                    positive(heat_capacity, || p("heat_capacity"))?;
                    BlockKind::Radiator {
                        dissipation,
                        heat_capacity,
                    }
                }
                RawKind::HeatEngine {
                    max_heat,
                    power_per_heat_permille,
                    heat_capacity,
                } => {
                    positive(max_heat, || p("max_heat"))?;
                    positive(heat_capacity, || p("heat_capacity"))?;
                    BlockKind::HeatEngine {
                        max_heat,
                        power_per_heat_permille,
                        heat_capacity,
                    }
                }
            };
            let mut cost = Vec::with_capacity(block.cost.len());
            for (j, (name, count)) in block.cost.iter().enumerate() {
                let path = || format!("{}[{j}]", p("cost"));
                positive(*count, path)?;
                cost.push(ItemStack {
                    item: resolve(&item_names, name, "item", path)?,
                    count: *count,
                });
            }
            let default_health = 60 * u32::from(block.size) * u32::from(block.size);
            let health = if block.health == 0 {
                default_health
            } else {
                block.health
            };
            blocks.push(BlockDef {
                name: block.name,
                size: block.size,
                cost,
                health,
                kind,
            });
        }

        Ok(Self {
            floors,
            items,
            ores,
            blocks,
            units,
            unit_names,
            fluids,
            fluid_names,
            floor_names,
            item_names,
            ore_names,
            block_names,
        })
    }

    /// Все типы пола в порядке ID.
    pub fn floors(&self) -> &[FloorDef] {
        &self.floors
    }
    /// Все ресурсы в порядке ID.
    pub fn items(&self) -> &[ItemDef] {
        &self.items
    }
    /// Все месторождения в порядке ID.
    pub fn ores(&self) -> &[OreDef] {
        &self.ores
    }
    /// Все постройки в порядке ID.
    pub fn blocks(&self) -> &[BlockDef] {
        &self.blocks
    }

    /// Определение пола. Паникует, если ID получен не из этого реестра.
    pub fn floor(&self, id: FloorId) -> &FloorDef {
        &self.floors[usize::from(id.0)]
    }
    /// Определение ресурса. Паникует, если ID получен не из этого реестра.
    pub fn item(&self, id: ItemId) -> &ItemDef {
        &self.items[usize::from(id.0)]
    }
    /// Определение месторождения. Паникует, если ID получен не из этого реестра.
    pub fn ore(&self, id: OreId) -> &OreDef {
        &self.ores[usize::from(id.0)]
    }
    /// Определение постройки. Паникует, если ID получен не из этого реестра.
    pub fn block(&self, id: BlockId) -> &BlockDef {
        &self.blocks[usize::from(id.0)]
    }

    /// Поиск пола по имени.
    pub fn floor_by_name(&self, name: &str) -> Option<FloorId> {
        self.floor_names.get(name).copied()
    }
    /// Поиск ресурса по имени.
    pub fn item_by_name(&self, name: &str) -> Option<ItemId> {
        self.item_names.get(name).copied()
    }
    /// Поиск месторождения по имени.
    pub fn ore_by_name(&self, name: &str) -> Option<OreId> {
        self.ore_names.get(name).copied()
    }
    /// Поиск постройки по имени.
    pub fn block_by_name(&self, name: &str) -> Option<BlockId> {
        self.block_names.get(name).copied()
    }

    /// Все типы юнитов в порядке ID.
    pub fn units(&self) -> &[UnitDef] {
        &self.units
    }

    /// Тип юнита. Паникует, если ID получен не из этого реестра.
    pub fn unit(&self, id: UnitTypeId) -> &UnitDef {
        &self.units[usize::from(id.0)]
    }

    /// Поиск типа юнита по имени.
    pub fn unit_by_name(&self, name: &str) -> Option<UnitTypeId> {
        self.unit_names.get(name).copied()
    }

    /// Все жидкости в порядке ID.
    pub fn fluids(&self) -> &[FluidDef] {
        &self.fluids
    }

    /// Определение жидкости. Паникует, если ID получен не из этого реестра.
    pub fn fluid(&self, id: FluidId) -> &FluidDef {
        &self.fluids[usize::from(id.0)]
    }

    /// Поиск жидкости по имени.
    pub fn fluid_by_name(&self, name: &str) -> Option<FluidId> {
        self.fluid_names.get(name).copied()
    }

    /// Таблица имён контента в порядке ID — используется форматом сохранений.
    pub fn name_table(&self) -> NameTable {
        NameTable {
            floors: self.floors.iter().map(|d| d.name.clone()).collect(),
            items: self.items.iter().map(|d| d.name.clone()).collect(),
            ores: self.ores.iter().map(|d| d.name.clone()).collect(),
            blocks: self.blocks.iter().map(|d| d.name.clone()).collect(),
            fluids: self.fluids.iter().map(|d| d.name.clone()).collect(),
            units: self.units.iter().map(|d| d.name.clone()).collect(),
        }
    }
}

/// Имена контента в порядке ID.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NameTable {
    /// Имена полов.
    pub floors: Vec<String>,
    /// Имена ресурсов.
    pub items: Vec<String>,
    /// Имена месторождений.
    pub ores: Vec<String>,
    /// Имена построек.
    pub blocks: Vec<String>,
    /// Имена жидкостей.
    #[serde(default)]
    pub fluids: Vec<String>,
    /// Имена типов юнитов.
    #[serde(default)]
    pub units: Vec<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn base_content_is_valid() {
        let reg = ContentRegistry::base().unwrap();
        assert!(reg.block_by_name("conveyor").is_some());
        assert!(reg.item_by_name("iron_ore").is_some());
        let vein = reg.ore(reg.ore_by_name("iron_vein").unwrap());
        assert_eq!(reg.item(vein.item).name, "iron_ore");
    }

    #[test]
    fn unknown_reference_reports_path() {
        let text = BASE_CONTENT.replace("item: \"iron_ore\"", "item: \"unobtainium\"");
        let err = ContentRegistry::from_ron(&text).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ores[0].item"), "{msg}");
        assert!(msg.contains("unobtainium"), "{msg}");
    }

    #[test]
    fn duplicate_names_rejected() {
        let text = BASE_CONTENT.replace("(name: \"carbon\")", "(name: \"iron_ore\")");
        let err = ContentRegistry::from_ron(&text).unwrap_err();
        assert!(
            matches!(err, ContentError::Duplicate { kind: "item", .. }),
            "{err}"
        );
    }

    #[test]
    fn zero_values_rejected() {
        let text = BASE_CONTENT.replace("speed: 16", "speed: 0");
        let err = ContentRegistry::from_ron(&text).unwrap_err();
        assert!(err.to_string().contains("speed"), "{err}");
    }

    #[test]
    fn syntax_error_is_error_not_panic() {
        assert!(matches!(
            ContentRegistry::from_ron("(floors: ["),
            Err(ContentError::Parse(_))
        ));
    }
}
