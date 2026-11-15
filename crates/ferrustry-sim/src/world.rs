//! Состояние мира и игровой тик.

use std::collections::{BTreeMap, BTreeSet};

use ferrustry_content::{BlockId, BlockKind, ContentRegistry, FloorId, ItemId, OreId};
use serde::{Deserialize, Serialize};

use crate::buildings::{Arena, Building, BuildingId, KindTag, Table};
use crate::geom::{Dir, TilePos};
use crate::grid::Grid;
use crate::power::PowerState;
use crate::rng::Rng;
use crate::segment::{Segment, TILE_LEN};

/// Частота симуляции (решение D-03).
pub const TICKS_PER_SECOND: u32 = 30;

/// Команда игрока — единственный способ изменить мир извне.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Поставить постройку; `origin` — левый нижний тайл.
    Place {
        /// Тип постройки.
        block: BlockId,
        /// Левый нижний тайл.
        origin: TilePos,
        /// Поворот.
        rot: Dir,
    },
    /// Снести постройку, занимающую тайл.
    Remove {
        /// Любой тайл постройки.
        at: TilePos,
    },
}

/// Причина отклонения команды.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// Неизвестный тип постройки.
    #[error("unknown block")]
    UnknownBlock,
    /// Постройка выходит за пределы карты.
    #[error("out of bounds")]
    OutOfBounds,
    /// Тайл уже занят.
    #[error("tile occupied")]
    Occupied,
    /// На этом полу нельзя строить.
    #[error("floor is not buildable")]
    NotBuildable,
    /// На тайле нет постройки.
    #[error("nothing to remove")]
    NothingThere,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConveyorRow {
    pub(crate) segment: u32,
    pub(crate) index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DrillRow {
    pub(crate) item: Option<ItemId>,
    pub(crate) ore_tiles: u32,
    pub(crate) progress: u32,
    pub(crate) buffer: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CrafterRow {
    pub(crate) inputs: Vec<u32>,
    pub(crate) outputs: Vec<u32>,
    /// Прогресс в тысячных долях тика при полной энергии.
    pub(crate) progress: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CoreRow {
    pub(crate) items: Vec<u32>,
}

/// Счётчики ресурсов (для тестов, отладки и баланса).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    /// Добыто и произведено.
    pub produced: Vec<u64>,
    /// Израсходовано рецептами.
    pub consumed: Vec<u64>,
    /// Уничтожено (снос построек и т. п.).
    pub destroyed: Vec<u64>,
    /// Создано отладочными/бенчмарк-инструментами.
    pub spawned: Vec<u64>,
}

impl Stats {
    fn new(items: usize) -> Self {
        Self {
            produced: vec![0; items],
            consumed: vec![0; items],
            destroyed: vec![0; items],
            spawned: vec![0; items],
        }
    }

    fn bump(v: &mut [u64], item: ItemId, n: u64) {
        if let Some(c) = v.get_mut(usize::from(item.0)) {
            *c += n;
        }
    }
}

/// Полное состояние симуляции.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    tick: u64,
    item_count: u32,
    pub(crate) grid: Grid,
    pub(crate) buildings: Arena,
    pub(crate) conveyors: Table<ConveyorRow>,
    pub(crate) drills: Table<DrillRow>,
    pub(crate) crafters: Table<CrafterRow>,
    pub(crate) generators: Table<()>,
    pub(crate) nodes: Table<()>,
    pub(crate) cores: Table<CoreRow>,
    pub(crate) segments: Vec<Option<Segment>>,
    free_segments: Vec<u32>,
    /// Битсет бодрствующих сегментов (бит на сегмент).
    seg_awake: Vec<u64>,
    /// Отключить усыпление (эталонный режим для дифференциальных тестов).
    no_sleep: bool,
    /// Эпоха топологии: растёт при любой стройке/сносе; инвалидирует кэши связей.
    topo_epoch: u64,
    /// Битсет бодрствующих буров по индексу ячейки арены.
    drill_awake: Vec<u64>,
    /// Колесо таймеров пробуждения: слот `tick % TIMER_SLOTS` → (тик, кто).
    timers: Vec<Vec<(u64, Sleeper)>>,
    pub(crate) power: PowerState,
    rng: Rng,
    stats: Stats,
}

const NO_SEGMENT: u32 = u32::MAX;

/// Разрешённая связь «откуда → куда» для передачи предметов (кэшируется по эпохе топологии).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum Link {
    /// На тайле нет постройки.
    #[default]
    Nothing,
    /// Постройка, которая никогда не примет предмет с этой стороны.
    Refuses,
    /// Производство или ядро.
    Building(BuildingId),
    /// Сегмент конвейера: тайл `k`, вход сзади или сбоку.
    Segment {
        /// Индекс сегмента.
        seg: u32,
        /// Тайл внутри сегмента.
        k: u32,
        /// Вход сзади (иначе сбоку).
        behind: bool,
    },
}

/// Кто может спать в ожидании события.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Sleeper {
    /// Сегмент конвейера.
    Segment(u32),
    /// Постройка (сейчас — бур).
    Building(BuildingId),
}

/// Запись об ожидании места в сегменте: кто ждёт и в какую позицию хочет вставить.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Waiter {
    pub(crate) who: Sleeper,
    pub(crate) pos: u32,
}

/// Число слотов колеса таймеров.
const TIMER_SLOTS: usize = 256;

/// Максимальная длина сегмента конвейера в тайлах.
pub const MAX_SEGMENT_TILES: usize = 32;

