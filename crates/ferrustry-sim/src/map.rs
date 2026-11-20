//! Описание карты (`MapDef`): рельеф, руды, стартовые постройки, точки появления врагов,
//! правила и стартовый запас — всё, что нужно, чтобы начать партию, но без её текущего
//! состояния (содержимого лент, юнитов, таймеров).
//!
//! Карта отделена от сохранения: её правят в редакторе и хранят в `.fmap`, а партия
//! создаётся из неё функцией [`MapDef::instantiate`].

use ferrustry_content::{BlockId, ContentRegistry, FloorId, ItemId, OreId};
use serde::{Deserialize, Serialize};

use crate::buildings::KindTag;
use crate::geom::{Dir, TilePos};
use crate::grid::Grid;
use crate::scenario::ScenarioError;
use crate::world::{Command, Detail, Rules, World};

/// Стартовая постройка карты.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapBuilding {
    /// Тип.
    pub block: BlockId,
    /// Левый нижний тайл.
    pub origin: TilePos,
    /// Поворот.
    pub rot: Dir,
    /// Фильтр сортировщика.
    pub filter: Option<ItemId>,
}

/// Описание карты.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapDef {
    /// Ширина в тайлах.
    pub width: u32,
    /// Высота в тайлах.
    pub height: u32,
    /// Пол по тайлам, построчно снизу вверх (`width * height`).
    pub floors: Vec<FloorId>,
    /// Руда по тайлам (`width * height`).
    pub ores: Vec<Option<OreId>>,
    /// Точки появления врагов.
    pub spawns: Vec<TilePos>,
    /// Стартовые постройки (ставятся бесплатно).
    pub buildings: Vec<MapBuilding>,
    /// Правила партии.
    pub rules: Rules,
    /// Стартовый запас в ядрах.
    pub starter: Vec<(ItemId, u32)>,
}

/// Ошибка проверки карты.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MapError {
    /// Размер вне допустимого диапазона или не совпадает с числом тайлов.
    #[error("map size is invalid")]
    BadSize,
    /// Точка появления вне карты.
    #[error("spawn point ({0}, {1}) is outside the map")]
    SpawnOutside(i32, i32),
    /// Постройка не ставится.
    #[error(transparent)]
    Scenario(#[from] ScenarioError),
}

