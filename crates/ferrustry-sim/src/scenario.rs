//! Детерминированные сценарии для тестов, headless-прогонов и бенчмарков.

use ferrustry_content::{BlockId, ContentRegistry, FloorId, ItemId, OreId};

use crate::geom::{Dir, TilePos};
use crate::world::{Command, CommandError, World};

/// Ошибка построения сценария.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    /// В контенте нет нужного определения.
    #[error("content is missing `{0}`")]
    MissingContent(&'static str),
    /// Команда построения отклонена.
    #[error("scenario command {index} rejected: {err}")]
    Rejected {
        /// Номер команды.
        index: usize,
        /// Причина.
        err: CommandError,
    },
}

/// Нужные сценариям ID базового контента.
#[derive(Debug, Clone, Copy)]
pub struct BaseIds {
    /// Пол по умолчанию.
    pub basalt: FloorId,
    /// Железная руда (ресурс).
    pub iron_ore: ItemId,
    /// Железный слиток.
    pub iron_ingot: ItemId,
    /// Железная жила (месторождение).
    pub iron_vein: OreId,
    /// Конвейер.
    pub conveyor: BlockId,
    /// Бур.
    pub bore: BlockId,
    /// Плавильня.
    pub smelter: BlockId,
    /// Ветряк.
    pub wind_turbine: BlockId,
    /// Узел энергосети.
    pub power_node: BlockId,
    /// Ядро.
    pub core: BlockId,
}

impl BaseIds {
    /// Ищет ID в реестре.
    pub fn resolve(reg: &ContentRegistry) -> Result<Self, ScenarioError> {
        use ScenarioError::MissingContent as M;
        Ok(Self {
            basalt: reg.floor_by_name("basalt").ok_or(M("basalt"))?,
            iron_ore: reg.item_by_name("iron_ore").ok_or(M("iron_ore"))?,
            iron_ingot: reg.item_by_name("iron_ingot").ok_or(M("iron_ingot"))?,
            iron_vein: reg.ore_by_name("iron_vein").ok_or(M("iron_vein"))?,
            conveyor: reg.block_by_name("conveyor").ok_or(M("conveyor"))?,
            bore: reg.block_by_name("bore").ok_or(M("bore"))?,
            smelter: reg.block_by_name("smelter").ok_or(M("smelter"))?,
            wind_turbine: reg.block_by_name("wind_turbine").ok_or(M("wind_turbine"))?,
            power_node: reg.block_by_name("power_node").ok_or(M("power_node"))?,
            core: reg.block_by_name("core").ok_or(M("core"))?,
        })
    }
}

fn place(block: BlockId, x: i32, y: i32, rot: Dir) -> Command {
    Command::Place {
        block,
        origin: TilePos::new(x, y),
        rot,
    }
}

/// Применяет команды построения, требуя успеха каждой.
pub fn build(
    world: &mut World,
    reg: &ContentRegistry,
    cmds: &[Command],
) -> Result<(), ScenarioError> {
    for (index, c) in cmds.iter().enumerate() {
        world
            .apply(reg, c)
            .map_err(|err| ScenarioError::Rejected { index, err })?;
    }
    Ok(())
}

/// Вертикальный срез этапа 1: 2 бура → конвейер → плавильня (2 ветряка) → конвейер → ядро.
///
/// ```text
///  y=6           . . . . . . . . W W . . . . . . . . . .
///  y=5           . . . . . . . . W W ← ветряки над плавильней (12,5),(13,5)
///  y=4/5  бурs   B B B B . . . . S S . . . . C C C
///  y=3   лента → → → → → → → → → S S → → → → C C C
///  y=2                                     C C C
/// ```
pub fn vertical_slice(reg: &ContentRegistry) -> Result<(World, BaseIds), ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let mut w = World::new(reg, 64, 64, ids.basalt, 1);
    for y in 4..6 {
        for x in 4..8 {
            w.set_ore(TilePos::new(x, y), Some(ids.iron_vein));
        }
    }
    let mut cmds = vec![
        place(ids.bore, 4, 4, Dir::Right),
        place(ids.bore, 6, 4, Dir::Right),
    ];
    for x in 4..12 {
        cmds.push(place(ids.conveyor, x, 3, Dir::Right));
    }
    cmds.push(place(ids.smelter, 12, 3, Dir::Right));
    cmds.push(place(ids.wind_turbine, 12, 5, Dir::Right));
    cmds.push(place(ids.wind_turbine, 13, 5, Dir::Right));
    for x in 14..18 {
        cmds.push(place(ids.conveyor, x, 3, Dir::Right));
    }
    cmds.push(place(ids.core, 18, 2, Dir::Right));
    build(&mut w, reg, &cmds)?;
    Ok((w, ids))
}

