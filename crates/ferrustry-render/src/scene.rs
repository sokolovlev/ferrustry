//! Построение кадра из состояния мира (CPU-часть рендера, тестируется без GPU).

use ferrustry_content::{BlockId, ContentRegistry};
use ferrustry_sim::segment::TILE_LEN;
use ferrustry_sim::{Dir, TilePos, World};

use crate::atlas::{Atlas, SpriteId};
use crate::batch::SpriteBatch;
use crate::camera::Camera;

/// Спрайты контента по числовым ID (без поиска по строкам в кадре).
#[derive(Debug, Clone)]
pub struct SpriteTable {
    floors: Vec<[f32; 4]>,
    ores: Vec<[f32; 4]>,
    /// UV ресурсов по ID.
    pub items: Vec<[f32; 4]>,
    blocks: Vec<[f32; 4]>,
    /// Белый пиксель (для прямоугольников).
    pub white: SpriteId,
}

impl SpriteTable {
    /// Разрешает спрайты для всего контента; отсутствующие заменяются белым.
    pub fn new(atlas: &Atlas, reg: &ContentRegistry) -> Self {
        let white = atlas.sprite("white").unwrap_or(SpriteId(0));
        let uv = |name: String| atlas.uv(atlas.sprite(&name).unwrap_or(white));
        Self {
            floors: reg
                .floors()
                .iter()
                .map(|d| uv(format!("floor:{}", d.name)))
                .collect(),
            ores: reg
                .ores()
                .iter()
                .map(|d| uv(format!("ore:{}", d.name)))
                .collect(),
            items: reg
                .items()
                .iter()
                .map(|d| uv(format!("item:{}", d.name)))
                .collect(),
            blocks: reg
                .blocks()
                .iter()
                .map(|d| uv(format!("block:{}", d.name)))
                .collect(),
            white,
        }
    }

    /// UV постройки.
    pub fn block(&self, id: BlockId) -> [f32; 4] {
        self.blocks[usize::from(id.0)]
    }
}

/// Призрак постройки под курсором.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ghost {
    /// Тип постройки.
    pub block: BlockId,
    /// Левый нижний тайл.
    pub origin: TilePos,
    /// Поворот.
    pub rot: Dir,
    /// Можно ли здесь строить.
    pub valid: bool,
}

/// Поворот в четвертях оборота против часовой стрелки (спрайты нарисованы «вправо»).
pub fn quarter_turns(d: Dir) -> f32 {
    match d {
        Dir::Right => 0.0,
        Dir::Up => 1.0,
        Dir::Left => 2.0,
        Dir::Down => 3.0,
    }
}

