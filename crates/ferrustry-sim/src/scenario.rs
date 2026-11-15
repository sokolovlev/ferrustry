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
