//! Процедурный атлас спрайтов.
//!
//! Вся графика v0 рисуется кодом в палитре брендбука Ferrustry (Industrial Sci-Fi: тёмный
//! металл, сигнальный оранжевый, срезанные углы). Это собственный арт проекта без внешних
//! ассетов; позже его заменят нарисованные спрайты (лицензия ассетов — CC BY-SA 4.0, D-12).

use std::collections::HashMap;

use ferrustry_content::ContentRegistry;

use crate::font::{CHARSET, GLYPH_H, GLYPH_W, glyph};

/// Пикселей на тайл в атласе.
pub const TILE_PX: u32 = 32;

/// Цвета брендбука (sRGB).
pub mod palette {
    /// Forge Orange #FF7A00.
    pub const FORGE_ORANGE: [u8; 4] = [0xFF, 0x7A, 0x00, 0xFF];
    /// Molten Gold #FFAA33.
    pub const MOLTEN_GOLD: [u8; 4] = [0xFF, 0xAA, 0x33, 0xFF];
    /// Steel White #E6E6E6.
    pub const STEEL_WHITE: [u8; 4] = [0xE6, 0xE6, 0xE6, 0xFF];
    /// Titanium #8A8FA3.
    pub const TITANIUM: [u8; 4] = [0x8A, 0x8F, 0xA3, 0xFF];
    /// Gunmetal #34384A.
    pub const GUNMETAL: [u8; 4] = [0x34, 0x38, 0x4A, 0xFF];
    /// Deep Space #1B1F2A.
    pub const DEEP_SPACE: [u8; 4] = [0x1B, 0x1F, 0x2A, 0xFF];
    /// Тёмная окантовка (производный от Gunmetal).
    pub const SHADOW: [u8; 4] = [0x20, 0x23, 0x2E, 0xFF];
}
use palette::*;

/// Идентификатор спрайта в атласе.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpriteId(pub u16);

/// Прямоугольник спрайта в пикселях атласа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteRect {
    /// X левого края.
    pub x: u32,
    /// Y верхнего края.
    pub y: u32,
    /// Ширина.
    pub w: u32,
    /// Высота.
    pub h: u32,
}

/// RGBA8-холст для рисования спрайта.
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<[u8; 4]>,
}

fn hash2(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

fn shade(c: [u8; 4], d: i32) -> [u8; 4] {
    let f = |v: u8| (i32::from(v) + d).clamp(0, 255) as u8;
    [f(c[0]), f(c[1]), f(c[2]), c[3]]
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![[0; 4]; (w * h) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            self.px[(y as u32 * self.w + x as u32) as usize] = c;
        }
    }

    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn ring(&mut self, cx: f32, cy: f32, r0: f32, r1: f32, c: [u8; 4]) {
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let d = dx * dx + dy * dy;
                if d >= r0 * r0 && d <= r1 * r1 {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn thick_line(&mut self, a: (f32, f32), b: (f32, f32), r: f32, c: [u8; 4]) {
        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) * 2.0)
            .ceil()
            .max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for yy in (y - r).floor() as i32..=(y + r).ceil() as i32 {
                for xx in (x - r).floor() as i32..=(x + r).ceil() as i32 {
                    let (dx, dy) = (xx as f32 + 0.5 - x, yy as f32 + 0.5 - y);
                    if dx * dx + dy * dy <= r * r {
                        self.set(xx, yy, c);
                    }
                }
            }
        }
    }

    /// Панель постройки: металл, тёмная окантовка, светлый внутренний кант, срезанные углы.
    fn frame(&mut self, border: i32, chamfer: i32) {
        let (w, h) = (self.w as i32, self.h as i32);
        for y in 0..h {
            for x in 0..w {
                let n = (hash2(x as u32 / 2, y as u32 / 2, 7) % 9) as i32 - 4;
                self.set(x, y, shade(GUNMETAL, n));
            }
        }
        for y in 0..h {
            for x in 0..w {
                let edge = x.min(y).min(w - 1 - x).min(h - 1 - y);
                if edge < border {
                    self.set(x, y, SHADOW);
                } else if edge == border {
                    self.set(x, y, shade(TITANIUM, -30));
                }
            }
        }
        // срезанные углы
        for y in 0..chamfer {
            for x in 0..chamfer - y {
                for (xx, yy) in [
                    (x, y),
                    (w - 1 - x, y),
                    (x, h - 1 - y),
                    (w - 1 - x, h - 1 - y),
                ] {
                    self.set(xx, yy, [0; 4]);
                }
            }
        }
    }
}

