//! Файл карты (`.fmap`): рельеф, руды, стартовые постройки, точки появления врагов, правила
//! и стартовый запас. Весь контент хранится по **именам** (как в схемах), поэтому карты
//! переживают изменение порядка контента и не зависят от внутренних структур симуляции.
//!
//! В таблицы имён полов и руд попадают только реально использованные на карте имена:
//! неизвестное имя в файле — всегда ошибка, а не догадка.

use std::collections::HashMap;

use ferrustry_content::{ContentRegistry, FloorId, ItemId, OreId};
use ferrustry_sim::map::{MapBuilding, MapDef};
use ferrustry_sim::{Dir, Rules, TilePos};
use serde::{Deserialize, Serialize};

use crate::{FileKind, FormatError, Sections, Writer, decompress, read_header};

const SECTION_MAP: u32 = 1;

#[derive(Serialize, Deserialize)]
struct BuildingRepr {
    block: String,
    x: i32,
    y: i32,
    rot: Dir,
    filter: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct RulesRepr {
    infinite_resources: bool,
    waves: bool,
    first_wave: u64,
    wave_spacing: u64,
}

#[derive(Serialize, Deserialize)]
struct Repr {
    width: u32,
    height: u32,
    floor_names: Vec<String>,
    ore_names: Vec<String>,
    /// Индекс в `floor_names` для каждого тайла.
    floors: Vec<u16>,
    /// 0 — нет руды, иначе индекс в `ore_names` + 1.
    ores: Vec<u16>,
    spawns: Vec<(i32, i32)>,
    buildings: Vec<BuildingRepr>,
    rules: RulesRepr,
    starter: Vec<(String, u32)>,
}

/// Локальная таблица имён: только использованные ID, в порядке первого появления.
struct Table<K> {
    index: HashMap<K, u16>,
    names: Vec<String>,
}

impl<K: std::hash::Hash + Eq + Copy> Table<K> {
    fn new() -> Self {
        Self {
            index: HashMap::new(),
            names: Vec::new(),
        }
    }

