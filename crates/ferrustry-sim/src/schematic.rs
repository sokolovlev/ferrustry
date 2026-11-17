//! Схемы: копия участка базы (постройки, повороты, фильтры) для вставки в другом месте.
//!
//! Вставка превращается в обычные команды (`Place` и `Configure`), поэтому схемы автоматически
//! подчиняются экономике, проверкам места, replay и будущему мультиплеер-протоколу.

use std::collections::BTreeMap;

use ferrustry_content::{BlockId, ContentRegistry, ItemId};
use serde::{Deserialize, Serialize};

use crate::geom::{Dir, TilePos};
use crate::world::{Command, Detail, World};

/// Постройка в схеме: смещение левого нижнего тайла от угла схемы.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemEntry {
    /// Тип.
    pub block: BlockId,
    /// Смещение по X.
    pub dx: i32,
    /// Смещение по Y.
    pub dy: i32,
    /// Поворот.
    pub rot: Dir,
    /// Фильтр сортировщика.
    pub filter: Option<ItemId>,
}

/// Схема.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schematic {
    /// Ширина в тайлах.
    pub width: i32,
    /// Высота в тайлах.
    pub height: i32,
    /// Постройки.
    pub entries: Vec<SchemEntry>,
}

fn rot_cw(d: Dir) -> Dir {
    match d {
        Dir::Right => Dir::Down,
        Dir::Down => Dir::Left,
        Dir::Left => Dir::Up,
        Dir::Up => Dir::Right,
    }
}

impl World {
    /// Копирует постройки, целиком лежащие в прямоугольнике между `a` и `b` (включительно).
    /// `None`, если там ничего нет.
    pub fn copy_schematic(
        &self,
        reg: &ContentRegistry,
        a: TilePos,
        b: TilePos,
    ) -> Option<Schematic> {
        let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
        let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
        let mut entries = Vec::new();
        for (_, bl) in self.buildings().iter() {
            let s = i32::from(bl.size);
            let o = bl.origin;
            if o.x < x0 || o.y < y0 || o.x + s - 1 > x1 || o.y + s - 1 > y1 {
                continue;
            }
            let filter = match self.inspect(reg, o).map(|i| i.detail) {
                Some(Detail::Sorter { filter, .. }) => filter,
                _ => None,
            };
            entries.push(SchemEntry {
                block: bl.block,
                dx: o.x - x0,
                dy: o.y - y0,
                rot: bl.rot,
                filter,
            });
        }
        if entries.is_empty() {
            return None;
        }
        // нормализовать к реальным границам построек
        let minx = entries.iter().map(|e| e.dx).min().unwrap_or(0);
        let miny = entries.iter().map(|e| e.dy).min().unwrap_or(0);
        let mut w = 0;
        let mut h = 0;
        for e in &mut entries {
            e.dx -= minx;
            e.dy -= miny;
            let s = i32::from(reg.block(e.block).size);
            w = w.max(e.dx + s);
            h = h.max(e.dy + s);
        }
        entries.sort_by_key(|e| (e.dy, e.dx));
        Some(Schematic {
            width: w,
            height: h,
            entries,
        })
    }
}

impl Schematic {
    /// Поворот на 90° по часовой стрелке (как клавиша R при стройке).
    pub fn rotated_cw(&self, reg: &ContentRegistry) -> Schematic {
        let mut entries: Vec<SchemEntry> = self
            .entries
            .iter()
            .map(|e| {
                let s = i32::from(reg.block(e.block).size);
                SchemEntry {
                    block: e.block,
                    dx: e.dy,
                    dy: self.width - e.dx - s,
                    rot: rot_cw(e.rot),
                    filter: e.filter,
                }
            })
            .collect();
        entries.sort_by_key(|e| (e.dy, e.dx));
        Schematic {
            width: self.height,
            height: self.width,
            entries,
        }
    }