/// Цвет ресурса по имени (для руд и предметов).
fn item_color(name: &str) -> [u8; 4] {
    match name {
        "iron_ore" => [0xB4, 0x60, 0x2E, 0xFF],
        "carbon" => [0x24, 0x25, 0x2B, 0xFF],
        "iron_ingot" => [0xC9, 0xCD, 0xD6, 0xFF],
        "steel" => [0x7C, 0x8F, 0xB0, 0xFF],
        _ => MOLTEN_GOLD,
    }
}

fn draw_floor(name: &str) -> Canvas {
    let mut c = Canvas::new(TILE_PX, TILE_PX);
    let (base, seed) = match name {
        "basalt" => ([0x2A, 0x2E, 0x3A, 0xFF], 1),
        "ash" => ([0x47, 0x48, 0x50, 0xFF], 2),
        "slag_pool" => ([0x3A, 0x1C, 0x14, 0xFF], 3),
        _ => ([0x40, 0x40, 0x40, 0xFF], 4),
    };
    for y in 0..TILE_PX {
        for x in 0..TILE_PX {
            let n = (hash2(x / 4, y / 4, seed) % 11) as i32 - 5;
            let speck = hash2(x, y, seed + 100).is_multiple_of(37);
            c.set(x as i32, y as i32, shade(base, if speck { -12 } else { n }));
        }
    }
    if name == "slag_pool" {
        for i in 0..3 {
            let y0 = 6.0 + i as f32 * 9.0;
            c.thick_line((2.0, y0), (30.0, y0 + 4.0), 0.7, shade(FORGE_ORANGE, -60));
        }
    }
    c
}

fn draw_ore(item: &str) -> Canvas {
    let mut c = Canvas::new(TILE_PX, TILE_PX);
    let col = item_color(item);
    for (i, &(x, y, r)) in [
        (9.0, 10.0, 4.5),
        (21.0, 8.0, 3.5),
        (15.0, 20.0, 5.0),
        (25.0, 24.0, 3.0),
        (6.0, 25.0, 2.5),
    ]
    .iter()
    .enumerate()
    {
        c.disc(x, y, r, shade(col, -10 - i as i32 * 3));
        c.disc(x - r * 0.3, y - r * 0.3, r * 0.45, shade(col, 35));
    }
    c
}

fn draw_item(name: &str) -> Canvas {
    let mut c = Canvas::new(12, 12);
    let col = item_color(name);
    if name.ends_with("_ingot") || name == "steel" {
        c.fill(1, 3, 10, 6, shade(col, -40));
        c.fill(2, 3, 8, 4, col);
        c.fill(3, 4, 5, 1, shade(col, 40));
    } else {
        c.disc(6.0, 6.0, 5.0, shade(col, -35));
        c.disc(6.0, 6.0, 4.0, col);
        c.disc(4.8, 4.8, 1.6, shade(col, 45));
    }
    c
}

