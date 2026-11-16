//! Пакеты спрайтов: плоский массив данных экземпляров для instanced-отрисовки.

use crate::atlas::{Atlas, SpriteId};
use crate::font::{GLYPH_H, GLYPH_W};

/// Число `f32` на экземпляр: центр (2), размер (2), UV (4), цвет (4), поворот в четвертях (1).
pub const INSTANCE_FLOATS: usize = 13;

/// Набор спрайтов одного слоя.
#[derive(Debug, Default, Clone)]
pub struct SpriteBatch {
    data: Vec<f32>,
}

impl SpriteBatch {
    /// Очищает пакет, сохраняя ёмкость (без аллокаций между кадрами).
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Число спрайтов.
    pub fn len(&self) -> usize {
        self.data.len() / INSTANCE_FLOATS
    }

    /// Пуст ли пакет.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Данные экземпляров.
    pub fn floats(&self) -> &[f32] {
        &self.data
    }

    /// Добавляет спрайт.
    #[inline]
    pub fn push(
        &mut self,
        uv: [f32; 4],
        center: [f32; 2],
        size: [f32; 2],
        quarter_turns: f32,
        color: [f32; 4],
    ) {
        self.data.extend_from_slice(&[
            center[0],
            center[1],
            size[0],
            size[1],
            uv[0],
            uv[1],
            uv[2],
            uv[3],
            color[0],
            color[1],
            color[2],
            color[3],
            quarter_turns,
        ]);
    }
}

/// Цвет sRGB 0..255 → линейный множитель 0..1 для шейдера.
///
/// Рендер пишет в sRGB-цели, а выборка из sRGB-атласа даёт линейные значения, поэтому оттенок
/// тоже должен быть линейным — иначе гамма применится дважды и цвета бренда «поплывут»
/// (Forge Orange #FF7A00 превратился бы в #FFB800).
pub fn rgba(c: [u8; 4]) -> [f32; 4] {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [lin(c[0]), lin(c[1]), lin(c[2]), f32::from(c[3]) / 255.0]
}

/// Слой интерфейса в пикселях экрана (0,0 — левый верхний угол).
#[derive(Debug, Default, Clone)]
pub struct Hud {
    /// Спрайты (внутри — координаты с Y вверх от нижнего края).
    pub batch: SpriteBatch,
    /// Высота экрана в пикселях.
    pub height: f32,
}

impl Hud {
    /// Начинает кадр для экрана высотой `height`.
    pub fn begin(&mut self, height: f32) {
        self.batch.clear();
        self.height = height;
    }

    /// Прямоугольник `[x, y, w, h]` (x, y — от левого верхнего угла).
    pub fn rect(&mut self, atlas: &Atlas, white: SpriteId, r: [f32; 4], color: [f32; 4]) {
        let [x, y, w, h] = r;
        let uv = atlas.uv(white);
        self.batch.push(
            uv,
            [x + w / 2.0, self.height - (y + h / 2.0)],
            [w, h],
            0.0,
            color,
        );
    }

    /// Строка текста; `scale` — пикселей на пиксель глифа. Возвращает ширину в пикселях.
    pub fn text(
        &mut self,
        atlas: &Atlas,
        x: f32,
        y: f32,
        scale: f32,
        s: &str,
        color: [f32; 4],
    ) -> f32 {
        let (gw, gh) = (GLYPH_W as f32 * scale, GLYPH_H as f32 * scale);
        let adv = (GLYPH_W + 1) as f32 * scale;
        let mut cx = x;
        let mut buf = [0u8; 4];
        for ch in s.chars() {
            let up = ch.to_ascii_uppercase();
            if up != ' ' {
                let name_tail = up.encode_utf8(&mut buf);
                if let Some(id) = glyph_id(atlas, name_tail) {
                    self.batch.push(
                        atlas.uv(id),
                        [cx + gw / 2.0, self.height - (y + gh / 2.0)],
                        [gw, gh],
                        0.0,
                        color,
                    );
                }
            }
            cx += adv;
        }
        cx - x
    }
}

fn glyph_id(atlas: &Atlas, ch: &str) -> Option<SpriteId> {
    // Небольшой стековый буфер вместо format!: без аллокаций в кадре.
    let mut name = [0u8; 16];
    let prefix = b"glyph:";
    let total = prefix.len() + ch.len();
    if total > name.len() {
        return None;
    }
    name[..prefix.len()].copy_from_slice(prefix);
    name[prefix.len()..total].copy_from_slice(ch.as_bytes());
    std::str::from_utf8(&name[..total])
        .ok()
        .and_then(|n| atlas.sprite(n))
}
