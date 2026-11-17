//! Энергосети v0.
//!
//! Сеть — связная компонента графа: постройки, касающиеся друг друга сторонами, плюс связи
//! узлов с постройками в радиусе. Ограничение v0: при любом изменении энергопостроек граф
//! перестраивается целиком (union-find по всем энергопостройкам); локальная перестройка — задача S3-02.

use ferrustry_content::{BlockKind, ContentRegistry};
use serde::{Deserialize, Serialize};

use crate::buildings::{Arena, BuildingId, KindTag};
use crate::world::World;

/// Сводка одной сети за текущий тик.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerNet {
    /// Выработка за тик.
    pub production: u64,
    /// Спрос за тик.
    pub demand: u64,
    /// Удовлетворение спроса в тысячных (0..=1000).
    pub satisfaction: u32,
    /// Энергия в батареях сети (после этого тика).
    pub stored: u64,
    /// Суммарная ёмкость батарей сети.
    pub capacity: u64,
    /// Поток батарей за тик: > 0 — заряд, < 0 — разряд.
    pub battery_flow: i64,
    /// Остаток потока при распределении по батареям (служебное).
    pub(crate) pending: i64,
}

/// Состояние энергосистемы.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerState {
    pub(crate) nets: Vec<PowerNet>,
    pub(crate) dirty: bool,
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        // меньший индекс — корень: результат не зависит от порядка объединений
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi] = lo;
    }
}

impl World {
    /// Сети энергосистемы.
    pub fn power_nets(&self) -> &[PowerNet] {
        &self.power.nets
    }

    /// Сеть постройки.
    pub fn power_net_of(&self, id: BuildingId) -> Option<&PowerNet> {
        let n = self.buildings.get(id)?.power_net?;
        self.power.nets.get(n as usize)
    }

    pub(crate) fn rebuild_power(&mut self, reg: &ContentRegistry) {
        let ids: Vec<BuildingId> = self
            .buildings
            .iter()
            .filter(|(_, b)| reg.block(b.block).is_power())
            .map(|(id, _)| id)
            .collect();
        let mut local = vec![usize::MAX; self.buildings.slot_count()];
        for (i, id) in ids.iter().enumerate() {
            local[Arena::slot_index(*id)] = i;
        }
        let mut parent: Vec<usize> = (0..ids.len()).collect();

        for (i, id) in ids.iter().enumerate() {
            let Some(b) = self.buildings.get(*id) else {
                continue;
            };
            // касание сторонами
            for p in 0..b.perimeter_len() {
                let (outer, _) = b.perimeter(p);
                if let Some(other) = self.grid.building(outer) {
                    let j = local[Arena::slot_index(other)];
                    if j != usize::MAX {
                        union(&mut parent, i, j);
                    }
                }
            }
            // лазерные связи узлов
            if b.tag == KindTag::PowerNode
                && let BlockKind::PowerNode { range } = reg.block(b.block).kind
            {
                let (cx, cy) = b.center2();
                let r2 = (2 * i64::from(range)).pow(2);
                for (j, other) in ids.iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    let Some(ob) = self.buildings.get(*other) else {
                        continue;
                    };
                    let (ox, oy) = ob.center2();
                    if (ox - cx).pow(2) + (oy - cy).pow(2) <= r2 {
                        union(&mut parent, i, j);
                    }
                }
            }
        }

