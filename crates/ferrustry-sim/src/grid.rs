//! Сетка тайлов, разбитая на чанки 32×32.

use ferrustry_content::{FloorId, OreId};
use serde::{Deserialize, Serialize};

use crate::buildings::BuildingId;
use crate::geom::TilePos;

/// Сторона чанка в тайлах.
pub const CHUNK: u32 = 32;
const CHUNK_AREA: usize = (CHUNK * CHUNK) as usize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Chunk {
    pub(crate) floor: Vec<FloorId>,
    pub(crate) ore: Vec<Option<OreId>>,
    pub(crate) building: Vec<Option<BuildingId>>,
}

/// Сетка мира.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grid {
    width: u32,
    height: u32,
    chunks_x: u32,
    pub(crate) chunks: Vec<Chunk>,
}

impl Grid {
    pub(crate) fn hash_part(&self, p: &mut crate::hash::PartHasher) {
        let Self {
            width,
            height,
            chunks_x,
            chunks,
        } = self;
        p.whole(&(width, height, chunks_x));
        p.split(chunks);
    }

    /// Максимальная сторона карты в тайлах.
    pub const MAX_SIDE: u32 = 8192;

    pub(crate) fn new(width: u32, height: u32, floor: FloorId) -> Self {
        let width = width.clamp(1, Self::MAX_SIDE);
        let height = height.clamp(1, Self::MAX_SIDE);
        let chunks_x = width.div_ceil(CHUNK);
        let chunks_y = height.div_ceil(CHUNK);
        let chunk = Chunk {
            floor: vec![floor; CHUNK_AREA],
            ore: vec![None; CHUNK_AREA],
            building: vec![None; CHUNK_AREA],
        };
        Self {
            width,
            height,
            chunks_x,
            chunks: vec![chunk; (chunks_x * chunks_y) as usize],
        }
    }

    /// Ширина в тайлах.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Высота в тайлах.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Число чанков по X и Y.
    pub fn chunk_dims(&self) -> (u32, u32) {
        (self.chunks_x, self.height.div_ceil(CHUNK))
    }

    /// Находится ли тайл внутри карты.
    pub fn in_bounds(&self, p: TilePos) -> bool {
        p.x >= 0 && p.y >= 0 && (p.x as u32) < self.width && (p.y as u32) < self.height
    }

    fn locate(&self, p: TilePos) -> Option<(usize, usize)> {
        if !self.in_bounds(p) {
            return None;
        }
        let (x, y) = (p.x as u32, p.y as u32);
        let chunk = (y / CHUNK) * self.chunks_x + x / CHUNK;
        let local = (y % CHUNK) * CHUNK + x % CHUNK;
        Some((chunk as usize, local as usize))
    }

    /// Пол тайла.
    pub fn floor(&self, p: TilePos) -> Option<FloorId> {
        let (c, l) = self.locate(p)?;
        Some(self.chunks[c].floor[l])
    }

    /// Руда тайла.
    pub fn ore(&self, p: TilePos) -> Option<OreId> {
        let (c, l) = self.locate(p)?;
        self.chunks[c].ore[l]
    }

    /// Постройка, занимающая тайл.
    pub fn building(&self, p: TilePos) -> Option<BuildingId> {
        let (c, l) = self.locate(p)?;
        self.chunks[c].building[l]
    }

    pub(crate) fn set_floor(&mut self, p: TilePos, floor: FloorId) {
        if let Some((c, l)) = self.locate(p) {
            self.chunks[c].floor[l] = floor;
        }
    }

    pub(crate) fn set_ore(&mut self, p: TilePos, ore: Option<OreId>) {
        if let Some((c, l)) = self.locate(p) {
            self.chunks[c].ore[l] = ore;
        }
    }

    pub(crate) fn set_building(&mut self, p: TilePos, b: Option<BuildingId>) {
        if let Some((c, l)) = self.locate(p) {
            self.chunks[c].building[l] = b;
        }
    }

    /// Проверка структурной целостности (для загруженных файлов).
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > Self::MAX_SIDE
            || self.height > Self::MAX_SIDE
        {
            return Err(format!("invalid map size {}x{}", self.width, self.height));
        }
        if self.chunks_x != self.width.div_ceil(CHUNK) {
            return Err("chunk count mismatch".into());
        }
        let (cx, cy) = self.chunk_dims();
        if self.chunks.len() != (cx * cy) as usize {
            return Err("chunk count mismatch".into());
        }
        for c in &self.chunks {
            if c.floor.len() != CHUNK_AREA
                || c.ore.len() != CHUNK_AREA
                || c.building.len() != CHUNK_AREA
            {
                return Err("chunk size mismatch".into());
            }
        }
        Ok(())
    }
}
