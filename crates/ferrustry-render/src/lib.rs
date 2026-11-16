//! 2D-рендерер Ferrustry на wgpu.
//!
//! Разделён на CPU-часть (атлас, камера, пакеты, построение сцены — тестируются без GPU) и
//! GPU-часть ([`gpu`]). Рендер только читает состояние мира и никогда его не изменяет.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod atlas;
pub mod batch;
pub mod camera;
pub mod font;
pub mod gpu;
pub mod scene;

/// Реэкспорт wgpu для клиента (одна версия во всём проекте).
pub use wgpu;

pub use atlas::Atlas;
pub use batch::{Hud, SpriteBatch, rgba};
pub use camera::Camera;
pub use gpu::{Gpu, RenderError, Renderer, render_offscreen, write_png};
pub use scene::{Ghost, SpriteTable, build_world};