fn draw_block(name: &str, size: u8) -> Canvas {
    let px = u32::from(size) * TILE_PX;
    let mut c = Canvas::new(px, px);
    let (w, h) = (px as f32, px as f32);
    match name {
        "conveyor" => {
            c.fill(0, 0, 32, 32, GUNMETAL);
            c.fill(0, 0, 32, 4, SHADOW);
            c.fill(0, 28, 32, 4, SHADOW);
            c.fill(0, 4, 32, 1, shade(TITANIUM, -35));
            c.fill(0, 27, 32, 1, shade(TITANIUM, -35));
            for x0 in [6.0, 17.0] {
                c.thick_line((x0, 9.0), (x0 + 7.0, 16.0), 1.4, TITANIUM);
                c.thick_line((x0, 23.0), (x0 + 7.0, 16.0), 1.4, TITANIUM);
            }
        }
        "bore" => {
            c.frame(2, 5);
            c.disc(w / 2.0, h / 2.0, 19.0, SHADOW);
            c.ring(w / 2.0, h / 2.0, 17.0, 19.0, shade(TITANIUM, -20));
            for (dx, dy) in [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)] {
                c.thick_line(
                    (w / 2.0, h / 2.0),
                    (w / 2.0 + dx * 14.0, h / 2.0 + dy * 14.0),
                    2.2,
                    TITANIUM,
                );
            }
            c.disc(w / 2.0, h / 2.0, 4.0, STEEL_WHITE);
            c.fill(52, 7, 5, 3, FORGE_ORANGE);
        }
        "smelter" | "steel_forge" => {
            c.frame(2, 5);
            for i in 0..4 {
                c.fill(12 + i * 11, 8, 6, 3, SHADOW);
            }
            c.fill(14, 26, 36, 26, SHADOW);
            let glow = if name == "smelter" {
                FORGE_ORANGE
            } else {
                [0x7C, 0x8F, 0xB0, 0xFF]
            };
            for y in 0..22 {
                let col = if y < 8 { MOLTEN_GOLD } else { glow };
                c.fill(16, 28 + y, 32, 1, shade(col, -(y * 3)));
            }
            c.fill(14, 24, 36, 2, shade(TITANIUM, -20));
        }
        "wind_turbine" => {
            c.frame(2, 3);
            let (cx, cy) = (16.0, 16.0);
            for a in [90.0f32, 210.0, 330.0] {
                let r = a.to_radians();
                c.thick_line(
                    (cx, cy),
                    (cx + r.cos() * 12.0, cy - r.sin() * 12.0),
                    1.8,
                    STEEL_WHITE,
                );
            }
            c.disc(cx, cy, 3.5, MOLTEN_GOLD);
        }
        "power_node" => {
            c.fill(8, 8, 16, 16, SHADOW);
            c.fill(9, 9, 14, 14, GUNMETAL);
            for (x, y) in [(15, 3), (15, 25), (3, 15), (25, 15)] {
                c.fill(x, y, 2, 4, TITANIUM);
                c.fill(y, x, 4, 2, TITANIUM);
            }
            c.disc(16.0, 16.0, 4.5, MOLTEN_GOLD);
            c.disc(15.0, 15.0, 1.5, STEEL_WHITE);
        }
        "core" => {
            c.frame(4, 9);
            let (cx, cy) = (w / 2.0, h / 2.0);
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                c.thick_line(
                    (cx + a.cos() * 28.0, cy + a.sin() * 28.0),
                    (cx + a.cos() * 35.0, cy + a.sin() * 35.0),
                    3.0,
                    TITANIUM,
                );
            }
            c.ring(cx, cy, 22.0, 30.0, TITANIUM);
            c.disc(cx, cy, 22.0, SHADOW);
            // «F» бренда Ferrustry
            c.fill(38, 34, 6, 28, FORGE_ORANGE);
            c.fill(38, 34, 20, 6, FORGE_ORANGE);
            c.fill(38, 45, 15, 5, FORGE_ORANGE);
            for (x, y) in [(10, 10), (82, 10), (10, 82), (82, 82)] {
                c.fill(x, y, 4, 4, FORGE_ORANGE);
            }
        }
        _ => {
            c.frame(2, 4);
            c.thick_line(
                (6.0, 6.0),
                (w - 6.0, h - 6.0),
                2.0,
                [0xFF, 0x00, 0xFF, 0xFF],
            );
            c.thick_line(
                (w - 6.0, 6.0),
                (6.0, h - 6.0),
                2.0,
                [0xFF, 0x00, 0xFF, 0xFF],
            );
        }
    }
    c
}

/// Атлас: RGBA8-пиксели и прямоугольники спрайтов.
#[derive(Debug, Clone)]
pub struct Atlas {
    /// Ширина в пикселях.
    pub width: u32,
    /// Высота в пикселях.
    pub height: u32,
    /// Пиксели RGBA8 (sRGB), построчно сверху вниз.
    pub pixels: Vec<u8>,
    rects: Vec<SpriteRect>,
    names: HashMap<String, SpriteId>,
}