        let mut net_of_root = vec![u32::MAX; ids.len()];
        let mut count = 0u32;
        for (i, id) in ids.iter().enumerate() {
            let r = find(&mut parent, i);
            if net_of_root[r] == u32::MAX {
                net_of_root[r] = count;
                count += 1;
            }
            if let Some(b) = self.buildings.get_mut(*id) {
                b.power_net = Some(net_of_root[r]);
            }
        }
        self.power.nets = (0..count)
            .map(|_| PowerNet {
                stored: 0,
                capacity: 0,
                battery_flow: 0,
                pending: 0,
                production: 0,
                demand: 0,
                satisfaction: 0,
            })
            .collect();
        self.power.dirty = false;
    }

    pub(crate) fn balance_power(&mut self, reg: &ContentRegistry) {
        for n in &mut self.power.nets {
            n.production = 0;
            n.demand = 0;
        }
        for owner in &self.generators.owners {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            if let (Some(n), BlockKind::Generator { power }) =
                (b.power_net, &reg.block(b.block).kind)
                && let Some(net) = self.power.nets.get_mut(n as usize)
            {
                net.production += u64::from(*power);
            }
        }
        // тепловые машины: выработка, рассчитанная в прошлом тике
        for (row, owner) in self.engines.rows.iter().zip(&self.engines.owners) {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            if let Some(net) = b
                .power_net
                .and_then(|n| self.power.nets.get_mut(n as usize))
            {
                net.production += u64::from(row.output);
            }
        }
        // конденсаторы: спрос, если в их жидкостной сети есть место
        for owner in &self.condensers.owners {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            let BlockKind::Condenser { power_use, .. } = reg.block(b.block).kind else {
                continue;
            };
            if power_use == 0 || !self.condenser_wants(reg, *owner) {
                continue;
            }
            if let Some(net) = b
                .power_net
                .and_then(|n| self.power.nets.get_mut(n as usize))
            {
                net.demand += u64::from(power_use);
            }
        }
        for (row, owner) in self.crafters.rows.iter().zip(&self.crafters.owners) {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            let BlockKind::Crafter { power_use, .. } = reg.block(b.block).kind else {
                continue;
            };
            if power_use == 0 || !World::crafter_ready(reg, b.block, row) {
                continue;
            }
            if let Some(net) = b
                .power_net
                .and_then(|n| self.power.nets.get_mut(n as usize))
            {
                net.demand += u64::from(power_use);
            }
        }
        // батареи: суммарный запас и ёмкость по сетям
        for n in &mut self.power.nets {
            n.stored = 0;
            n.capacity = 0;
            n.battery_flow = 0;
        }
        for (row, owner) in self.batteries.rows.iter().zip(&self.batteries.owners) {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            let BlockKind::Battery { capacity } = reg.block(b.block).kind else {
                continue;
            };
            if let Some(net) = b
                .power_net
                .and_then(|n| self.power.nets.get_mut(n as usize))
            {
                net.stored += row.stored;
                net.capacity += u64::from(capacity);
            }
        }
        for n in &mut self.power.nets {
            if n.production >= n.demand {
                n.satisfaction = 1000;
                let charge = (n.production - n.demand).min(n.capacity - n.stored.min(n.capacity));
                n.battery_flow = i64::try_from(charge).unwrap_or(0);
            } else {
                let deficit = n.demand - n.production;
                let drain = deficit.min(n.stored);
                n.battery_flow = -i64::try_from(drain).unwrap_or(0);
                let available = n.production + drain;
                n.satisfaction =
                    u32::try_from((available * 1000 / n.demand).min(1000)).unwrap_or(1000);
            }
        }
        // распределить заряд/разряд по батареям сети в порядке таблицы (детерминированно)
        for n in &mut self.power.nets {
            n.stored = 0;
            n.pending = n.battery_flow;
        }
        for r in 0..self.batteries.len() {
            let owner = self.batteries.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let BlockKind::Battery { capacity } = reg.block(b.block).kind else {
                continue;
            };
            let Some(net) = b
                .power_net
                .and_then(|n| self.power.nets.get_mut(n as usize))
            else {
                continue;
            };
            let row = &mut self.batteries.rows[r];
            if net.pending > 0 {
                let want = u64::try_from(net.pending).unwrap_or(0);
                let put = want.min(u64::from(capacity).saturating_sub(row.stored));
                row.stored += put;
                net.pending -= i64::try_from(put).unwrap_or(0);
            } else if net.pending < 0 {
                let want = net.pending.unsigned_abs();
                let take = want.min(row.stored);
                row.stored -= take;
                net.pending += i64::try_from(take).unwrap_or(0);
            }
            net.stored += row.stored;
        }
    }
}
