//! Детерминированная headless-симуляция Ferrustry.
//!
//! Мир изменяется только командами ([`Command`]) и тиком ([`World::tick`]). Одинаковые
//! начальное состояние, контент и поток команд дают одинаковый [`state_hash`].

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod buildings;
pub mod combat;
pub mod geom;
pub mod grid;
mod hash;
mod logistics;
pub mod mapgen;
pub mod power;
pub mod rng;
pub mod scenario;
pub mod schematic;
pub mod segment;
pub mod thermal;
mod world;

pub use buildings::{Building, BuildingId, KindTag};
pub use geom::{Dir, TilePos};

/// Версия симуляции: replay воспроизводим только той же версией.
pub const SIM_VERSION: &str = env!("CARGO_PKG_VERSION");
pub use hash::state_hash;
pub use world::{
    Command, CommandError, Detail, Inspection, MAX_SEGMENT_TILES, Rules, Stats, TICKS_PER_SECOND,
    World,
};
