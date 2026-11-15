//! Сегменты конвейеров (transport lines).
//!
//! Цепочка конвейеров (не длиннее [`crate::MAX_SEGMENT_TILES`]) объединяется в один сегмент.
//! Предметы хранятся от выхода к входу как `(предмет, зазор)`, где зазор — расстояние до
//! предыдущего предмета (для первого — до конца сегмента). Сдвиг ленты — это изменение
//! нескольких зазоров у «головы» очереди, а не обход всех предметов; сжатый префикс у
//! заблокированного выхода кэшируется.
//!
//! ## Раскладка в памяти
//!
//! Предмет и зазор упакованы в одно `u32` (`item << 16 | gap`) и лежат в одном плотном массиве
//! точной ёмкости `4·L + 1` (физический максимум предметов на ленте длины `L`). Окно занятых
//! ячеек `[head, head + len)` сдвигается при выдаче предметов и уплотняется при упоре в конец.
//! Никаких кольцевых индексов, перевыделений и указателей в горячем цикле: `advance` и поиск
//! позиции работают по непрерывному срезу, а поиск суммирует зазоры блоками по 16 —
//! такой цикл компилятор разворачивает в SIMD-инструкции.
//!
//! Формат сохранения от раскладки не зависит: сегмент сериализуется как список `(предмет, зазор)`.

use ferrustry_content::ItemId;
use serde::{Deserialize, Serialize};

use crate::geom::{Dir, TilePos};
use crate::world::{Link, Waiter};

/// Длина тайла в единицах пути.
pub const TILE_LEN: u32 = 256;
/// Минимальное расстояние между предметами (4 предмета на тайл).
pub const ITEM_SPACING: u32 = 64;
/// Максимальная длина сегмента, при которой зазор помещается в 16 бит.
const MAX_TILES_U16: usize = (u16::MAX as u32 / TILE_LEN) as usize;
/// Ширина блока при поиске позиции.
const SEARCH_BLOCK: usize = 16;

#[inline(always)]
fn gap_of(slot: u32) -> u32 {
    slot & 0xFFFF
}

#[inline(always)]
fn item_of(slot: u32) -> ItemId {
    ItemId((slot >> 16) as u16)
}

#[inline(always)]
fn pack(item: ItemId, gap: u32) -> u32 {
    debug_assert!(gap <= 0xFFFF);
    (u32::from(item.0) << 16) | gap
}

#[inline(always)]
fn with_gap(slot: u32, gap: u32) -> u32 {
    debug_assert!(gap <= 0xFFFF);
    (slot & 0xFFFF_0000) | gap
}

/// Зазор `gap` у предмета с индексом `i` вмещает ещё один предмет.
#[inline(always)]
fn is_wide(i: usize, gap: u32) -> bool {
    i > 0 && gap >= 2 * ITEM_SPACING
}

/// Физический максимум предметов на сегменте из `tiles` тайлов.
fn capacity_for(tiles: usize) -> usize {
    tiles * (TILE_LEN / ITEM_SPACING) as usize + 1
}

/// Сегмент конвейера.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "SegmentRepr", try_from = "SegmentRepr")]
pub struct Segment {
    /// Тайлы от входа к выходу.
    pub(crate) tiles: Vec<TilePos>,
    /// Направление последнего тайла (куда выходят предметы).
    pub(crate) out_dir: Dir,
    /// Скорость в единицах за тик.
    pub(crate) speed: u32,
    /// Упакованные предметы; занято окно `[head, head + len)`.
    slots: Box<[u32]>,
    head: u32,
    len: u32,
    /// Длина сжатого неподвижного префикса.
    compressed: u32,
    /// Расстояние от конца до последнего предмета (сумма зазоров).
    tail_pos: u32,
    /// Число внутренних зазоров (не у первого предмета), в которые помещается ещё один предмет.
    /// Позволяет отклонить вставку в насыщенную ленту за O(1).
    wide: u32,
    /// Сегменты, уснувшие в ожидании места в этом сегменте.
    pub(crate) waiters: Vec<Waiter>,
    /// Кэш получателя на выходе (действителен при `link_epoch == эпохе топологии мира`).
    pub(crate) link: Link,
    /// Эпоха, для которой разрешён `link` (0 — не разрешён).
    pub(crate) link_epoch: u64,
}

/// Представление сегмента в сохранениях — не зависит от раскладки в памяти.
#[derive(Serialize, Deserialize)]
struct SegmentRepr {
    tiles: Vec<TilePos>,
    out_dir: Dir,
    speed: u32,
    items: Vec<(ItemId, u32)>,
    compressed: u32,
    waiters: Vec<Waiter>,
    link: Link,
    link_epoch: u64,
}

