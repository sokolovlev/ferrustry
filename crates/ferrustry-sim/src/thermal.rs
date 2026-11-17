//! Жидкостные и тепловые сети (компартментная модель, решение D-09; тепло — D-02).
//!
//! Постройки, касающиеся друг друга сторонами, образуют сеть: у жидкостной — общий объём и
//! один вид жидкости, у тепловой — общий запас тепла и теплоёмкость. Тик — O(построек сети),
//! без численной гидродинамики. При перестройке (стройка/снос) содержимое старой сети делится
//! между уцелевшими участниками пропорционально ёмкости; доля снесённых теряется и учитывается.

use ferrustry_content::{BlockKind, ContentRegistry, FluidId};
use serde::{Deserialize, Serialize};

use crate::buildings::{Arena, BuildingId};
use crate::world::World;

/// Температура (тысячные заполнения сети), выше которой производства замедляются.
pub const THROTTLE_FROM: u32 = 700;

/// Жидкостная сеть.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FluidNet {
    /// Жидкость в сети (`None` — пусто).
    pub fluid: Option<FluidId>,
    /// Количество.
    pub amount: u64,
    /// Объём.
    pub capacity: u64,
}

/// Тепловая сеть.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeatNet {
    /// Запас тепла.
    pub stored: u64,
    /// Теплоёмкость.
    pub capacity: u64,
    /// Заполнение, тысячные (0..=1000).
    pub temperature: u32,
    /// Выделено за последний тик.
    pub generated: u64,
    /// Отведено за последний тик (охлаждение, машины).
    pub removed: u64,
}

/// Состояние сетей одного вида.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nets<N> {
    pub(crate) nets: Vec<N>,
    pub(crate) dirty: bool,
}

impl<N> Default for Nets<N> {
    fn default() -> Self {
        Self {
            nets: Vec::new(),
            dirty: false,
        }
    }
}

/// Учёт жидкостей (для проверки сохранения).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FluidStats {
    /// Произведено по ID жидкости.
    pub produced: Vec<u64>,
    /// Израсходовано.
    pub consumed: Vec<u64>,
    /// Потеряно при сносе/перестройке.
    pub destroyed: Vec<u64>,
}

impl FluidStats {
    pub(crate) fn bump(v: &mut Vec<u64>, f: FluidId, n: u64) {
        let i = usize::from(f.0);
        if v.len() <= i {
            v.resize(i + 1, 0);
        }
        v[i] += n;
    }
}

/// Коэффициент скорости производства от температуры его тепловой сети (тысячные).
pub fn heat_factor(temperature: u32) -> u32 {
    if temperature <= THROTTLE_FROM {
        1000
    } else {
        (1000 - temperature.min(1000)) * 1000 / (1000 - THROTTLE_FROM)
    }
}

/// Состояние тепловой машины.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EngineRow {
    /// Выработка энергии на следующий тик.
    pub(crate) output: u32,
    /// Дробная часть выработки, тысячные.
    pub(crate) frac: u32,
}

/// Состояние конденсатора.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CondenserRow {
    /// Накопленная выработка, тысячные единицы.
    pub(crate) acc: u32,
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NetKind {
    Fluid,
    Heat,
}

impl World {
    /// Жидкостные сети.
    pub fn fluid_nets(&self) -> &[FluidNet] {
        &self.fluids.nets
    }

    /// Тепловые сети.
    pub fn heat_nets(&self) -> &[HeatNet] {
        &self.heat.nets
    }

    /// Учёт жидкостей.
    pub fn fluid_stats(&self) -> &FluidStats {
        &self.fluid_stats
    }

    /// Жидкость во всех сетях по ID.
    pub fn fluid_in_nets(&self, reg: &ContentRegistry) -> Vec<u64> {
        let mut v = vec![0; reg.fluids().len()];
        for n in &self.fluids.nets {
            if let Some(f) = n.fluid
                && let Some(x) = v.get_mut(usize::from(f.0))
            {
                *x += n.amount;
            }
        }
        v
    }

    fn capacity_of(
        reg: &ContentRegistry,
        kind: NetKind,
        block: ferrustry_content::BlockId,
    ) -> Option<u32> {
        let d = reg.block(block);
        match kind {
            NetKind::Fluid => d.fluid_capacity(),
            NetKind::Heat => d.heat_capacity(),
        }
    }

