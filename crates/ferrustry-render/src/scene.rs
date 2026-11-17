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
    units: Vec<[f32; 4]>,
    projectile: [f32; 4],
    spit: [f32; 4],
    barrel: [f32; 4],
    white_uv: [f32; 4],
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
            units: reg
                .units()
                .iter()
                .map(|d| uv(format!("unit:{}", d.name)))
                .collect(),
            projectile: uv("projectile".into()),
            spit: uv("spit".into()),
            barrel: uv("turret_barrel".into()),
            white_uv: atlas.uv(white),
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

/// Полоска прочности: тёмная подложка и цветная часть (зелёная → красная).
fn health_bar(batch: &mut SpriteBatch, white: [f32; 4], c: [f32; 2], w: f32, hp: u32, max: u32) {
    let k = (hp as f32 / max.max(1) as f32).clamp(0.0, 1.0);
    let h = 0.09;
    batch.push(white, c, [w, h], 0.0, [0.02, 0.02, 0.03, 0.85]);
    let fill = [1.0 - k, 0.25 + 0.6 * k, 0.15, 1.0];
    batch.push(
        white,
        [c[0] - w * (1.0 - k) / 2.0, c[1]],
        [w * k, h],
        0.0,
        fill,
    );
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
/// Свечение разряда дуговой турели (линейные цвета).
const ARC_GLOW: [f32; 4] = [0.30, 0.55, 1.0, 0.75];

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
            ferrustry_sim::KindTag::Conveyor
                | ferrustry_sim::KindTag::Bridge
                | ferrustry_sim::KindTag::Depot
        );
        let rot = if directional {
            quarter_turns(b.rot)
        } else {
            0.0
        };
        // перегрев (выше порога замедления) окрашивает постройку в красный
        let tint = match world.building_temperature(b) {
            Some(t) if t > ferrustry_sim::thermal::THROTTLE_FROM => {
                let k = (t - ferrustry_sim::thermal::THROTTLE_FROM) as f32
                    / (1000 - ferrustry_sim::thermal::THROTTLE_FROM) as f32;
                [1.0, 1.0 - 0.65 * k, 1.0 - 0.75 * k, 1.0]
            }
            _ => WHITE,
        };
        batch.push(table.block(b.block), c, [s, s], rot, tint);
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

    // юниты, снаряды, стволы турелей, полоски прочности
    let sub = ferrustry_sim::combat::SUB as f32;
    let white_uv = table.white_uv;
    for (kind, x, y, hp) in world.units() {
        let (fx, fy) = (x as f32 / sub, y as f32 / sub);
        if !visible(fx, fy, 1.0) {
            continue;
        }
        let def = reg.unit(kind);
        let size = def.radius as f32 / sub * 2.4;
        batch.push(
            table.units[usize::from(kind.0)],
            [fx, fy],
            [size, size],
            0.0,
            WHITE,
        );
        if hp < def.health {
            health_bar(
                batch,
                white_uv,
                [fx, fy + size * 0.55],
                size,
                hp,
                def.health,
            );
        }
    }
    for (_, b) in world.buildings().iter() {
        let s = f32::from(b.size);
        let c = [b.origin.x as f32 + s / 2.0, b.origin.y as f32 + s / 2.0];
        if !visible(c[0], c[1], s) {
            continue;
        }
        if let Some((bx, by)) = world.arc_beam(b) {
            // разряд: отрезок от центра турели до цели (поворот спрайта «белого пикселя»)
            let (ex, ey) = (bx as f32 / sub, by as f32 / sub);
            let (dx, dy) = (ex - c[0], ey - c[1]);
            let len = (dx * dx + dy * dy).sqrt();
            let turns = dy.atan2(dx) / std::f32::consts::FRAC_PI_2;
            let mid = [(c[0] + ex) / 2.0, (c[1] + ey) / 2.0];
            batch.push(white_uv, mid, [len, 0.14], turns, ARC_GLOW);
            batch.push(white_uv, mid, [len, 0.05], turns, WHITE);
        }
        if let Some((ax, ay)) = world.turret_aim(b)
            && !matches!(
                reg.block(b.block).kind,
                ferrustry_content::BlockKind::ArcTurret { .. }
            )
        {
            let turns = if ax == 0 && ay == 0 {
                0.0
            } else {
                (ay as f32).atan2(ax as f32) / std::f32::consts::FRAC_PI_2
            };
            batch.push(table.barrel, c, [s, s], turns, WHITE);
        }
        let max = reg.block(b.block).health;
        if b.health() < max {
            health_bar(
                batch,
                white_uv,
                [c[0], c[1] + s * 0.5 + 0.12],
                s * 0.9,
                b.health(),
                max,
            );
        }
    }
    for (x, y, hostile) in world.projectiles() {
        let (fx, fy) = (x as f32 / sub, y as f32 / sub);
        if visible(fx, fy, 0.5) {
            let (sprite, size) = if hostile {
                (table.spit, 0.3)
            } else {
                (table.projectile, 0.25)
            };
            batch.push(sprite, [fx, fy], [size, size], 0.0, WHITE);
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
                | ferrustry_content::BlockKind::Depot { .. }
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