impl From<Segment> for SegmentRepr {
    fn from(s: Segment) -> Self {
        let items = s
            .window()
            .iter()
            .map(|&x| (item_of(x), gap_of(x)))
            .collect();
        Self {
            tiles: s.tiles,
            out_dir: s.out_dir,
            speed: s.speed,
            items,
            compressed: s.compressed,
            waiters: s.waiters,
            link: s.link,
            link_epoch: s.link_epoch,
        }
    }
}

impl TryFrom<SegmentRepr> for Segment {
    type Error = String;

    fn try_from(r: SegmentRepr) -> Result<Self, String> {
        if r.tiles.is_empty() || r.tiles.len() > MAX_TILES_U16 {
            return Err("segment length out of range".into());
        }
        let mut s = Segment::new(r.tiles, r.out_dir, r.speed);
        if r.items.len() > s.slots.len() {
            return Err("too many items on segment".into());
        }
        for (i, &(item, gap)) in r.items.iter().enumerate() {
            if gap > s.len_units() {
                return Err("item gap out of range".into());
            }
            s.slots[i] = pack(item, gap);
            s.tail_pos = s.tail_pos.saturating_add(gap);
            s.wide += u32::from(is_wide(i, gap));
        }
        s.len = r.items.len() as u32;
        s.compressed = r.compressed;
        s.waiters = r.waiters;
        s.link = r.link;
        s.link_epoch = r.link_epoch;
        s.validate()?;
        Ok(s)
    }
}

impl PartialEq for Segment {
    fn eq(&self, o: &Self) -> bool {
        self.tiles == o.tiles
            && self.out_dir == o.out_dir
            && self.speed == o.speed
            && self.window() == o.window()
            && self.compressed == o.compressed
            && self.tail_pos == o.tail_pos
            && self.wide == o.wide
            && self.waiters == o.waiters
            && self.link == o.link
            && self.link_epoch == o.link_epoch
    }
}

impl Eq for Segment {}

impl Segment {
    pub(crate) fn new(tiles: Vec<TilePos>, out_dir: Dir, speed: u32) -> Self {
        debug_assert!(!tiles.is_empty() && tiles.len() <= MAX_TILES_U16);
        let cap = capacity_for(tiles.len());
        Self {
            tiles,
            out_dir,
            speed,
            slots: vec![0; cap].into_boxed_slice(),
            head: 0,
            len: 0,
            compressed: 0,
            tail_pos: 0,
            wide: 0,
            waiters: Vec::new(),
            link: Link::Nothing,
            link_epoch: 0,
        }
    }

    #[inline(always)]
    fn window(&self) -> &[u32] {
        &self.slots[self.head as usize..(self.head + self.len) as usize]
    }

    #[inline(always)]
    fn window_mut(&mut self) -> &mut [u32] {
        &mut self.slots[self.head as usize..(self.head + self.len) as usize]
    }

    /// Переносит окно в начало массива, освобождая место в конце.
    fn compact(&mut self) {
        let (h, n) = (self.head as usize, self.len as usize);
        if h > 0 {
            self.slots.copy_within(h..h + n, 0);
            self.slots[n..h + n].fill(0);
            self.head = 0;
        }
    }

    /// Освобождает ячейку после конца окна; `false`, если массив полон.
    fn reserve_back(&mut self) -> bool {
        if (self.len as usize) >= self.slots.len() {
            return false;
        }
        if (self.head + self.len) as usize == self.slots.len() {
            self.compact();
        }
        true
    }

    /// Длина в единицах пути.
    pub fn len_units(&self) -> u32 {
        self.tiles.len() as u32 * TILE_LEN
    }

    /// Тайлы сегмента от входа к выходу.
    pub fn tiles(&self) -> &[TilePos] {
        &self.tiles
    }

    /// Число предметов на сегменте.
    pub fn item_count(&self) -> usize {
        self.len as usize
    }