    fn get(&mut self, k: K, name: impl FnOnce() -> String) -> u16 {
        if let Some(&i) = self.index.get(&k) {
            return i;
        }
        let i = self.names.len() as u16;
        self.names.push(name());
        self.index.insert(k, i);
        i
    }
}

/// Записывает карту.
pub fn write_map(def: &MapDef, reg: &ContentRegistry) -> Result<Vec<u8>, FormatError> {
    let mut floors_t: Table<FloorId> = Table::new();
    let mut ores_t: Table<OreId> = Table::new();
    let floors = def
        .floors
        .iter()
        .map(|&f| floors_t.get(f, || reg.floor(f).name.clone()))
        .collect();
    let ores = def
        .ores
        .iter()
        .map(|o| o.map_or(0, |o| ores_t.get(o, || reg.ore(o).name.clone()) + 1))
        .collect();
    let item_name = |i: ItemId| reg.item(i).name.clone();
    let repr = Repr {
        width: def.width,
        height: def.height,
        floor_names: floors_t.names,
        ore_names: ores_t.names,
        floors,
        ores,
        spawns: def.spawns.iter().map(|p| (p.x, p.y)).collect(),
        buildings: def
            .buildings
            .iter()
            .map(|b| BuildingRepr {
                block: reg.block(b.block).name.clone(),
                x: b.origin.x,
                y: b.origin.y,
                rot: b.rot,
                filter: b.filter.map(item_name),
            })
            .collect(),
        rules: RulesRepr {
            infinite_resources: def.rules.infinite_resources,
            waves: def.rules.waves,
            first_wave: def.rules.first_wave,
            wave_spacing: def.rules.wave_spacing,
        },
        starter: def
            .starter
            .iter()
            .map(|&(i, n)| (item_name(i), n))
            .collect(),
    };
    let mut w = Writer::new(FileKind::Map);
    w.section(SECTION_MAP, &postcard::to_allocvec(&repr)?)?;
    Ok(w.buf)
}

fn invalid(msg: String) -> FormatError {
    FormatError::InvalidWorld(msg)
}

/// Читает карту, сопоставляя имена с текущим контентом. Проверяет размеры, индексы
/// таблиц и точки появления; ставятся ли постройки, проверяет [`MapDef::instantiate`].
pub fn read_map(bytes: &[u8], reg: &ContentRegistry) -> Result<MapDef, FormatError> {
    let mut sections = Sections {
        rest: read_header(bytes, FileKind::Map)?,
    };
    let mut repr: Option<Repr> = None;
    while let Some((id, data)) = sections.next()? {
        if id == SECTION_MAP {
            repr = Some(postcard::from_bytes(&decompress(id, data)?)?);
        }
    }
    let r = repr.ok_or(FormatError::MissingSection(SECTION_MAP))?;
    let floor_ids = r
        .floor_names
        .iter()
        .map(|n| {
            reg.floor_by_name(n)
                .ok_or_else(|| invalid(format!("unknown floor `{n}` in map")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ore_ids = r
        .ore_names
        .iter()
        .map(|n| {
            reg.ore_by_name(n)
                .ok_or_else(|| invalid(format!("unknown ore `{n}` in map")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let item = |n: &str| {
        reg.item_by_name(n)
            .ok_or_else(|| invalid(format!("unknown item `{n}` in map")))
    };
    let floors = r
        .floors
        .iter()
        .map(|&i| {
            floor_ids
                .get(usize::from(i))
                .copied()
                .ok_or_else(|| invalid("floor index out of range".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ores = r
        .ores
        .iter()
        .map(|&i| match i {
            0 => Ok(None),
            i => ore_ids
                .get(usize::from(i - 1))
                .copied()
                .map(Some)
                .ok_or_else(|| invalid("ore index out of range".into())),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut buildings = Vec::with_capacity(r.buildings.len());
    for b in r.buildings {
        let block = reg
            .block_by_name(&b.block)
            .ok_or_else(|| invalid(format!("unknown block `{}` in map", b.block)))?;
        let filter = b.filter.as_deref().map(item).transpose()?;
        buildings.push(MapBuilding {
            block,
            origin: TilePos::new(b.x, b.y),
            rot: b.rot,
            filter,
        });
    }
    let starter = r
        .starter
        .iter()
        .map(|(n, c)| Ok((item(n)?, *c)))
        .collect::<Result<Vec<_>, FormatError>>()?;
    let def = MapDef {
        width: r.width,
        height: r.height,
        floors,
        ores,
        spawns: r.spawns.iter().map(|&(x, y)| TilePos::new(x, y)).collect(),
        buildings,
        rules: Rules {
            infinite_resources: r.rules.infinite_resources,
            waves: r.rules.waves,
            first_wave: r.rules.first_wave,
            wave_spacing: r.rules.wave_spacing,
        },
        starter,
    };
    def.check().map_err(|e| invalid(e.to_string()))?;
    Ok(def)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrustry_sim::mapgen::{self, Difficulty, MapSettings, MapSize};

    fn generated(reg: &ContentRegistry) -> MapDef {
        let w = mapgen::generate(
            reg,
            &MapSettings {
                size: MapSize::Medium,
                difficulty: Difficulty::Hard,
                seed: 11,
            },
        )
        .unwrap();
        MapDef::from_world(&w, reg)
    }

    #[test]
    fn map_roundtrip_and_play() {
        let reg = ContentRegistry::base().unwrap();
        let def = generated(&reg);
        let bytes = write_map(&def, &reg).unwrap();
        let back = read_map(&bytes, &reg).unwrap();
        assert_eq!(back, def);
        let a = def.instantiate(&reg, 11).unwrap();
        let b = back.instantiate(&reg, 11).unwrap();
        assert_eq!(ferrustry_sim::state_hash(&a), ferrustry_sim::state_hash(&b));
        // карта небольшая: таблицы имён + сжатие
        assert!(bytes.len() < 64 * 1024, "map file is {} bytes", bytes.len());
    }

    #[test]
    fn map_survives_content_reordering_and_rejects_unknown_names() {
        let reg = ContentRegistry::base().unwrap();
        let def = generated(&reg);
        let bytes = write_map(&def, &reg).unwrap();

        // переименовали неиспользуемую на карте постройку — карта читается, ID те же по имени
        let renamed = ferrustry_content::BASE_CONTENT.replacen(
            "name: \"conveyor\"",
            "name: \"conveyor_tmp\"",
            1,
        );
        let other = ContentRegistry::from_ron(&renamed).unwrap();
        assert!(read_map(&bytes, &other).is_ok());

        // переименовали использованный пол — ошибка с именем, а не догадка
        let renamed =
            ferrustry_content::BASE_CONTENT.replacen("name: \"basalt\"", "name: \"basalt_tmp\"", 1);
        let other = ContentRegistry::from_ron(&renamed).unwrap();
        let err = read_map(&bytes, &other).unwrap_err().to_string();
        assert!(err.contains("basalt"), "{err}");

        // сохранение не читается как карта, карта — как схема
        let save = crate::write_save(&def.instantiate(&reg, 1).unwrap(), &reg).unwrap();
        assert!(matches!(
            read_map(&save, &reg),
            Err(FormatError::WrongKind { .. })
        ));
        assert!(matches!(
            crate::read_schematic(&bytes, &reg),
            Err(FormatError::WrongKind { .. })
        ));
    }

    #[test]
    fn map_version_is_independent_of_save_version() {
        let reg = ContentRegistry::base().unwrap();
        let bytes = write_map(&generated(&reg), &reg).unwrap();
        assert_eq!(
            u16::from_le_bytes([bytes[9], bytes[10]]),
            crate::MAP_VERSION.0
        );
        let mut future = bytes.clone();
        future[9..11].copy_from_slice(&(crate::MAP_VERSION.0 + 1).to_le_bytes());
        assert!(matches!(
            read_map(&future, &reg),
            Err(FormatError::UnsupportedVersion(..))
        ));
        let mut broken = bytes;
        let n = broken.len();
        broken[n - 1] ^= 0xFF;
        assert!(read_map(&broken, &reg).is_err());
    }
}