    /// Перестраивает сети вида `kind`, сохраняя содержимое пропорционально ёмкости.
    fn rebuild_nets(&mut self, reg: &ContentRegistry, kind: NetKind) {
        let members: Vec<(BuildingId, u64)> = self
            .buildings
            .iter()
            .filter_map(|(id, b)| Self::capacity_of(reg, kind, b.block).map(|c| (id, u64::from(c))))
            .collect();

        // 1. доли старого содержимого у уцелевших участников
        let old_count = match kind {
            NetKind::Fluid => self.fluids.nets.len(),
            NetKind::Heat => self.heat.nets.len(),
        };
        let mut left: Vec<u64> = (0..old_count)
            .map(|n| match kind {
                NetKind::Fluid => self.fluids.nets[n].amount,
                NetKind::Heat => self.heat.nets[n].stored,
            })
            .collect();
        let mut shares: Vec<(u64, Option<FluidId>)> = Vec::with_capacity(members.len());
        let mut first_of: Vec<Option<usize>> = vec![None; old_count];
        for (i, &(id, cap)) in members.iter().enumerate() {
            let old = self.buildings.get(id).and_then(|b| match kind {
                NetKind::Fluid => b.fluid_net,
                NetKind::Heat => b.heat_net,
            });
            let share = match old.map(|n| n as usize).filter(|&n| n < old_count) {
                Some(n) => {
                    first_of[n].get_or_insert(i);
                    let (amount, total, fluid) = match kind {
                        NetKind::Fluid => {
                            let o = &self.fluids.nets[n];
                            (o.amount, o.capacity, o.fluid)
                        }
                        NetKind::Heat => {
                            let o = &self.heat.nets[n];
                            (o.stored, o.capacity, None)
                        }
                    };
                    let s = (amount * cap).checked_div(total).unwrap_or(0).min(left[n]);
                    left[n] -= s;
                    (s, fluid)
                }
                None => (0, None),
            };
            shares.push(share);
        }
        // остаток от деления — первому уцелевшему участнику; если таких нет — потеря
        for (n, rest) in left.iter().enumerate() {
            if *rest == 0 {
                continue;
            }
            match first_of[n] {
                Some(i) => shares[i].0 += rest,
                None => {
                    if kind == NetKind::Fluid
                        && let Some(f) = self.fluids.nets[n].fluid
                    {
                        FluidStats::bump(&mut self.fluid_stats.destroyed, f, *rest);
                    }
                }
            }
        }

        // 2. связность по касанию сторонами
        let mut local = vec![usize::MAX; self.buildings.slot_count()];
        for (i, (id, _)) in members.iter().enumerate() {
            local[Arena::slot_index(*id)] = i;
        }
        let mut parent: Vec<usize> = (0..members.len()).collect();
        for (i, (id, _)) in members.iter().enumerate() {
            let Some(b) = self.buildings.get(*id) else {
                continue;
            };
            for p in 0..b.perimeter_len() {
                let (outer, _) = b.perimeter(p);
                if let Some(o) = self.grid.building(outer) {
                    let j = local[Arena::slot_index(o)];
                    if j != usize::MAX {
                        let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                        if ra != rb {
                            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                            parent[hi] = lo;
                        }
                    }
                }
            }
        }

        // 3. новые сети
        let mut net_of_root = vec![u32::MAX; members.len()];
        let mut fluid_nets: Vec<FluidNet> = Vec::new();
        let mut heat_nets: Vec<HeatNet> = Vec::new();
        for (i, &(id, cap)) in members.iter().enumerate() {
            let r = find(&mut parent, i);
            if net_of_root[r] == u32::MAX {
                net_of_root[r] = match kind {
                    NetKind::Fluid => fluid_nets.len(),
                    NetKind::Heat => heat_nets.len(),
                } as u32;
                match kind {
                    NetKind::Fluid => fluid_nets.push(FluidNet {
                        fluid: None,
                        amount: 0,
                        capacity: 0,
                    }),
                    NetKind::Heat => heat_nets.push(HeatNet {
                        stored: 0,
                        capacity: 0,
                        temperature: 0,
                        generated: 0,
                        removed: 0,
                    }),
                }
            }
            let n = net_of_root[r];
            let (share, fluid) = shares[i];
            match kind {
                NetKind::Fluid => {
                    let net = &mut fluid_nets[n as usize];
                    net.capacity += cap;
                    match (net.fluid, fluid) {
                        (_, None) => {}
                        (None, Some(f)) => {
                            net.fluid = Some(f);
                            net.amount += share;
                        }
                        (Some(a), Some(f)) if a == f => net.amount += share,
                        (Some(_), Some(f)) => {
                            FluidStats::bump(&mut self.fluid_stats.destroyed, f, share)
                        }
                    }
                }
                NetKind::Heat => {
                    let net = &mut heat_nets[n as usize];
                    net.capacity += cap;
                    net.stored += share;
                }
            }
            if let Some(b) = self.buildings.get_mut(id) {
                match kind {
                    NetKind::Fluid => b.fluid_net = Some(n),
                    NetKind::Heat => b.heat_net = Some(n),
                }
            }
        }
        match kind {
            NetKind::Fluid => {
                // объём мог уменьшиться (снос): лишнее теряется
                for net in &mut fluid_nets {
                    if net.amount > net.capacity
                        && let Some(f) = net.fluid
                    {
                        FluidStats::bump(
                            &mut self.fluid_stats.destroyed,
                            f,
                            net.amount - net.capacity,
                        );
                        net.amount = net.capacity;
                    }
                    if net.amount == 0 {
                        net.fluid = None;
                    }
                }
                self.fluids = Nets {
                    nets: fluid_nets,
                    dirty: false,
                };
            }
            NetKind::Heat => {
                for net in &mut heat_nets {
                    net.stored = net.stored.min(net.capacity);
                    net.temperature =
                        u32::try_from((net.stored * 1000).checked_div(net.capacity).unwrap_or(0))
                            .unwrap_or(1000);
                }
                self.heat = Nets {
                    nets: heat_nets,
                    dirty: false,
                };
            }
        }
    }

