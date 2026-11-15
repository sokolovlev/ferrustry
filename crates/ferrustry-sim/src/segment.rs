//! Сегменты конвейеров (transport lines).
//!
//! Цепочка конвейеров объединяется в один сегмент. Предметы хранятся от выхода к входу как
//! `(предмет, зазор)`, где зазор — расстояние до предыдущего предмета (для первого — до конца
//! сегмента). Сдвиг всей ленты — это изменение нескольких зазоров у «головы» очереди, а не
//! обход всех предметов; плотно сжатый префикс у заблокированного выхода кэшируется.

use std::collections::VecDeque;

use ferrustry_content::ItemId;
use serde::{Deserialize, Serialize};

use crate::geom::{Dir, TilePos};

/// Длина тайла в единицах пути.
pub const TILE_LEN: u32 = 256;
/// Минимальное расстояние между предметами (4 предмета на тайл).
pub const ITEM_SPACING: u32 = 64;

/// Сегмент конвейера.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// Тайлы от входа к выходу.
    pub(crate) tiles: Vec<TilePos>,
    /// Направление последнего тайла (куда выходят предметы).
    pub(crate) out_dir: Dir,
    /// Скорость в единицах за тик.
    pub(crate) speed: u32,
    /// Предметы от выхода к входу: (предмет, зазор до предыдущего / до конца).
    pub(crate) items: VecDeque<(ItemId, u32)>,
    /// Длина сжатого неподвижного префикса.
    compressed: usize,
    /// Расстояние от конца до последнего предмета (сумма зазоров).
    tail_pos: u32,
}

impl Segment {
    pub(crate) fn new(tiles: Vec<TilePos>, out_dir: Dir, speed: u32) -> Self {
        Self {
            tiles,
            out_dir,
            speed,
            items: VecDeque::new(),
            compressed: 0,
            tail_pos: 0,
        }
    }

    /// Длина в единицах пути.
    pub fn len_units(&self) -> u32 {
        u32::try_from(self.tiles.len()).unwrap_or(u32::MAX / TILE_LEN) * TILE_LEN
    }

    /// Тайлы сегмента от входа к выходу.
    pub fn tiles(&self) -> &[TilePos] {
        &self.tiles
    }