    /// Команды вставки с левым нижним углом в `origin`: сначала все постройки, затем настройки.
    pub fn commands(&self, origin: TilePos) -> Vec<Command> {
        let mut v: Vec<Command> = self
            .entries
            .iter()
            .map(|e| Command::Place {
                block: e.block,
                origin: TilePos::new(origin.x + e.dx, origin.y + e.dy),
                rot: e.rot,
            })
            .collect();
        for e in &self.entries {
            if e.filter.is_some() {
                v.push(Command::Configure {
                    at: TilePos::new(origin.x + e.dx, origin.y + e.dy),
                    item: e.filter,
                });
            }
        }
        v
    }

    /// Суммарная стоимость вставки по ресурсам.
    pub fn cost(&self, reg: &ContentRegistry) -> BTreeMap<ItemId, u64> {
        let mut m = BTreeMap::new();
        for e in &self.entries {
            for st in &reg.block(e.block).cost {
                *m.entry(st.item).or_insert(0) += u64::from(st.count);
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{self, BaseIds};

    fn place(b: BlockId, x: i32, y: i32, rot: Dir) -> Command {
        Command::Place {
            block: b,
            origin: TilePos::new(x, y),
            rot,
        }
    }

    #[test]
    fn copy_paste_reproduces_layout_and_filters() {
        let reg = ContentRegistry::base().unwrap();
        let ids = BaseIds::resolve(&reg).unwrap();
        let sorter = reg.block_by_name("sorter").unwrap();
        let mut w = World::new(&reg, 64, 64, ids.basalt, 1);
        let mut cmds = vec![
            place(ids.smelter, 5, 5, Dir::Right),
            place(sorter, 7, 5, Dir::Up),
        ];
        cmds.extend((3..7).map(|x| place(ids.conveyor, x, 8, Dir::Right)));
        scenario::build(&mut w, &reg, &cmds).unwrap();
        w.apply(
            &reg,
            &Command::Configure {
                at: TilePos::new(7, 5),
                item: Some(ids.iron_ingot),
            },
        )
        .unwrap();

        let s = w
            .copy_schematic(&reg, TilePos::new(2, 4), TilePos::new(9, 9))
            .unwrap();
        assert_eq!((s.width, s.height), (5, 4)); // x 3..7, y 5..8
        assert_eq!(s.entries.len(), 6);
        scenario::build(&mut w, &reg, &s.commands(TilePos::new(30, 30))).unwrap();
        let again = w
            .copy_schematic(&reg, TilePos::new(30, 30), TilePos::new(34, 33))
            .unwrap();
        assert_eq!(
            again, s,
            "pasted copy is identical, including the sorter filter"
        );
        w.validate(&reg).unwrap();
    }

    #[test]
    fn four_rotations_are_identity_and_respect_sizes() {
        let reg = ContentRegistry::base().unwrap();
        let ids = BaseIds::resolve(&reg).unwrap();
        let mut w = World::new(&reg, 64, 64, ids.basalt, 1);
        let mut cmds = vec![
            place(ids.core, 10, 10, Dir::Right),
            place(ids.bore, 13, 10, Dir::Right),
        ];
        cmds.extend((10..16).map(|x| place(ids.conveyor, x, 13, Dir::Right)));
        scenario::build(&mut w, &reg, &cmds).unwrap();
        let s = w
            .copy_schematic(&reg, TilePos::new(10, 10), TilePos::new(15, 13))
            .unwrap();
        let r1 = s.rotated_cw(&reg);
        assert_eq!((r1.width, r1.height), (s.height, s.width));
        // повёрнутая схема ставится без перекрытий
        scenario::build(&mut w, &reg, &r1.commands(TilePos::new(30, 30))).unwrap();
        // лента после поворота по часовой смотрит вниз
        assert!(
            r1.entries
                .iter()
                .filter(|e| e.block == ids.conveyor)
                .all(|e| e.rot == Dir::Down)
        );
        let r4 = r1.rotated_cw(&reg).rotated_cw(&reg).rotated_cw(&reg);
        assert_eq!(r4, s);
        w.validate(&reg).unwrap();
    }
}
