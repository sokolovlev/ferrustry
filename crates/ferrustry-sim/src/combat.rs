//! Бой: юниты (враги волн), поле путей, пространственная сетка, турели, снаряды, волны.
//!
//! Вся математика — целочисленная: позиции в 1/256 тайла ([`SUB`]), расстояния через
//! целочисленный квадратный корень. Поэтому бой детерминирован на любых платформах и
//! воспроизводится replay'ем бит в бит.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use ferrustry_content::{BlockKind, ContentRegistry, UnitTypeId};
use serde::{Deserialize, Serialize};

use crate::buildings::{BuildingId, KindTag};
use crate::geom::{Dir, TilePos};
use crate::world::World;

/// Единиц позиции на тайл.
pub const SUB: i32 = 256;
/// Сторона ячейки пространственной сетки в тайлах.
const CELL_TILES: i32 = 2;
const NONE: u32 = u32::MAX;

/// Юниты (SoA).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Units {
    pub(crate) kind: Vec<UnitTypeId>,
    pub(crate) x: Vec<i32>,
    pub(crate) y: Vec<i32>,
    pub(crate) health: Vec<u32>,
    pub(crate) cooldown: Vec<u32>,
    /// Смещение за последний тик (для упреждения при стрельбе).
    pub(crate) vx: Vec<i32>,
    pub(crate) vy: Vec<i32>,
    /// Дальнобойный юнит стоит и стреляет (до конца перезарядки).
    pub(crate) hold: Vec<bool>,
}

impl Units {
    /// Число юнитов.
    pub fn len(&self) -> usize {
        self.kind.len()
    }

    /// Пусто ли.
    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    fn swap_remove(&mut self, i: usize) {
        self.kind.swap_remove(i);
        self.x.swap_remove(i);
        self.y.swap_remove(i);
        self.health.swap_remove(i);
        self.cooldown.swap_remove(i);
        self.vx.swap_remove(i);
        self.vy.swap_remove(i);
        self.hold.swap_remove(i);
    }
}

/// Снаряды (SoA).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projectiles {
    pub(crate) x: Vec<i32>,
    pub(crate) y: Vec<i32>,
    pub(crate) vx: Vec<i32>,
    pub(crate) vy: Vec<i32>,
    pub(crate) damage: Vec<u32>,
    pub(crate) ttl: Vec<u32>,
    /// Снаряд врага: бьёт постройки, а не юнитов.
    pub(crate) hostile: Vec<bool>,
}

impl Projectiles {
    fn swap_remove(&mut self, i: usize) {
        self.x.swap_remove(i);
        self.y.swap_remove(i);
        self.vx.swap_remove(i);
        self.vy.swap_remove(i);
        self.damage.swap_remove(i);
        self.ttl.swap_remove(i);
        self.hostile.swap_remove(i);
    }

    fn push(&mut self, x: i32, y: i32, v: (i32, i32), damage: u32, ttl: u32, hostile: bool) {
        self.x.push(x);
        self.y.push(y);
        self.vx.push(v.0);
        self.vy.push(v.1);
        self.damage.push(damage);
        self.ttl.push(ttl);
        self.hostile.push(hostile);
    }
}

/// Поле путей к ядрам: «стоимость дойти» для каждого тайла.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowField {
    pub(crate) epoch: u64,
    pub(crate) dist: Vec<u32>,
}

/// Состояние волн.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waves {
    /// Номер следующей волны (с 1).
    pub next_number: u32,
    /// Тик следующей волны.
    pub next_tick: u64,
}

/// Исход партии.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Партия идёт.
    #[default]
    Running,
    /// Последнее ядро разрушено.
    Lost {
        /// Тик поражения.
        tick: u64,
    },
}

/// Состояние турели.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TurretRow {
    /// Выстрелов в запасе.
    pub(crate) ammo: u32,
    pub(crate) cooldown: u32,
    /// Направление последнего выстрела (для отрисовки ствола).
    pub(crate) aim: (i32, i32),
    /// Дуговая турель: накопленная энергия перезарядки, тысячные тика.
    pub(crate) charge: u32,
    /// Дуговая турель: тиков, пока виден разряд, и его конец (1/256 тайла).
    pub(crate) flash: u32,
    pub(crate) beam: (i32, i32),
}

