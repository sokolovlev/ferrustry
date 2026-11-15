//! Координаты тайлов и направления.

use serde::{Deserialize, Serialize};

/// Позиция тайла. Ось Y направлена вверх.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TilePos {
    /// Координата X.
    pub x: i32,
    /// Координата Y.
    pub y: i32,
}

impl TilePos {
    /// Создаёт позицию.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Соседний тайл в направлении `dir`.
    pub const fn step(self, dir: Dir) -> Self {
        let (dx, dy) = dir.delta();
        Self::new(self.x + dx, self.y + dy)
    }

    /// Четыре соседа в порядке [`Dir::ALL`].
    pub fn neighbors(self) -> [TilePos; 4] {
        Dir::ALL.map(|d| self.step(d))
    }
}

/// Направление (поворот постройки).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dir {
    /// +X.
    Right,
    /// +Y.
    Up,
    /// −X.
    Left,
    /// −Y.
    Down,
}

impl Dir {
    /// Все направления.
    pub const ALL: [Dir; 4] = [Dir::Right, Dir::Up, Dir::Left, Dir::Down];

    /// Смещение на один тайл.
    pub const fn delta(self) -> (i32, i32) {
        match self {
            Dir::Right => (1, 0),
            Dir::Up => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Down => (0, -1),
        }
    }

    /// Противоположное направление.
    pub const fn opposite(self) -> Dir {
        match self {
            Dir::Right => Dir::Left,
            Dir::Up => Dir::Down,
            Dir::Left => Dir::Right,
            Dir::Down => Dir::Up,
        }
    }
}