/// Позиция предмета в мире: центр тайла `tile` + смещение вдоль направления ленты.
/// `off` — расстояние до выходного края тайла (0..=TILE_LEN).
pub fn item_world_pos(tile: TilePos, dir: Dir, off: u32) -> [f32; 2] {
    let (dx, dy) = dir.delta();
    let t = 0.5 - off as f32 / TILE_LEN as f32;
    [
        tile.x as f32 + 0.5 + dx as f32 * t,
        tile.y as f32 + 0.5 + dy as f32 * t,
    ]
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Заполняет пакет мира: пол и руда видимых тайлов, постройки, предметы на лентах, призраки.
pub fn build_world(
    batch: &mut SpriteBatch,
    table: &SpriteTable,
    reg: &ContentRegistry,
    world: &World,
    camera: &Camera,
    ghosts: &[Ghost],
    dir_cache: &mut Vec<Dir>,
) {
    let grid = world.grid();
    let (lo, hi) = camera.visible_rect();
    let x0 = (lo[0].floor() as i32 - 1).max(0);
    let y0 = (lo[1].floor() as i32 - 1).max(0);
    let x1 = (hi[0].ceil() as i32 + 1).min(grid.width() as i32);
    let y1 = (hi[1].ceil() as i32 + 1).min(grid.height() as i32);
    let visible = |cx: f32, cy: f32, r: f32| {
        cx + r >= lo[0] && cx - r <= hi[0] && cy + r >= lo[1] && cy - r <= hi[1]
    };

    // пол
    for y in y0..y1 {
        for x in x0..x1 {
            let p = TilePos::new(x, y);
            if let Some(f) = grid.floor(p) {
                let c = [x as f32 + 0.5, y as f32 + 0.5];
                batch.push(table.floors[usize::from(f.0)], c, [1.0, 1.0], 0.0, WHITE);
                if let Some(o) = grid.ore(p) {
                    batch.push(table.ores[usize::from(o.0)], c, [1.0, 1.0], 0.0, WHITE);
                }
            }
        }
    }

    // постройки
    for (_, b) in world.buildings().iter() {
        let s = f32::from(b.size);
        let c = [b.origin.x as f32 + s / 2.0, b.origin.y as f32 + s / 2.0];
        if !visible(c[0], c[1], s) {
            continue;
        }
        let directional = matches!(
            b.tag,
            ferrustry_sim::KindTag::Conveyor | ferrustry_sim::KindTag::Bridge
        );
        let rot = if directional {
            quarter_turns(b.rot)
        } else {
            0.0
        };
        batch.push(table.block(b.block), c, [s, s], rot, WHITE);
        if b.tag == ferrustry_sim::KindTag::Sorter
            && let Some(ins) = world.inspect(reg, b.origin)
            && let ferrustry_sim::Detail::Sorter {
                filter: Some(item), ..
            } = ins.detail
        {
            batch.push(
                table.items[usize::from(item.0)],
                c,
                [0.45, 0.45],
                0.0,
                WHITE,
            );
        }
    }

    // предметы на лентах
    for seg in world.segments() {
        let tiles = seg.tiles();
        let (Some(first), Some(last)) = (tiles.first(), tiles.last()) else {
            continue;
        };
        let reach = tiles.len() as f32 + 1.0;
        let mid = [
            (first.x + last.x) as f32 / 2.0 + 0.5,
            (first.y + last.y) as f32 / 2.0 + 0.5,
        ];
        if !visible(mid[0], mid[1], reach) {
            continue;
        }
        dir_cache.clear();
        for &t in tiles {
            let d = grid
                .building(t)
                .and_then(|id| world.buildings().get(id))
                .map_or(Dir::Right, |b| b.rot);
            dir_cache.push(d);
        }
        let l = tiles.len();
        for (item, pos) in seg.item_positions() {
            let from_end = ((pos / TILE_LEN) as usize).min(l - 1);
            let k = l - 1 - from_end;
            let off = pos - from_end as u32 * TILE_LEN;
            let p = item_world_pos(tiles[k], dir_cache[k], off);
            batch.push(
                table.items[usize::from(item.0)],
                p,
                [0.42, 0.42],
                0.0,
                WHITE,
            );
        }
    }

    // призраки
    for g in ghosts {
        let s = f32::from(reg.block(g.block).size);
        let c = [g.origin.x as f32 + s / 2.0, g.origin.y as f32 + s / 2.0];
        let tint = if g.valid {
            [0.55, 1.0, 0.6, 0.6]
        } else {
            [1.0, 0.35, 0.3, 0.6]
        };
        let rot = if matches!(
            reg.block(g.block).kind,
            ferrustry_content::BlockKind::Conveyor { .. }
                | ferrustry_content::BlockKind::Bridge { .. }
        ) {
            quarter_turns(g.rot)
        } else {
            0.0
        };
        batch.push(table.block(g.block), c, [s, s], rot, tint);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ferrustry_sim::scenario;

    #[test]
    fn item_positions_follow_belt_direction() {
        // выход тайла (off=0) — на краю по направлению движения
        assert_eq!(
            item_world_pos(TilePos::new(2, 3), Dir::Right, 0),
            [3.0, 3.5]
        );
        assert_eq!(
            item_world_pos(TilePos::new(2, 3), Dir::Up, TILE_LEN),
            [2.5, 3.0]
        );
        assert_eq!(
            item_world_pos(TilePos::new(0, 0), Dir::Left, TILE_LEN / 2),
            [0.5, 0.5]
        );
    }

    #[test]
    fn scene_culls_and_draws_everything_visible() {
        let reg = ContentRegistry::base().unwrap();
        let atlas = Atlas::build(&reg);
        let table = SpriteTable::new(&atlas, &reg);
        let (mut w, _) = scenario::vertical_slice(&reg).unwrap();
        for _ in 0..300 {
            w.step(&reg);
        }
        let mut b = SpriteBatch::default();
        let mut dirs = Vec::new();
        // камера на весь срез 64×64
        let cam = Camera::new([32.0, 32.0], 8.0, [512.0, 512.0]);
        build_world(&mut b, &table, &reg, &w, &cam, &[], &mut dirs);
        let items: u64 = w.items_on_conveyors().iter().sum();
        assert!(items > 0);
        let ore_tiles = 8;
        let expected = 64 * 64 + ore_tiles + w.buildings().len() + items as usize;
        assert_eq!(b.len(), expected);
        // камера далеко от карты — ничего
        let far = Camera::new([-500.0, -500.0], 8.0, [512.0, 512.0]);
        b.clear();
        build_world(&mut b, &table, &reg, &w, &far, &[], &mut dirs);
        assert_eq!(b.len(), 0);
    }
}