/// Служебный буфер: не сохраняется и не участвует в сравнении миров (пересобирается каждый тик).
#[derive(Debug, Clone, Default)]
pub(crate) struct Scratch<T>(pub(crate) T);

impl<T> PartialEq for Scratch<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Eq for Scratch<T> {}

/// Пространственная сетка юнитов: списки по ячейкам (голова + следующий), без аллокаций в тике.
#[derive(Debug, Clone, Default)]
pub(crate) struct UnitGrid {
    cells_x: i32,
    cells_y: i32,
    head: Vec<u32>,
    next: Vec<u32>,
}

fn isqrt(v: i64) -> i64 {
    if v <= 0 { 0 } else { v.isqrt() }
}

impl UnitGrid {
    fn rebuild(&mut self, w: i32, h: i32, units: &Units) {
        let cell = CELL_TILES * SUB;
        self.cells_x = (w * SUB + cell - 1) / cell;
        self.cells_y = (h * SUB + cell - 1) / cell;
        let n = (self.cells_x * self.cells_y).max(1) as usize;
        self.head.clear();
        self.head.resize(n, NONE);
        self.next.clear();
        self.next.resize(units.len(), NONE);
        // вставка в обратном порядке → обход списка ячейки идёт по возрастанию индекса
        for i in (0..units.len()).rev() {
            let c = self.cell_of(units.x[i], units.y[i]);
            self.next[i] = self.head[c];
            self.head[c] = i as u32;
        }
    }

    fn cell_of(&self, x: i32, y: i32) -> usize {
        let cell = CELL_TILES * SUB;
        let cx = (x / cell).clamp(0, self.cells_x - 1);
        let cy = (y / cell).clamp(0, self.cells_y - 1);
        (cy * self.cells_x + cx) as usize
    }

    /// Обходит юнитов в ячейках, пересекающих квадрат с центром (x, y) и полустороной r.
    fn for_each_near(&self, x: i32, y: i32, r: i32, mut f: impl FnMut(usize) -> bool) {
        let cell = CELL_TILES * SUB;
        let (x0, x1) = (
            ((x - r) / cell).max(0),
            ((x + r) / cell).min(self.cells_x - 1),
        );
        let (y0, y1) = (
            ((y - r) / cell).max(0),
            ((y + r) / cell).min(self.cells_y - 1),
        );
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let mut i = self.head[(cy * self.cells_x + cx) as usize];
                while i != NONE {
                    if !f(i as usize) {
                        return;
                    }
                    i = self.next[i as usize];
                }
            }
        }
    }
}

