//! Энергосети.
//!
//! Сеть — связная компонента графа: энергопостройки, касающиеся друг друга сторонами, плюс
//! связи узлов с энергопостройками в радиусе узла. Сеть поддерживается **локально** (S3-02):
//!
//! - стройка присоединяет постройку к сети соседей; если она соединила несколько сетей,
//!   перенумеровываются все, кроме самой большой (параллельный обход в ногу останавливается,
//!   как только остальные исчерпаны);
//! - снос запускает обходы в ногу от бывших соседей: если сеть распалась, новые номера получают
//!   только меньшие куски. Цена — размер меньших кусков, а не всей сети;
//! - соседи ищутся по сетке (периметр и квадрат вокруг центра в радиусе узлов), а не
//!   перебором всех энергопостроек.
//!
//! Номера сетей зависят от порядка изменений, но разбиение на сети всегда совпадает с полным
//! пересчётом (проверяет `World::validate`), а поведение зависит только от разбиения.
//! Освободившиеся номера переиспользуются (`free`).

use std::collections::{HashMap, HashSet, VecDeque};

use ferrustry_content::{BlockId, BlockKind, ContentRegistry};
use serde::{Deserialize, Serialize};

use crate::buildings::{Arena, Building, BuildingId, KindTag};
use crate::geom::TilePos;
use crate::world::{Stats, World};

/// Сводка одной сети за текущий тик.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Выработка генераторов без топлива: постоянна, пока сеть не меняется (служебное).
    pub(crate) steady: u64,
}

/// Состояние энергосистемы.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerState {
    pub(crate) nets: Vec<PowerNet>,
    /// Нужен полный пересчёт (не используется обычной стройкой; запасной путь).
    pub(crate) dirty: bool,
    /// Свободные номера сетей (сети без построек).
    pub(crate) free: Vec<u32>,
}

/// Что нужно знать о постройке для поиска её энергососедей (постройки может уже не быть).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PowerShape {
    pub(crate) id: BuildingId,
    pub(crate) block: BlockId,
    pub(crate) origin: TilePos,
    pub(crate) size: u8,
}

impl PowerShape {
    pub(crate) fn of(id: BuildingId, b: &Building) -> Self {
        Self {
            id,
            block: b.block,
            origin: b.origin,
            size: b.size,
        }
    }

    fn building(self) -> Building {
        Building::shape(self.block, self.origin, self.size)
    }
}

/// Дальность энергоузла (`None` — не узел).
fn node_range(reg: &ContentRegistry, block: BlockId) -> Option<i64> {
    match reg.block(block).kind {
        BlockKind::PowerNode { range } => Some(i64::from(range)),
        _ => None,
    }
}

