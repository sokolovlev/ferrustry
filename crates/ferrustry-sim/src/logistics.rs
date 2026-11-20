//! Разветвители, сортировщики и мосты.
//!
//! Все три — постройки 1×1, которым важна сторона входа ([`Link::Directed`]). Буфер
//! разветвителя и сортировщика — один предмет; мост хранит до `buffer` предметов.

use ferrustry_content::{BlockKind, ContentRegistry, ItemId};

use crate::buildings::{BuildingId, KindTag};
use crate::geom::Dir;
use crate::world::{World, rot_left, rot_right};

impl World {
    /// Приём предмета разветвителем/сортировщиком/мостом со стороны `side`.
    pub(crate) fn offer_directed(
        &mut self,
        reg: &ContentRegistry,
        id: BuildingId,
        side: Dir,
        item: ItemId,
    ) -> bool {
        let Some(b) = self.buildings.get(id) else {
            return false;
        };
        let row = b.row as usize;
        match b.tag {
            KindTag::Splitter | KindTag::Sorter | KindTag::Overflow => {
                match self.routers.rows.get_mut(row) {
                    Some(r) if r.item.is_none() => {
                        r.item = Some(item);
                        r.from = side;
                        true
                    }
                    _ => false,
                }
            }
            KindTag::Bridge => {
                let BlockKind::Bridge { buffer, .. } = reg.block(b.block).kind else {
                    return false;
                };
                match self.bridges.rows.get_mut(row) {
                    Some(r) if r.items.len() < buffer as usize => {
                        r.items.push(item);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Разветвители: раздача по кругу на три стороны, кроме входной.
    /// Сортировщики: фильтр — прямо, остальное — поочерёдно влево/вправо.
    pub(crate) fn update_routers(&mut self, reg: &ContentRegistry) {
        for r in 0..self.routers.len() {
            let Some(item) = self.routers.rows[r].item else {
                continue;
            };
            let owner = self.routers.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let (tag, p) = (b.tag, b.origin);
            let row = &self.routers.rows[r];
            let (from, filter, cursor) = (row.from, row.filter, row.cursor);
            // направление движения предмета — от входной стороны
            let travel = from.opposite();
            let mut out: [Option<Dir>; 3] = [None; 3];
            match tag {
                KindTag::Splitter => {
                    let mut n = 0;
                    for k in 0..4u8 {
                        let d = Dir::ALL[usize::from((cursor + k) % 4)];
                        if d != from {
                            out[n] = Some(d);
                            n += 1;
                        }
                    }
                }
                KindTag::Overflow => {
                    // прямо, пока впереди принимают; иначе вбок, поочерёдно
                    out[0] = Some(travel);
                    if cursor % 2 == 0 {
                        out[1] = Some(rot_left(travel));
                        out[2] = Some(rot_right(travel));
                    } else {
                        out[1] = Some(rot_right(travel));
                        out[2] = Some(rot_left(travel));
                    }
                }
                _ => {
                    if filter == Some(item) {
                        out[0] = Some(travel);
                    } else if cursor % 2 == 0 {
                        out[0] = Some(rot_left(travel));
                        out[1] = Some(rot_right(travel));
                    } else {
                        out[0] = Some(rot_right(travel));
                        out[1] = Some(rot_left(travel));
                    }
                }
            }
            for d in out.into_iter().flatten() {
                if self.offer(reg, p.step(d), p, item) {
                    let row = &mut self.routers.rows[r];
                    row.item = None;
                    row.cursor = match tag {
                        KindTag::Splitter => (d as u8 + 1) % 4,
                        // чередование сторон — только когда предмет ушёл вбок
                        KindTag::Overflow if d == travel => row.cursor,
                        _ => row.cursor.wrapping_add(1),
                    };
                    break;
                }
            }
        }
    }

    /// Партнёр моста: первый мост того же направления в пределах `range` тайлов впереди.
    fn bridge_partner(&self, reg: &ContentRegistry, id: BuildingId) -> Option<BuildingId> {
        let b = self.buildings.get(id)?;
        let BlockKind::Bridge { range, .. } = reg.block(b.block).kind else {
            return None;
        };
        let mut q = b.origin;
        for _ in 0..range {
            q = q.step(b.rot);
            if let Some(o) = self.grid.building(q)
                && let Some(ob) = self.buildings.get(o)
                && ob.tag == KindTag::Bridge
                && ob.rot == b.rot
            {
                return Some(o);
            }
        }
        None
    }

    /// Мосты: вход передаёт предметы партнёру (1 за `interval` тиков), выход выдаёт вперёд.
    pub(crate) fn update_bridges(&mut self, reg: &ContentRegistry) {
        let epoch = self.topo_epoch();
        for r in 0..self.bridges.len() {
            let owner = self.bridges.owners[r];
            if self.bridges.rows[r].partner_epoch != epoch {
                let partner = self.bridge_partner(reg, owner);
                let row = &mut self.bridges.rows[r];
                row.partner = partner;
                row.partner_epoch = epoch;
            }
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let (p, rot, block) = (b.origin, b.rot, b.block);
            let BlockKind::Bridge {
                interval, buffer, ..
            } = reg.block(block).kind
            else {
                continue;
            };
            let row = &mut self.bridges.rows[r];
            row.cooldown = row.cooldown.saturating_sub(1);
            let Some(&item) = row.items.first() else {
                continue;
            };
            match row.partner {
                Some(partner) => {
                    if row.cooldown > 0 {
                        continue;
                    }
                    let Some(pb) = self.buildings.get(partner) else {
                        continue;
                    };
                    let pr = pb.row as usize;
                    if self
                        .bridges
                        .rows
                        .get(pr)
                        .is_some_and(|x| x.items.len() < buffer as usize)
                    {
                        self.bridges.rows[r].items.remove(0);
                        self.bridges.rows[r].cooldown = interval;
                        self.bridges.rows[pr].items.push(item);
                    }
                }
                None => {
                    if self.offer(reg, p.step(rot), p, item) {
                        self.bridges.rows[r].items.remove(0);
                    }
                }
            }
        }
    }
}

impl World {
    /// Склады: каждый выдаёт не больше одного предмета за тик на передние тайлы (по кругу).
    /// Если очередной предмет никто не принимает, курсор переходит к следующему виду —
    /// склад со смесью не застревает перед производством, которому нужен другой предмет.
    pub(crate) fn update_depots(&mut self, reg: &ContentRegistry) {
        for r in 0..self.depots.len() {
            if self.depots.rows[r].total == 0 {
                continue;
            }
            let Some(b) = self.buildings.get(self.depots.owners[r]) else {
                continue;
            };
            let s = u32::from(b.size);
            let side = match b.rot {
                Dir::Down => 0,
                Dir::Right => 1,
                Dir::Up => 2,
                Dir::Left => 3,
            };
            let front: [_; 8] = std::array::from_fn(|k| b.perimeter(side * s + (k as u32 % s)));
            let row = &self.depots.rows[r];
            let n = row.items.len() as u32;
            let Some(idx) = (0..n)
                .map(|k| (row.next_item + k) % n)
                .find(|&i| row.items[i as usize] > 0)
            else {
                continue;
            };
            let item = ItemId(idx as u16);
            let start = row.next_side;
            let mut sent = None;
            for k in 0..s {
                let t = (start + k) % s;
                let (outer, inner) = front[t as usize];
                if self.offer(reg, outer, inner, item) {
                    sent = Some(t);
                    break;
                }
            }
            let row = &mut self.depots.rows[r];
            row.next_item = (idx + 1) % n;
            if let Some(t) = sent {
                row.items[idx as usize] -= 1;
                row.total -= 1;
                row.next_side = (t + 1) % s;
            }
        }
    }
}