impl World {
    /// Юниты: (тип, x, y, прочность) в 1/256 тайла.
    pub fn units(&self) -> impl Iterator<Item = (UnitTypeId, i32, i32, u32)> + '_ {
        (0..self.units.len()).map(|i| {
            (
                self.units.kind[i],
                self.units.x[i],
                self.units.y[i],
                self.units.health[i],
            )
        })
    }

    /// Снаряды: (x, y) в 1/256 тайла и признак вражеского снаряда.
    pub fn projectiles(&self) -> impl Iterator<Item = (i32, i32, bool)> + '_ {
        let p = &self.projectiles;
        (0..p.x.len()).map(|i| (p.x[i], p.y[i], p.hostile[i]))
    }

    /// Видимый разряд дуговой турели: конец разряда в 1/256 тайла.
    pub fn arc_beam(&self, b: &crate::buildings::Building) -> Option<(i32, i32)> {
        if b.tag != KindTag::Turret {
            return None;
        }
        let r = self.turrets.rows.get(b.row as usize)?;
        (r.flash > 0).then_some(r.beam)
    }

    /// Исход партии.
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Номер следующей волны и тиков до неё (`None`, если волны выключены).
    pub fn wave_info(&self) -> Option<(u32, u64)> {
        self.rules().waves.then(|| {
            (
                self.waves.next_number,
                self.waves.next_tick.saturating_sub(self.tick_count()),
            )
        })
    }

    /// Убито врагов.
    pub fn kills(&self) -> u64 {
        self.kills
    }

    /// Точки появления врагов.
    pub fn spawns(&self) -> &[TilePos] {
        &self.spawns
    }

    /// Задать точки появления врагов (генерация карты).
    pub fn set_spawns(&mut self, spawns: Vec<TilePos>) {
        self.spawns = spawns;
    }

    /// Направление ствола турели (для отрисовки).
    pub fn turret_aim(&self, b: &crate::buildings::Building) -> Option<(i32, i32)> {
        (b.tag == KindTag::Turret).then(|| self.turrets.rows.get(b.row as usize).map(|r| r.aim))?
    }

    /// Создать юнита в точке (в 1/256 тайла). Для волн, тестов и бенчмарков.
    pub fn spawn_unit(&mut self, reg: &ContentRegistry, kind: UnitTypeId, x: i32, y: i32) {
        self.units.kind.push(kind);
        self.units.x.push(x);
        self.units.y.push(y);
        self.units.health.push(reg.unit(kind).health);
        self.units.cooldown.push(0);
        self.units.vx.push(0);
        self.units.vy.push(0);
        self.units.hold.push(false);
    }

    /// Состав волны `n` (с 1): (тип, количество).
    pub fn wave_composition(reg: &ContentRegistry, n: u32) -> Vec<(UnitTypeId, u32)> {
        let mut v = Vec::new();
        if let Some(c) = reg.unit_by_name("crawler") {
            v.push((c, 3 + 2 * n));
        }
        if let Some(b) = reg.unit_by_name("brute")
            && n / 2 > 0
        {
            v.push((b, n / 2));
        }
        // с 3-й волны — дальнобойные, с 6-й — бронированные
        if let Some(s) = reg.unit_by_name("spitter")
            && n >= 3
        {
            v.push((s, (n - 1) / 2));
        }
        if let Some(s) = reg.unit_by_name("shellback")
            && n >= 6
        {
            v.push((s, (n - 4) / 2));
        }
        v
    }

    fn update_waves(&mut self, reg: &ContentRegistry) {
        if !self.rules().waves || self.spawns.is_empty() || self.tick_count() < self.waves.next_tick
        {
            return;
        }
        let n = self.waves.next_number.max(1);
        let mut k = 0usize;
        for (kind, count) in Self::wave_composition(reg, n) {
            for _ in 0..count {
                let sp = self.spawns[k % self.spawns.len()];
                k += 1;
                let jx = self.rng_mut().below(200) as i32 - 100;
                let jy = self.rng_mut().below(200) as i32 - 100;
                self.spawn_unit(
                    reg,
                    kind,
                    sp.x * SUB + SUB / 2 + jx,
                    sp.y * SUB + SUB / 2 + jy,
                );
            }
        }
        self.waves.next_number = n + 1;
        self.waves.next_tick = self.tick_count() + self.rules().wave_spacing.max(1);
    }

    /// Поле путей: Дейкстра от всех тайлов ядер; вход в пустой тайл стоит 1, в тайл постройки —
    /// 1 + прочность/8 (стена 400 ≈ 51 тайл обхода) (враги обходят постройки, но проламывают, если обход дороже).
    fn rebuild_flow(&mut self, reg: &ContentRegistry) {
        let (w, h) = (self.grid.width() as i32, self.grid.height() as i32);
        let n = (w * h) as usize;
        let mut dist = std::mem::take(&mut self.flow.dist);
        dist.clear();
        dist.resize(n, u32::MAX);
        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        for (_, b) in self.buildings.iter() {
            if b.tag != KindTag::Core {
                continue;
            }
            for dy in 0..i32::from(b.size) {
                for dx in 0..i32::from(b.size) {
                    let i = ((b.origin.y + dy) * w + b.origin.x + dx) as usize;
                    dist[i] = 0;
                    heap.push(Reverse((0, i as u32)));
                }
            }
        }
        while let Some(Reverse((d, i))) = heap.pop() {
            if d > dist[i as usize] {
                continue;
            }
            let p = TilePos::new(i as i32 % w, i as i32 / w);
            for dir in Dir::ALL {
                let q = p.step(dir);
                if !self.grid.in_bounds(q) {
                    continue;
                }
                let Some(cost) = self.enter_cost(reg, q) else {
                    continue;
                };
                let j = (q.y * w + q.x) as usize;
                let nd = d.saturating_add(cost);
                if nd < dist[j] {
                    dist[j] = nd;
                    heap.push(Reverse((nd, j as u32)));
                }
            }
        }
        self.flow.dist = dist;
        self.flow.epoch = self.topo_epoch();
    }

    /// Стоимость входа в тайл для наземного юнита (`None` — непроходим).
    fn enter_cost(&self, reg: &ContentRegistry, p: TilePos) -> Option<u32> {
        let floor = self.grid.floor(p)?;
        if !reg.floor(floor).buildable {
            return None;
        }
        match self.grid.building(p).and_then(|id| self.buildings.get(id)) {
            Some(b) if b.tag == KindTag::Core => Some(1),
            Some(b) => Some(1 + b.health / 8),
            None => Some(1),
        }
    }

    fn flow_at(&self, p: TilePos) -> u32 {
        let w = self.grid.width() as i32;
        if !self.grid.in_bounds(p) {
            return u32::MAX;
        }
        self.flow
            .dist
            .get((p.y * w + p.x) as usize)
            .copied()
            .unwrap_or(u32::MAX)
    }

    /// Нанести урон постройке; при нуле — разрушить. Возвращает `true`, если разрушена.
    pub(crate) fn damage_building(
        &mut self,
        reg: &ContentRegistry,
        id: BuildingId,
        dmg: u32,
    ) -> bool {
        let Some(b) = self.buildings.get_mut(id) else {
            return false;
        };
        b.health = b.health.saturating_sub(dmg);
        if b.health > 0 {
            return false;
        }
        let at = b.origin;
        self.destroy_at(reg, at);
        if self.cores.len() == 0 && self.rules().waves && self.outcome == Outcome::Running {
            self.outcome = Outcome::Lost {
                tick: self.tick_count(),
            };
        }
        true
    }

    fn update_units(&mut self, reg: &ContentRegistry) {
        if self.units.is_empty() {
            return;
        }
        if self.flow.epoch != self.topo_epoch() || self.flow.dist.is_empty() {
            self.rebuild_flow(reg);
        }
        for i in 0..self.units.len() {
            if self.units.health[i] == 0 {
                continue;
            }
            let def = reg.unit(self.units.kind[i]);
            let (speed, damage, cd) = (def.speed as i32, def.damage, def.cooldown);
            self.units.cooldown[i] = self.units.cooldown[i].saturating_sub(1);
            let (x, y) = (self.units.x[i], self.units.y[i]);
            self.units.vx[i] = 0;
            self.units.vy[i] = 0;
            if def.range > 0 {
                if self.units.cooldown[i] > 0 && self.units.hold[i] {
                    continue; // перезаряжается на позиции
                }
                if self.units.cooldown[i] == 0 {
                    let target = self.nearest_building(x, y, def.range as i32);
                    self.units.hold[i] = target.is_some();
                    if let Some((tx, ty)) = target {
                        let sp = i64::from(def.projectile_speed);
                        let (dx, dy) = (i64::from(tx - x), i64::from(ty - y));
                        let d = isqrt(dx * dx + dy * dy).max(1);
                        let v = ((dx * sp / d) as i32, (dy * sp / d) as i32);
                        let ttl = (d / sp.max(1)) as u32 + 2;
                        self.projectiles.push(x, y, v, damage, ttl, true);
                        self.units.cooldown[i] = cd;
                        continue;
                    }
                }
            }
            let here = TilePos::new(x.div_euclid(SUB), y.div_euclid(SUB));
            let cur = self.flow_at(here);
            if cur == u32::MAX {
                continue; // путь не найден
            }
            if cur == 0 {
                // стоит на тайле ядра (ядро поставили поверх) — бить его
                if let Some(id) = self.grid.building(here)
                    && self.units.cooldown[i] == 0
                {
                    self.units.cooldown[i] = cd;
                    self.damage_building(reg, id, damage);
                }
                continue;
            }
            // лучший сосед (строго меньшая стоимость; при равенстве — фиксированный порядок)
            let mut best: Option<(u32, TilePos)> = None;
            for dir in Dir::ALL {
                let q = here.step(dir);
                let d = self.flow_at(q);
                if d < cur && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, q));
                }
            }
            let Some((_, next)) = best else { continue };
            if let Some(id) = self.grid.building(next) {
                // путь лежит через постройку — ломать её
                if self.units.cooldown[i] == 0 {
                    self.units.cooldown[i] = cd;
                    self.damage_building(reg, id, damage);
                }
                continue;
            }
            let (tx, ty) = (next.x * SUB + SUB / 2, next.y * SUB + SUB / 2);
            let (dx, dy) = (i64::from(tx - x), i64::from(ty - y));
            let d = isqrt(dx * dx + dy * dy);
            if d == 0 {
                continue;
            }
            let step = d.min(i64::from(speed));
            let (mx, my) = ((dx * step / d) as i32, (dy * step / d) as i32);
            self.units.x[i] = x + mx;
            self.units.y[i] = y + my;
            self.units.vx[i] = mx;
            self.units.vy[i] = my;
        }
    }

    /// Центр ближайшей постройки (1/256 тайла), чей тайл в радиусе `range` тайлов от точки.
    /// Обход квадрата по строкам, при равенстве расстояний — первый найденный (детерминированно).
    fn nearest_building(&self, x: i32, y: i32, range: i32) -> Option<(i32, i32)> {
        let (tx, ty) = (x.div_euclid(SUB), y.div_euclid(SUB));
        let r2 = i64::from(range * SUB).pow(2);
        let mut best: Option<(i64, i32, i32)> = None;
        for py in ty - range..=ty + range {
            for px in tx - range..=tx + range {
                let p = TilePos::new(px, py);
                if !self.grid.in_bounds(p) || self.grid.building(p).is_none() {
                    continue;
                }
                let (cx, cy) = (px * SUB + SUB / 2, py * SUB + SUB / 2);
                let (dx, dy) = (i64::from(cx - x), i64::from(cy - y));
                let d2 = dx * dx + dy * dy;
                if d2 <= r2 && best.is_none_or(|(bd, ..)| d2 < bd) {
                    best = Some((d2, cx, cy));
                }
            }
        }
        best.map(|(_, cx, cy)| (cx, cy))
    }

    /// Расталкивание перекрывающихся юнитов (не заходя на тайлы построек).
    fn separate_units(&mut self, reg: &ContentRegistry) {
        let n = self.units.len();
        for i in 0..n {
            let ri = reg.unit(self.units.kind[i]).radius as i32;
            let (xi, yi) = (self.units.x[i], self.units.y[i]);
            let (mut px, mut py) = (0i64, 0i64);
            let grid = &self.unit_grid.0;
            let units = &self.units;
            grid.for_each_near(xi, yi, 2 * ri, |j| {
                if j != i {
                    let rj = reg.unit(units.kind[j]).radius as i32;
                    let (dx, dy) = (i64::from(xi - units.x[j]), i64::from(yi - units.y[j]));
                    let min = i64::from(ri + rj);
                    let d2 = dx * dx + dy * dy;
                    if d2 < min * min {
                        let d = isqrt(d2);
                        let push = (min - d) / 4;
                        if d == 0 {
                            // ровно в одной точке: развести по индексу
                            px += if i < j { push } else { -push };
                        } else {
                            px += dx * push / d;
                            py += dy * push / d;
                        }
                    }
                }
                true
            });
            if px == 0 && py == 0 {
                continue;
            }
            let (nx, ny) = (xi + px as i32, yi + py as i32);
            let t = TilePos::new(nx.div_euclid(SUB), ny.div_euclid(SUB));
            if self.grid.in_bounds(t)
                && self.grid.building(t).is_none()
                && self.enter_cost(reg, t).is_some()
            {
                self.units.x[i] = nx;
                self.units.y[i] = ny;
            }
        }
    }

    fn rebuild_unit_grid(&mut self) {
        let (w, h) = (self.grid.width() as i32, self.grid.height() as i32);
        let mut g = std::mem::take(&mut self.unit_grid.0);
        g.rebuild(w, h, &self.units);
        self.unit_grid.0 = g;
    }

    /// Перезарядка турелей — каждый тик, даже без врагов. Пушки: тик за тиком; дуговые —
    /// пропорционально питанию сети (500‰ питания — вдвое дольше).
    fn reload_turrets(&mut self, reg: &ContentRegistry) {
        for r in 0..self.turrets.len() {
            let row = &mut self.turrets.rows[r];
            row.flash = row.flash.saturating_sub(1);
            if row.cooldown == 0 {
                continue;
            }
            let Some(b) = self.buildings.get(self.turrets.owners[r]) else {
                continue;
            };
            if !matches!(reg.block(b.block).kind, BlockKind::ArcTurret { .. }) {
                self.turrets.rows[r].cooldown -= 1;
                continue;
            }
            let sat = b
                .power_net
                .and_then(|n| self.power.nets.get(n as usize))
                .map_or(0, |n| n.satisfaction);
            let row = &mut self.turrets.rows[r];
            row.charge += sat;
            while row.charge >= 1000 && row.cooldown > 0 {
                row.charge -= 1000;
                row.cooldown -= 1;
            }
            if row.cooldown == 0 {
                row.charge = 0;
            }
        }
    }

    /// Ближайший живой юнит в радиусе `reach` от точки: (квадрат расстояния, индекс).
    fn nearest_unit(&self, cx: i32, cy: i32, reach: i32) -> Option<(i64, usize)> {
        let mut best: Option<(i64, usize)> = None;
        let units = &self.units;
        self.unit_grid.0.for_each_near(cx, cy, reach, |j| {
            if units.health[j] > 0 {
                let (dx, dy) = (i64::from(units.x[j] - cx), i64::from(units.y[j] - cy));
                let d2 = dx * dx + dy * dy;
                if d2 <= i64::from(reach) * i64::from(reach) && best.is_none_or(|(bd, _)| d2 < bd) {
                    best = Some((d2, j));
                }
            }
            true
        });
        best
    }

    fn update_turrets(&mut self, reg: &ContentRegistry) {
        if self.units.is_empty() {
            return;
        }
        for r in 0..self.turrets.len() {
            let row = &self.turrets.rows[r];
            if row.cooldown > 0 {
                continue;
            }
            let has_ammo = row.ammo > 0;
            let owner = self.turrets.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let half = i32::from(b.size) * SUB / 2;
            let (cx, cy) = (b.origin.x * SUB + half, b.origin.y * SUB + half);
            let (range, damage, reload, projectile_speed) = match reg.block(b.block).kind {
                BlockKind::Turret {
                    range,
                    damage,
                    reload,
                    projectile_speed,
                    ..
                } if has_ammo => (range, damage, reload, projectile_speed),
                BlockKind::ArcTurret {
                    range,
                    damage,
                    reload,
                    ..
                } => {
                    // разряд: мгновенно, сквозь броню; без питания перезарядка не идёт
                    let Some((_, j)) = self.nearest_unit(cx, cy, range as i32 * SUB) else {
                        continue;
                    };
                    let (ux, uy) = (self.units.x[j], self.units.y[j]);
                    self.units.health[j] = self.units.health[j].saturating_sub(damage);
                    let row = &mut self.turrets.rows[r];
                    row.cooldown = reload;
                    row.charge = 0;
                    row.flash = 4;
                    row.beam = (ux, uy);
                    row.aim = (ux - cx, uy - cy);
                    continue;
                }
                _ => continue,
            };
            let Some((d2, j)) = self.nearest_unit(cx, cy, range as i32 * SUB) else {
                continue;
            };
            let sp = i64::from(projectile_speed);
            // упреждение: куда цель сместится за время полёта снаряда (одна итерация)
            let flight = isqrt(d2) / sp.max(1);
            let tx = i64::from(self.units.x[j]) + i64::from(self.units.vx[j]) * flight;
            let ty = i64::from(self.units.y[j]) + i64::from(self.units.vy[j]) * flight;
            let (dx, dy) = (tx - i64::from(cx), ty - i64::from(cy));
            let d = isqrt(dx * dx + dy * dy).max(1);
            let ttl = (u64::from(range) * SUB as u64 / u64::from(projectile_speed)) as u32 + 4;
            self.projectiles.push(
                cx,
                cy,
                ((dx * sp / d) as i32, (dy * sp / d) as i32),
                damage,
                ttl,
                false,
            );
            let row = &mut self.turrets.rows[r];
            row.ammo -= 1;
            row.cooldown = reload;
            row.aim = (dx as i32, dy as i32);
        }
    }

    fn update_projectiles(&mut self, reg: &ContentRegistry) {
        let mut i = 0;
        while i < self.projectiles.x.len() {
            let p = &mut self.projectiles;
            p.x[i] += p.vx[i];
            p.y[i] += p.vy[i];
            p.ttl[i] -= 1;
            let (x, y, dmg) = (p.x[i], p.y[i], p.damage[i]);
            if p.hostile[i] {
                // снаряд врага: попадает в любую постройку на своём тайле
                let t = TilePos::new(x.div_euclid(SUB), y.div_euclid(SUB));
                let target = self
                    .grid
                    .in_bounds(t)
                    .then(|| self.grid.building(t))
                    .flatten();
                if let Some(id) = target {
                    self.damage_building(reg, id, dmg);
                }
                if target.is_some() || self.projectiles.ttl[i] == 0 {
                    self.projectiles.swap_remove(i);
                } else {
                    i += 1;
                }
                continue;
            }
            let mut hit = None;
            let units = &self.units;
            self.unit_grid.0.for_each_near(x, y, SUB, |j| {
                if units.health[j] > 0 {
                    let r = i64::from(reg.unit(units.kind[j]).radius) + 32;
                    let (dx, dy) = (i64::from(units.x[j] - x), i64::from(units.y[j] - y));
                    if dx * dx + dy * dy <= r * r {
                        hit = Some(j);
                        return false;
                    }
                }
                true
            });
            if let Some(j) = hit {
                // броня: вычитается из урона, но урон не ниже трети исходного
                let armor = reg.unit(self.units.kind[j]).armor;
                let eff = dmg.saturating_sub(armor).max(dmg / 3);
                self.units.health[j] = self.units.health[j].saturating_sub(eff);
            }
            if hit.is_some() || self.projectiles.ttl[i] == 0 {
                self.projectiles.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn remove_dead_units(&mut self) {
        let mut i = self.units.len();
        while i > 0 {
            i -= 1;
            if self.units.health[i] == 0 {
                self.units.swap_remove(i);
                self.kills += 1;
            }
        }
    }

    /// Боевой шаг тика.
    pub(crate) fn update_combat(&mut self, reg: &ContentRegistry) {
        self.update_waves(reg);
        self.reload_turrets(reg);
        if self.units.is_empty() && self.projectiles.x.is_empty() {
            return;
        }
        self.update_units(reg);
        self.rebuild_unit_grid();
        self.separate_units(reg);
        self.rebuild_unit_grid();
        self.update_turrets(reg);
        self.update_projectiles(reg);
        self.remove_dead_units();
    }
}