/// Наибольшая дальность узлов в контенте.
fn max_node_range(reg: &ContentRegistry) -> i64 {
    reg.blocks()
        .iter()
        .filter_map(|d| match d.kind {
            BlockKind::PowerNode { range } => Some(i64::from(range)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Постоянная выработка постройки (генератор без топлива), учитываемая в `steady`.
fn steady_of(reg: &ContentRegistry, block: BlockId) -> u64 {
    match reg.block(block).kind {
        BlockKind::Generator {
            power, fuel: None, ..
        } => u64::from(power),
        _ => 0,
    }
}

impl World {
    /// Сети энергосистемы (включая свободные номера без построек).
    pub fn power_nets(&self) -> &[PowerNet] {
        &self.power.nets
    }

    /// Число сетей, в которых есть постройки.
    pub fn live_power_nets(&self) -> usize {
        self.power.nets.len() - self.power.free.len()
    }

    /// Сеть постройки.
    pub fn power_net_of(&self, id: BuildingId) -> Option<&PowerNet> {
        let n = self.buildings.get(id)?.power_net?;
        self.power.nets.get(n as usize)
    }

    /// Энергососеди: касание сторонами и связи узлов. Порядок детерминирован (по слоту).
    pub(crate) fn power_neighbors(
        &self,
        reg: &ContentRegistry,
        s: PowerShape,
        out: &mut Vec<BuildingId>,
    ) {
        out.clear();
        let b = s.building();
        for p in 0..b.perimeter_len() {
            let (outer, _) = b.perimeter(p);
            if let Some(o) = self.grid.building(outer)
                && o != s.id
                && let Some(ob) = self.buildings.get(o)
                && reg.block(ob.block).is_power()
            {
                out.push(o);
            }
        }
        // узлы: своя дальность (если это узел) или дальность узлов вокруг
        let own = node_range(reg, s.block);
        let reach = own.unwrap_or(0).max(max_node_range(reg));
        if reach > 0 {
            let (cx, cy) = b.center2();
            // центр постройки лежит в её «центральном тайле»: сканируем квадрат вокруг центра
            let (x0, x1) = ((cx - 2 * reach) / 2 - 1, (cx + 2 * reach) / 2 + 1);
            let (y0, y1) = ((cy - 2 * reach) / 2 - 1, (cy + 2 * reach) / 2 + 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let t = TilePos::new(x as i32, y as i32);
                    let Some(o) = self.grid.building(t) else {
                        continue;
                    };
                    if o == s.id {
                        continue;
                    }
                    let Some(ob) = self.buildings.get(o) else {
                        continue;
                    };
                    let (ox, oy) = ob.center2();
                    // каждую постройку — один раз: в тайле, где лежит её центр
                    if TilePos::new((ox / 2) as i32, (oy / 2) as i32) != t {
                        continue;
                    }
                    if !reg.block(ob.block).is_power() {
                        continue;
                    }
                    let d2 = (ox - cx).pow(2) + (oy - cy).pow(2);
                    let mine = own.is_some_and(|r| d2 <= (2 * r).pow(2));
                    let theirs = node_range(reg, ob.block).is_some_and(|r| d2 <= (2 * r).pow(2));
                    if mine || theirs {
                        out.push(o);
                    }
                }
            }
        }
        out.sort_unstable_by_key(|&o| Arena::slot_index(o));
        out.dedup();
    }

    fn alloc_net(&mut self) -> u32 {
        match self.power.free.pop() {
            Some(n) => {
                self.power.nets[n as usize] = PowerNet::default();
                n
            }
            None => {
                self.power.nets.push(PowerNet::default());
                (self.power.nets.len() - 1) as u32
            }
        }
    }

    fn free_net(&mut self, n: u32) {
        self.power.nets[n as usize] = PowerNet::default();
        self.power.free.push(n);
    }

    /// Записать номер сети постройке (и плотной строке производства).
    fn set_power_net(&mut self, id: BuildingId, net: Option<u32>) {
        if let Some(b) = self.buildings.get_mut(id) {
            b.power_net = net;
            if b.tag == KindTag::Crafter
                && let Some(row) = self.crafters.rows.get_mut(b.row as usize)
            {
                row.power_net = net;
            }
        }
    }

    /// Новая энергопостройка: присоединить к сетям соседей (объединив их).
    pub(crate) fn power_attach(&mut self, reg: &ContentRegistry, id: BuildingId) {
        let Some(b) = self.buildings.get(id) else {
            return;
        };
        let shape = PowerShape::of(id, b);
        let mut nbrs = Vec::new();
        self.power_neighbors(reg, shape, &mut nbrs);
        // сети соседей с представителем каждой, в порядке первого появления
        let mut groups: Vec<(u32, BuildingId)> = Vec::new();
        for &o in &nbrs {
            if let Some(n) = self.buildings.get(o).and_then(|ob| ob.power_net)
                && !groups.iter().any(|&(g, _)| g == n)
            {
                groups.push((n, o));
            }
        }
        let net = match groups.len() {
            0 => self.alloc_net(),
            1 => groups[0].0,
            _ => self.power_merge(reg, &groups),
        };
        self.set_power_net(id, Some(net));
        self.power.nets[net as usize].steady += steady_of(reg, shape.block);
    }

    /// Объединить сети: самая большая сохраняет номер, остальные перенумеровываются.
    fn power_merge(&mut self, reg: &ContentRegistry, groups: &[(u32, BuildingId)]) -> u32 {
        let k = groups.len();
        let mut queues: Vec<VecDeque<BuildingId>> = groups
            .iter()
            .map(|&(_, rep)| VecDeque::from([rep]))
            .collect();
        let mut members: Vec<Vec<BuildingId>> = vec![Vec::new(); k];
        let mut seen: HashSet<usize> = groups
            .iter()
            .map(|&(_, rep)| Arena::slot_index(rep))
            .collect();
        let mut nbrs = Vec::new();
        // обход в ногу, пока хотя бы две сети не исчерпаны
        while queues.iter().filter(|q| !q.is_empty()).count() > 1 {
            for i in 0..k {
                let Some(x) = queues[i].pop_front() else {
                    continue;
                };
                members[i].push(x);
                let Some(xb) = self.buildings.get(x) else {
                    continue;
                };
                self.power_neighbors(reg, PowerShape::of(x, xb), &mut nbrs);
                for &o in &nbrs {
                    if self.buildings.get(o).and_then(|ob| ob.power_net) == Some(groups[i].0)
                        && seen.insert(Arena::slot_index(o))
                    {
                        queues[i].push_back(o);
                    }
                }
            }
        }
        let survivor = queues
            .iter()
            .position(|q| !q.is_empty())
            .unwrap_or_else(|| {
                // все исчерпаны одновременно: больше всех построек, при равенстве — меньший номер
                (0..k)
                    .max_by_key(|&i| (members[i].len(), std::cmp::Reverse(groups[i].0)))
                    .unwrap_or(0)
            });
        let keep = groups[survivor].0;
        for i in 0..k {
            if i == survivor {
                continue;
            }
            let old = groups[i].0;
            for &m in &members[i] {
                self.set_power_net(m, Some(keep));
            }
            let steady = self.power.nets[old as usize].steady;
            self.power.nets[keep as usize].steady += steady;
            self.free_net(old);
        }
        keep
    }

    /// Снесённая энергопостройка (её уже нет в сетке и арене): сеть может распасться.
    pub(crate) fn power_detach(
        &mut self,
        reg: &ContentRegistry,
        shape: PowerShape,
        net: Option<u32>,
    ) {
        let Some(n) = net else {
            return;
        };
        let st = &mut self.power.nets[n as usize].steady;
        *st = st.saturating_sub(steady_of(reg, shape.block));
        let mut nbrs = Vec::new();
        self.power_neighbors(reg, shape, &mut nbrs);
        nbrs.retain(|&o| self.buildings.get(o).and_then(|b| b.power_net) == Some(n));
        match nbrs.len() {
            0 => self.free_net(n),
            1 => {}
            _ => self.power_split(reg, n, &nbrs),
        }
    }

    /// Обходы в ногу от бывших соседей; встретившиеся обходы сливаются. Исчерпанные куски
    /// (кроме последнего) — отдельные сети с новыми номерами; остальное сохраняет номер `n`.
    fn power_split(&mut self, reg: &ContentRegistry, n: u32, seeds: &[BuildingId]) {
        let k = seeds.len();
        let mut parent: Vec<usize> = (0..k).collect();
        fn root(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        let mut queues: Vec<VecDeque<BuildingId>> =
            seeds.iter().map(|&s| VecDeque::from([s])).collect();
        let mut members: Vec<Vec<BuildingId>> = vec![Vec::new(); k];
        let mut owner: HashMap<usize, usize> = seeds
            .iter()
            .enumerate()
            .map(|(i, &s)| (Arena::slot_index(s), i))
            .collect();
        let mut nbrs = Vec::new();
        let active = |parent: &mut [usize], queues: &[VecDeque<BuildingId>]| {
            (0..k)
                .filter(|&i| root(parent, i) == i && !queues[i].is_empty())
                .count()
        };
        while active(&mut parent, &queues) > 1 {
            for i in 0..k {
                if root(&mut parent, i) != i {
                    continue;
                }
                let Some(x) = queues[i].pop_front() else {
                    continue;
                };
                let mut r = i;
                members[r].push(x);
                let Some(xb) = self.buildings.get(x) else {
                    continue;
                };
                self.power_neighbors(reg, PowerShape::of(x, xb), &mut nbrs);
                for &o in &nbrs {
                    if self.buildings.get(o).and_then(|ob| ob.power_net) != Some(n) {
                        continue;
                    }
                    let slot = Arena::slot_index(o);
                    match owner.get(&slot) {
                        None => {
                            owner.insert(slot, r);
                            queues[r].push_back(o);
                        }
                        Some(&g) => {
                            let rg = root(&mut parent, g);
                            if rg != r {
                                // обходы встретились: один кусок — сливаем в меньший индекс
                                let (lo, hi) = (r.min(rg), r.max(rg));
                                parent[hi] = lo;
                                let q = std::mem::take(&mut queues[hi]);
                                queues[lo].extend(q);
                                let m = std::mem::take(&mut members[hi]);
                                members[lo].extend(m);
                                r = lo;
                            }
                        }
                    }
                }
            }
        }
        let roots: Vec<usize> = (0..k).filter(|&i| root(&mut parent, i) == i).collect();
        if roots.len() < 2 {
            return;
        }
        let survivor = roots
            .iter()
            .copied()
            .find(|&i| !queues[i].is_empty())
            .unwrap_or_else(|| {
                roots
                    .iter()
                    .copied()
                    .max_by_key(|&i| (members[i].len(), std::cmp::Reverse(i)))
                    .unwrap_or(roots[0])
            });
        for &i in &roots {
            if i == survivor {
                continue;
            }
            let m = self.alloc_net();
            let mut moved = 0u64;
            for &b in &members[i] {
                if let Some(block) = self.buildings.get(b).map(|bb| bb.block) {
                    moved += steady_of(reg, block);
                }
                self.set_power_net(b, Some(m));
            }
            let st = &mut self.power.nets[n as usize].steady;
            *st = st.saturating_sub(moved);
            self.power.nets[m as usize].steady += moved;
        }
    }

    /// Разбиение энергопостроек на сети полным обходом (слот → номер компоненты в порядке
    /// арены). Для полного пересчёта и проверки инкрементального.
    pub(crate) fn power_components(&self, reg: &ContentRegistry) -> Vec<(BuildingId, u32)> {
        let mut comp: HashMap<usize, u32> = HashMap::new();
        let mut out = Vec::new();
        let mut nbrs = Vec::new();
        let mut count = 0u32;
        let ids: Vec<BuildingId> = self
            .buildings
            .iter()
            .filter(|(_, b)| reg.block(b.block).is_power())
            .map(|(id, _)| id)
            .collect();
        for &start in &ids {
            if comp.contains_key(&Arena::slot_index(start)) {
                continue;
            }
            comp.insert(Arena::slot_index(start), count);
            let mut q = VecDeque::from([start]);
            while let Some(x) = q.pop_front() {
                let Some(xb) = self.buildings.get(x) else {
                    continue;
                };
                self.power_neighbors(reg, PowerShape::of(x, xb), &mut nbrs);
                for &o in &nbrs {
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        comp.entry(Arena::slot_index(o))
                    {
                        e.insert(count);
                        q.push_back(o);
                    }
                }
            }
            count += 1;
        }
        for id in ids {
            out.push((id, comp[&Arena::slot_index(id)]));
        }
        out
    }

    /// Полный пересчёт сетей (запасной путь: `power.dirty`).
    pub(crate) fn rebuild_power(&mut self, reg: &ContentRegistry) {
        let comps = self.power_components(reg);
        let count = comps.iter().map(|&(_, c)| c + 1).max().unwrap_or(0);
        self.power.nets = vec![PowerNet::default(); count as usize];
        self.power.free.clear();
        for (id, c) in comps {
            if let Some(b) = self.buildings.get_mut(id) {
                b.power_net = Some(c);
            }
        }
        for (_, b) in self.buildings.iter() {
            if let Some(n) = b.power_net {
                self.power.nets[n as usize].steady += steady_of(reg, b.block);
            }
        }
        self.power.dirty = false;
        self.sync_crafter_nets();
    }

    /// Проверка: разбиение совпадает с полным пересчётом, свободные номера пусты, `steady` верен.
    pub(crate) fn validate_power(&self, reg: &ContentRegistry) -> Result<(), String> {
        let free: HashSet<u32> = self.power.free.iter().copied().collect();
        if free.len() != self.power.free.len() {
            return Err("duplicate free power net".into());
        }
        let mut comp_to_net: HashMap<u32, u32> = HashMap::new();
        let mut net_to_comp: HashMap<u32, u32> = HashMap::new();
        let mut steady = vec![0u64; self.power.nets.len()];
        for (id, c) in self.power_components(reg) {
            let b = self.buildings.get(id).ok_or("power building vanished")?;
            let n = b
                .power_net
                .ok_or_else(|| format!("power building at {:?} has no net", b.origin))?;
            if n as usize >= self.power.nets.len() || free.contains(&n) {
                return Err(format!(
                    "building at {:?} uses a free or missing net {n}",
                    b.origin
                ));
            }
            if *comp_to_net.entry(c).or_insert(n) != n || *net_to_comp.entry(n).or_insert(c) != c {
                return Err(format!(
                    "power nets differ from a full rebuild at {:?}",
                    b.origin
                ));
            }
            steady[n as usize] += steady_of(reg, b.block);
        }
        for (n, net) in self.power.nets.iter().enumerate() {
            if net.steady != steady[n] {
                return Err(format!(
                    "power net {n} steady output {} != {}",
                    net.steady, steady[n]
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn balance_power(&mut self, reg: &ContentRegistry) {
        // Проходов по сетям — два (этот и баланс): сетей бывают тысячи, каждый проход по ним —
        // промахи кэша (P-04). Поля батарей тоже обнуляются здесь: потребители их не трогают.
        for n in &mut self.power.nets {
            n.production = n.steady;
            n.demand = 0;
            n.stored = 0;
            n.capacity = 0;
            n.battery_flow = 0;
        }
        // топливные генераторы горят, пока есть топливо (горение не зависит от спроса — v0)
        let (nets, stats) = (&mut self.power.nets, &mut self.stats);
        let mut noise = 0u32;
        for (g, owner) in self.generators.rows.iter_mut().zip(&self.generators.owners) {
            if !g.fueled {
                continue; // учтён в `steady`
            }
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            let def = reg.block(b.block);
            let BlockKind::Generator {
                power,
                fuel,
                burn_ticks,
                ..
            } = def.kind
            else {
                continue;
            };
            let Some(fuel) = fuel else { continue };
            if g.burn == 0 && g.fuel > 0 {
                g.fuel -= 1;
                g.burn = burn_ticks;
                Stats::bump(&mut stats.consumed, fuel, 1);
            }
            if g.burn == 0 {
                continue;
            }
            g.burn -= 1;
            noise += def.noise;
            if let Some(net) = b.power_net.and_then(|n| nets.get_mut(n as usize)) {
                net.production += u64::from(power);
            }
        }
        self.noise_acc += noise;
        // дуговые турели: спрос, пока идёт перезарядка
        for (row, owner) in self.turrets.rows.iter().zip(&self.turrets.owners) {
            if row.cooldown == 0 {
                continue;
            }
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            if let BlockKind::ArcTurret { power_use, .. } = reg.block(b.block).kind
                && let Some(net) = b
                    .power_net
                    .and_then(|n| self.power.nets.get_mut(n as usize))
            {
                net.demand += u64::from(power_use);
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
        // производства: всё нужное — в плотной строке, без обращения к постройке (P-04)
        for row in &self.crafters.rows {
            let BlockKind::Crafter { power_use, .. } = reg.block(row.block).kind else {
                continue;
            };
            if power_use == 0 || !World::crafter_ready(reg, row.block, row) {
                continue;
            }
            if let Some(net) = row
                .power_net
                .and_then(|n| self.power.nets.get_mut(n as usize))
            {
                net.demand += u64::from(power_use);
            }
        }
        // батареи: суммарный запас и ёмкость по сетям
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
            // подготовка распределения по батареям (в том же проходе)
            n.stored = 0;
            n.pending = n.battery_flow;
        }
        // распределить заряд/разряд по батареям сети в порядке таблицы (детерминированно)
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::geom::Dir;
    use crate::rng::Rng;
    use crate::world::{Command, Rules};

    /// Канонический вид: сети перенумерованы полным пересчётом (по порядку арены), сводки
    /// сетей обнулены. Совпадение хешей канонических копий значит, что разбиение одинаково,
    /// а всё остальное состояние (батареи, производства, предметы, статистика) — тоже.
    fn canonical(w: &World, reg: &ContentRegistry) -> u64 {
        let mut c = w.clone();
        c.rebuild_power(reg);
        crate::state_hash(&c)
    }

    fn sandbox(reg: &ContentRegistry, seed: u64) -> World {
        let basalt = reg.floor_by_name("basalt").unwrap();
        let mut w = World::new(reg, 40, 40, basalt, seed);
        w.set_rules(Rules::default());
        w
    }

    #[test]
    fn local_rebuild_matches_full_rebuild_on_random_edits() {
        let reg = ContentRegistry::base().unwrap();
        let names = [
            "power_node",
            "power_node",
            "wind_turbine",
            "wind_turbine",
            "battery",
            "smelter",
            "burner",
            "arc_tower",
            "iron_wall",
        ];
        let blocks: Vec<BlockId> = names
            .iter()
            .map(|n| reg.block_by_name(n).unwrap())
            .collect();
        let (mut merges, mut splits) = (0, 0);
        for seed in [1u64, 2, 3, 4] {
            let mut local = sandbox(&reg, seed);
            let mut full = sandbox(&reg, seed);
            let mut rng = Rng::new(seed * 7919);
            for tick in 0..300 {
                let mut cmds = Vec::new();
                for _ in 0..1 + rng.below(4) {
                    let p = TilePos::new(rng.below(40) as i32, rng.below(40) as i32);
                    let existing: Vec<TilePos> =
                        local.buildings().iter().map(|(_, b)| b.origin).collect();
                    // снос — существующей постройки (чтобы сети распадались), когда их много
                    if existing.len() > 40 && rng.below(100) < 45 {
                        let at = existing[rng.below(existing.len() as u32) as usize];
                        cmds.push(Command::Remove { at });
                    } else {
                        let block = blocks[rng.below(blocks.len() as u32) as usize];
                        cmds.push(Command::Place {
                            block,
                            origin: p,
                            rot: Dir::Right,
                        });
                    }
                }
                let before = local.live_power_nets();
                let ra = local.tick(&reg, &cmds);
                full.power.dirty = true; // эталон: полный пересчёт каждый тик
                let rb = full.tick(&reg, &cmds);
                assert_eq!(ra, rb, "seed {seed} tick {tick}: same command results");
                let after = local.live_power_nets();
                // число сетей падает — было объединение, растёт без новых одиночек — распад
                if after < before {
                    merges += 1;
                }
                if after > before && cmds.iter().all(|c| matches!(c, Command::Remove { .. })) {
                    splits += 1;
                }
                local
                    .validate(&reg)
                    .unwrap_or_else(|e| panic!("seed {seed} tick {tick}: {e}"));
                // поведение одинаково: сводки по каждой постройке и всё состояние
                for (id, _) in local.buildings().iter() {
                    assert_eq!(
                        local
                            .power_net_of(id)
                            .map(|n| (n.production, n.demand, n.satisfaction)),
                        full.power_net_of(id)
                            .map(|n| (n.production, n.demand, n.satisfaction)),
                        "seed {seed} tick {tick}: power summary differs"
                    );
                }
                assert_eq!(
                    canonical(&local, &reg),
                    canonical(&full, &reg),
                    "seed {seed} tick {tick}: state differs"
                );
            }
        }
        // сценарий должен реально объединять и разбивать сети
        assert!(
            merges > 50 && splits > 10,
            "merges {merges}, splits {splits}"
        );
    }

    #[test]
    fn removing_a_bridge_node_splits_and_reuses_net_ids() {
        let reg = ContentRegistry::base().unwrap();
        let node = reg.block_by_name("power_node").unwrap();
        let turbine = reg.block_by_name("wind_turbine").unwrap();
        let mut w = sandbox(&reg, 1);
        let place = |w: &mut World, b, x, y| {
            w.apply(
                &reg,
                &Command::Place {
                    block: b,
                    origin: TilePos::new(x, y),
                    rot: Dir::Right,
                },
            )
            .unwrap();
        };
        // два ветряка в 10 тайлах друг от друга: две сети; узел посередине их объединяет
        place(&mut w, turbine, 5, 5);
        place(&mut w, turbine, 15, 5);
        assert_eq!(w.live_power_nets(), 2);
        place(&mut w, node, 10, 5);
        assert_eq!(w.live_power_nets(), 1);
        assert_eq!(w.power.free.len(), 1, "the absorbed net id is free");
        w.step(&reg);
        let id = |w: &World, x, y| w.grid.building(TilePos::new(x, y)).unwrap();
        assert_eq!(w.power_net_of(id(&w, 5, 5)).unwrap().production, 4);
        // снос узла: снова две сети, освободившийся номер переиспользован
        w.apply(
            &reg,
            &Command::Remove {
                at: TilePos::new(10, 5),
            },
        )
        .unwrap();
        assert_eq!(w.live_power_nets(), 2);
        assert!(w.power.free.is_empty());
        w.step(&reg);
        assert_eq!(w.power_net_of(id(&w, 5, 5)).unwrap().production, 2);
        assert_eq!(w.power_net_of(id(&w, 15, 5)).unwrap().production, 2);
        w.validate(&reg).unwrap();
        // снос одиночки освобождает её сеть
        w.apply(
            &reg,
            &Command::Remove {
                at: TilePos::new(15, 5),
            },
        )
        .unwrap();
        assert_eq!(w.live_power_nets(), 1);
        w.validate(&reg).unwrap();
    }
}
