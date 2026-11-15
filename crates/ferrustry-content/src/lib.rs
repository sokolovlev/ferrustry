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
    kind: RawKind,
}

#[derive(Debug, Deserialize)]
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
}

/// Тип постройки.
#[derive(Debug, Clone)]
pub struct BlockDef {
    /// Уникальное имя.
    pub name: String,
    /// Размер стороны в тайлах.
    pub size: u8,
    /// Поведение.
    pub kind: BlockKind,
}

impl BlockDef {
    /// Участвует ли постройка в энергосети.
    pub fn is_power(&self) -> bool {
        match &self.kind {
            BlockKind::Generator { .. } | BlockKind::PowerNode { .. } => true,
            BlockKind::Crafter { power_use, .. } => *power_use > 0,
            _ => false,
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
            let kind = match block.kind {
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
                } => {
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
            };
            blocks.push(BlockDef {
                name: block.name,
                size: block.size,
                kind,
            });
        }

        Ok(Self {
            floors,
            items,
            ores,
            blocks,
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

    /// Таблица имён контента в порядке ID — используется форматом сохранений.
    pub fn name_table(&self) -> NameTable {
        NameTable {
            floors: self.floors.iter().map(|d| d.name.clone()).collect(),
            items: self.items.iter().map(|d| d.name.clone()).collect(),
            ores: self.ores.iter().map(|d| d.name.clone()).collect(),
            blocks: self.blocks.iter().map(|d| d.name.clone()).collect(),
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
