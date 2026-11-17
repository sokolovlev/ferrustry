//! Хранилище построек: generational-арена с общими данными и плотные таблицы по виду поведения.

use ferrustry_content::BlockId;
use serde::{Deserialize, Serialize};

use crate::geom::{Dir, TilePos};

/// Устойчивый идентификатор постройки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BuildingId {
    index: u32,
    generation: u32,
}

/// Вид поведения постройки — определяет, в какой таблице лежит её состояние.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KindTag {
    /// Конвейер.
    Conveyor,
    /// Бур.
    Drill,
    /// Производство.
    Crafter,
    /// Генератор.
    Generator,
    /// Узел энергосети.
    PowerNode,
    /// Ядро.
    Core,
    /// Разветвитель.
    Splitter,
    /// Сортировщик.
    Sorter,
    /// Мост.
    Bridge,
    /// Батарея.
    Battery,
    /// Труба.
    Pipe,
    /// Конденсатор.
    Condenser,
    /// Градирня.
    CoolingTower,
    /// Тепловая труба.
    HeatPipe,
    /// Радиатор.
    Radiator,
    /// Тепловая машина.
    HeatEngine,
}

/// Общие данные постройки.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Building {
    /// Тип постройки.
    pub block: BlockId,
    /// Левый нижний тайл.
    pub origin: TilePos,
    /// Сторона в тайлах.
    pub size: u8,
    /// Поворот.
    pub rot: Dir,
    /// Таблица состояния.
    pub tag: KindTag,
    /// Строка в таблице состояния.
    pub(crate) row: u32,
    /// Энергосеть, к которой подключена постройка.
    pub(crate) power_net: Option<u32>,
    /// Тепловая сеть.
    pub(crate) heat_net: Option<u32>,
    /// Жидкостная сеть.
    pub(crate) fluid_net: Option<u32>,
    /// Курсор round-robin выгрузки по списку получателей.
    pub(crate) dump_cursor: u32,
    /// Эпоха топологии, для которой построен `dump_links`.
    pub(crate) dump_epoch: u64,
    /// Кэш получателей выгрузки (только те, кто в принципе может принять предмет).
    pub(crate) dump_links: Vec<crate::world::Link>,
}

impl Building {
    /// Содержит ли постройка тайл.
    pub fn contains(&self, p: TilePos) -> bool {
        let s = i32::from(self.size);
        p.x >= self.origin.x
            && p.y >= self.origin.y
            && p.x < self.origin.x + s
            && p.y < self.origin.y + s
    }

    /// Число тайлов периметра снаружи постройки.
    pub fn perimeter_len(&self) -> u32 {
        4 * u32::from(self.size)
    }

    /// `i`-й тайл периметра: (снаружи, соседний внутренний тайл).
    pub fn perimeter(&self, i: u32) -> (TilePos, TilePos) {
        let s = u32::from(self.size);
        let side = (i / s) % 4;
        let j = (i % s) as i32;
        let si = i32::from(self.size);
        let o = self.origin;
        match side {
            0 => (TilePos::new(o.x + j, o.y - 1), TilePos::new(o.x + j, o.y)),
            1 => (
                TilePos::new(o.x + si, o.y + j),
                TilePos::new(o.x + si - 1, o.y + j),
            ),
            2 => (
                TilePos::new(o.x + j, o.y + si),
                TilePos::new(o.x + j, o.y + si - 1),
            ),
            _ => (TilePos::new(o.x - 1, o.y + j), TilePos::new(o.x, o.y + j)),
        }
    }

    /// Центр в «удвоенных» координатах тайлов (целочисленно для чётных и нечётных размеров).
    pub fn center2(&self) -> (i64, i64) {
        let s = i64::from(self.size);
        (
            2 * i64::from(self.origin.x) + s,
            2 * i64::from(self.origin.y) + s,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Slot {
    generation: u32,
    building: Option<Building>,
}

/// Generational-арена построек.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arena {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

impl Arena {
    pub(crate) fn insert(&mut self, b: Building) -> BuildingId {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.building = Some(b);
            BuildingId {
                index,
                generation: slot.generation,
            }
        } else {
            let index = u32::try_from(self.slots.len()).unwrap_or(u32::MAX);
            self.slots.push(Slot {
                generation: 0,
                building: Some(b),
            });
            BuildingId {
                index,
                generation: 0,
            }
        }
    }

    /// Постройка по ID.
    pub fn get(&self, id: BuildingId) -> Option<&Building> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation == id.generation {
            slot.building.as_ref()
        } else {
            None
        }
    }

    pub(crate) fn get_mut(&mut self, id: BuildingId) -> Option<&mut Building> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation == id.generation {
            slot.building.as_mut()
        } else {
            None
        }
    }

    pub(crate) fn remove(&mut self, id: BuildingId) -> Option<Building> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let b = slot.building.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
        Some(b)
    }

    /// Все живые постройки в порядке индексов (детерминированно).
    pub fn iter(&self) -> impl Iterator<Item = (BuildingId, &Building)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.building.as_ref().map(|b| {
                (
                    BuildingId {
                        index: i as u32,
                        generation: s.generation,
                    },
                    b,
                )
            })
        })
    }

    /// ID живой постройки в ячейке `slot`.
    pub(crate) fn id_at(&self, slot: usize) -> Option<BuildingId> {
        let s = self.slots.get(slot)?;
        s.building.as_ref().map(|_| BuildingId {
            index: slot as u32,
            generation: s.generation,
        })
    }

    /// Количество живых построек.
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// Пуста ли арена.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn slot_index(id: BuildingId) -> usize {
        id.index as usize
    }
}

/// Плотная таблица состояния одного вида поведения.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Table<T> {
    pub(crate) rows: Vec<T>,
    pub(crate) owners: Vec<BuildingId>,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            owners: Vec::new(),
        }
    }
}

impl<T> Table<T> {
    pub(crate) fn push(&mut self, owner: BuildingId, row: T) -> u32 {
        self.rows.push(row);
        self.owners.push(owner);
        u32::try_from(self.rows.len() - 1).unwrap_or(u32::MAX)
    }

    /// Удаляет строку; возвращает удалённую строку и владельца, переехавшего на её место.
    pub(crate) fn swap_remove(&mut self, row: u32) -> Option<(T, Option<BuildingId>)> {
        let row = row as usize;
        if row >= self.rows.len() {
            return None;
        }
        let value = self.rows.swap_remove(row);
        self.owners.swap_remove(row);
        Some((value, self.owners.get(row).copied()))
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
}