/// B1: пустая карта 512×512.
pub fn bench_empty(reg: &ContentRegistry) -> Result<World, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    Ok(World::new(reg, 512, 512, ids.basalt, 1))
}

/// Команды замкнутой прямоугольной ленты `w×h` (по часовой стрелке снизу слева).
pub fn loop_commands(conveyor: BlockId, x0: i32, y0: i32, w: i32, h: i32) -> Vec<Command> {
    let (x1, y1) = (x0 + w - 1, y0 + h - 1);
    let mut c = Vec::new();
    for x in x0..x1 {
        c.push(place(conveyor, x, y0, Dir::Right));
    }
    for y in y0..y1 {
        c.push(place(conveyor, x1, y, Dir::Up));
    }
    for x in (x0 + 1..=x1).rev() {
        c.push(place(conveyor, x, y1, Dir::Left));
    }
    for y in (y0 + 1..=y1).rev() {
        c.push(place(conveyor, x0, y, Dir::Down));
    }
    c
}

/// B2: замкнутые ленты общей длиной не меньше `conveyors`, заполненные наполовину
/// (предметы движутся постоянно).
pub fn bench_conveyor_loops(reg: &ContentRegistry, conveyors: u32) -> Result<World, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let side = 50; // периметр 196 конвейеров
    let per_loop = 4 * (side - 1);
    let loops = conveyors.div_ceil(per_loop as u32) as i32;
    let cols = (f64::from(loops).sqrt().ceil() as i32).max(1);
    let rows = (loops + cols - 1) / cols;
    let pitch = side + 2;
    let mut w = World::new(
        reg,
        (cols * pitch) as u32,
        (rows * pitch) as u32,
        ids.basalt,
        1,
    );
    for i in 0..loops {
        let (cx, cy) = (i % cols, i / cols);
        build(
            &mut w,
            reg,
            &loop_commands(ids.conveyor, cx * pitch + 1, cy * pitch + 1, side, side),
        )?;
    }
    w.debug_fill_conveyors(ids.iron_ore, 2 * crate::segment::ITEM_SPACING);
    Ok(w)
}

/// B2-blocked: прямые ленты по 100 тайлов, упирающиеся в пустоту и забитые полностью.
pub fn bench_blocked_lines(reg: &ContentRegistry, conveyors: u32) -> Result<World, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let len = 100;
    let lines = conveyors.div_ceil(len as u32) as i32;
    let mut w = World::new(reg, (len + 4) as u32, (lines * 2 + 2) as u32, ids.basalt, 1);
    for l in 0..lines {
        let cmds: Vec<_> = (0..len)
            .map(|x| place(ids.conveyor, x, l * 2, Dir::Right))
            .collect();
        build(&mut w, reg, &cmds)?;
    }
    w.debug_fill_conveyors(ids.iron_ore, crate::segment::ITEM_SPACING);
    Ok(w)
}

/// B2-comb: «гребёнки» — основная лента длиной 99 в ядро и ветки по 6 тайлов через каждые
/// 3 тайла, питаемые бурами и вливающиеся в основную ленту сбоку. Основная лента насыщается,
/// ветки частично блокируются — смесь движения и заторов, типичная для реальной базы.
pub fn bench_combs(reg: &ContentRegistry, conveyors: u32) -> Result<World, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let main_len = 99;
    let branch_len = 6;
    let branches = main_len / 3;
    let per_comb = (main_len + branches * branch_len) as u32;
    let combs = conveyors.div_ceil(per_comb) as i32;
    let pitch = 11;
    let mut w = World::new(
        reg,
        (main_len + 4) as u32,
        (combs * pitch + 2) as u32,
        ids.basalt,
        1,
    );
    for c in 0..combs {
        let y = c * pitch + 9; // основная лента
        let mut cmds: Vec<_> = (0..main_len)
            .map(|x| place(ids.conveyor, x, y, Dir::Right))
            .collect();
        cmds.push(place(ids.core, main_len, y - 1, Dir::Right));
        for b in 0..branches {
            let bx = b * 3;
            for k in 0..branch_len {
                cmds.push(place(ids.conveyor, bx, y - branch_len + k, Dir::Up));
            }
            let by = y - branch_len - 2;
            for dy in 0..2 {
                for dx in 0..2 {
                    w.set_ore(TilePos::new(bx + dx, by + dy), Some(ids.iron_vein));
                }
            }
            cmds.push(place(ids.bore, bx, by, Dir::Right));
        }
        build(&mut w, reg, &cmds)?;
    }
    Ok(w)
}

