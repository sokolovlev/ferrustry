//! Детерминированная headless-симуляция Ferrustry.
//!
//! Мир изменяется только командами ([`Command`]) и тиком ([`World::tick`]). Одинаковые
//! начальное состояние, контент и поток команд дают одинаковый [`state_hash`].

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod buildings;
pub mod geom;
pub mod grid;
mod hash;
pub mod power;
pub mod rng;
pub mod scenario;
pub mod segment;
mod world;

pub use buildings::{Building, BuildingId, KindTag};
pub use geom::{Dir, TilePos};
pub use hash::state_hash;
pub use world::{Command, CommandError, MAX_SEGMENT_TILES, Stats, TICKS_PER_SECOND, World};