    /// Предметы (от выхода к входу).
    pub fn items(&self) -> impl Iterator<Item = ItemId> + '_ {
        self.window().iter().map(|&s| item_of(s))
    }

    /// Предметы с расстоянием от конца сегмента, от выхода к входу.
    pub fn item_positions(&self) -> impl Iterator<Item = (ItemId, u32)> + '_ {
        self.window().iter().scan(0u32, |pos, &s| {
            *pos += gap_of(s);
            Some((item_of(s), *pos))
        })
    }

    /// Предмет, готовый к выходу (стоит в самом конце).
    #[inline]
    pub(crate) fn ready_front(&self) -> Option<ItemId> {
        if self.len == 0 {
            return None;
        }
        let s = self.slots[self.head as usize];
        (gap_of(s) == 0).then(|| item_of(s))
    }

    pub(crate) fn pop_front(&mut self) -> Option<ItemId> {
        if self.len == 0 {
            return None;
        }
        let h = self.head as usize;
        let first = self.slots[h];
        self.slots[h] = 0;
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
            self.tail_pos = 0;
        } else {
            self.head += 1;
            let next = self.slots[h + 1];
            // бывший второй предмет становится первым: его зазор больше не «внутренний»
            if is_wide(1, gap_of(next)) {
                self.wide -= 1;
            }
            self.slots[h + 1] = with_gap(next, gap_of(next) + gap_of(first));
        }
        self.compressed = 0;
        Some(item_of(first))
    }

    fn push_back(&mut self, item: ItemId, gap: u32) -> bool {
        if !self.reserve_back() {
            return false;
        }
        self.slots[(self.head + self.len) as usize] = pack(item, gap);
        self.len += 1;
        true
    }

    /// Вставка в окно перед индексом `i` (сдвигает хвост на одну ячейку).
    fn insert_at_index(&mut self, i: usize, slot: u32) -> bool {
        if !self.reserve_back() {
            return false;
        }
        let (h, n) = (self.head as usize, self.len as usize);
        if i == 0 && h > 0 {
            self.head -= 1;
            self.slots[h - 1] = slot;
        } else {
            self.slots.copy_within(h + i..h + n, h + i + 1);
            self.slots[h + i] = slot;
        }
        self.len += 1;
        true
    }

    /// Первый индекс `i`, для которого `pos <= префиксная сумма зазоров [0..=i]`, и эта сумма
    /// вместе с предыдущей. Суммы считаются блоками по [`SEARCH_BLOCK`] (векторизуется).
    #[inline]
    fn locate(&self, pos: u32) -> Option<(usize, u32, u32)> {
        let w = self.window();
        let mut acc = 0u32;
        let mut base = 0usize;
        let mut blocks = w.chunks_exact(SEARCH_BLOCK);
        for block in &mut blocks {
            let sum: u32 = block.iter().map(|&s| gap_of(s)).sum();
            if acc + sum >= pos {
                break;
            }
            acc += sum;
            base += SEARCH_BLOCK;
        }
        for (j, &s) in w[base..].iter().enumerate() {
            let next = acc + gap_of(s);
            if pos <= next {
                return Some((base + j, acc, next));
            }
            acc = next;
        }
        None
    }

    /// Вставка предмета на расстоянии `pos` от конца сегмента.
    pub(crate) fn try_insert_at(&mut self, item: ItemId, pos: u32) -> bool {
        if pos > self.len_units() {
            return false;
        }
        if self.len == 0 {
            if !self.push_back(item, pos) {
                return false;
            }
            self.tail_pos = pos;
            self.compressed = 0;
            return true;
        }
        let front_gap = gap_of(self.slots[self.head as usize]);
        // быстрый путь: за последним предметом
        if pos >= self.tail_pos + ITEM_SPACING {
            let gap = pos - self.tail_pos;
            let idx = self.len as usize;
            if !self.push_back(item, gap) {
                return false;
            }
            self.wide += u32::from(is_wide(idx, gap));
            self.tail_pos = pos;
            return true;
        }
        // быстрый путь: перед первым предметом
        if pos + ITEM_SPACING <= front_gap {
            let rest = front_gap - pos;
            if !self.insert_at_index(0, pack(item, pos)) {
                return false;
            }
            let h = self.head as usize;
            self.slots[h + 1] = with_gap(self.slots[h + 1], rest);
            self.wide += u32::from(is_wide(1, rest));
            self.compressed = 0;
            return true;
        }
        // внутри: возможно только при наличии широкого зазора
        if self.wide == 0 {
            return false;
        }
        let Some((i, prev, next)) = self.locate(pos) else {
            return false;
        };
        if i == 0 || pos < prev + ITEM_SPACING || next - pos < ITEM_SPACING {
            return false;
        }
        let gap = next - prev;
        let (a, b) = (pos - prev, next - pos);
        if !self.insert_at_index(i, pack(item, a)) {
            return false;
        }
        let h = self.head as usize;
        self.slots[h + i + 1] = with_gap(self.slots[h + i + 1], b);
        self.wide = self.wide - u32::from(is_wide(i, gap))
            + u32::from(is_wide(i, a))
            + u32::from(is_wide(i + 1, b));
        self.compressed = self.compressed.min(i as u32);
        true
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

    /// Длина сжатого неподвижного префикса.
    pub(crate) fn compressed_len(&self) -> u32 {
        self.compressed
    }

    /// Свободное место перед первым предметом (у выхода); для пустого сегмента — без ограничения.
    pub(crate) fn front_free(&self) -> u32 {
        if self.len == 0 {
            u32::MAX
        } else {
            gap_of(self.slots[self.head as usize])
        }
    }

    /// Позиция вставки (расстояние от конца) для входа на тайл `k` сзади или сбоку.
    pub(crate) fn insert_pos(&self, k: usize, behind: bool) -> u32 {
        let l = self.tiles.len().max(k + 1);
        let base = (l - k) as u32 * TILE_LEN;
        if behind { base } else { base - TILE_LEN / 2 }
    }

    /// Нижняя оценка числа тиков, через которое вставка в позицию `pos` может стать возможной,
    /// если ничего, кроме равномерного движения несжатых предметов, не произойдёт.
    /// `None` — при текущем движении не станет возможной никогда (нужен внешний повод: выдача
    /// предмета из заблокированной ленты, освобождение головы, перестройка).
    ///
    /// Оценка занижена (округление вниз, без учёта будущих остановок), поэтому разбудить
    /// ожидающего по ней можно только раньше, чем вставка реально станет возможной.
    pub(crate) fn earliest_insert(&self, pos: u32) -> Option<u32> {
        let n = self.len as usize;
        if n == 0 {
            return Some(0);
        }
        let d = i64::from(self.speed.max(1));
        let c = self.compressed as usize;
        let pos = i64::from(pos);
        let sp = i64::from(ITEM_SPACING);
        let mut best: Option<i64> = None;
        let mut take = |t: i64| {
            let t = t.max(0) / d;
            best = Some(best.map_or(t, |b: i64| b.min(t)));
        };
        let mut prev = 0i64;
        for (i, &s) in self.window().iter().enumerate() {
            let q = prev + i64::from(gap_of(s));
            // зазор между движущимися предметами i-1 и i проходит мимо `pos`
            if i > c && i64::from(gap_of(s)) >= 2 * sp && q - sp - pos >= 0 {
                take(prev + sp - pos);
            }
            prev = q;
        }
        // свободный хвост за последним (движущимся) предметом
        if n > c && pos <= i64::from(self.len_units()) {
            take(prev + sp - pos);
        }
        best.map(|t| u32::try_from(t).unwrap_or(u32::MAX))
    }

    /// Все предметы стоят вплотную у заблокированного выхода.
    pub(crate) fn is_fully_compressed(&self) -> bool {
        self.len > 0 && self.compressed == self.len
    }

    /// Сдвигает все предметы на `speed` с учётом блокировки и сжатия.
    /// Возвращает `true`, если сдвинулся хотя бы один предмет.
    #[inline]
    pub(crate) fn advance(&mut self) -> bool {
        let d = self.speed;
        let start = self.compressed as usize;
        let mut compressed = self.compressed;
        let mut wide = self.wide;
        let w = self.window_mut();
        let n = w.len();
        if n == 0 {
            return false;
        }
        let mut prev_m = 0u32;
        let mut all_full = false;
        let mut moved = false;
        let mut i = start;
        while i < n {
            let min = if i == 0 { 0 } else { ITEM_SPACING };
            let old = gap_of(w[i]);
            let m = (old - min + prev_m).min(d);
            let new = old + prev_m - m;
            if new != old {
                w[i] = with_gap(w[i], new);
                wide = wide - u32::from(is_wide(i, old)) + u32::from(is_wide(i, new));
            }
            moved |= m > 0;
            if m == d {
                all_full = true;
                break;
            }
            if m == 0 && i as u32 == compressed {
                compressed += 1;
            }
            prev_m = m;
            i += 1;
        }
        self.compressed = compressed;
        self.wide = wide;
        self.tail_pos -= if all_full { d } else { prev_m };
        moved
    }

    /// Разбирает сегмент на предметы с привязкой к тайлам: (индекс тайла, предмет, смещение от конца тайла).
    pub(crate) fn drain_by_tile(&mut self, mut f: impl FnMut(usize, ItemId, u32)) {
        let l = self.tiles.len();
        let mut pos = 0u32;
        for &s in self.window() {
            pos += gap_of(s);
            let from_end = ((pos / TILE_LEN) as usize).min(l.saturating_sub(1));
            let k = l - 1 - from_end;
            f(k, item_of(s), pos - from_end as u32 * TILE_LEN);
        }
        self.slots.fill(0);
        self.head = 0;
        self.len = 0;
        self.tail_pos = 0;
        self.compressed = 0;
        self.wide = 0;
    }

    /// Заполняет пустой сегмент предметами, отсортированными по расстоянию от конца; раздвигает
    /// слишком близкие. Возвращает число не поместившихся предметов.
    pub(crate) fn fill_sorted(&mut self, sorted: impl Iterator<Item = (ItemId, u32)>) -> u32 {
        debug_assert_eq!(self.len, 0);
        let len = self.len_units();
        let mut lost = 0;
        let mut prev: Option<u32> = None;
        for (item, pos) in sorted {
            let pos = match prev {
                Some(p) => pos.max(p + ITEM_SPACING),
                None => pos,
            };
            let gap = pos - prev.unwrap_or(0);
            let idx = self.len as usize;
            if pos > len || !self.push_back(item, gap) {
                lost += 1;
                continue;
            }
            self.wide += u32::from(is_wide(idx, gap));
            prev = Some(pos);
        }
        self.tail_pos = prev.unwrap_or(0);
        self.compressed = 0;
        lost
    }

    /// Проверка инвариантов (для тестов и загруженных файлов).
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.tiles.is_empty() || self.tiles.len() > MAX_TILES_U16 {
            return Err("segment length out of range".into());
        }
        if self.slots.len() != capacity_for(self.tiles.len())
            || (self.head + self.len) as usize > self.slots.len()
        {
            return Err("segment storage inconsistent".into());
        }
        let mut sum = 0u64;
        let mut wide = 0u32;
        for (i, &s) in self.window().iter().enumerate() {
            let gap = gap_of(s);
            if i > 0 && gap < ITEM_SPACING {
                return Err(format!("item {i} closer than spacing"));
            }
            sum += u64::from(gap);
            wide += u32::from(is_wide(i, gap));
        }
        if sum > u64::from(self.len_units()) || sum != u64::from(self.tail_pos) {
            return Err("segment item positions inconsistent".into());
        }
        if wide != self.wide {
            return Err(format!("wide-gap counter {} != actual {wide}", self.wide));
        }
        if self.compressed > self.len {
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
    fn random_operations_keep_invariants() {
        let mut rng = crate::rng::Rng::new(99);
        for len in [1, 2, 5, 17, 32] {
            let mut s = seg(len, 1 + rng.below(40));
            let mut count = 0usize;
            for _ in 0..20_000 {
                match rng.below(6) {
                    0 => {
                        if s.ready_front().is_some() {
                            s.pop_front();
                            count -= 1;
                        }
                    }
                    1 => {
                        count +=
                            usize::from(s.try_insert_behind(IRON, rng.below(len as u32) as usize))
                    }
                    2 => {
                        count +=
                            usize::from(s.try_insert_side(IRON, rng.below(len as u32) as usize))
                    }
                    3 => count += usize::from(s.try_insert_at(IRON, rng.below(s.len_units() + 1))),
                    _ => {
                        s.advance();
                    }
                }
                s.validate().unwrap();
                assert_eq!(s.item_count(), count);
            }
        }
    }

    /// Прогноз — нижняя оценка: при одном лишь движении вставка не удаётся раньше него.
    #[test]
    fn earliest_insert_is_a_lower_bound() {
        let mut rng = crate::rng::Rng::new(7);
        let mut checked = 0;
        for _ in 0..3000 {
            let len = 1 + rng.below(32) as i32;
            let mut s = seg(len, 1 + rng.below(32));
            // случайное заполнение и немного движения/блокировки
            for _ in 0..rng.below(200) {
                match rng.below(3) {
                    0 => {
                        let p = rng.below(s.len_units() + 1);
                        s.try_insert_at(IRON, p);
                    }
                    _ => {
                        s.advance();
                    }
                }
            }
            let pos = rng.below(s.len_units() + 1);
            let mut probe = s.clone();
            if probe.try_insert_at(IRON, pos) {
                continue; // вставка возможна уже сейчас
            }
            let predicted = s.earliest_insert(pos);
            // имитация: выход заблокирован (ничего не выдаётся), только движение
            let horizon = 2 * s.len_units() / s.speed + 2;
            let mut sim = s.clone();
            for t in 1..=horizon {
                sim.advance();
                let mut probe = sim.clone();
                if probe.try_insert_at(IRON, pos) {
                    let p = predicted.expect("insert became possible although predicted never");
                    assert!(p <= t, "predicted {p}, but possible at {t}");
                    break;
                }
            }
            checked += 1;
        }
        assert!(checked > 500, "too few informative cases: {checked}");
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