/// B3 «мегабаза»: ряды производственных ячеек на общей шине.
///
/// Ячейка (ширина 8): 2 бура на железной жиле → лента с поворотом → плавильня (2 ветряка) →
/// лента вверх, вливается сбоку в шину ряда. Шина (12 ячеек = 96 тайлов) идёт в ядро и при
/// 12 ячейках насыщается. Около 240 конвейеров, 24 бура, 12 плавилен, 24 ветряка на ряд.
pub fn bench_megabase(reg: &ContentRegistry, conveyors: u32) -> Result<World, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let cells = 12;
    let cell_w = 8;
    let row_h = 14;
    let per_row = 240;
    let rows = conveyors.div_ceil(per_row).max(1) as i32;
    let bus_len = cells * cell_w;
    let mut w = World::new(
        reg,
        (bus_len + 4) as u32,
        (rows * row_h + 2) as u32,
        ids.basalt,
        1,
    );
    for r in 0..rows {
        let y0 = r * row_h;
        let bus_y = y0 + 12;
        let mut cmds = Vec::new();
        for c in 0..cells {
            let x0 = c * cell_w;
            for dy in 0..2 {
                for dx in 0..4 {
                    w.set_ore(TilePos::new(x0 + dx, y0 + dy), Some(ids.iron_vein));
                }
            }
            cmds.push(place(ids.bore, x0, y0, Dir::Right));
            cmds.push(place(ids.bore, x0 + 2, y0, Dir::Right));
            for x in x0..x0 + 4 {
                cmds.push(place(ids.conveyor, x, y0 + 2, Dir::Right));
            }
            for y in y0 + 2..y0 + 5 {
                cmds.push(place(ids.conveyor, x0 + 4, y, Dir::Up));
            }
            cmds.push(place(ids.smelter, x0 + 4, y0 + 5, Dir::Right));
            cmds.push(place(ids.wind_turbine, x0 + 6, y0 + 5, Dir::Right));
            cmds.push(place(ids.wind_turbine, x0 + 6, y0 + 6, Dir::Right));
            for y in y0 + 7..bus_y {
                cmds.push(place(ids.conveyor, x0 + 4, y, Dir::Up));
            }
        }
        for x in 0..bus_len {
            cmds.push(place(ids.conveyor, x, bus_y, Dir::Right));
        }
        cmds.push(place(ids.core, bus_len, bus_y - 1, Dir::Right));
        build(&mut w, reg, &cmds)?;
    }
    Ok(w)
}

/// Генератор случайной стройки и сноса — для тестов, replay и стресс-прогонов.
#[derive(Debug, Clone)]
pub struct RandomBuilder {
    rng: crate::rng::Rng,
    blocks: Vec<BlockId>,
    size: u32,
}

impl RandomBuilder {
    /// Мир `size×size` с узором железной руды и генератор команд с заданным seed.
    pub fn new(
        reg: &ContentRegistry,
        size: u32,
        seed: u64,
    ) -> Result<(World, Self), ScenarioError> {
        let ids = BaseIds::resolve(reg)?;
        let mut w = World::new(reg, size, size, ids.basalt, seed);
        for y in 0..size as i32 {
            for x in 0..size as i32 {
                if (x * 7 + y * 3) % 5 == 0 {
                    w.set_ore(TilePos::new(x, y), Some(ids.iron_vein));
                }
            }
        }
        let mut blocks = vec![
            ids.conveyor,
            ids.conveyor,
            ids.conveyor,
            ids.bore,
            ids.core,
            ids.smelter,
            ids.wind_turbine,
        ];
        for extra in [
            "splitter",
            "sorter",
            "bridge_conveyor",
            "battery",
            "power_node",
        ] {
            if let Some(b) = reg.block_by_name(extra) {
                blocks.push(b);
            }
        }
        Ok((
            w,
            Self {
                rng: crate::rng::Rng::new(seed),
                blocks,
                size,
            },
        ))
    }

    /// Команды для тика `tick` (каждый третий тик — одна постройка или снос).
    pub fn commands(&mut self, tick: u64) -> Vec<Command> {
        let mut cmds = Vec::new();
        if tick.is_multiple_of(3) {
            let p = TilePos::new(
                self.rng.below(self.size) as i32,
                self.rng.below(self.size) as i32,
            );
            if self.rng.below(4) == 0 {
                cmds.push(Command::Remove { at: p });
            } else {
                let block = self.blocks[self.rng.below(self.blocks.len() as u32) as usize];
                cmds.push(Command::Place {
                    block,
                    origin: p,
                    rot: Dir::ALL[self.rng.below(4) as usize],
                });
            }
        }
        cmds
    }
}