impl World {
    /// Создаёт пустой мир, залитый одним полом.
    pub fn new(reg: &ContentRegistry, width: u32, height: u32, floor: FloorId, seed: u64) -> Self {
        let items = reg.items().len();
        Self {
            tick: 0,
            item_count: u32::try_from(items).unwrap_or(u32::MAX),
            grid: Grid::new(width, height, floor),
            buildings: Arena::default(),
            conveyors: Table::default(),
            drills: Table::default(),
            crafters: Table::default(),
            generators: Table::default(),
            nodes: Table::default(),
            cores: Table::default(),
            segments: Vec::new(),
            free_segments: Vec::new(),
            seg_awake: Vec::new(),
            no_sleep: false,
            topo_epoch: 1,
            drill_awake: Vec::new(),
            timers: vec![Vec::new(); TIMER_SLOTS],
            power: PowerState::default(),
            rng: Rng::new(seed),
            stats: Stats::new(items),
        }
    }

    /// Номер текущего тика.
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Сетка мира.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Постройки.
    pub fn buildings(&self) -> &Arena {
        &self.buildings
    }

    /// Счётчики ресурсов.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// ГПСЧ мира (для генерации и будущих систем).
    pub fn rng_mut(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// Активные сегменты конвейеров.
    pub fn segments(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter().flatten()
    }

    /// Задать пол тайла (генерация карты).
    pub fn set_floor(&mut self, p: TilePos, floor: FloorId) {
        self.grid.set_floor(p, floor);
    }

    /// Задать руду тайла (генерация карты).
    pub fn set_ore(&mut self, p: TilePos, ore: Option<OreId>) {
        self.grid.set_ore(p, ore);
    }

    /// Содержимое ядра, которому принадлежит тайл.
    pub fn core_items(&self, at: TilePos) -> Option<&[u32]> {
        let id = self.grid.building(at)?;
        let b = self.buildings.get(id)?;
        (b.tag == KindTag::Core).then(|| {
            self.cores
                .rows
                .get(b.row as usize)
                .map(|r| r.items.as_slice())
        })?
    }

    /// Сумма ресурсов во всех ядрах.
    pub fn total_core_items(&self) -> Vec<u64> {
        let mut total = vec![0u64; self.item_count as usize];
        for row in &self.cores.rows {
            for (t, &n) in total.iter_mut().zip(&row.items) {
                *t += u64::from(n);
            }
        }
        total
    }

    /// Число предметов каждого вида, находящихся на конвейерах.
    pub fn items_on_conveyors(&self) -> Vec<u64> {
        let mut total = vec![0u64; self.item_count as usize];
        for seg in self.segments() {
            for item in seg.items() {
                if let Some(t) = total.get_mut(usize::from(item.0)) {
                    *t += 1;
                }
            }
        }
        total
    }

    /// Число предметов каждого вида во внутренних буферах построек (буры, производства).
    pub fn items_in_buffers(&self, reg: &ContentRegistry) -> Vec<u64> {
        let mut total = vec![0u64; self.item_count as usize];
        let mut add = |item: ItemId, n: u32| {
            if let Some(t) = total.get_mut(usize::from(item.0)) {
                *t += u64::from(n);
            }
        };
        for row in &self.drills.rows {
            if let Some(item) = row.item {
                add(item, row.buffer);
            }
        }
        for (row, owner) in self.crafters.rows.iter().zip(&self.crafters.owners) {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            if let BlockKind::Crafter {
                inputs, outputs, ..
            } = &reg.block(b.block).kind
            {
                for (s, &n) in inputs.iter().zip(&row.inputs) {
                    add(s.item, n);
                }
                for (s, &n) in outputs.iter().zip(&row.outputs) {
                    add(s.item, n);
                }
            }
        }
        total
    }

    // -----------------------------------------------------------------------
    // Тик
    // -----------------------------------------------------------------------

    /// Выполняет один тик: применяет команды по порядку, затем все системы.
    /// Возвращает результат каждой команды.
    pub fn tick(
        &mut self,
        reg: &ContentRegistry,
        commands: &[Command],
    ) -> Vec<Result<(), CommandError>> {
        let results = commands.iter().map(|c| self.apply(reg, c)).collect();
        self.step(reg);
        results
    }

    /// Выполняет один тик без команд (не выделяет память в стационарном режиме).
    pub fn step(&mut self, reg: &ContentRegistry) {
        if self.power.dirty {
            self.rebuild_power(reg);
        }
        self.balance_power(reg);
        self.fire_timers();
        self.update_drills(reg);
        self.update_crafters(reg);
        self.update_conveyors(reg);
        self.tick += 1;
    }

    /// Применяет одну команду немедленно.
    pub fn apply(&mut self, reg: &ContentRegistry, cmd: &Command) -> Result<(), CommandError> {
        match *cmd {
            Command::Place { block, origin, rot } => {
                self.place(reg, block, origin, rot).map(|_| ())
            }
            Command::Remove { at } => self.remove(reg, at),
        }
    }

    // -----------------------------------------------------------------------
    // Строительство
    // -----------------------------------------------------------------------

    fn place(
        &mut self,
        reg: &ContentRegistry,
        block: BlockId,
        origin: TilePos,
        rot: Dir,
    ) -> Result<BuildingId, CommandError> {
        let def = reg
            .blocks()
            .get(usize::from(block.0))
            .ok_or(CommandError::UnknownBlock)?;
        let size = i32::from(def.size);
        for dy in 0..size {
            for dx in 0..size {
                let p = TilePos::new(origin.x + dx, origin.y + dy);
                let floor = self.grid.floor(p).ok_or(CommandError::OutOfBounds)?;
                if !reg.floor(floor).buildable {
                    return Err(CommandError::NotBuildable);
                }
                if self.grid.building(p).is_some() {
                    return Err(CommandError::Occupied);
                }
            }
        }

        let tag = match def.kind {
            BlockKind::Conveyor { .. } => KindTag::Conveyor,
            BlockKind::Drill { .. } => KindTag::Drill,
            BlockKind::Crafter { .. } => KindTag::Crafter,
            BlockKind::Generator { .. } => KindTag::Generator,
            BlockKind::PowerNode { .. } => KindTag::PowerNode,
            BlockKind::Core { .. } => KindTag::Core,
        };
        self.topo_epoch += 1;
        let id = self.buildings.insert(Building {
            block,
            origin,
            size: def.size,
            rot,
            tag,
            row: 0,
            power_net: None,
            dump_cursor: 0,
            dump_epoch: 0,
            dump_links: Vec::new(),
        });
        for dy in 0..size {
            for dx in 0..size {
                self.grid
                    .set_building(TilePos::new(origin.x + dx, origin.y + dy), Some(id));
            }
        }

        let row = match &def.kind {
            BlockKind::Conveyor { .. } => self.conveyors.push(
                id,
                ConveyorRow {
                    segment: NO_SEGMENT,
                    index: 0,
                },
            ),
            BlockKind::Drill { .. } => {
                let (item, ore_tiles) = self.ore_under(reg, origin, def.size);
                self.drills.push(
                    id,
                    DrillRow {
                        item,
                        ore_tiles,
                        progress: 0,
                        buffer: 0,
                    },
                )
            }
            BlockKind::Crafter {
                inputs, outputs, ..
            } => self.crafters.push(
                id,
                CrafterRow {
                    inputs: vec![0; inputs.len()],
                    outputs: vec![0; outputs.len()],
                    progress: 0,
                },
            ),
            BlockKind::Generator { .. } => self.generators.push(id, ()),
            BlockKind::PowerNode { .. } => self.nodes.push(id, ()),
            BlockKind::Core { .. } => self.cores.push(
                id,
                CoreRow {
                    items: vec![0; self.item_count as usize],
                },
            ),
        };
        if let Some(b) = self.buildings.get_mut(id) {
            b.row = row;
        }
        if def.is_power() {
            self.power.dirty = true;
        }
        if tag == KindTag::Drill {
            self.wake_drill(id);
        }
        if let Some(b) = self.buildings.get(id).cloned() {
            self.wake_adjacent_drills(&b);
            if tag != KindTag::Conveyor {
                self.wake_perimeter(&b);
            }
        }
        if tag == KindTag::Conveyor {
            self.rebuild_conveyors_around(reg, origin, &[]);
        }
        Ok(id)
    }

    /// Самая распространённая руда под постройкой (при равенстве — меньший ID ресурса).
    fn ore_under(&self, reg: &ContentRegistry, origin: TilePos, size: u8) -> (Option<ItemId>, u32) {
        let mut counts: BTreeMap<ItemId, u32> = BTreeMap::new();
        let s = i32::from(size);
        for dy in 0..s {
            for dx in 0..s {
                if let Some(ore) = self.grid.ore(TilePos::new(origin.x + dx, origin.y + dy)) {
                    *counts.entry(reg.ore(ore).item).or_insert(0) += 1;
                }
            }
        }
        let mut best: Option<(ItemId, u32)> = None;
        for (item, n) in counts {
            if best.is_none_or(|(_, b)| n > b) {
                best = Some((item, n));
            }
        }
        match best {
            Some((item, n)) => (Some(item), n),
            None => (None, 0),
        }
    }

    fn remove(&mut self, reg: &ContentRegistry, at: TilePos) -> Result<(), CommandError> {
        let id = self.grid.building(at).ok_or(CommandError::NothingThere)?;
        let b = self
            .buildings
            .remove(id)
            .ok_or(CommandError::NothingThere)?;
        let s = i32::from(b.size);
        for dy in 0..s {
            for dx in 0..s {
                self.grid
                    .set_building(TilePos::new(b.origin.x + dx, b.origin.y + dy), None);
            }
        }
        let def = reg.block(b.block);
        if def.is_power() {
            self.power.dirty = true;
        }
        let mut old_segment = None;
        let moved = match b.tag {
            KindTag::Conveyor => self.conveyors.swap_remove(b.row).map(|(row, m)| {
                old_segment = Some(row.segment);
                m
            }),
            KindTag::Drill => self.drills.swap_remove(b.row).map(|(row, m)| {
                if let Some(item) = row.item {
                    Stats::bump(&mut self.stats.destroyed, item, u64::from(row.buffer));
                }
                m
            }),
            KindTag::Crafter => self.crafters.swap_remove(b.row).map(|(row, m)| {
                if let BlockKind::Crafter {
                    inputs, outputs, ..
                } = &def.kind
                {
                    for (st, &n) in inputs
                        .iter()
                        .zip(&row.inputs)
                        .chain(outputs.iter().zip(&row.outputs))
                    {
                        Stats::bump(&mut self.stats.destroyed, st.item, u64::from(n));
                    }
                }
                m
            }),
            KindTag::Generator => self.generators.swap_remove(b.row).map(|(_, m)| m),
            KindTag::PowerNode => self.nodes.swap_remove(b.row).map(|(_, m)| m),
            KindTag::Core => self.cores.swap_remove(b.row).map(|(row, m)| {
                for (i, &n) in row.items.iter().enumerate() {
                    Stats::bump(&mut self.stats.destroyed, ItemId(i as u16), u64::from(n));
                }
                m
            }),
        };
        if let Some(Some(moved)) = moved
            && let Some(mb) = self.buildings.get_mut(moved)
        {
            mb.row = b.row;
        }
        self.wake_adjacent_drills(&b);
        if let Some(seg) = old_segment {
            self.rebuild_conveyors_around(reg, at, &[seg]);
        } else {
            self.wake_perimeter(&b);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Конвейеры: построение сегментов
    // -----------------------------------------------------------------------

    fn conveyor_info(&self, p: TilePos) -> Option<(BlockId, Dir, &ConveyorRow)> {
        let id = self.grid.building(p)?;
        let b = self.buildings.get(id)?;
        if b.tag != KindTag::Conveyor {
            return None;
        }
        Some((b.block, b.rot, self.conveyors.rows.get(b.row as usize)?))
    }

    fn conveyor_row_mut(&mut self, p: TilePos) -> Option<&mut ConveyorRow> {
        let id = self.grid.building(p)?;
        let b = self.buildings.get(id)?;
        if b.tag != KindTag::Conveyor {
            return None;
        }
        self.conveyors.rows.get_mut(b.row as usize)
    }

    /// Основной предшественник конвейера: тот, что стоит сзади и смотрит в него; если сзади
    /// никого нет — единственный боковой (поворот ленты). Только того же типа.
    fn primary_pred(&self, p: TilePos) -> Option<TilePos> {
        let (block, dir, _) = self.conveyor_info(p)?;
        let feeds = |q: TilePos| {
            self.conveyor_info(q)
                .is_some_and(|(b, d, _)| b == block && q.step(d) == p)
        };
        let behind = p.step(dir.opposite());
        if feeds(behind) {
            return Some(behind);
        }
        let mut side = None;
        for d in [rot_left(dir), rot_right(dir)] {
            let q = p.step(d);
            if feeds(q) {
                if side.is_some() {
                    return None;
                }
                side = Some(q);
            }
        }
        side
    }

    fn alloc_segment(&mut self, seg: Segment) -> u32 {
        let i = if let Some(i) = self.free_segments.pop() {
            self.segments[i as usize] = Some(seg);
            i
        } else {
            self.segments.push(Some(seg));
            u32::try_from(self.segments.len() - 1).unwrap_or(NO_SEGMENT)
        };
        let words = self.segments.len().div_ceil(64);
        if self.seg_awake.len() < words {
            self.seg_awake.resize(words, 0);
        }
        self.wake_segment(i);
        i
    }

    /// Перестраивает сегменты, затронутые изменением тайла `around`, сохраняя предметы.
    fn rebuild_conveyors_around(&mut self, reg: &ContentRegistry, around: TilePos, extra: &[u32]) {
        let mut dissolve: BTreeSet<u32> =
            extra.iter().copied().filter(|&s| s != NO_SEGMENT).collect();
        let mut tiles: BTreeSet<TilePos> = BTreeSet::new();
        for p in std::iter::once(around).chain(around.neighbors()) {
            if let Some((_, _, row)) = self.conveyor_info(p) {
                if row.segment != NO_SEGMENT {
                    dissolve.insert(row.segment);
                } else {
                    tiles.insert(p);
                }
            }
        }

        // Разобрать сегменты, запомнив предметы по тайлам.
        let mut tile_items: BTreeMap<TilePos, Vec<(ItemId, u32)>> = BTreeMap::new();
        for s in dissolve {
            let Some(mut seg) = self.segments.get_mut(s as usize).and_then(Option::take) else {
                continue;
            };
            self.free_segments.push(s);
            self.sleep_segment(s as usize);
            for wt in std::mem::take(&mut seg.waiters) {
                self.wake(wt.who);
            }
            let seg_tiles = std::mem::take(&mut seg.tiles);
            seg.tiles.clone_from(&seg_tiles);
            seg.drain_by_tile(|k, item, off| {
                tile_items
                    .entry(seg_tiles[k])
                    .or_default()
                    .push((item, off))
            });
            tiles.extend(seg_tiles);
        }

        // Тайлы, которые больше не конвейеры: предметы уничтожаются.
        tiles.retain(|&p| self.conveyor_info(p).is_some());
        let lost: Vec<(TilePos, Vec<(ItemId, u32)>)> = tile_items
            .iter()
            .filter(|(p, _)| !tiles.contains(p))
            .map(|(p, v)| (*p, v.clone()))
            .collect();
        for (p, v) in lost {
            tile_items.remove(&p);
            for (item, _) in v {
                Stats::bump(&mut self.stats.destroyed, item, 1);
            }
        }

        // Построить цепочки: сначала от «голов», затем оставшиеся циклы.
        let mut visited: BTreeSet<TilePos> = BTreeSet::new();
        let heads: Vec<TilePos> = tiles
            .iter()
            .copied()
            .filter(|&p| self.primary_pred(p).is_none_or(|q| !tiles.contains(&q)))
            .collect();
        for h in heads {
            self.build_chain(reg, h, &tiles, &mut visited, &mut tile_items);
        }
        let rest: Vec<TilePos> = tiles
            .iter()
            .copied()
            .filter(|p| !visited.contains(p))
            .collect();
        for p in rest {
            if !visited.contains(&p) {
                self.build_chain(reg, p, &tiles, &mut visited, &mut tile_items);
            }
        }
    }

    fn build_chain(
        &mut self,
        reg: &ContentRegistry,
        head: TilePos,
        tiles: &BTreeSet<TilePos>,
        visited: &mut BTreeSet<TilePos>,
        tile_items: &mut BTreeMap<TilePos, Vec<(ItemId, u32)>>,
    ) {
        let Some((block, mut dir, _)) = self.conveyor_info(head) else {
            return;
        };
        let mut chain = vec![head];
        let mut dirs = vec![dir];
        visited.insert(head);
        let mut cur = head;
        loop {
            let next = cur.step(dir);
            if !tiles.contains(&next)
                || visited.contains(&next)
                || self.primary_pred(next) != Some(cur)
            {
                break;
            }
            let Some((_, d, _)) = self.conveyor_info(next) else {
                break;
            };
            visited.insert(next);
            chain.push(next);
            dirs.push(d);
            cur = next;
            dir = d;
        }
        let speed = match reg.block(block).kind {
            BlockKind::Conveyor { speed } => speed,
            _ => 1,
        };
        // Длинные цепочки режутся на сегменты не длиннее MAX_SEGMENT_TILES: это ограничивает
        // поиск позиции при вставке, стоимость перестройки и даёт единицу работы для потоков.
        for (piece, piece_dirs) in chain
            .chunks(MAX_SEGMENT_TILES)
            .zip(dirs.chunks(MAX_SEGMENT_TILES))
        {
            let out_dir = piece_dirs.last().copied().unwrap_or(dir);
            let l = piece.len();
            let mut items: Vec<(ItemId, u32)> = Vec::new();
            for (k, p) in piece.iter().enumerate() {
                if let Some(v) = tile_items.remove(p) {
                    for (item, off) in v {
                        items.push((item, (l - 1 - k) as u32 * TILE_LEN + off));
                    }
                }
            }
            items.sort_by_key(|&(_, pos)| pos);
            let mut seg = Segment::new(piece.to_vec(), out_dir, speed);
            let lost = seg.fill_sorted(items.iter().copied());
            if lost > 0 {
                // предметы, не поместившиеся при слиянии, уничтожаются (крайний случай)
                for &(item, _) in items.iter().rev().take(lost as usize) {
                    Stats::bump(&mut self.stats.destroyed, item, 1);
                }
            }
            let s = self.alloc_segment(seg);
            for (k, p) in piece.iter().enumerate() {
                if let Some(row) = self.conveyor_row_mut(*p) {
                    row.segment = s;
                    row.index = k as u32;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Передача предметов
    // -----------------------------------------------------------------------

    /// Куда попадёт предмет, переданный с тайла `from` на соседний тайл `target`.
    /// Чистое чтение; результат можно кэшировать до следующего изменения топологии.
    fn resolve_link(&self, target: TilePos, from: TilePos) -> Link {
        let Some(id) = self.grid.building(target) else {
            return Link::Nothing;
        };
        let Some(b) = self.buildings.get(id) else {
            return Link::Nothing;
        };
        match b.tag {
            KindTag::Conveyor => {
                if target.step(b.rot) == from {
                    return Link::Refuses; // лента смотрит на источник
                }
                let Some(cr) = self.conveyors.rows.get(b.row as usize) else {
                    return Link::Nothing;
                };
                Link::Segment {
                    seg: cr.segment,
                    k: cr.index,
                    behind: target.step(b.rot.opposite()) == from,
                }
            }
            KindTag::Crafter | KindTag::Core => Link::Building(id),
            KindTag::Drill | KindTag::Generator | KindTag::PowerNode => Link::Refuses,
        }
    }

    /// Передаёт предмет по разрешённой связи.
    fn deliver(&mut self, reg: &ContentRegistry, link: Link, item: ItemId) -> bool {
        match link {
            Link::Nothing | Link::Refuses => false,
            Link::Building(id) => self.offer_building(reg, id, item),
            Link::Segment { seg, k, behind } => {
                let Some(sg) = self.segments.get_mut(seg as usize).and_then(Option::as_mut) else {
                    return false;
                };
                let ok = if behind {
                    sg.try_insert_behind(item, k as usize)
                } else {
                    sg.try_insert_side(item, k as usize)
                };
                if ok {
                    self.wake_segment(seg);
                }
                ok
            }
        }
    }

    /// Передача предмета постройке, не являющейся конвейером.
    fn offer_building(&mut self, reg: &ContentRegistry, id: BuildingId, item: ItemId) -> bool {
        let Some(b) = self.buildings.get(id) else {
            return false;
        };
        let row = b.row as usize;
        match b.tag {
            KindTag::Crafter => {
                let BlockKind::Crafter { inputs, buffer, .. } = &reg.block(b.block).kind else {
                    return false;
                };
                let Some(slot) = inputs.iter().position(|s| s.item == item) else {
                    return false;
                };
                let Some(cr) = self.crafters.rows.get_mut(row) else {
                    return false;
                };
                if cr.inputs[slot] < *buffer {
                    cr.inputs[slot] += 1;
                    true
                } else {
                    false
                }
            }
            KindTag::Core => {
                let BlockKind::Core { capacity } = reg.block(b.block).kind else {
                    return false;
                };
                let Some(core) = self.cores.rows.get_mut(row) else {
                    return false;
                };
                match core.items.get_mut(usize::from(item.0)) {
                    Some(n) if *n < capacity => {
                        *n += 1;
                        true
                    }
                    _ => false,
                }
            }
            KindTag::Conveyor | KindTag::Drill | KindTag::Generator | KindTag::PowerNode => false,
        }
    }

    /// Выгрузка одного предмета в соседние постройки по кругу.
    /// Список получателей кэшируется до следующего изменения топологии.
    fn dump(&mut self, reg: &ContentRegistry, owner: BuildingId, item: ItemId) -> bool {
        let epoch = self.topo_epoch;
        let Some(b) = self.buildings.get(owner) else {
            return false;
        };
        if b.dump_epoch != epoch {
            let mut links = Vec::new();
            for p in 0..b.perimeter_len() {
                let (outer, inner) = b.perimeter(p);
                let link = self.resolve_link(outer, inner);
                if !matches!(link, Link::Nothing | Link::Refuses) {
                    links.push(link);
                }
            }
            let Some(bm) = self.buildings.get_mut(owner) else {
                return false;
            };
            bm.dump_links = links;
            bm.dump_epoch = epoch;
        }
        let Some(b) = self.buildings.get(owner) else {
            return false;
        };
        let n = b.dump_links.len() as u32;
        if n == 0 {
            return false;
        }
        let start = b.dump_cursor % n;
        for i in 0..n {
            let idx = (start + i) % n;
            let Some(link) = self
                .buildings
                .get(owner)
                .map(|b| b.dump_links[idx as usize])
            else {
                return false;
            };
            if self.deliver(reg, link, item) {
                if let Some(bm) = self.buildings.get_mut(owner) {
                    bm.dump_cursor = (idx + 1) % n;
                }
                return true;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Системы
    // -----------------------------------------------------------------------

    fn update_drills(&mut self, reg: &ContentRegistry) {
        // Бодрствующие буры по возрастанию индекса ячейки арены (детерминированно).
        let mut w = 0;
        while w < self.drill_awake.len() {
            let mut from_bit = 0u32;
            while from_bit < 64 {
                let bits = self.drill_awake[w] & (u64::MAX << from_bit);
                if bits == 0 {
                    break;
                }
                let bit = bits.trailing_zeros();
                self.update_drill(reg, w * 64 + bit as usize);
                from_bit = bit + 1;
            }
            w += 1;
        }
    }

    fn update_drill(&mut self, reg: &ContentRegistry, slot: usize) {
        let Some(owner) = self.buildings.id_at(slot) else {
            self.sleep_drill(slot);
            return;
        };
        let Some(b) = self.buildings.get(owner) else {
            return;
        };
        let (KindTag::Drill, BlockKind::Drill { base_ticks, buffer }) =
            (b.tag, &reg.block(b.block).kind)
        else {
            self.sleep_drill(slot);
            return;
        };
        let (base_ticks, cap) = (*base_ticks, *buffer);
        let r = b.row as usize;
        let threshold = base_ticks * u32::from(b.size) * u32::from(b.size);
        let Some(row) = self.drills.rows.get_mut(r) else {
            return;
        };
        let Some(item) = row.item else {
            // руды под буром нет — работать он не начнёт никогда
            self.sleep_drill(slot);
            return;
        };
        if row.buffer < cap {
            row.progress += row.ore_tiles;
            if row.progress >= threshold {
                row.progress -= threshold;
                row.buffer += 1;
                Stats::bump(&mut self.stats.produced, item, 1);
            }
        }
        if self.drills.rows[r].buffer == 0 {
            return;
        }
        if self.dump(reg, owner, item) {
            self.drills.rows[r].buffer -= 1;
            return;
        }
        if self.drills.rows[r].buffer < cap {
            return; // ещё добывает
        }
        // Буфер полон и никто не принял предмет: бур может уснуть, если все получатели —
        // ленты (их освобождение отслеживается); производства и ядра пока всегда опрашиваются.
        let n = self.buildings.get(owner).map_or(0, |b| b.dump_links.len());
        let has_building = (0..n).any(|j| {
            matches!(
                self.buildings.get(owner).map(|b| b.dump_links[j]),
                Some(Link::Building(_))
            )
        });
        if has_building {
            return;
        }
        for j in 0..n {
            let Some(link) = self.buildings.get(owner).map(|b| b.dump_links[j]) else {
                return;
            };
            if let Link::Segment { seg, k, behind } = link {
                self.wait_on_segment(Sleeper::Building(owner), seg, k, behind);
            }
        }
        self.sleep_drill(slot);
    }

    /// Может ли производство сейчас работать (есть входы и место под выходы).
    pub(crate) fn crafter_ready(reg: &ContentRegistry, block: BlockId, row: &CrafterRow) -> bool {
        let BlockKind::Crafter {
            inputs,
            outputs,
            buffer,
            ..
        } = &reg.block(block).kind
        else {
            return false;
        };
        inputs.iter().zip(&row.inputs).all(|(s, &n)| n >= s.count)
            && outputs
                .iter()
                .zip(&row.outputs)
                .all(|(s, &n)| n + s.count <= *buffer)
    }

    fn update_crafters(&mut self, reg: &ContentRegistry) {
        for r in 0..self.crafters.len() {
            let owner = self.crafters.owners[r];
            let Some(b) = self.buildings.get(owner) else {
                continue;
            };
            let block = b.block;
            let net = b.power_net;
            let BlockKind::Crafter {
                inputs,
                outputs,
                craft_ticks,
                power_use,
                ..
            } = &reg.block(block).kind
            else {
                continue;
            };

            if Self::crafter_ready(reg, block, &self.crafters.rows[r]) {
                let sat = if *power_use == 0 {
                    1000
                } else {
                    net.and_then(|n| self.power.nets.get(n as usize))
                        .map_or(0, |n| n.satisfaction)
                };
                let row = &mut self.crafters.rows[r];
                row.progress += u64::from(sat);
                let need = u64::from(*craft_ticks) * 1000;
                if row.progress >= need {
                    row.progress -= need;
                    for (s, n) in inputs.iter().zip(row.inputs.iter_mut()) {
                        *n -= s.count;
                        Stats::bump(&mut self.stats.consumed, s.item, u64::from(s.count));
                    }
                    for (s, n) in outputs.iter().zip(row.outputs.iter_mut()) {
                        *n += s.count;
                        Stats::bump(&mut self.stats.produced, s.item, u64::from(s.count));
                    }
                }
            }

            // выгрузить один предмет за тик
            for (slot, s) in outputs.iter().enumerate() {
                if self.crafters.rows[r].outputs[slot] > 0 && self.dump(reg, owner, s.item) {
                    self.crafters.rows[r].outputs[slot] -= 1;
                    break;
                }
            }
        }
    }

    fn update_conveyors(&mut self, reg: &ContentRegistry) {
        // Обходим только бодрствующие сегменты строго по возрастанию индекса — тот же порядок,
        // что при обходе всех сегментов. Сегмент с большим индексом, разбуженный в этом тике,
        // обработается в этом же тике; с меньшим — в следующем (как и без усыпления, где его
        // ход в этом тике уже прошёл вхолостую).
        let mut w = 0;
        while w < self.seg_awake.len() {
            let mut from_bit = 0u32;
            while from_bit < 64 {
                let bits = self.seg_awake[w] & (u64::MAX << from_bit);
                if bits == 0 {
                    break;
                }
                let bit = bits.trailing_zeros();
                self.update_segment(reg, w * 64 + bit as usize);
                from_bit = bit + 1;
            }
            w += 1;
        }
    }

    fn update_segment(&mut self, reg: &ContentRegistry, i: usize) {
        let Some(seg) = self.segments.get(i).and_then(Option::as_ref) else {
            self.sleep_segment(i);
            return;
        };
        let mut target = None;
        let mut delivered = false;
        if let Some(item) = seg.ready_front()
            && let Some(&tail) = seg.tiles.last()
        {
            let link = if seg.link_epoch == self.topo_epoch {
                seg.link
            } else {
                let link = self.resolve_link(tail.step(seg.out_dir), tail);
                if let Some(sm) = self.segments.get_mut(i).and_then(Option::as_mut) {
                    sm.link = link;
                    sm.link_epoch = self.topo_epoch;
                }
                link
            };
            delivered = self.deliver(reg, link, item);
            target = Some(link);
        }
        let Some(seg) = self.segments.get_mut(i).and_then(Option::as_mut) else {
            return;
        };
        if delivered {
            let was_blocked = seg.compressed_len() > 0;
            seg.pop_front();
            let front_free = seg.front_free();
            self.release_waiters(i, was_blocked, front_free);
        }
        let Some(seg) = self.segments.get_mut(i).and_then(Option::as_mut) else {
            return;
        };
        let moved = seg.advance();
        if delivered || moved {
            return;
        }
        // Ничего не изменилось: можно ли уснуть без изменения поведения?
        let empty = seg.item_count() == 0;
        let stuck = seg.is_fully_compressed();
        match target {
            None if empty => self.sleep_segment(i),
            Some(Link::Nothing | Link::Refuses) if stuck => self.sleep_segment(i),
            Some(Link::Segment { seg: t, k, behind })
                if stuck
                    && t as usize != i
                    && self.wait_on_segment(Sleeper::Segment(i as u32), t, k, behind) =>
            {
                self.sleep_segment(i);
            }
            _ => {}
        }
    }

    /// Регистрирует `who` в ожидании места в сегменте `t` (вход на тайл `k`) и ставит таймер
    /// по нижней оценке. Возвращает `false`, если сегмента нет.
    fn wait_on_segment(&mut self, who: Sleeper, t: u32, k: u32, behind: bool) -> bool {
        let now = self.tick;
        let Some(ts) = self.segments.get_mut(t as usize).and_then(Option::as_mut) else {
            return false;
        };
        let pos = ts.insert_pos(k as usize, behind);
        let w = Waiter { who, pos };
        // Ожидающий, разбуженный таймером, остаётся в списке; повторная регистрация не должна
        // его дублировать (иначе список растёт без ограничений). Списки короткие — линейный поиск.
        if !ts.waiters.contains(&w) {
            ts.waiters.push(w);
        }
        if let Some(dt) = ts.earliest_insert(pos) {
            self.schedule(now + u64::from(dt.max(1)), who);
        }
        true
    }

    /// После выдачи предмета сегментом `i`: будит ожидающих, у которых могло появиться место.
    /// Если лента была заблокирована (часть предметов стояла), её движение меняется — будим всех;
    /// иначе — только тех, кому хватит освободившегося места у выхода.
    fn release_waiters(&mut self, i: usize, was_blocked: bool, front_free: u32) {
        let Some(seg) = self.segments.get_mut(i).and_then(Option::as_mut) else {
            return;
        };
        if seg.waiters.is_empty() {
            return;
        }
        let mut ws = std::mem::take(&mut seg.waiters);
        ws.retain(|w| {
            let wake =
                was_blocked || w.pos.saturating_add(crate::segment::ITEM_SPACING) <= front_free;
            if wake {
                self.wake(w.who);
            }
            !wake
        });
        if let Some(seg) = self.segments.get_mut(i).and_then(Option::as_mut) {
            seg.waiters = ws; // ёмкость сохраняется: без аллокаций в стационарном режиме
        }
    }

    fn wake(&mut self, who: Sleeper) {
        match who {
            Sleeper::Segment(s) => self.wake_segment(s),
            Sleeper::Building(id) => self.wake_drill(id),
        }
    }

    fn schedule(&mut self, at: u64, who: Sleeper) {
        if self.no_sleep {
            return;
        }
        let slot = (at % TIMER_SLOTS as u64) as usize;
        if let Some(v) = self.timers.get_mut(slot) {
            v.push((at, who));
        }
    }

    fn fire_timers(&mut self) {
        let now = self.tick;
        let slot = (now % TIMER_SLOTS as u64) as usize;
        let Some(v) = self.timers.get_mut(slot) else {
            return;
        };
        if v.is_empty() {
            return;
        }
        let mut v = std::mem::take(v);
        v.retain(|&(at, who)| {
            if at <= now {
                self.wake(who);
                false
            } else {
                true
            }
        });
        if let Some(slot) = self.timers.get_mut(slot) {
            *slot = v;
        }
    }

    fn wake_drill(&mut self, id: BuildingId) {
        let s = Arena::slot_index(id);
        let words = self.buildings.slot_count().div_ceil(64);
        if self.drill_awake.len() < words {
            self.drill_awake.resize(words, 0);
        }
        if let Some(w) = self.drill_awake.get_mut(s / 64) {
            *w |= 1u64 << (s % 64);
        }
    }

    fn sleep_drill(&mut self, slot: usize) {
        if self.no_sleep && self.buildings.id_at(slot).is_some() {
            return;
        }
        if let Some(w) = self.drill_awake.get_mut(slot / 64) {
            *w &= !(1u64 << (slot % 64));
        }
    }

    /// Будит буры, касающиеся постройки (её появление/снос меняет их получателей).
    fn wake_adjacent_drills(&mut self, b: &Building) {
        for p in 0..b.perimeter_len() {
            let (outer, _) = b.perimeter(p);
            if let Some(id) = self.grid.building(outer)
                && self
                    .buildings
                    .get(id)
                    .is_some_and(|ob| ob.tag == KindTag::Drill)
            {
                self.wake_drill(id);
            }
        }
    }

    fn wake_segment(&mut self, s: u32) {
        let s = s as usize;
        if let Some(w) = self.seg_awake.get_mut(s / 64) {
            *w |= 1u64 << (s % 64);
        }
    }

    fn sleep_segment(&mut self, s: usize) {
        if self.no_sleep && self.segments.get(s).is_some_and(Option::is_some) {
            return;
        }
        if let Some(w) = self.seg_awake.get_mut(s / 64) {
            *w &= !(1u64 << (s % 64));
        }
    }

    /// Будит сегменты конвейеров, касающихся постройки (её появление/снос меняет их выход).
    fn wake_perimeter(&mut self, b: &Building) {
        for p in 0..b.perimeter_len() {
            let (outer, _) = b.perimeter(p);
            if let Some((_, _, row)) = self.conveyor_info(outer) {
                let s = row.segment;
                self.wake_segment(s);
            }
        }
    }

    /// Включает эталонный режим без усыпления сегментов (для тестов: результат обязан совпадать).
    pub fn set_sleep_enabled(&mut self, enabled: bool) {
        self.no_sleep = !enabled;
        for i in 0..self.segments.len() {
            if self.segments[i].is_some() {
                self.wake_segment(i as u32);
            }
        }
        let drills: Vec<BuildingId> = self.drills.owners.clone();
        for id in drills {
            self.wake_drill(id);
        }
    }

    /// Число бодрствующих буров (диагностика и бенчмарки).
    pub fn awake_drills(&self) -> usize {
        self.drill_awake
            .iter()
            .map(|w| w.count_ones() as usize)
            .sum()
    }

    /// Число бодрствующих сегментов (диагностика и бенчмарки).
    pub fn awake_segments(&self) -> usize {
        self.seg_awake.iter().map(|w| w.count_ones() as usize).sum()
    }

    // -----------------------------------------------------------------------
    // Отладка и бенчмарки
    // -----------------------------------------------------------------------

    /// Заполняет все конвейеры предметом с шагом `spacing` единиц (только для тестов и бенчмарков;
    /// предметы учитываются в `stats.spawned`).
    pub fn debug_fill_conveyors(&mut self, item: ItemId, spacing: u32) {
        let spacing = spacing.max(crate::segment::ITEM_SPACING);
        let mut spawned = 0u64;
        for seg in self.segments.iter_mut().flatten() {
            let len = seg.len_units();
            let mut pos = len;
            loop {
                if seg.try_insert_at(item, pos) {
                    spawned += 1;
                }
                if pos < spacing {
                    break;
                }
                pos -= spacing;
            }
        }
        Stats::bump(&mut self.stats.spawned, item, spawned);
        for i in 0..self.segments.len() {
            self.wake_segment(i as u32);
        }
    }

    /// Проверка структурной целостности мира (после загрузки файла или в тестах).
    pub fn validate(&self, reg: &ContentRegistry) -> Result<(), String> {
        self.grid.validate()?;
        if self.item_count as usize != reg.items().len() {
            return Err("item count does not match content".into());
        }
        for (id, b) in self.buildings.iter() {
            if usize::from(b.block.0) >= reg.blocks().len() {
                return Err("unknown block id".into());
            }
            let def = reg.block(b.block);
            if def.size != b.size {
                return Err("building size does not match block".into());
            }
            let s = i32::from(b.size);
            for dy in 0..s {
                for dx in 0..s {
                    let p = TilePos::new(b.origin.x + dx, b.origin.y + dy);
                    if self.grid.building(p) != Some(id) {
                        return Err(format!("tile {p:?} does not reference its building"));
                    }
                }
            }
            let (len, owner) = match b.tag {
                KindTag::Conveyor => (
                    self.conveyors.len(),
                    self.conveyors.owners.get(b.row as usize),
                ),
                KindTag::Drill => (self.drills.len(), self.drills.owners.get(b.row as usize)),
                KindTag::Crafter => (
                    self.crafters.len(),
                    self.crafters.owners.get(b.row as usize),
                ),
                KindTag::Generator => (
                    self.generators.len(),
                    self.generators.owners.get(b.row as usize),
                ),
                KindTag::PowerNode => (self.nodes.len(), self.nodes.owners.get(b.row as usize)),
                KindTag::Core => (self.cores.len(), self.cores.owners.get(b.row as usize)),
            };
            if (b.row as usize) >= len || owner != Some(&id) {
                return Err("building table row mismatch".into());
            }
            if b.dump_epoch > self.topo_epoch {
                return Err("dump cache from the future".into());
            }
            if let Some(n) = b.power_net
                && (n as usize) >= self.power.nets.len()
            {
                return Err("power net out of range".into());
            }
            if b.tag == KindTag::Crafter {
                let BlockKind::Crafter {
                    inputs, outputs, ..
                } = &def.kind
                else {
                    return Err("crafter kind mismatch".into());
                };
                let row = &self.crafters.rows[b.row as usize];
                if row.inputs.len() != inputs.len() || row.outputs.len() != outputs.len() {
                    return Err("crafter buffers mismatch".into());
                }
            }
        }
        for row in &self.cores.rows {
            if row.items.len() != self.item_count as usize {
                return Err("core storage size mismatch".into());
            }
        }
        if self.timers.len() != TIMER_SLOTS {
            return Err("timer wheel size mismatch".into());
        }
        if self.seg_awake.len() < self.segments.len().div_ceil(64) {
            return Err("awake bitset too short".into());
        }
        for (i, seg) in self.segments.iter().enumerate() {
            let Some(seg) = seg else { continue };
            seg.validate()?;
            if seg
                .waiters
                .iter()
                .any(|w| matches!(w.who, Sleeper::Segment(s) if s as usize >= self.segments.len()))
            {
                return Err("segment waiter out of range".into());
            }
            for (k, &p) in seg.tiles.iter().enumerate() {
                match self.conveyor_info(p) {
                    Some((_, _, row)) if row.segment as usize == i && row.index as usize == k => {}
                    _ => return Err(format!("segment {i} tile {k} inconsistent")),
                }
            }
            for item in seg.items() {
                if u32::from(item.0) >= self.item_count {
                    return Err("unknown item id".into());
                }
            }
        }
        for row in &self.conveyors.rows {
            if self
                .segments
                .get(row.segment as usize)
                .is_none_or(Option::is_none)
            {
                return Err("conveyor without segment".into());
            }
        }
        Ok(())
    }
}

fn rot_left(d: Dir) -> Dir {
    match d {
        Dir::Right => Dir::Up,
        Dir::Up => Dir::Left,
        Dir::Left => Dir::Down,
        Dir::Down => Dir::Right,
    }
}

fn rot_right(d: Dir) -> Dir {
    rot_left(d).opposite()
}