impl Atlas {
    /// Строит атлас для контента: полы, руды, предметы, постройки, глифы, белый пиксель.
    pub fn build(reg: &ContentRegistry) -> Self {
        let mut items: Vec<(String, Canvas)> = Vec::new();
        items.push(("white".into(), {
            let mut c = Canvas::new(4, 4);
            c.fill(0, 0, 4, 4, [255; 4]);
            c
        }));
        for f in reg.floors() {
            items.push((format!("floor:{}", f.name), draw_floor(&f.name)));
        }
        for o in reg.ores() {
            items.push((format!("ore:{}", o.name), draw_ore(&reg.item(o.item).name)));
        }
        for i in reg.items() {
            items.push((format!("item:{}", i.name), draw_item(&i.name)));
        }
        for b in reg.blocks() {
            items.push((format!("block:{}", b.name), draw_block(&b.name, b.size)));
        }
        for ch in CHARSET.chars() {
            let mut c = Canvas::new(GLYPH_W, GLYPH_H);
            if let Some(rows) = glyph(ch) {
                for (y, row) in rows.iter().enumerate() {
                    for (x, p) in row.chars().enumerate() {
                        if p == '#' {
                            c.set(x as i32, y as i32, [255; 4]);
                        }
                    }
                }
            }
            items.push((format!("glyph:{ch}"), c));
        }

        // Полочная упаковка: от высоких к низким, с отступом 2 px от просачивания.
        let width = 512u32;
        let pad = 2u32;
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(items[i].1.h), i));
        let mut rects = vec![
            SpriteRect {
                x: 0,
                y: 0,
                w: 0,
                h: 0
            };
            items.len()
        ];
        let (mut x, mut y, mut shelf) = (pad, pad, 0u32);
        for &i in &order {
            let c = &items[i].1;
            if x + c.w + pad > width {
                x = pad;
                y += shelf + pad;
                shelf = 0;
            }
            rects[i] = SpriteRect {
                x,
                y,
                w: c.w,
                h: c.h,
            };
            x += c.w + pad;
            shelf = shelf.max(c.h);
        }
        let height = (y + shelf + pad).next_power_of_two();
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut names = HashMap::new();
        for (i, (name, c)) in items.iter().enumerate() {
            let r = rects[i];
            for yy in 0..c.h {
                for xx in 0..c.w {
                    let src = c.px[(yy * c.w + xx) as usize];
                    let dst = (((r.y + yy) * width + r.x + xx) * 4) as usize;
                    pixels[dst..dst + 4].copy_from_slice(&src);
                }
            }
            names.insert(name.clone(), SpriteId(i as u16));
        }
        Self {
            width,
            height,
            pixels,
            rects,
            names,
        }
    }

    /// Спрайт по имени (`floor:basalt`, `block:core`, `glyph:A`, `white`...).
    pub fn sprite(&self, name: &str) -> Option<SpriteId> {
        self.names.get(name).copied()
    }

    /// Прямоугольник спрайта.
    pub fn rect(&self, id: SpriteId) -> SpriteRect {
        self.rects[usize::from(id.0)]
    }

    /// UV-координаты спрайта (u0, v0, u1, v1), вдавленные на полпикселя от краёв.
    pub fn uv(&self, id: SpriteId) -> [f32; 4] {
        let r = self.rect(id);
        let (w, h) = (self.width as f32, self.height as f32);
        [
            (r.x as f32 + 0.5) / w,
            (r.y as f32 + 0.5) / h,
            ((r.x + r.w) as f32 - 0.5) / w,
            ((r.y + r.h) as f32 - 0.5) / h,
        ]
    }

    /// Число спрайтов.
    pub fn len(&self) -> usize {
        self.rects.len()
    }

    /// Пуст ли атлас.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn atlas_contains_all_content_and_no_overlaps() {
        let reg = ContentRegistry::base().unwrap();
        let a = Atlas::build(&reg);
        for b in reg.blocks() {
            let id = a.sprite(&format!("block:{}", b.name)).unwrap();
            assert_eq!(a.rect(id).w, u32::from(b.size) * TILE_PX);
        }
        for i in reg.items() {
            assert!(a.sprite(&format!("item:{}", i.name)).is_some());
        }
        assert!(a.sprite("glyph:7").is_some() && a.sprite("white").is_some());
        let rects: Vec<SpriteRect> = (0..a.len()).map(|i| a.rect(SpriteId(i as u16))).collect();
        for (i, r) in rects.iter().enumerate() {
            assert!(r.x + r.w <= a.width && r.y + r.h <= a.height);
            for q in &rects[i + 1..] {
                let disjoint =
                    r.x + r.w <= q.x || q.x + q.w <= r.x || r.y + r.h <= q.y || q.y + q.h <= r.y;
                assert!(disjoint, "{r:?} overlaps {q:?}");
            }
        }
    }

    #[test]
    fn atlas_is_deterministic() {
        let reg = ContentRegistry::base().unwrap();
        assert_eq!(Atlas::build(&reg).pixels, Atlas::build(&reg).pixels);
    }
}
