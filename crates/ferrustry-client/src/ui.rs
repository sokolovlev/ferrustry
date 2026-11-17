//! Минимальный immediate-mode интерфейс: панели и кнопки, перехват кликов.
//!
//! Виджеты регистрируют свои прямоугольники в кадре; на следующем кадре клик внутри любого
//! из них принадлежит интерфейсу, а не карте (иначе нажатие на кнопку строило бы под ней).

use ferrustry_render::atlas::{SpriteId, palette};
use ferrustry_render::{Atlas, Hud, rgba};

/// Прямоугольник `[x, y, w, h]` в пикселях окна (от левого верхнего угла).
pub type Rect = [f32; 4];

/// Попадает ли точка в прямоугольник.
pub fn contains(r: Rect, p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[1] >= r[1] && p[0] < r[0] + r[2] && p[1] < r[1] + r[3]
}

/// Ширина строки встроенным шрифтом (5 px глиф + 1 px интервал).
pub fn text_width(s: &str, scale: f32) -> f32 {
    s.chars().count() as f32 * 6.0 * scale - scale
}

/// Состояние интерфейса между кадрами.
#[derive(Debug, Default, Clone)]
pub struct Ui {
    /// Курсор.
    pub mouse: [f32; 2],
    /// Где нажата левая кнопка, если нажатие принадлежит интерфейсу.
    pub pressed_at: Option<[f32; 2]>,
    /// Левая кнопка отпущена в этом кадре (после нажатия на интерфейсе).
    pub released: bool,
    blocking: Vec<Rect>,
    next_blocking: Vec<Rect>,
}

impl Ui {
    /// Закрывает ли интерфейс прошлого кадра точку `p`.
    pub fn blocks(&self, p: [f32; 2]) -> bool {
        self.blocking.iter().any(|&r| contains(r, p))
    }

    /// Начало кадра.
    pub fn begin(&mut self) {
        self.next_blocking.clear();
    }

    /// Конец кадра: прямоугольники этого (последнего нарисованного) кадра становятся
    /// «блокирующими» для ввода до следующего кадра; событие отпускания обработано.
    pub fn end(&mut self) {
        std::mem::swap(&mut self.blocking, &mut self.next_blocking);
        if self.released {
            self.released = false;
            self.pressed_at = None;
        }
    }

    /// Регистрирует область, перехватывающую клики (без отрисовки).
    pub fn block(&mut self, r: Rect) {
        self.next_blocking.push(r);
    }

    /// Панель-подложка.
    pub fn panel(
        &mut self,
        hud: &mut Hud,
        atlas: &Atlas,
        white: SpriteId,
        r: Rect,
        color: [f32; 4],
    ) {
        self.block(r);
        hud.rect(atlas, white, r, color);
    }

    /// Наведён ли курсор на `r`.
    pub fn hovered(&self, r: Rect) -> bool {
        contains(r, self.mouse)
    }

    /// Был ли клик по `r` в этом кадре (нажатие и отпускание внутри).
    pub fn clicked(&self, r: Rect) -> bool {
        self.released && self.pressed_at.is_some_and(|p| contains(r, p)) && contains(r, self.mouse)
    }

    /// Кнопка с подписью по центру. Возвращает `true`, если по ней кликнули.
    #[allow(clippy::too_many_arguments)]
    pub fn button(
        &mut self,
        hud: &mut Hud,
        atlas: &Atlas,
        white: SpriteId,
        r: Rect,
        label: &str,
        scale: f32,
        enabled: bool,
    ) -> bool {
        self.block(r);
        let hover = enabled && self.hovered(r);
        let pressed = hover && self.pressed_at.is_some_and(|p| contains(r, p)) && !self.released;
        let border = if hover {
            rgba(palette::FORGE_ORANGE)
        } else {
            rgba(palette::SHADOW)
        };
        let base = match (enabled, pressed, hover) {
            (false, _, _) => rgba([0x26, 0x29, 0x35, 0xF0]),
            (true, true, _) => rgba([0x24, 0x27, 0x33, 0xFF]),
            (true, false, true) => rgba([0x44, 0x49, 0x5E, 0xFF]),
            (true, false, false) => rgba(palette::GUNMETAL),
        };
        let b = (2.0 * scale / 2.0).max(2.0);
        hud.rect(atlas, white, r, border);
        hud.rect(
            atlas,
            white,
            [r[0] + b, r[1] + b, r[2] - 2.0 * b, r[3] - 2.0 * b],
            base,
        );
        let tw = text_width(label, scale);
        let th = 7.0 * scale;
        let color = if !enabled {
            rgba(palette::TITANIUM)
        } else if hover {
            rgba(palette::MOLTEN_GOLD)
        } else {
            rgba(palette::STEEL_WHITE)
        };
        hud.text(
            atlas,
            r[0] + (r[2] - tw) / 2.0,
            r[1] + (r[3] - th) / 2.0,
            scale,
            label,
            color,
        );
        enabled && self.clicked(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_requires_press_and_release_inside() {
        let mut ui = Ui::default();
        let r = [10.0, 10.0, 100.0, 40.0];
        ui.mouse = [50.0, 20.0];
        ui.pressed_at = Some([50.0, 20.0]);
        assert!(!ui.clicked(r), "not released yet");
        ui.released = true;
        assert!(ui.clicked(r));
        ui.mouse = [500.0, 20.0];
        assert!(!ui.clicked(r), "released outside");
        ui.pressed_at = Some([500.0, 500.0]);
        ui.mouse = [50.0, 20.0];
        assert!(!ui.clicked(r), "pressed outside");
    }

    #[test]
    fn blocking_rects_are_those_of_the_last_drawn_frame() {
        let mut ui = Ui::default();
        ui.begin();
        ui.block([0.0, 0.0, 10.0, 10.0]);
        assert!(!ui.blocks([5.0, 5.0]), "not yet drawn");
        ui.end();
        // ввод между кадрами видит последний нарисованный кадр
        assert!(ui.blocks([5.0, 5.0]));
        assert!(!ui.blocks([50.0, 5.0]));
        // следующий кадр без этой панели — она больше не перехватывает клики
        ui.begin();
        ui.end();
        assert!(!ui.blocks([5.0, 5.0]));
    }
}