    pub(crate) fn rebuild_thermal(&mut self, reg: &ContentRegistry) {
        if self.fluids.dirty {
            self.rebuild_nets(reg, NetKind::Fluid);
        }
        if self.heat.dirty {
            self.rebuild_nets(reg, NetKind::Heat);
        }
    }

    /// Может ли конденсатор работать (в сети есть место и жидкость совместима).
    pub(crate) fn condenser_wants(&self, reg: &ContentRegistry, id: BuildingId) -> bool {
        let Some(b) = self.buildings.get(id) else {
            return false;
        };
        let BlockKind::Condenser { fluid, .. } = reg.block(b.block).kind else {
            return false;
        };
        b.fluid_net
            .and_then(|n| self.fluids.nets.get(n as usize))
            .is_some_and(|net| net.amount < net.capacity && net.fluid.is_none_or(|f| f == fluid))
    }

    /// Конденсаторы: выработка жидкости пропорционально удовлетворению энергией.
    pub(crate) fn update_condensers(&mut self, reg: &ContentRegistry) {
        for r in 0..self.condensers.len() {
            let owner = self.condensers.owners[r];
            if !self.condenser_wants(reg, owner) {
                continue;
            }
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let BlockKind::Condenser {
                fluid,
                rate,
                power_use,
                ..
            } = reg.block(b.block).kind
            else {
                continue;
            };
            let sat = if power_use == 0 {
                1000
            } else {
                b.power_net
                    .and_then(|n| self.power.nets.get(n as usize))
                    .map_or(0, |n| n.satisfaction)
            };
            let Some(n) = b.fluid_net else { continue };
            let row = &mut self.condensers.rows[r];
            row.acc += rate * sat;
            let units = row.acc / 1000;
            row.acc %= 1000;
            let Some(net) = self.fluids.nets.get_mut(n as usize) else {
                continue;
            };
            let put = u64::from(units).min(net.capacity - net.amount);
            if put > 0 {
                net.fluid = Some(fluid);
                net.amount += put;
                FluidStats::bump(&mut self.fluid_stats.produced, fluid, put);
            }
        }
    }