/// Демо-карта для клиента: базальт с участками пепла и шлаковыми озёрами, пятна железа и угля,
/// стартовая база (вертикальный срез) в центре.
pub fn demo_map(reg: &ContentRegistry, seed: u64) -> Result<World, ScenarioError> {
    let ids = BaseIds::resolve(reg)?;
    let ash = reg
        .floor_by_name("ash")
        .ok_or(ScenarioError::MissingContent("ash"))?;
    let slag = reg
        .floor_by_name("slag_pool")
        .ok_or(ScenarioError::MissingContent("slag_pool"))?;
    let carbon = reg
        .ore_by_name("carbon_seam")
        .ok_or(ScenarioError::MissingContent("carbon_seam"))?;
    let (w, h) = (192i32, 128i32);
    let mut world = World::new(reg, w as u32, h as u32, ids.basalt, seed);
    let mut rng = crate::rng::Rng::new(seed ^ 0xD3A0);
    let (cx0, cy0) = (w / 2 - 10, h / 2 - 4);
    let near_base = |x: i32, y: i32| (x - cx0 - 10).abs() < 18 && (y - cy0 - 2).abs() < 12;
    let blob = |world: &mut World,
                rng: &mut crate::rng::Rng,
                f: &mut dyn FnMut(&mut World, TilePos),
                r_min: u32,
                r_max: u32| {
        let (bx, by) = (rng.below(w as u32) as i32, rng.below(h as u32) as i32);
        let r = (r_min + rng.below(r_max - r_min + 1)) as i32;
        for y in by - r..=by + r {
            for x in bx - r..=bx + r {
                let d2 = (x - bx).pow(2) + (y - by).pow(2);
                let jitter = rng.below(3) as i32;
                if d2 <= (r - 1 + jitter).pow(2)
                    && x >= 0
                    && y >= 0
                    && x < w
                    && y < h
                    && !near_base(x, y)
                {
                    f(world, TilePos::new(x, y));
                }
            }
        }
    };
    for _ in 0..14 {
        blob(
            &mut world,
            &mut rng,
            &mut |wd, p| wd.set_floor(p, ash),
            4,
            9,
        );
    }
    for _ in 0..5 {
        blob(
            &mut world,
            &mut rng,
            &mut |wd, p| wd.set_floor(p, slag),
            2,
            4,
        );
    }
    for _ in 0..10 {
        let ore = ids.iron_vein;
        blob(
            &mut world,
            &mut rng,
            &mut |wd, p| wd.set_ore(p, Some(ore)),
            2,
            4,
        );
    }
    for _ in 0..6 {
        blob(
            &mut world,
            &mut rng,
            &mut |wd, p| wd.set_ore(p, Some(carbon)),
            2,
            3,
        );
    }
    // стартовая база — вертикальный срез, сдвинутый в центр
    for y in 4..6 {
        for x in 4..8 {
            world.set_ore(TilePos::new(cx0 + x, cy0 + y), Some(ids.iron_vein));
        }
    }
    let at = |x: i32, y: i32| (cx0 + x, cy0 + y);
    let mut cmds = Vec::new();
    for (x, y) in [at(4, 4), at(6, 4)] {
        cmds.push(place(ids.bore, x, y, Dir::Right));
    }
    for x in 4..12 {
        let (px, py) = at(x, 3);
        cmds.push(place(ids.conveyor, px, py, Dir::Right));
    }
    let (sx, sy) = at(12, 3);
    cmds.push(place(ids.smelter, sx, sy, Dir::Right));
    for x in [12, 13] {
        let (tx, ty) = at(x, 5);
        cmds.push(place(ids.wind_turbine, tx, ty, Dir::Right));
    }
    for x in 14..18 {
        let (px, py) = at(x, 3);
        cmds.push(place(ids.conveyor, px, py, Dir::Right));
    }
    let (kx, ky) = at(18, 2);
    cmds.push(place(ids.core, kx, ky, Dir::Right));
    build(&mut world, reg, &cmds)?;
    // экономика: стартовая база бесплатна, дальше стройка стоит ресурсов из ядра
    world.set_rules(crate::world::Rules {
        infinite_resources: false,
    });
    world.grant(reg, ids.iron_ingot, STARTER_INGOTS);
    Ok(world)
}

/// Стартовый запас слитков на демо-карте.
pub const STARTER_INGOTS: u32 = 150;