impl MapDef {
    /// Снимает описание карты с мира: рельеф, постройки, точки появления, правила и
    /// содержимое ядер как стартовый запас.
    pub fn from_world(world: &World, reg: &ContentRegistry) -> Self {
        let g = world.grid();
        let (w, h) = (g.width(), g.height());
        let mut floors = Vec::with_capacity((w * h) as usize);
        let mut ores = Vec::with_capacity((w * h) as usize);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let p = TilePos::new(x, y);
                floors.push(g.floor(p).unwrap_or(FloorId(0)));
                ores.push(g.ore(p));
            }
        }
        let mut buildings = Vec::new();
        let mut starter = vec![0u64; reg.items().len()];
        for (_, b) in world.buildings().iter() {
            let filter = match world.inspect(reg, b.origin).map(|i| i.detail) {
                Some(Detail::Sorter { filter, .. }) => filter,
                _ => None,
            };
            buildings.push(MapBuilding {
                block: b.block,
                origin: b.origin,
                rot: b.rot,
                filter,
            });
            if b.tag == KindTag::Core
                && let Some(items) = world.core_items(b.origin)
            {
                for (s, &n) in starter.iter_mut().zip(items) {
                    *s += u64::from(n);
                }
            }
        }
        buildings.sort_by_key(|b| (b.origin.y, b.origin.x));
        Self {
            width: w,
            height: h,
            floors,
            ores,
            spawns: world.spawns().to_vec(),
            buildings,
            rules: world.rules().clone(),
            starter: starter
                .into_iter()
                .enumerate()
                .filter(|&(_, n)| n > 0)
                .map(|(i, n)| (ItemId(i as u16), u32::try_from(n).unwrap_or(u32::MAX)))
                .collect(),
        }
    }

    /// Проверка размеров и точек появления (без контента).
    pub fn check(&self) -> Result<(), MapError> {
        let n = (self.width as usize).checked_mul(self.height as usize);
        if !(1..=Grid::MAX_SIDE).contains(&self.width)
            || !(1..=Grid::MAX_SIDE).contains(&self.height)
            || n != Some(self.floors.len())
            || n != Some(self.ores.len())
        {
            return Err(MapError::BadSize);
        }
        for s in &self.spawns {
            if s.x < 0 || s.y < 0 || s.x >= self.width as i32 || s.y >= self.height as i32 {
                return Err(MapError::SpawnOutside(s.x, s.y));
            }
        }
        Ok(())
    }

    /// Создаёт партию по карте: постройки ставятся бесплатно, затем включаются правила
    /// карты и выдаётся стартовый запас. Для карты, снятой с только что созданного мира,
    /// результат совпадает с ним бит в бит (тот же `state_hash`).
    pub fn instantiate(&self, reg: &ContentRegistry, seed: u64) -> Result<World, MapError> {
        self.check()?;
        let mut world = World::new(
            reg,
            self.width,
            self.height,
            self.floors.first().copied().unwrap_or(FloorId(0)),
            seed,
        );
        let w = self.width as usize;
        for (i, (&f, &o)) in self.floors.iter().zip(&self.ores).enumerate() {
            let p = TilePos::new((i % w) as i32, (i / w) as i32);
            world.set_floor(p, f);
            world.set_ore(p, o);
        }
        world.set_spawns(self.spawns.clone());
        let mut cmds: Vec<Command> = self
            .buildings
            .iter()
            .map(|b| Command::Place {
                block: b.block,
                origin: b.origin,
                rot: b.rot,
            })
            .collect();
        cmds.extend(self.buildings.iter().filter_map(|b| {
            b.filter.map(|item| Command::Configure {
                at: b.origin,
                item: Some(item),
            })
        }));
        crate::scenario::build(&mut world, reg, &cmds)?;
        world.set_rules(self.rules.clone());
        for &(item, n) in &self.starter {
            world.grant(reg, item, n);
        }
        world.prepare_flow(reg);
        Ok(world)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapgen::{self, Difficulty, MapSettings, MapSize};

    #[test]
    fn generated_map_roundtrips_through_mapdef_bit_for_bit() {
        let reg = ContentRegistry::base().unwrap();
        for seed in [1, 7, 42] {
            let s = MapSettings {
                size: MapSize::Small,
                difficulty: Difficulty::Normal,
                seed,
            };
            let w = mapgen::generate(&reg, &s).unwrap();
            let def = MapDef::from_world(&w, &reg);
            let w2 = def.instantiate(&reg, seed).unwrap();
            assert_eq!(crate::state_hash(&w), crate::state_hash(&w2));
            assert_eq!(MapDef::from_world(&w2, &reg), def);
        }
    }

    #[test]
    fn invalid_maps_are_rejected() {
        let reg = ContentRegistry::base().unwrap();
        let w = mapgen::generate(
            &reg,
            &MapSettings {
                size: MapSize::Small,
                difficulty: Difficulty::Easy,
                seed: 3,
            },
        )
        .unwrap();
        let def = MapDef::from_world(&w, &reg);
        let mut bad = def.clone();
        bad.floors.pop();
        assert_eq!(bad.instantiate(&reg, 0).unwrap_err(), MapError::BadSize);
        let mut bad = def.clone();
        bad.spawns.push(TilePos::new(-1, 5));
        assert!(matches!(
            bad.instantiate(&reg, 0),
            Err(MapError::SpawnOutside(-1, 5))
        ));
        let mut bad = def;
        let b = bad.buildings[0].clone();
        bad.buildings.push(b); // та же постройка дважды — место занято
        assert!(matches!(
            bad.instantiate(&reg, 0),
            Err(MapError::Scenario(_))
        ));
    }
}