    /// Тепло: тепловые машины забирают тепло (энергия на следующий тик), затем пассивное
    /// охлаждение, радиаторы и градирни; температура пересчитывается.
    pub(crate) fn update_heat(&mut self, reg: &ContentRegistry) {
        if self.heat.nets.is_empty() {
            return; // нет тепловых построек — система ничего не стоит
        }
        for n in &mut self.heat.nets {
            n.removed = 0;
        }
        // тепловые машины
        for r in 0..self.engines.len() {
            let owner = self.engines.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let BlockKind::HeatEngine {
                max_heat,
                power_per_heat_permille,
                ..
            } = reg.block(b.block).kind
            else {
                continue;
            };
            let taken = match b.heat_net.and_then(|n| self.heat.nets.get_mut(n as usize)) {
                Some(net) => {
                    let t = u64::from(max_heat).min(net.stored);
                    net.stored -= t;
                    net.removed += t;
                    t
                }
                None => 0,
            };
            let row = &mut self.engines.rows[r];
            let total = taken * u64::from(power_per_heat_permille) + u64::from(row.frac);
            row.output = u32::try_from(total / 1000).unwrap_or(u32::MAX);
            row.frac = u32::try_from(total % 1000).unwrap_or(0);
        }
        // пассивное охлаждение производств и радиаторы — только участники тепловых сетей
        for r in 0..self.crafters.len() {
            let owner = self.crafters.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let BlockKind::Crafter { heat, cooling, .. } = reg.block(b.block).kind else {
                continue;
            };
            if heat == 0 {
                continue;
            }
            if let Some(net) = b.heat_net.and_then(|n| self.heat.nets.get_mut(n as usize)) {
                let t = u64::from(cooling).min(net.stored);
                net.stored -= t;
                net.removed += t;
            }
        }
        for r in 0..self.passive.len() {
            let owner = self.passive.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let BlockKind::Radiator { dissipation, .. } = reg.block(b.block).kind else {
                continue;
            };
            if let Some(net) = b.heat_net.and_then(|n| self.heat.nets.get_mut(n as usize)) {
                let t = u64::from(dissipation).min(net.stored);
                net.stored -= t;
                net.removed += t;
            }
        }
        // градирни: вода → снятие тепла
        for r in 0..self.passive.len() {
            let owner = self.passive.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let BlockKind::CoolingTower {
                fluid,
                rate,
                cooling,
                ..
            } = reg.block(b.block).kind
            else {
                continue;
            };
            let (Some(hn), Some(fnet)) = (b.heat_net, b.fluid_net) else {
                continue;
            };
            let Some(heat) = self.heat.nets.get(hn as usize) else {
                continue;
            };
            if heat.stored == 0 || cooling == 0 {
                continue;
            }
            let need = heat.stored.div_ceil(u64::from(cooling));
            let Some(water) = self.fluids.nets.get_mut(fnet as usize) else {
                continue;
            };
            if water.fluid != Some(fluid) {
                continue;
            }
            let used = u64::from(rate).min(water.amount).min(need);
            if used == 0 {
                continue;
            }
            water.amount -= used;
            if water.amount == 0 {
                water.fluid = None;
            }
            FluidStats::bump(&mut self.fluid_stats.consumed, fluid, used);
            if let Some(net) = self.heat.nets.get_mut(hn as usize) {
                let t = (used * u64::from(cooling)).min(net.stored);
                net.stored -= t;
                net.removed += t;
            }
        }
        for n in &mut self.heat.nets {
            n.stored = n.stored.min(n.capacity);
            n.temperature = u32::try_from((n.stored * 1000).checked_div(n.capacity).unwrap_or(0))
                .unwrap_or(1000);
        }
    }

    /// Добавить тепло в сеть постройки (вызывается производствами); возвращает `generated`.
    pub(crate) fn add_heat(&mut self, net: Option<u32>, amount: u64) {
        if amount == 0 {
            return;
        }
        if let Some(n) = net.and_then(|n| self.heat.nets.get_mut(n as usize)) {
            n.stored = (n.stored + amount).min(n.capacity);
            n.generated += amount;
        }
    }

    /// Температура тепловой сети постройки, тысячные (`None` — не участвует в тепловой сети).
    pub fn building_temperature(&self, b: &crate::buildings::Building) -> Option<u32> {
        b.heat_net
            .and_then(|n| self.heat.nets.get(n as usize))
            .map(|n| n.temperature)
    }

    /// Температура сети постройки (0, если не участвует).
    pub(crate) fn temperature_of(&self, net: Option<u32>) -> u32 {
        net.and_then(|n| self.heat.nets.get(n as usize))
            .map_or(0, |n| n.temperature)
    }

    /// Сбросить счётчик выделенного тепла перед тиком производств.
    pub(crate) fn begin_heat_tick(&mut self) {
        for n in &mut self.heat.nets {
            n.generated = 0;
        }
    }

    /// Пометить сети к перестройке, если постройка в них участвует.
    pub(crate) fn mark_thermal_dirty(
        &mut self,
        reg: &ContentRegistry,
        block: ferrustry_content::BlockId,
    ) {
        let d = reg.block(block);
        if d.fluid_capacity().is_some() {
            self.fluids.dirty = true;
        }
        if d.heat_capacity().is_some() {
            self.heat.dirty = true;
        }
    }
}
