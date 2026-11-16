//! 2D-камера: мир в тайлах (Y вверх) ↔ экран в пикселях (Y вниз).

/// Камера сверху.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Точка мира в центре экрана (в тайлах).
    pub center: [f32; 2],
    /// Масштаб: пикселей на тайл.
    pub zoom: f32,
    /// Размер области вывода в пикселях.
    pub viewport: [f32; 2],
}

impl Camera {
    /// Минимальный масштаб (пикселей на тайл).
    pub const MIN_ZOOM: f32 = 4.0;
    /// Максимальный масштаб.
    pub const MAX_ZOOM: f32 = 128.0;

    /// Новая камера.
    pub fn new(center: [f32; 2], zoom: f32, viewport: [f32; 2]) -> Self {
        Self {
            center,
            zoom: zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM),
            viewport,
        }
    }

    /// Коэффициенты перевода мира в clip-пространство: `clip = world * scale + offset`.
    pub fn clip_transform(&self) -> ([f32; 2], [f32; 2]) {
        let sx = 2.0 * self.zoom / self.viewport[0].max(1.0);
        let sy = 2.0 * self.zoom / self.viewport[1].max(1.0);
        ([sx, sy], [-self.center[0] * sx, -self.center[1] * sy])
    }

    /// Пиксель экрана (0,0 — левый верхний угол) → точка мира.
    pub fn screen_to_world(&self, px: [f32; 2]) -> [f32; 2] {
        [
            self.center[0] + (px[0] - self.viewport[0] / 2.0) / self.zoom,
            self.center[1] - (px[1] - self.viewport[1] / 2.0) / self.zoom,
        ]
    }

    /// Точка мира → пиксель экрана.
    pub fn world_to_screen(&self, w: [f32; 2]) -> [f32; 2] {
        [
            (w[0] - self.center[0]) * self.zoom + self.viewport[0] / 2.0,
            -(w[1] - self.center[1]) * self.zoom + self.viewport[1] / 2.0,
        ]
    }

    /// Видимая область мира: (min, max) в тайлах.
    pub fn visible_rect(&self) -> ([f32; 2], [f32; 2]) {
        let hw = self.viewport[0] / 2.0 / self.zoom;
        let hh = self.viewport[1] / 2.0 / self.zoom;
        (
            [self.center[0] - hw, self.center[1] - hh],
            [self.center[0] + hw, self.center[1] + hh],
        )
    }

    /// Масштабирование колесом с сохранением точки под курсором.
    pub fn zoom_at(&mut self, factor: f32, cursor_px: [f32; 2]) {
        let before = self.screen_to_world(cursor_px);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let after = self.screen_to_world(cursor_px);
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }

    /// Сдвиг на вектор в пикселях экрана (перетаскивание).
    pub fn pan_pixels(&mut self, dx: f32, dy: f32) {
        self.center[0] -= dx / self.zoom;
        self.center[1] += dy / self.zoom;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_world_roundtrip() {
        let c = Camera::new([10.0, 5.0], 32.0, [800.0, 600.0]);
        assert_eq!(c.screen_to_world([400.0, 300.0]), [10.0, 5.0]);
        let w = c.screen_to_world([123.0, 456.0]);
        let s = c.world_to_screen(w);
        assert!((s[0] - 123.0).abs() < 1e-3 && (s[1] - 456.0).abs() < 1e-3);
        // Y мира вверх: верх экрана — больший Y
        assert!(c.screen_to_world([400.0, 0.0])[1] > 5.0);
    }

    #[test]
    fn zoom_keeps_cursor_point() {
        let mut c = Camera::new([0.0, 0.0], 32.0, [800.0, 600.0]);
        let p = [600.0, 100.0];
        let before = c.screen_to_world(p);
        c.zoom_at(1.5, p);
        let after = c.screen_to_world(p);
        assert!((before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3);
    }

    #[test]
    fn clip_transform_maps_center_to_origin() {
        let c = Camera::new([3.0, 4.0], 16.0, [640.0, 480.0]);
        let (s, o) = c.clip_transform();
        assert!((3.0 * s[0] + o[0]).abs() < 1e-6 && (4.0 * s[1] + o[1]).abs() < 1e-6);
    }
}
