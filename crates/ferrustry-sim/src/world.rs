//! Состояние мира и игровой тик.

use std::collections::{BTreeMap, BTreeSet};

use ferrustry_content::{
    BlockId, BlockKind, ContentRegistry, FloorId, ItemId, MAX_RECIPE_SLOTS, OreId,
};
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
    /// Настроить постройку (сейчас — фильтр сортировщика; `None` — без фильтра).
    Configure {
        /// Любой тайл постройки.
        at: TilePos,
        /// Ресурс фильтра.
        item: Option<ItemId>,
    },
    /// Отремонтировать руину (D-01): она заменяется постройкой-оригиналом за часть стоимости.
    Repair {
        /// Любой тайл руины.
        at: TilePos,
    },
}

/// Правила партии.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    /// Песочница: стройка бесплатна (тесты, бенчмарки, редактор).
    pub infinite_resources: bool,
    /// Волны врагов включены.
    pub waves: bool,
    /// Тиков до первой волны.
    pub first_wave: u64,
    /// Тиков между волнами.
    pub wave_spacing: u64,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            infinite_resources: true,
            waves: false,
            first_wave: 5 * 60 * u64::from(TICKS_PER_SECOND),
            wave_spacing: 2 * 60 * u64::from(TICKS_PER_SECOND),
        }
    }
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
    /// В ядрах не хватает ресурса на постройку.
    #[error("not enough resources")]
    NotEnough(ItemId),
    /// Постройку нельзя так настроить.
    #[error("building is not configurable")]
    NotConfigurable,
    /// Эту постройку нельзя поставить командой (руина вне редактора).
    #[error("this building cannot be built")]
    NotPlaceable,
    /// Ремонтировать можно только руину.
    #[error("only ruins can be repaired")]
    NotARuin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RouterRow {
    /// Предмет в буфере (вместимость — 1).
    pub(crate) item: Option<ItemId>,
    /// Сторона, с которой он пришёл.
    pub(crate) from: Dir,
    /// Фильтр (только сортировщик).
    pub(crate) filter: Option<ItemId>,
    /// Курсор очерёдности выходов.
    pub(crate) cursor: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BridgeRow {
    pub(crate) items: Vec<ItemId>,
    pub(crate) cooldown: u32,
    pub(crate) partner: Option<BuildingId>,
    pub(crate) partner_epoch: u64,
}

/// Генератор: топливо в буфере и сколько тиков ещё горит текущий предмет.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GeneratorRow {
    /// Принимает ли топливо (копия из контента: связи строятся без реестра).
    pub(crate) fueled: bool,
    pub(crate) fuel: u32,
    pub(crate) burn: u32,
}