    /// Число предметов на сегменте.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Предметы с расстоянием от конца сегмента, от выхода к входу.
    pub fn item_positions(&self) -> impl Iterator<Item = (ItemId, u32)> + '_ {
        self.items.iter().scan(0u32, |pos, &(item, gap)| {
            *pos += gap;
            Some((item, *pos))
        })
    }

    /// Предмет, готовый к выходу (стоит в самом конце).
    pub(crate) fn ready_front(&self) -> Option<ItemId> {
        match self.items.front() {
            Some(&(item, 0)) => Some(item),
            _ => None,
        }
    }

    pub(crate) fn pop_front(&mut self) -> Option<ItemId> {
        let (item, gap) = self.items.pop_front()?;
        if let Some(next) = self.items.front_mut() {
            next.1 += gap;
        } else {
            self.tail_pos = 0;
        }
        self.compressed = 0;
        Some(item)
    }

    /// Вставка предмета на расстоянии `pos` от конца сегмента.
    pub(crate) fn try_insert_at(&mut self, item: ItemId, pos: u32) -> bool {
        if pos > self.len_units() {
            return false;
        }
        if self.items.is_empty() {
            self.items.push_back((item, pos));
            self.tail_pos = pos;
            self.compressed = 0;
            return true;
        }
        if pos >= self.tail_pos + ITEM_SPACING {
            self.items.push_back((item, pos - self.tail_pos));
            self.tail_pos = pos;
            return true;
        }
        let mut prev = 0u32;
        for i in 0..self.items.len() {
            let next = prev + self.items[i].1;
            if pos <= next {
                let front_ok = i == 0 || pos - prev >= ITEM_SPACING;
                let back_ok = next - pos >= ITEM_SPACING;
                if !(front_ok && back_ok) {
                    return false;
                }
                self.items[i].1 = next - pos;
                self.items.insert(i, (item, pos - prev));
                self.compressed = self.compressed.min(i);
                return true;
            }
            prev = next;
        }
        false
    }

    /// Вставка на входе тайла `k` (сзади, по направлению движения).
    pub(crate) fn try_insert_behind(&mut self, item: ItemId, k: usize) -> bool {
        let l = self.tiles.len();
        if k >= l {
            return false;
        }
        self.try_insert_at(item, (l - k) as u32 * TILE_LEN)
    }

    /// Вставка в центр тайла `k` (сбоку).
    pub(crate) fn try_insert_side(&mut self, item: ItemId, k: usize) -> bool {
        let l = self.tiles.len();
        if k >= l {
            return false;
        }
        self.try_insert_at(item, (l - k) as u32 * TILE_LEN - TILE_LEN / 2)
    }

    /// Сдвигает все предметы на `speed` с учётом блокировки и сжатия.
    pub(crate) fn advance(&mut self) {
        let d = self.speed;
        let n = self.items.len();
        let mut prev_m = 0u32;
        let mut i = self.compressed;
        let mut all_full = false;
        while i < n {
            let min = if i == 0 { 0 } else { ITEM_SPACING };
            let gap = &mut self.items[i].1;
            let free = *gap - min;
            let m = (free + prev_m).min(d);
            *gap = *gap + prev_m - m;
            if m == d {
                all_full = true;
                break;
            }
            if m == 0 && i == self.compressed {
                self.compressed += 1;
            }
            prev_m = m;
            i += 1;
        }
        if n > 0 {
            let tail_move = if all_full { d } else { prev_m };
            self.tail_pos -= tail_move;
        }
    }

    /// Разбирает сегмент на предметы с привязкой к тайлам: (индекс тайла, предмет, смещение от конца тайла).
    pub(crate) fn drain_by_tile(&mut self, mut f: impl FnMut(usize, ItemId, u32)) {
        let l = self.tiles.len();
        let mut pos = 0u32;
        for (item, gap) in self.items.drain(..) {
            pos += gap;
            let from_end = ((pos / TILE_LEN) as usize).min(l.saturating_sub(1));
            let k = l - 1 - from_end;
            f(k, item, pos - from_end as u32 * TILE_LEN);
        }
        self.tail_pos = 0;
        self.compressed = 0;
    }

    /// Заполняет сегмент предметами, отсортированными по расстоянию от конца; раздвигает
    /// слишком близкие. Возвращает число не поместившихся предметов.
    pub(crate) fn fill_sorted(&mut self, sorted: impl Iterator<Item = (ItemId, u32)>) -> u32 {
        let len = self.len_units();
        let mut lost = 0;
        let mut prev: Option<u32> = None;
        for (item, pos) in sorted {
            let pos = match prev {
                Some(p) => pos.max(p + ITEM_SPACING),
                None => pos,
            };
            if pos > len {
                lost += 1;
                continue;
            }
            let gap = pos - prev.unwrap_or(0);
            self.items.push_back((item, gap));
            prev = Some(pos);
        }
        self.tail_pos = prev.unwrap_or(0);
        self.compressed = 0;
        lost
    }

    /// Проверка инвариантов (для тестов и загруженных файлов).
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.tiles.is_empty() {
            return Err("empty segment".into());
        }
        let mut sum = 0u64;
        for (i, &(_, gap)) in self.items.iter().enumerate() {
            if i > 0 && gap < ITEM_SPACING {
                return Err(format!("item {i} closer than spacing"));
            }
            sum += u64::from(gap);
        }
        if sum > u64::from(self.len_units()) || sum != u64::from(self.tail_pos) {
            return Err("segment item positions inconsistent".into());
        }
        if self.compressed > self.items.len() {
            return Err("segment compressed prefix out of range".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const IRON: ItemId = ItemId(0);

    fn seg(len: i32, speed: u32) -> Segment {
        Segment::new(
            (0..len).map(|x| TilePos::new(x, 0)).collect(),
            Dir::Right,
            speed,
        )
    }

    #[test]
    fn item_travels_full_length() {
        let mut s = seg(4, 16);
        assert!(s.try_insert_behind(IRON, 0));
        let mut ticks = 0;
        while s.ready_front().is_none() {
            s.advance();
            ticks += 1;
            s.validate().unwrap();
        }
        assert_eq!(ticks, 4 * TILE_LEN / 16);
    }

    #[test]
    fn blocked_line_compresses_to_spacing() {
        let mut s = seg(2, 16);
        for _ in 0..200 {
            s.try_insert_behind(IRON, 0);
            s.advance();
            s.validate().unwrap();
        }
        // выход заблокирован: 2 тайла по 4 предмета + один на самом входе
        assert_eq!(s.item_count(), 9);
        let positions: Vec<u32> = s.item_positions().map(|(_, p)| p).collect();
        assert_eq!(positions[0], 0);
        for w in positions.windows(2) {
            assert_eq!(w[1] - w[0], ITEM_SPACING);
        }
    }

    #[test]
    fn unblocking_resumes_motion() {
        let mut s = seg(2, 16);
        for _ in 0..200 {
            s.try_insert_behind(IRON, 0);
            s.advance();
        }
        let mut out = 0;
        for _ in 0..1000 {
            if s.ready_front().is_some() {
                s.pop_front();
                out += 1;
            }
            s.advance();
            s.validate().unwrap();
        }
        assert_eq!(out, 9);
        assert_eq!(s.item_count(), 0);
    }

    #[test]
    fn side_insert_respects_spacing() {
        let mut s = seg(3, 16);
        assert!(s.try_insert_side(IRON, 1));
        assert!(!s.try_insert_side(IRON, 1));
        assert!(s.try_insert_side(IRON, 0));
        assert!(s.try_insert_side(IRON, 2));
        s.validate().unwrap();
        assert_eq!(s.item_count(), 3);
    }

    #[test]
    fn drain_and_fill_roundtrip() {
        let mut s = seg(3, 16);
        s.try_insert_side(IRON, 0);
        s.try_insert_side(IRON, 2);
        s.advance();
        let before: Vec<_> = s.item_positions().collect();
        let mut items = Vec::new();
        s.drain_by_tile(|k, item, off| items.push((item, (2 - k as u32) * TILE_LEN + off)));
        items.sort_by_key(|&(_, p)| p);
        assert_eq!(s.fill_sorted(items.into_iter()), 0);
        let after: Vec<_> = s.item_positions().collect();
        assert_eq!(before, after);
    }
}