/// Склад: количество каждого предмета и курсоры выдачи.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DepotRow {
    pub(crate) items: Vec<u32>,
    pub(crate) total: u32,
    /// Следующий предмет для выдачи (по кругу).
    pub(crate) next_item: u32,
    /// Следующий передний тайл для выдачи (по кругу).
    pub(crate) next_side: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BatteryRow {
    pub(crate) stored: u64,
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
    /// Прогресс на один предмет (`base_ticks × площадь`) и ёмкость буфера — копии из контента,
    /// чтобы тик бура не обращался к постройке и реестру (P-04).
    pub(crate) threshold: u32,
    pub(crate) cap: u32,
    /// Шум за тик добычи (копия из контента, D-02).
    pub(crate) noise: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CrafterRow {
    /// Вид постройки (копия `Building::block`: горячие циклы читают только строку, P-04).
    pub(crate) block: BlockId,
    /// Энергосеть и тепловая сеть (копии из `Building`, обновляются при перестройке сетей).
    pub(crate) power_net: Option<u32>,
    pub(crate) heat_net: Option<u32>,
    /// Входной и выходной буферы по слотам рецепта; лишние слоты всегда 0.
    pub(crate) inputs: [u32; MAX_RECIPE_SLOTS],
    pub(crate) outputs: [u32; MAX_RECIPE_SLOTS],
    /// Прогресс в тысячных долях тика при полной энергии.
    pub(crate) progress: u64,
    /// Дробная часть выделенного тепла, тысячные (без потерь при замедлении).
    pub(crate) heat_acc: u32,
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
    /// Добыто бурами и сделано рецептами — без возвратов стоимости (цели кампании).
    pub made: Vec<u64>,
    /// Отремонтировано руин (D-01).
    pub repaired: u64,
}

impl Stats {
    fn new(items: usize) -> Self {
        Self {
            produced: vec![0; items],
            consumed: vec![0; items],
            destroyed: vec![0; items],
            spawned: vec![0; items],
            made: vec![0; items],
            repaired: 0,
        }
    }

    pub(crate) fn bump(v: &mut [u64], item: ItemId, n: u64) {
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
    pub(crate) generators: Table<GeneratorRow>,
    pub(crate) depots: Table<DepotRow>,
    pub(crate) nodes: Table<()>,
    pub(crate) cores: Table<CoreRow>,
    pub(crate) routers: Table<RouterRow>,
    pub(crate) bridges: Table<BridgeRow>,
    pub(crate) batteries: Table<BatteryRow>,
    /// Трубы, тепловые трубы, радиаторы, градирни (состояние — в сетях).
    pub(crate) passive: Table<()>,
    pub(crate) condensers: Table<crate::thermal::CondenserRow>,
    pub(crate) engines: Table<crate::thermal::EngineRow>,
    pub(crate) fluids: crate::thermal::Nets<crate::thermal::FluidNet>,
    pub(crate) heat: crate::thermal::Nets<crate::thermal::HeatNet>,
    pub(crate) fluid_stats: crate::thermal::FluidStats,
    pub(crate) turrets: Table<crate::combat::TurretRow>,
    pub(crate) units: crate::combat::Units,
    pub(crate) projectiles: crate::combat::Projectiles,
    pub(crate) flow: crate::combat::FlowField,
    pub(crate) waves: crate::combat::Waves,
    pub(crate) spawns: Vec<TilePos>,
    pub(crate) outcome: crate::combat::Outcome,
    pub(crate) kills: u64,
    #[serde(skip)]
    pub(crate) unit_grid: crate::combat::Scratch<crate::combat::UnitGrid>,
    /// «Ячейка арены → строка бура» (служебное): `.0` — таблица построена; дальше
    /// поддерживается точечно при стройке и сносе буров (раньше пересобиралась при любой стройке).
    #[serde(skip)]
    pub(crate) drill_rows: crate::combat::Scratch<(bool, Vec<u32>)>,
    /// Шум базы за прошлый тик (D-02): сумма шума работавших построек; ускоряет таймер волн.
    pub(crate) noise: u32,
    /// Шум, накопленный в текущем тике.
    pub(crate) noise_acc: u32,
    /// Эталонный режим: поле путей всегда перестраивается целиком (бенчмарки, тесты).
    #[serde(skip)]
    pub(crate) flow_full_only: bool,
    /// Буферы поля путей (служебное).
    #[serde(skip)]
    pub(crate) flow_scratch: crate::combat::Scratch<crate::combat::FlowScratch>,
    /// Буферы переназначения индексов юнитов при удалении убитых (служебное).
    #[serde(skip)]
    pub(crate) unit_remap: crate::combat::Scratch<(Vec<u32>, Vec<u32>)>,
    rules: Rules,
    pub(crate) segments: crate::segment::Segments,
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
    pub(crate) stats: Stats,
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
    /// Производство, ядро, турель, топливный генератор, склад.
    Building(BuildingId),
    /// Постройка 1×1, которой важна сторона входа (разветвитель, сортировщик, мост);
    /// `side` — направление от неё к источнику.
    Directed {
        /// Получатель.
        id: BuildingId,
        /// Сторона входа.
        side: Dir,
    },
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
    /// Часть `k` из `n` скользящей контрольной суммы состояния (D-20): за `n` подряд идущих
    /// частей хешируется всё сохраняемое состояние, а одна часть стоит ~1/`n` полного хеша.
    /// Сервер и клиенты в одинаковом тике хешируют одинаковую часть, поэтому у совпадающих
    /// миров совпадают и части. Поля перечислены без `..`: новое поле мира не скомпилируется,
    /// пока его не добавят сюда.
    pub fn checksum_part(&self, k: u64, n: u64) -> u64 {
        let mut p = crate::hash::PartHasher::new(k, n);
        let World {
            tick,
            item_count,
            grid,
            buildings,
            conveyors,
            drills,
            crafters,
            generators,
            depots,
            nodes,
            cores,
            routers,
            bridges,
            batteries,
            passive,
            condensers,
            engines,
            fluids,
            heat,
            fluid_stats,
            turrets,
            units,
            projectiles,
            flow,
            waves,
            spawns,
            outcome,
            kills,
            unit_grid: _,
            drill_rows: _,
            noise,
            noise_acc,
            flow_full_only: _,
            flow_scratch: _,
            unit_remap: _,
            rules,
            segments,
            free_segments,
            seg_awake,
            no_sleep,
            topo_epoch,
            drill_awake,
            timers,
            power,
            rng,
            stats,
        } = self;
        p.whole(&(
            tick, item_count, kills, noise, noise_acc, no_sleep, topo_epoch,
        ));
        p.whole(rules);
        p.whole(rng);
        p.whole(stats);
        p.whole(waves);
        p.whole(spawns);
        p.whole(outcome);
        p.whole(fluid_stats);
        grid.hash_part(&mut p);
        buildings.hash_part(&mut p);
        conveyors.hash_part(&mut p);
        drills.hash_part(&mut p);
        crafters.hash_part(&mut p);
        generators.hash_part(&mut p);
        depots.hash_part(&mut p);
        nodes.hash_part(&mut p);
        cores.hash_part(&mut p);
        routers.hash_part(&mut p);
        bridges.hash_part(&mut p);
        batteries.hash_part(&mut p);
        passive.hash_part(&mut p);
        condensers.hash_part(&mut p);
        engines.hash_part(&mut p);
        turrets.hash_part(&mut p);
        let crate::thermal::Nets { nets, dirty } = fluids;
        p.split(nets);
        p.whole(dirty);
        let crate::thermal::Nets { nets, dirty } = heat;
        p.split(nets);
        p.whole(dirty);
        let crate::power::PowerState { nets, dirty, free } = power;
        p.split(nets);
        p.whole(&(dirty, free));
        let crate::combat::Units {
            kind,
            x,
            y,
            health,
            cooldown,
            vx,
            vy,
            hold,
            incoming,
        } = units;
        p.split(kind);
        p.split(x);
        p.split(y);
        p.split(health);
        p.split(cooldown);
        p.split(vx);
        p.split(vy);
        p.split(hold);
        p.split(incoming);
        let crate::combat::Projectiles {
            x,
            y,
            vx,
            vy,
            damage,
            ttl,
            hostile,
            target,
        } = projectiles;
        p.split(x);
        p.split(y);
        p.split(vx);
        p.split(vy);
        p.split(damage);
        p.split(ttl);
        p.split(hostile);
        p.split(target);
        let crate::combat::FlowField { dist, dirty, full } = flow;
        p.split(dist);
        p.split(dirty);
        p.whole(full);
        segments.hash_part(&mut p);
        p.split(free_segments);
        p.split(seg_awake);
        p.split(drill_awake);
        p.split(timers);
        p.finish()
    }

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
            depots: Table::default(),
            nodes: Table::default(),
            cores: Table::default(),
            routers: Table::default(),
            bridges: Table::default(),
            batteries: Table::default(),
            passive: Table::default(),
            condensers: Table::default(),
            engines: Table::default(),
            fluids: Default::default(),
            heat: Default::default(),
            fluid_stats: Default::default(),
            turrets: Table::default(),
            units: Default::default(),
            projectiles: Default::default(),
            flow: Default::default(),
            waves: Default::default(),
            spawns: Vec::new(),
            outcome: Default::default(),
            kills: 0,
            unit_grid: Default::default(),
            unit_remap: Default::default(),
            flow_scratch: Default::default(),
            flow_full_only: false,
            noise: 0,
            noise_acc: 0,
            drill_rows: Default::default(),
            rules: Rules::default(),
            segments: Default::default(),
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

    pub(crate) fn topo_epoch(&self) -> u64 {
        self.topo_epoch
    }

    /// Правила партии.
    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// Задать правила (до начала игры или в редакторе). Включение волн запускает таймер.
    pub fn set_rules(&mut self, rules: Rules) {
        if rules.waves && self.waves.next_number == 0 {
            self.waves.next_number = 1;
            self.waves.remaining = rules.first_wave.saturating_mul(1000);
        }
        self.rules = rules;
    }

    /// Выдать ресурс в ядра извне (стартовый запас, награды, тесты); учитывается в
    /// `stats.spawned`. Возвращает, сколько поместилось.
    pub fn grant(&mut self, reg: &ContentRegistry, item: ItemId, n: u32) -> u32 {
        let put = self.add_to_cores(reg, item, n);
        Stats::bump(&mut self.stats.spawned, item, u64::from(put));
        put
    }

    /// Положить ресурс в ядра (без учёта в статистике). Возвращает, сколько поместилось.
    pub(crate) fn add_to_cores(&mut self, reg: &ContentRegistry, item: ItemId, mut n: u32) -> u32 {
        let start = n;
        for (row, owner) in self.cores.rows.iter_mut().zip(&self.cores.owners) {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            let BlockKind::Core { capacity } = reg.block(b.block).kind else {
                continue;
            };
            if let Some(c) = row.items.get_mut(usize::from(item.0)) {
                let put = n.min(capacity.saturating_sub(*c));
                *c += put;
                n -= put;
            }
        }
        start - n
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
    pub fn segments(&self) -> impl Iterator<Item = crate::segment::SegRef<'_>> {
        self.segments.iter()
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
        for row in &self.routers.rows {
            if let Some(item) = row.item {
                add(item, 1);
            }
        }
        for row in &self.bridges.rows {
            for &item in &row.items {
                add(item, 1);
            }
        }
        for row in &self.depots.rows {
            for (i, &n) in row.items.iter().enumerate() {
                add(ItemId(i as u16), n);
            }
        }
        for (row, owner) in self.generators.rows.iter().zip(&self.generators.owners) {
            if let Some(b) = self.buildings.get(*owner)
                && let BlockKind::Generator {
                    fuel: Some(fuel), ..
                } = reg.block(b.block).kind
            {
                add(fuel, row.fuel);
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
        self.noise_acc = 0;
        if self.power.dirty {
            self.rebuild_power(reg);
        }
        self.rebuild_thermal(reg);
        self.balance_power(reg);
        self.update_condensers(reg);
        self.fire_timers();
        self.update_drills(reg);
        self.begin_heat_tick();
        self.update_crafters(reg);
        self.update_heat(reg);
        self.update_routers(reg);
        self.update_depots(reg);
        self.update_bridges(reg);
        self.update_conveyors(reg);
        self.noise = self.noise_acc;
        self.update_combat(reg);
        self.tick += 1;
    }

    /// Применяет одну команду немедленно.
    pub fn apply(&mut self, reg: &ContentRegistry, cmd: &Command) -> Result<(), CommandError> {
        match *cmd {
            Command::Place { block, origin, rot } => {
                // руины ставят только генератор карты и редактор (песочница)
                if usize::from(block.0) < reg.blocks().len()
                    && matches!(reg.block(block).kind, BlockKind::Ruin { .. })
                    && !self.rules.infinite_resources
                {
                    return Err(CommandError::NotPlaceable);
                }
                self.place(reg, block, origin, rot).map(|_| ())
            }
            Command::Remove { at } => self.remove(reg, at),
            Command::Configure { at, item } => self.configure(at, item),
            Command::Repair { at } => self.repair(reg, at),
        }
    }

    fn configure(&mut self, at: TilePos, item: Option<ItemId>) -> Result<(), CommandError> {
        let id = self.grid.building(at).ok_or(CommandError::NothingThere)?;
        let b = self.buildings.get(id).ok_or(CommandError::NothingThere)?;
        if b.tag != KindTag::Sorter || item.is_some_and(|i| u32::from(i.0) >= self.item_count) {
            return Err(CommandError::NotConfigurable);
        }
        let row = self
            .routers
            .rows
            .get_mut(b.row as usize)
            .ok_or(CommandError::NotConfigurable)?;
        row.filter = item;
        Ok(())
    }

    /// Ремонт руины: постройка-оригинал на том же месте за долю стоимости (D-01).
    fn repair(&mut self, reg: &ContentRegistry, at: TilePos) -> Result<(), CommandError> {
        let id = self.grid.building(at).ok_or(CommandError::NothingThere)?;
        let b = self.buildings.get(id).ok_or(CommandError::NothingThere)?;
        let BlockKind::Ruin { of } = reg.block(b.block).kind else {
            return Err(CommandError::NotARuin);
        };
        let (ruin, origin, rot) = (b.block, b.origin, b.rot);
        let cost = reg.ruin_rule().repair_cost(&reg.block(of).cost);
        self.check_stacks(&cost)?;
        // руина уходит без добычи; оригинал встаёт бесплатно, оплачивается доля
        self.remove_impl(reg, at, false)?;
        match self.place_free(reg, of, origin, rot) {
            Ok(_) => {
                self.pay_stacks(&cost);
                self.stats.repaired += 1;
                Ok(())
            }
            Err(e) => {
                let _ = self.place_free(reg, ruin, origin, rot);
                Err(e)
            }
        }
    }

    /// Поставить постройку без оплаты (ремонт оплачивает свою долю сам).
    fn place_free(
        &mut self,
        reg: &ContentRegistry,
        block: BlockId,
        origin: TilePos,
        rot: Dir,
    ) -> Result<BuildingId, CommandError> {
        let infinite = self.rules.infinite_resources;
        self.rules.infinite_resources = true;
        let res = self.place(reg, block, origin, rot);
        self.rules.infinite_resources = infinite;
        res
    }

    /// Хватает ли в ядрах ресурсов.
    fn check_stacks(&self, cost: &[ferrustry_content::ItemStack]) -> Result<(), CommandError> {
        if self.rules.infinite_resources {
            return Ok(());
        }
        let total = self.total_core_items();
        for st in cost {
            if total.get(usize::from(st.item.0)).copied().unwrap_or(0) < u64::from(st.count) {
                return Err(CommandError::NotEnough(st.item));
            }
        }
        Ok(())
    }

    /// Списывает ресурсы из ядер (по порядку ядер). Вызывать после `check_stacks`.
    fn pay_stacks(&mut self, cost: &[ferrustry_content::ItemStack]) {
        if self.rules.infinite_resources {
            return;
        }
        for st in cost {
            let mut left = st.count;
            for row in &mut self.cores.rows {
                if let Some(c) = row.items.get_mut(usize::from(st.item.0)) {
                    let take = left.min(*c);
                    *c -= take;
                    left -= take;
                }
                if left == 0 {
                    break;
                }
            }
            Stats::bump(
                &mut self.stats.consumed,
                st.item,
                u64::from(st.count - left),
            );
        }
    }

    /// Хватает ли в ядрах ресурсов на постройку.
    fn check_cost(&self, reg: &ContentRegistry, block: BlockId) -> Result<(), CommandError> {
        if self.rules.infinite_resources {
            return Ok(());
        }
        let total = self.total_core_items();
        for st in &reg.block(block).cost {
            if total.get(usize::from(st.item.0)).copied().unwrap_or(0) < u64::from(st.count) {
                return Err(CommandError::NotEnough(st.item));
            }
        }
        Ok(())
    }

    /// Списывает стоимость из ядер (по порядку ядер). Вызывать после `check_cost`.
    fn pay_cost(&mut self, reg: &ContentRegistry, block: BlockId) {
        if self.rules.infinite_resources {
            return;
        }
        for st in &reg.block(block).cost {
            let mut left = st.count;
            for row in &mut self.cores.rows {
                if let Some(c) = row.items.get_mut(usize::from(st.item.0)) {
                    let take = left.min(*c);
                    *c -= take;
                    left -= take;
                }
                if left == 0 {
                    break;
                }
            }
            Stats::bump(
                &mut self.stats.consumed,
                st.item,
                u64::from(st.count - left),
            );
        }
    }

    /// Возвращает стоимость в ядра (что не поместилось — теряется).
    fn refund_cost(&mut self, reg: &ContentRegistry, block: BlockId) {
        if self.rules.infinite_resources {
            return;
        }
        // снос руины даёт долю стоимости оригинала (D-01)
        let back = match reg.block(block).kind {
            BlockKind::Ruin { of } => reg.ruin_rule().salvage(&reg.block(of).cost),
            _ => reg.block(block).cost.clone(),
        };
        for st in back {
            let put = self.add_to_cores(reg, st.item, st.count);
            Stats::bump(&mut self.stats.produced, st.item, u64::from(put));
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
        self.can_place(reg, block, origin)?;
        self.pay_cost(reg, block);
        let def = reg.block(block);
        let size = i32::from(def.size);

        let tag = match def.kind {
            BlockKind::Conveyor { .. } => KindTag::Conveyor,
            BlockKind::Drill { .. } => KindTag::Drill,
            BlockKind::Crafter { .. } => KindTag::Crafter,
            BlockKind::Generator { .. } => KindTag::Generator,
            BlockKind::PowerNode { .. } => KindTag::PowerNode,
            BlockKind::Core { .. } => KindTag::Core,
            BlockKind::Splitter => KindTag::Splitter,
            BlockKind::Sorter => KindTag::Sorter,
            BlockKind::Overflow => KindTag::Overflow,
            BlockKind::Bridge { .. } => KindTag::Bridge,
            BlockKind::Battery { .. } => KindTag::Battery,
            BlockKind::Pipe { .. } => KindTag::Pipe,
            BlockKind::Condenser { .. } => KindTag::Condenser,
            BlockKind::CoolingTower { .. } => KindTag::CoolingTower,
            BlockKind::HeatPipe { .. } => KindTag::HeatPipe,
            BlockKind::Radiator { .. } => KindTag::Radiator,
            BlockKind::HeatEngine { .. } => KindTag::HeatEngine,
            BlockKind::Turret { .. } | BlockKind::ArcTurret { .. } => KindTag::Turret,
            BlockKind::Depot { .. } => KindTag::Depot,
            BlockKind::Wall => KindTag::Wall,
            BlockKind::Ruin { .. } => KindTag::Ruin,
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
            heat_net: None,
            fluid_net: None,
            dump_cursor: 0,
            health: def.health,
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
            &BlockKind::Drill { base_ticks, buffer } => {
                let (item, ore_tiles) = self.ore_under(reg, origin, def.size);
                self.drills.push(
                    id,
                    DrillRow {
                        item,
                        ore_tiles,
                        progress: 0,
                        buffer: 0,
                        threshold: base_ticks * u32::from(def.size) * u32::from(def.size),
                        cap: buffer,
                        noise: def.noise,
                    },
                )
            }
            BlockKind::Crafter { .. } => self.crafters.push(
                id,
                CrafterRow {
                    block,
                    power_net: None,
                    heat_net: None,
                    inputs: [0; MAX_RECIPE_SLOTS],
                    outputs: [0; MAX_RECIPE_SLOTS],
                    progress: 0,
                    heat_acc: 0,
                },
            ),
            BlockKind::Generator { fuel, .. } => self.generators.push(
                id,
                GeneratorRow {
                    fueled: fuel.is_some(),
                    ..Default::default()
                },
            ),
            BlockKind::Depot { .. } => self.depots.push(
                id,
                DepotRow {
                    items: vec![0; self.item_count as usize],
                    ..Default::default()
                },
            ),
            BlockKind::PowerNode { .. } => self.nodes.push(id, ()),
            BlockKind::Core { .. } => self.cores.push(
                id,
                CoreRow {
                    items: vec![0; self.item_count as usize],
                },
            ),
            BlockKind::Splitter | BlockKind::Sorter | BlockKind::Overflow => self.routers.push(
                id,
                RouterRow {
                    item: None,
                    from: Dir::Left,
                    filter: None,
                    cursor: 0,
                },
            ),
            BlockKind::Bridge { buffer, .. } => self.bridges.push(
                id,
                BridgeRow {
                    items: Vec::with_capacity(*buffer as usize),
                    cooldown: 0,
                    partner: None,
                    partner_epoch: 0,
                },
            ),
            BlockKind::Battery { .. } => self.batteries.push(id, BatteryRow { stored: 0 }),
            BlockKind::Pipe { .. }
            | BlockKind::CoolingTower { .. }
            | BlockKind::HeatPipe { .. }
            | BlockKind::Radiator { .. }
            | BlockKind::Wall
            | BlockKind::Ruin { .. } => self.passive.push(id, ()),
            BlockKind::Turret { .. } | BlockKind::ArcTurret { .. } => {
                self.turrets.push(id, Default::default())
            }
            BlockKind::Condenser { .. } => self.condensers.push(id, Default::default()),
            BlockKind::HeatEngine { .. } => self.engines.push(id, Default::default()),
        };
        self.mark_thermal_dirty(reg, block);
        if let Some(b) = self.buildings.get_mut(id) {
            b.row = row;
        }
        if tag == KindTag::Drill {
            self.drill_rows_set(Arena::slot_index(id), row);
        }
        if def.is_power() {
            self.power_attach(reg, id);
        }
        if tag == KindTag::Drill {
            self.wake_drill(id);
        }
        if let Some(b) = self.buildings.get(id).cloned() {
            self.flow_touch(&b);
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

    /// Можно ли поставить постройку (без изменения мира).
    pub fn can_place(
        &self,
        reg: &ContentRegistry,
        block: BlockId,
        origin: TilePos,
    ) -> Result<(), CommandError> {
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
        self.check_cost(reg, block)
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
        self.remove_impl(reg, at, true)
    }

    /// Разрушение (бой): как снос, но без возврата стоимости.
    pub(crate) fn destroy_at(&mut self, reg: &ContentRegistry, at: TilePos) {
        let _ = self.remove_impl(reg, at, false);
    }

    fn remove_impl(
        &mut self,
        reg: &ContentRegistry,
        at: TilePos,
        refund: bool,
    ) -> Result<(), CommandError> {
        let id = self.grid.building(at).ok_or(CommandError::NothingThere)?;
        let b = self
            .buildings
            .remove(id)
            .ok_or(CommandError::NothingThere)?;
        // снос меняет топологию: кэши связей (сегменты, выгрузка, мосты) недействительны
        self.topo_epoch += 1;
        let s = i32::from(b.size);
        for dy in 0..s {
            for dx in 0..s {
                self.grid
                    .set_building(TilePos::new(b.origin.x + dx, b.origin.y + dy), None);
            }
        }
        let def = reg.block(b.block);
        if def.is_power() {
            self.power_detach(reg, crate::power::PowerShape::of(id, &b), b.power_net);
        }
        self.mark_thermal_dirty(reg, b.block);
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
            KindTag::Splitter | KindTag::Sorter | KindTag::Overflow => {
                self.routers.swap_remove(b.row).map(|(row, m)| {
                    if let Some(item) = row.item {
                        Stats::bump(&mut self.stats.destroyed, item, 1);
                    }
                    m
                })
            }
            KindTag::Bridge => self.bridges.swap_remove(b.row).map(|(row, m)| {
                for item in row.items {
                    Stats::bump(&mut self.stats.destroyed, item, 1);
                }
                m
            }),
            KindTag::Battery => self.batteries.swap_remove(b.row).map(|(_, m)| m),
            KindTag::Pipe
            | KindTag::CoolingTower
            | KindTag::HeatPipe
            | KindTag::Radiator
            | KindTag::Wall
            | KindTag::Ruin => self.passive.swap_remove(b.row).map(|(_, m)| m),
            KindTag::Turret => self.turrets.swap_remove(b.row).map(|(_, m)| m),
            KindTag::Condenser => self.condensers.swap_remove(b.row).map(|(_, m)| m),
            KindTag::HeatEngine => self.engines.swap_remove(b.row).map(|(_, m)| m),
            KindTag::Generator => self.generators.swap_remove(b.row).map(|(row, m)| {
                if let BlockKind::Generator {
                    fuel: Some(fuel), ..
                } = def.kind
                {
                    Stats::bump(&mut self.stats.destroyed, fuel, u64::from(row.fuel));
                }
                m
            }),
            KindTag::Depot => self.depots.swap_remove(b.row).map(|(row, m)| {
                for (i, &n) in row.items.iter().enumerate() {
                    Stats::bump(&mut self.stats.destroyed, ItemId(i as u16), u64::from(n));
                }
                m
            }),
            KindTag::PowerNode => self.nodes.swap_remove(b.row).map(|(_, m)| m),
            KindTag::Core => self.cores.swap_remove(b.row).map(|(row, m)| {
                for (i, &n) in row.items.iter().enumerate() {
                    Stats::bump(&mut self.stats.destroyed, ItemId(i as u16), u64::from(n));
                }
                m
            }),
        };
        if b.tag == KindTag::Drill {
            self.drill_rows_set(Arena::slot_index(id), u32::MAX);
            if let Some(Some(m)) = moved {
                self.drill_rows_set(Arena::slot_index(m), b.row);
            }
        }
        if let Some(Some(moved)) = moved
            && let Some(mb) = self.buildings.get_mut(moved)
        {
            mb.row = b.row;
        }
        // возврат стоимости — после удаления из таблиц (сносимое ядро уже не получатель)
        if refund {
            self.refund_cost(reg, b.block);
        }
        self.flow_touch(&b);
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
            self.segments.place(i as usize, seg);
            i
        } else {
            let i = self.segments.len();
            self.segments.place(i, seg);
            u32::try_from(i).unwrap_or(NO_SEGMENT)
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
            let Some(mut seg) = self.segments.get_mut(s as usize) else {
                continue;
            };
            let waiters = std::mem::take(&mut seg.waiters);
            let seg_tiles = seg.tiles.clone();
            seg.drain_by_tile(|k, item, off| {
                tile_items
                    .entry(seg_tiles[k])
                    .or_default()
                    .push((item, off))
            });
            self.segments.remove(s as usize);
            self.free_segments.push(s);
            self.sleep_segment(s as usize);
            for wt in waiters {
                self.wake(wt.who);
            }
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
            let s = self.alloc_segment(Segment::new(piece.to_vec(), out_dir, speed));
            let lost = self
                .segments
                .get_mut(s as usize)
                .map_or(0, |mut seg| seg.fill_sorted(items.iter().copied()));
            if lost > 0 {
                // предметы, не поместившиеся при слиянии, уничтожаются (крайний случай)
                for &(item, _) in items.iter().rev().take(lost as usize) {
                    Stats::bump(&mut self.stats.destroyed, item, 1);
                }
            }
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
            KindTag::Crafter | KindTag::Core | KindTag::Turret => Link::Building(id),
            KindTag::Generator => match self.generators.rows.get(b.row as usize) {
                Some(g) if g.fueled => Link::Building(id),
                _ => Link::Refuses,
            },
            KindTag::Depot => {
                // склад не принимает с передней стороны: туда он выдаёт
                if dir_between(target, from) == Some(b.rot) {
                    Link::Refuses
                } else {
                    Link::Building(id)
                }
            }
            KindTag::Splitter | KindTag::Sorter | KindTag::Overflow | KindTag::Bridge => {
                let Some(side) = dir_between(target, from) else {
                    return Link::Refuses;
                };
                if b.tag == KindTag::Bridge && side == b.rot {
                    return Link::Refuses; // мост не принимает со стороны выхода
                }
                Link::Directed { id, side }
            }
            KindTag::Drill
            | KindTag::PowerNode
            | KindTag::Battery
            | KindTag::Pipe
            | KindTag::Condenser
            | KindTag::CoolingTower
            | KindTag::HeatPipe
            | KindTag::Radiator
            | KindTag::HeatEngine
            | KindTag::Wall
            | KindTag::Ruin => Link::Refuses,
        }
    }

    /// Передаёт предмет по разрешённой связи.
    fn deliver(&mut self, reg: &ContentRegistry, link: Link, item: ItemId) -> bool {
        match link {
            Link::Nothing | Link::Refuses => false,
            Link::Building(id) => self.offer_building(reg, id, item),
            Link::Directed { id, side } => self.offer_directed(reg, id, side, item),
            Link::Segment { seg, k, behind } => {
                let Some(mut sg) = self.segments.get_mut(seg as usize) else {
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

    /// Передача предмета с тайла `from` на соседний `target` без кэша связей
    /// (разветвители, сортировщики, мосты — их выходы зависят от предмета).
    pub(crate) fn offer(
        &mut self,
        reg: &ContentRegistry,
        target: TilePos,
        from: TilePos,
        item: ItemId,
    ) -> bool {
        let link = self.resolve_link(target, from);
        self.deliver(reg, link, item)
    }

    /// Передача предмета постройке, не являющейся конвейером.
    fn offer_building(&mut self, reg: &ContentRegistry, id: BuildingId, item: ItemId) -> bool {
        let Some(b) = self.buildings.get(id) else {
            return false;
        };
        let row = b.row as usize;
        let def = reg.block(b.block);
        match b.tag {
            KindTag::Generator => {
                let BlockKind::Generator {
                    fuel: Some(fuel),
                    buffer,
                    ..
                } = def.kind
                else {
                    return false;
                };
                match self.generators.rows.get_mut(row) {
                    Some(g) if item == fuel && g.fuel < buffer => {
                        g.fuel += 1;
                        true
                    }
                    _ => false,
                }
            }
            KindTag::Depot => {
                let BlockKind::Depot { capacity } = def.kind else {
                    return false;
                };
                let Some(d) = self.depots.rows.get_mut(row) else {
                    return false;
                };
                match d.items.get_mut(usize::from(item.0)) {
                    Some(n) if d.total < capacity => {
                        *n += 1;
                        d.total += 1;
                        true
                    }
                    _ => false,
                }
            }
            KindTag::Turret => {
                let BlockKind::Turret {
                    ammo,
                    shots_per_item,
                    buffer,
                    ..
                } = def.kind
                else {
                    return false;
                };
                let Some(t) = self.turrets.rows.get_mut(row) else {
                    return false;
                };
                if item != ammo || t.ammo + shots_per_item > buffer * shots_per_item {
                    return false;
                }
                t.ammo += shots_per_item;
                // предмет становится боеприпасом
                Stats::bump(&mut self.stats.consumed, item, 1);
                true
            }
            KindTag::Crafter => {
                let BlockKind::Crafter { inputs, buffer, .. } = &def.kind else {
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
                let BlockKind::Core { capacity } = def.kind else {
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
            _ => false,
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

    /// Записать в таблицу «ячейка → строка бура» (если она уже построена).
    fn drill_rows_set(&mut self, slot: usize, row: u32) {
        let map = &mut self.drill_rows.0;
        if !map.0 {
            return; // построится целиком при первом обращении
        }
        if map.1.len() <= slot {
            map.1.resize(slot + 1, u32::MAX);
        }
        map.1[slot] = row;
    }

    /// Таблица «ячейка арены → строка бура» (служебная): тик бура не читает саму постройку
    /// (P-04). Строится один раз (после создания или загрузки мира), дальше — точечно.
    fn drill_row_of_slot(&mut self, slot: usize) -> Option<usize> {
        let map = &mut self.drill_rows.0;
        if !map.0 {
            map.1.clear();
            map.1.resize(self.buildings.slot_count(), u32::MAX);
            for (r, owner) in self.drills.owners.iter().enumerate() {
                if let Some(v) = map.1.get_mut(Arena::slot_index(*owner)) {
                    *v = r as u32;
                }
            }
            map.0 = true;
        }
        map.1
            .get(slot)
            .copied()
            .filter(|&r| r != u32::MAX)
            .map(|r| r as usize)
    }

    fn update_drill(&mut self, reg: &ContentRegistry, slot: usize) {
        let Some(r) = self.drill_row_of_slot(slot) else {
            self.sleep_drill(slot);
            return;
        };
        let owner = self.drills.owners[r];
        let row = &mut self.drills.rows[r];
        let (threshold, cap) = (row.threshold, row.cap);
        let Some(item) = row.item else {
            // руды под буром нет — работать он не начнёт никогда
            self.sleep_drill(slot);
            return;
        };
        if row.buffer < cap {
            row.progress += row.ore_tiles;
            let noise = row.noise;
            if row.progress >= threshold {
                row.progress -= threshold;
                row.buffer += 1;
                Stats::bump(&mut self.stats.produced, item, 1);
                Stats::bump(&mut self.stats.made, item, 1);
            }
            self.noise_acc += noise;
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
                Some(Link::Building(_) | Link::Directed { .. })
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

    /// Обновить копии сетей в строках производств (после перестройки энергосетей или тепла).
    pub(crate) fn sync_crafter_nets(&mut self) {
        for (row, owner) in self.crafters.rows.iter_mut().zip(&self.crafters.owners) {
            if let Some(b) = self.buildings.get(*owner) {
                row.power_net = b.power_net;
                row.heat_net = b.heat_net;
            }
        }
    }

    fn update_crafters(&mut self, reg: &ContentRegistry) {
        for r in 0..self.crafters.len() {
            let owner = self.crafters.owners[r];
            let row = &self.crafters.rows[r];
            let (block, net, heat_net) = (row.block, row.power_net, row.heat_net);
            let def = reg.block(block);
            let BlockKind::Crafter {
                inputs,
                outputs,
                craft_ticks,
                power_use,
                heat,
                ..
            } = &def.kind
            else {
                continue;
            };

            if Self::crafter_ready(reg, block, &self.crafters.rows[r]) {
                let power_sat = if *power_use == 0 {
                    1000
                } else {
                    net.and_then(|n| self.power.nets.get(n as usize))
                        .map_or(0, |n| n.satisfaction)
                };
                // перегрев замедляет работу (тепловая сеть, D-02)
                let sat = if *heat > 0 {
                    power_sat * crate::thermal::heat_factor(self.temperature_of(heat_net)) / 1000
                } else {
                    power_sat
                };
                if sat > 0 {
                    self.noise_acc += def.noise;
                }
                let row = &mut self.crafters.rows[r];
                row.heat_acc += *heat * sat;
                let heat_units = u64::from(row.heat_acc / 1000);
                row.heat_acc %= 1000;
                row.progress += u64::from(sat);
                let need = u64::from(*craft_ticks) * 1000;
                self.add_heat(heat_net, heat_units);
                let row = &mut self.crafters.rows[r];
                if row.progress >= need {
                    row.progress -= need;
                    for (s, n) in inputs.iter().zip(row.inputs.iter_mut()) {
                        *n -= s.count;
                        Stats::bump(&mut self.stats.consumed, s.item, u64::from(s.count));
                    }
                    for (s, n) in outputs.iter().zip(row.outputs.iter_mut()) {
                        *n += s.count;
                        Stats::bump(&mut self.stats.produced, s.item, u64::from(s.count));
                        Stats::bump(&mut self.stats.made, s.item, u64::from(s.count));
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
        let Some(seg) = self.segments.get(i) else {
            self.sleep_segment(i);
            return;
        };
        let mut target = None;
        let mut delivered = false;
        if let Some(item) = seg.ready_front() {
            // последний тайл (отдельный массив в куче) нужен только для пересчёта связи
            let link = if seg.link_epoch == self.topo_epoch {
                seg.link
            } else {
                let Some(&tail) = seg.tiles.last() else {
                    return;
                };
                let link = self.resolve_link(tail.step(seg.out_dir), tail);
                if let Some(mut sm) = self.segments.get_mut(i) {
                    sm.link = link;
                    sm.link_epoch = self.topo_epoch;
                }
                link
            };
            delivered = self.deliver(reg, link, item);
            target = Some(link);
        }
        // Один вид сегмента на выдачу, сдвиг и решение о сне (P-03: каждый вид — проверки
        // границ пула). Будить ожидающих можно после сдвига: это трогает только их биты и список
        // ожидающих этого сегмента, но не его предметы.
        let Some(mut seg) = self.segments.get_mut(i) else {
            return;
        };
        let mut release = None;
        if delivered {
            let was_blocked = seg.compressed_len() > 0;
            seg.pop_front();
            release = Some((was_blocked, seg.front_free()));
        }
        let moved = seg.advance();
        if delivered || moved {
            if let Some((was_blocked, front_free)) = release {
                self.release_waiters(i, was_blocked, front_free);
            }
            return;
        }
        let empty = seg.item_count() == 0;
        let stuck = seg.is_fully_compressed();
        // Ничего не изменилось: можно ли уснуть без изменения поведения?
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
        let Some(mut ts) = self.segments.get_mut(t as usize) else {
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
        let Some(mut seg) = self.segments.get_mut(i) else {
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
        if let Some(mut seg) = self.segments.get_mut(i) {
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
        if self.no_sleep && self.segments.is_live(s) {
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

    /// Локальная починка поля путей (по умолчанию включена); выключенная — эталон: поле
    /// перестраивается целиком при каждом изменении построек. Результат обязан совпадать.
    pub fn set_incremental_flow(&mut self, enabled: bool) {
        self.flow_full_only = !enabled;
    }

    /// Включает эталонный режим без усыпления сегментов (для тестов: результат обязан совпадать).
    pub fn set_sleep_enabled(&mut self, enabled: bool) {
        self.no_sleep = !enabled;
        for i in 0..self.segments.len() {
            if self.segments.is_live(i) {
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
        self.segments.for_each_mut(|mut seg| {
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
        });
        Stats::bump(&mut self.stats.spawned, item, spawned);
        for i in 0..self.segments.len() {
            self.wake_segment(i as u32);
        }
    }

    /// Кладёт ресурс во вход производства на тайле (только тесты и бенчмарки; учитывается в
    /// `stats.spawned`). Возвращает, сколько поместилось.
    pub fn debug_fill_crafter(
        &mut self,
        reg: &ContentRegistry,
        at: TilePos,
        item: ItemId,
        n: u32,
    ) -> u32 {
        let Some(b) = self.grid.building(at).and_then(|id| self.buildings.get(id)) else {
            return 0;
        };
        let BlockKind::Crafter { inputs, buffer, .. } = &reg.block(b.block).kind else {
            return 0;
        };
        let Some(slot) = inputs.iter().position(|s| s.item == item) else {
            return 0;
        };
        let Some(row) = self.crafters.rows.get_mut(b.row as usize) else {
            return 0;
        };
        let put = n.min(buffer.saturating_sub(row.inputs[slot]));
        row.inputs[slot] += put;
        Stats::bump(&mut self.stats.spawned, item, u64::from(put));
        put
    }

    /// Заполнить все турели боеприпасами до предела (только бенчмарки и тесты).
    pub fn debug_fill_turrets(&mut self, reg: &ContentRegistry) {
        for (row, owner) in self.turrets.rows.iter_mut().zip(&self.turrets.owners) {
            let Some(b) = self.buildings.get(*owner) else {
                continue;
            };
            if let BlockKind::Turret {
                shots_per_item,
                buffer,
                ..
            } = reg.block(b.block).kind
            {
                row.ammo = shots_per_item * buffer;
            }
        }
    }

    /// Проверка структурной целостности мира (после загрузки файла или в тестах).
    pub fn validate(&self, reg: &ContentRegistry) -> Result<(), String> {
        self.grid.validate()?;
        if !self.power.dirty {
            self.validate_power(reg)?;
        }
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
                KindTag::Splitter | KindTag::Sorter | KindTag::Overflow => {
                    (self.routers.len(), self.routers.owners.get(b.row as usize))
                }
                KindTag::Bridge => (self.bridges.len(), self.bridges.owners.get(b.row as usize)),
                KindTag::Battery => (
                    self.batteries.len(),
                    self.batteries.owners.get(b.row as usize),
                ),
                KindTag::Pipe
                | KindTag::CoolingTower
                | KindTag::HeatPipe
                | KindTag::Radiator
                | KindTag::Wall
                | KindTag::Ruin => (self.passive.len(), self.passive.owners.get(b.row as usize)),
                KindTag::Turret => (self.turrets.len(), self.turrets.owners.get(b.row as usize)),
                KindTag::Depot => (self.depots.len(), self.depots.owners.get(b.row as usize)),
                KindTag::Condenser => (
                    self.condensers.len(),
                    self.condensers.owners.get(b.row as usize),
                ),
                KindTag::HeatEngine => {
                    (self.engines.len(), self.engines.owners.get(b.row as usize))
                }
            };
            if (b.row as usize) >= len || owner != Some(&id) {
                return Err("building table row mismatch".into());
            }
            if b.dump_epoch > self.topo_epoch {
                return Err("dump cache from the future".into());
            }
            if b.dump_epoch == self.topo_epoch {
                // действующий кэш выгрузки обязан совпадать со свежим расчётом (строки получателей)
                let fresh: Vec<Link> = (0..b.perimeter_len())
                    .map(|p| {
                        let (outer, inner) = b.perimeter(p);
                        self.resolve_link(outer, inner)
                    })
                    .filter(|l| !matches!(l, Link::Nothing | Link::Refuses))
                    .collect();
                if fresh != b.dump_links {
                    return Err(format!("stale dump links at {:?}", b.origin));
                }
            }
            if let Some(n) = b.power_net
                && (n as usize) >= self.power.nets.len()
            {
                return Err("power net out of range".into());
            }
            if b.power_net.is_some() != def.is_power() {
                return Err(format!("power net presence is wrong at {:?}", b.origin));
            }
            if let (KindTag::Drill, &BlockKind::Drill { base_ticks, buffer }) = (b.tag, &def.kind) {
                let row = &self.drills.rows[b.row as usize];
                if row.threshold != base_ticks * u32::from(b.size) * u32::from(b.size)
                    || row.cap != buffer
                {
                    return Err("drill row copies differ from the content".into());
                }
            }
            if b.tag == KindTag::Crafter {
                let BlockKind::Crafter {
                    inputs, outputs, ..
                } = &def.kind
                else {
                    return Err("crafter kind mismatch".into());
                };
                let row = &self.crafters.rows[b.row as usize];
                if row.inputs[inputs.len()..].iter().any(|&n| n != 0)
                    || row.outputs[outputs.len()..].iter().any(|&n| n != 0)
                {
                    return Err("crafter buffers mismatch".into());
                }
                if row.block != b.block
                    || row.power_net != b.power_net
                    || row.heat_net != b.heat_net
                {
                    return Err("crafter row copies differ from the building".into());
                }
            }
        }
        for row in &self.cores.rows {
            if row.items.len() != self.item_count as usize {
                return Err("core storage size mismatch".into());
            }
        }
        for row in &self.depots.rows {
            if row.items.len() != self.item_count as usize
                || row.items.iter().map(|&n| u64::from(n)).sum::<u64>() != u64::from(row.total)
            {
                return Err("depot storage mismatch".into());
            }
        }
        let u = &self.units;
        if [
            u.x.len(),
            u.y.len(),
            u.health.len(),
            u.cooldown.len(),
            u.vx.len(),
            u.vy.len(),
            u.hold.len(),
            u.incoming.len(),
        ]
        .iter()
        .any(|&n| n != u.kind.len())
        {
            return Err("unit arrays out of sync".into());
        }
        if u.kind.iter().any(|k| usize::from(k.0) >= reg.units().len()) {
            return Err("unknown unit type".into());
        }
        let p = &self.projectiles;
        if [
            p.y.len(),
            p.vx.len(),
            p.vy.len(),
            p.damage.len(),
            p.ttl.len(),
            p.hostile.len(),
            p.target.len(),
        ]
        .iter()
        .any(|&n| n != p.x.len())
        {
            return Err("projectile arrays out of sync".into());
        }
        self.check_incoming(reg)?;
        // действующая таблица «ячейка → строка бура» обязана совпадать с таблицей буров
        let map = &self.drill_rows.0;
        if map.0 {
            for d in &self.drills.owners {
                if map
                    .1
                    .get(Arena::slot_index(*d))
                    .is_none_or(|&r| r == u32::MAX)
                {
                    return Err(format!(
                        "drill row table misses slot {}",
                        Arena::slot_index(*d)
                    ));
                }
            }
            for (slot, &r) in map.1.iter().enumerate() {
                let ok = if r == u32::MAX {
                    self.buildings
                        .id_at(slot)
                        .and_then(|id| self.buildings.get(id))
                        .is_none_or(|b| b.tag != KindTag::Drill)
                } else {
                    self.drills
                        .owners
                        .get(r as usize)
                        .is_some_and(|o| Arena::slot_index(*o) == slot)
                };
                if !ok {
                    return Err(format!("stale drill row table at slot {slot}"));
                }
            }
        }
        if self.timers.len() != TIMER_SLOTS {
            return Err("timer wheel size mismatch".into());
        }
        if self.seg_awake.len() < self.segments.len().div_ceil(64) {
            return Err("awake bitset too short".into());
        }
        for (i, seg) in self.segments.enumerate() {
            seg.validate()?;
            if seg.link_epoch == self.topo_epoch
                && let Some(&tail) = seg.tiles.last()
                && seg.link != self.resolve_link(tail.step(seg.out_dir), tail)
            {
                return Err(format!("segment {i}: stale output link"));
            }
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
            if !self.segments.is_live(row.segment as usize) {
                return Err("conveyor without segment".into());
            }
        }
        Ok(())
    }
}

/// Направление от соседнего тайла `a` к тайлу `b` (если они соседи по стороне).
pub(crate) fn dir_between(a: TilePos, b: TilePos) -> Option<Dir> {
    Dir::ALL.into_iter().find(|&d| a.step(d) == b)
}

pub(crate) fn rot_left(d: Dir) -> Dir {
    match d {
        Dir::Right => Dir::Up,
        Dir::Up => Dir::Left,
        Dir::Left => Dir::Down,
        Dir::Down => Dir::Right,
    }
}

pub(crate) fn rot_right(d: Dir) -> Dir {
    rot_left(d).opposite()
}

/// Состояние постройки для инспектора интерфейса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    /// Конвейер: предметов на его сегменте.
    Conveyor {
        /// Предметов на сегменте.
        items_on_segment: usize,
    },
    /// Бур.
    Drill {
        /// Добываемый ресурс (нет руды — `None`).
        ore: Option<ItemId>,
        /// Тайлов руды под буром.
        ore_tiles: u32,
        /// Предметов в буфере.
        buffer: u32,
    },
    /// Производство.
    Crafter {
        /// Входной буфер.
        inputs: Vec<(ItemId, u32)>,
        /// Выходной буфер.
        outputs: Vec<(ItemId, u32)>,
        /// Прогресс текущего цикла, тысячные.
        progress: u32,
        /// Хватает ли входов и места для выходов.
        ready: bool,
    },
    /// Ядро: содержимое по ID ресурса.
    Core {
        /// Количество каждого ресурса.
        items: Vec<u32>,
    },
    /// Генератор.
    Generator {
        /// Выработка за тик (когда работает).
        power: u32,
        /// Топливо в буфере (`None` — генератор без топлива).
        fuel: Option<(ItemId, u32)>,
        /// Работает ли сейчас.
        running: bool,
    },
    /// Склад.
    Depot {
        /// Непустые запасы.
        items: Vec<(ItemId, u32)>,
        /// Всего предметов.
        total: u32,
        /// Ёмкость.
        capacity: u32,
    },
    /// Узел энергосети.
    PowerNode,
    /// Батарея.
    Battery {
        /// Запас.
        stored: u64,
        /// Ёмкость.
        capacity: u32,
    },
    /// Разветвитель.
    Splitter {
        /// Предмет в буфере.
        item: Option<ItemId>,
    },
    /// Переполнитель.
    Overflow {
        /// Предмет в буфере.
        item: Option<ItemId>,
    },
    /// Сортировщик.
    Sorter {
        /// Предмет в буфере.
        item: Option<ItemId>,
        /// Фильтр.
        filter: Option<ItemId>,
    },
    /// Мост.
    Bridge {
        /// Предметы в буфере.
        items: Vec<ItemId>,
        /// Найден ли партнёр (иначе это выход моста).
        linked: bool,
    },
    /// Труба, конденсатор, градирня, тепловая труба, радиатор (состояние — в сетях).
    Network,
    /// Тепловая машина.
    HeatEngine {
        /// Выработка энергии за тик.
        output: u32,
    },
    /// Турель.
    Turret {
        /// Выстрелов в запасе.
        ammo: u32,
    },
    /// Дуговая турель.
    ArcTurret {
        /// Готовность разряда, тысячные.
        charge: u32,
    },
    /// Стена.
    Wall,
    /// Руина (D-01): что это было, цена ремонта и добыча при сносе.
    Ruin {
        /// Постройка-оригинал.
        of: BlockId,
        /// Цена ремонта.
        repair: Vec<ferrustry_content::ItemStack>,
        /// Что вернёт снос.
        salvage: Vec<ferrustry_content::ItemStack>,
    },
}

/// Результат осмотра постройки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// Тип постройки.
    pub block: BlockId,
    /// Левый нижний тайл.
    pub origin: TilePos,
    /// Поворот.
    pub rot: Dir,
    /// Прочность.
    pub health: u32,
    /// Максимальная прочность.
    pub max_health: u32,
    /// Энергосеть постройки.
    pub power: Option<crate::power::PowerNet>,
    /// Тепловая сеть.
    pub heat: Option<crate::thermal::HeatNet>,
    /// Жидкостная сеть.
    pub fluid: Option<crate::thermal::FluidNet>,
    /// Состояние по виду поведения.
    pub detail: Detail,
}

impl World {
    /// Осмотр постройки, занимающей тайл.
    pub fn inspect(&self, reg: &ContentRegistry, at: TilePos) -> Option<Inspection> {
        let id = self.grid.building(at)?;
        let b = self.buildings.get(id)?;
        let row = b.row as usize;
        let def = reg.block(b.block);
        let detail = match (&def.kind, b.tag) {
            (_, KindTag::Conveyor) => {
                let s = self.conveyors.rows.get(row)?.segment;
                Detail::Conveyor {
                    items_on_segment: self
                        .segments
                        .get(s as usize)
                        .map_or(0, |seg| seg.item_count()),
                }
            }
            (_, KindTag::Drill) => {
                let r = self.drills.rows.get(row)?;
                Detail::Drill {
                    ore: r.item,
                    ore_tiles: r.ore_tiles,
                    buffer: r.buffer,
                }
            }
            (
                BlockKind::Crafter {
                    inputs,
                    outputs,
                    craft_ticks,
                    ..
                },
                KindTag::Crafter,
            ) => {
                let r = self.crafters.rows.get(row)?;
                let need = u64::from(*craft_ticks) * 1000;
                Detail::Crafter {
                    inputs: inputs
                        .iter()
                        .zip(&r.inputs)
                        .map(|(s, &n)| (s.item, n))
                        .collect(),
                    outputs: outputs
                        .iter()
                        .zip(&r.outputs)
                        .map(|(s, &n)| (s.item, n))
                        .collect(),
                    progress: u32::try_from(r.progress * 1000 / need.max(1)).unwrap_or(1000),
                    ready: Self::crafter_ready(reg, b.block, r),
                }
            }
            (_, KindTag::Core) => Detail::Core {
                items: self.cores.rows.get(row)?.items.clone(),
            },
            (BlockKind::Generator { power, fuel, .. }, _) => {
                let r = self.generators.rows.get(row)?;
                Detail::Generator {
                    power: *power,
                    fuel: fuel.map(|f| (f, r.fuel)),
                    running: fuel.is_none() || r.burn > 0,
                }
            }
            (BlockKind::Depot { capacity }, _) => {
                let r = self.depots.rows.get(row)?;
                Detail::Depot {
                    items: r
                        .items
                        .iter()
                        .enumerate()
                        .filter(|&(_, &n)| n > 0)
                        .map(|(i, &n)| (ItemId(i as u16), n))
                        .collect(),
                    total: r.total,
                    capacity: *capacity,
                }
            }
            (BlockKind::ArcTurret { reload, .. }, KindTag::Turret) => {
                let cd = self.turrets.rows.get(row)?.cooldown;
                Detail::ArcTurret {
                    charge: 1000 - (cd * 1000 / (*reload).max(1)).min(1000),
                }
            }
            (_, KindTag::PowerNode) => Detail::PowerNode,
            (BlockKind::Battery { capacity }, _) => Detail::Battery {
                stored: self.batteries.rows.get(row)?.stored,
                capacity: *capacity,
            },
            (_, KindTag::Splitter) => Detail::Splitter {
                item: self.routers.rows.get(row)?.item,
            },
            (_, KindTag::Overflow) => Detail::Overflow {
                item: self.routers.rows.get(row)?.item,
            },
            (_, KindTag::Sorter) => {
                let r = self.routers.rows.get(row)?;
                Detail::Sorter {
                    item: r.item,
                    filter: r.filter,
                }
            }
            (_, KindTag::Bridge) => {
                let r = self.bridges.rows.get(row)?;
                Detail::Bridge {
                    items: r.items.clone(),
                    linked: r.partner.is_some() && r.partner_epoch == self.topo_epoch,
                }
            }
            (
                _,
                KindTag::Pipe
                | KindTag::Condenser
                | KindTag::CoolingTower
                | KindTag::HeatPipe
                | KindTag::Radiator,
            ) => Detail::Network,
            (_, KindTag::Turret) => Detail::Turret {
                ammo: self.turrets.rows.get(row)?.ammo,
            },
            (_, KindTag::Wall) => Detail::Wall,
            (BlockKind::Ruin { of }, _) => {
                let rule = reg.ruin_rule();
                let cost = &reg.block(*of).cost;
                Detail::Ruin {
                    of: *of,
                    repair: rule.repair_cost(cost),
                    salvage: rule.salvage(cost),
                }
            }
            (_, KindTag::HeatEngine) => Detail::HeatEngine {
                output: self.engines.rows.get(row)?.output,
            },
            _ => return None,
        };
        Some(Inspection {
            block: b.block,
            origin: b.origin,
            rot: b.rot,
            health: b.health,
            max_health: def.health,
            heat: b
                .heat_net
                .and_then(|n| self.heat.nets.get(n as usize))
                .cloned(),
            fluid: b
                .fluid_net
                .and_then(|n| self.fluids.nets.get(n as usize))
                .cloned(),
            power: b
                .power_net
                .and_then(|n| self.power.nets.get(n as usize))
                .cloned(),
            detail,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use ferrustry_content::ContentRegistry;

    use super::{Command, Link, World};
    use crate::geom::{Dir, TilePos};
    use crate::scenario::{self, BaseIds};

    /// Таблица «ячейка → строка бура» поддерживается точечно: случайная стройка и снос буров
    /// (с перестановкой строк при сносе) вперемешку с другими постройками — таблица всегда
    /// верна, а поведение совпадает с эталоном, пересобирающим её каждый тик.
    #[test]
    fn drill_row_table_is_kept_in_place() {
        let reg = ContentRegistry::base().unwrap();
        let ids = BaseIds::resolve(&reg).unwrap();
        let wall = reg.block_by_name("iron_wall").unwrap();
        let mut a = World::new(&reg, 48, 48, ids.basalt, 3);
        for y in 0..48 {
            for x in 0..48 {
                if (x / 4 + y / 4) % 2 == 0 {
                    a.set_ore(TilePos::new(x, y), Some(ids.iron_vein));
                }
            }
        }
        let mut b = a.clone();
        let mut rng = crate::rng::Rng::new(11);
        let (mut placed, mut removed) = (0, 0);
        for tick in 0..400 {
            let mut cmds = Vec::new();
            for _ in 0..3 {
                let p = TilePos::new(rng.below(46) as i32, rng.below(46) as i32);
                let existing: Vec<TilePos> = a
                    .buildings()
                    .iter()
                    .filter(|(_, bl)| bl.tag == crate::buildings::KindTag::Drill)
                    .map(|(_, bl)| bl.origin)
                    .collect();
                match rng.below(5) {
                    0 | 1 if !existing.is_empty() => {
                        let at = existing[rng.below(existing.len() as u32) as usize];
                        cmds.push(Command::Remove { at });
                    }
                    2 => cmds.push(Command::Place {
                        block: wall,
                        origin: p,
                        rot: Dir::Right,
                    }),
                    _ => cmds.push(Command::Place {
                        block: ids.bore,
                        origin: p,
                        rot: Dir::Right,
                    }),
                }
            }
            let ra = a.tick(&reg, &cmds);
            b.drill_rows.0.0 = false; // эталон: таблица собирается заново каждый тик
            let rb = b.tick(&reg, &cmds);
            assert_eq!(ra, rb);
            for (c, r) in cmds.iter().zip(&ra) {
                if r.is_ok() {
                    match c {
                        Command::Remove { .. } => removed += 1,
                        _ => placed += 1,
                    }
                }
            }
            a.validate(&reg)
                .unwrap_or_else(|e| panic!("tick {tick}: {e}"));
            assert_eq!(crate::state_hash(&a), crate::state_hash(&b), "tick {tick}");
        }
        assert!(
            placed > 100 && removed > 100,
            "placed {placed}, removed {removed}"
        );
        // дать оставшимся бурам поработать: добыча копится в буферах (выгружать некуда)
        for _ in 0..600 {
            a.step(&reg);
            b.drill_rows.0.0 = false;
            b.step(&reg);
        }
        assert_eq!(crate::state_hash(&a), crate::state_hash(&b));
        assert!(a.drills.rows.iter().any(|r| r.buffer > 0), "drills worked");
    }

    /// `validate` ловит устаревшую кэшированную связь на выходе ленты.
    #[test]
    fn validate_detects_stale_cached_links() {
        let reg = ContentRegistry::base().unwrap();
        let ids = BaseIds::resolve(&reg).unwrap();
        let mut w = World::new(&reg, 20, 10, ids.basalt, 1);
        let mut cmds = vec![
            Command::Place {
                block: ids.smelter,
                origin: TilePos::new(8, 4),
                rot: Dir::Right,
            },
            Command::Place {
                block: ids.smelter,
                origin: TilePos::new(14, 4),
                rot: Dir::Right,
            },
        ];
        cmds.extend((2..8).map(|x| Command::Place {
            block: ids.conveyor,
            origin: TilePos::new(x, 4),
            rot: Dir::Right,
        }));
        scenario::build(&mut w, &reg, &cmds).unwrap();
        w.debug_fill_conveyors(ids.iron_ore, 64);
        for _ in 0..60 {
            w.step(&reg);
        }
        w.validate(&reg).unwrap();
        // испортить строку получателя в действующем кэше выхода ленты
        let seg = w
            .segments
            .records_mut()
            .find(|s| matches!(s.link, Link::Building(_)))
            .unwrap();
        seg.link = Link::Refuses;
        let err = w.validate(&reg).unwrap_err();
        assert!(err.contains("stale output link"), "{err}");
    }

    /// P-03: массовая перестройка лент с предметами уплотняет пул, не теряя и не создавая
    /// предметов; инварианты сегментов держатся на каждом шаге.
    #[test]
    fn belt_rebuilds_compact_the_pool_without_losing_items() {
        let reg = ContentRegistry::base().unwrap();
        let ids = BaseIds::resolve(&reg).unwrap();
        let mut w = World::new(&reg, 120, 70, ids.basalt, 3);
        // 60 параллельных лент по 100 тайлов (по 4 сегмента)
        let cmds: Vec<Command> = (0..60)
            .flat_map(|y| {
                (0..100).map(move |x| Command::Place {
                    block: ids.conveyor,
                    origin: TilePos::new(x, y),
                    rot: Dir::Right,
                })
            })
            .collect();
        scenario::build(&mut w, &reg, &cmds).unwrap();
        w.debug_fill_conveyors(ids.iron_ore, 64);
        let total = |w: &World| -> u64 {
            let s = w.stats();
            let i = usize::from(ids.iron_ore.0);
            s.spawned[i] - s.destroyed[i]
        };
        let peak = w.segments.pool_len();
        let mut rng = crate::rng::Rng::new(11);
        let mut shrank = false;
        for round in 0..400 {
            // снести и поставить заново случайный тайл: сегменты разбираются и собираются
            let at = TilePos::new(rng.below(100) as i32, rng.below(60) as i32);
            w.apply(&reg, &Command::Remove { at }).unwrap();
            w.apply(
                &reg,
                &Command::Place {
                    block: ids.conveyor,
                    origin: at,
                    rot: Dir::Right,
                },
            )
            .unwrap();
            w.step(&reg);
            if round % 20 == 0 {
                w.validate(&reg).unwrap();
            }
            shrank |= w.segments.pool_len() < peak;
            let on_belts: u64 = w.items_on_conveyors().iter().sum();
            assert_eq!(
                on_belts,
                total(&w),
                "round {round}: items appeared or vanished"
            );
        }
        w.validate(&reg).unwrap();
        assert!(shrank, "the pool must have been compacted at least once");
    }
}
