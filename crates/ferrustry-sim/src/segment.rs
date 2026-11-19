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
//!
//! ## Пул (P-03)
//!
//! Предметы всех сегментов лежат в одном массиве [`Segments`]: у сегмента — свой диапазон
//! точной ёмкости, окно `[head, head + len)` внутри него. Запись сегмента ([`Segment`]) хранит
//! только метаданные; с предметами работают виды [`SegRef`] и [`SegMut`].

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

/// Сегмент конвейера: запись без предметов. Предметы лежат в общем пуле хранилища
/// [`Segments`] в диапазоне `[base, base + cap)`; работать с ними можно через виды
/// [`SegRef`] и [`SegMut`] (запись + свой срез пула).
#[derive(Debug, Clone)]
pub struct Segment {
    /// Тайлы от входа к выходу.
    pub(crate) tiles: Vec<TilePos>,
    /// Направление последнего тайла (куда выходят предметы).
    pub(crate) out_dir: Dir,
    /// Скорость в единицах за тик.
    pub(crate) speed: u32,
    /// Начало своего диапазона в пуле и его длина (`capacity_for(tiles)`).
    base: u32,
    cap: u32,
    /// Занято окно `[head, head + len)` внутри своего диапазона.
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
#[derive(Deserialize)]
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

/// То же представление для записи — без копирования (поля и порядок совпадают с
/// [`SegmentRepr`], поэтому байты сохранения не зависят от раскладки в памяти).
#[derive(Serialize)]
struct SegmentReprRef<'a> {
    tiles: &'a [TilePos],
    out_dir: Dir,
    speed: u32,
    items: ItemsSer<'a>,
    compressed: u32,
    waiters: &'a [Waiter],
    link: Link,
    link_epoch: u64,
}

/// Окно предметов как последовательность `(предмет, зазор)`.
struct ItemsSer<'a>(&'a [u32]);

impl Serialize for ItemsSer<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter().map(|&x| (item_of(x), gap_of(x))))
    }
}

impl Segment {
    pub(crate) fn new(tiles: Vec<TilePos>, out_dir: Dir, speed: u32) -> Self {
        debug_assert!(!tiles.is_empty() && tiles.len() <= MAX_TILES_U16);
        let cap = capacity_for(tiles.len()) as u32;
        Self {
            tiles,
            out_dir,
            speed,
            base: 0,
            cap,
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

    fn range(&self) -> std::ops::Range<usize> {
        self.base as usize..(self.base + self.cap) as usize
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

    /// Длина сжатого неподвижного префикса.
    pub(crate) fn compressed_len(&self) -> u32 {
        self.compressed
    }

    /// Позиция вставки (расстояние от конца) для входа на тайл `k` сзади или сбоку.
    pub(crate) fn insert_pos(&self, k: usize, behind: bool) -> u32 {
        let l = self.tiles.len().max(k + 1);
        let base = (l - k) as u32 * TILE_LEN;
        if behind { base } else { base - TILE_LEN / 2 }
    }

    /// Все предметы стоят вплотную у заблокированного выхода.
    pub(crate) fn is_fully_compressed(&self) -> bool {
        self.len > 0 && self.compressed == self.len
    }
}

/// Сегмент для чтения: запись и её диапазон пула.
#[derive(Debug, Clone, Copy)]
pub struct SegRef<'a> {
    seg: &'a Segment,
    slots: &'a [u32],
}

impl std::ops::Deref for SegRef<'_> {
    type Target = Segment;
    fn deref(&self) -> &Segment {
        self.seg
    }
}

/// Сегмент для изменения: запись и её диапазон пула.
#[derive(Debug)]
pub(crate) struct SegMut<'a> {
    seg: &'a mut Segment,
    slots: &'a mut [u32],
}

impl std::ops::Deref for SegMut<'_> {
    type Target = Segment;
    fn deref(&self) -> &Segment {
        self.seg
    }
}

impl std::ops::DerefMut for SegMut<'_> {
    fn deref_mut(&mut self) -> &mut Segment {
        self.seg
    }
}

impl<'a> SegRef<'a> {
    #[inline(always)]
    fn window(&self) -> &'a [u32] {
        &self.slots[self.seg.head as usize..(self.seg.head + self.seg.len) as usize]
    }

    /// Предметы (от выхода к входу).
    pub fn items(&self) -> impl Iterator<Item = ItemId> + 'a {
        self.window().iter().map(|&s| item_of(s))
    }

    /// Предметы с расстоянием от конца сегмента, от выхода к входу.
    pub fn item_positions(&self) -> impl Iterator<Item = (ItemId, u32)> + 'a {
        self.window().iter().scan(0u32, |pos, &s| {
            *pos += gap_of(s);
            Some((item_of(s), *pos))
        })
    }

    /// Предмет, готовый к выходу (стоит в самом конце).
    #[inline]
    pub(crate) fn ready_front(&self) -> Option<ItemId> {
        if self.seg.len == 0 {
            return None;
        }
        let s = self.slots[self.seg.head as usize];
        (gap_of(s) == 0).then(|| item_of(s))
    }

    /// Свободное место перед первым предметом (у выхода); для пустого сегмента — без ограничения.
    pub(crate) fn front_free(&self) -> u32 {
        if self.seg.len == 0 {
            u32::MAX
        } else {
            gap_of(self.slots[self.seg.head as usize])
        }
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

    /// Нижняя оценка числа тиков, через которое вставка в позицию `pos` может стать возможной,
    /// если ничего, кроме равномерного движения несжатых предметов, не произойдёт.
    /// `None` — при текущем движении не станет возможной никогда (нужен внешний повод: выдача
    /// предмета из заблокированной ленты, освобождение головы, перестройка).
    ///
    /// Оценка занижена (округление вниз, без учёта будущих остановок), поэтому разбудить
    /// ожидающего по ней можно только раньше, чем вставка реально станет возможной.
    pub(crate) fn earliest_insert(&self, pos: u32) -> Option<u32> {
        let n = self.seg.len as usize;
        if n == 0 {
            return Some(0);
        }
        let d = i64::from(self.seg.speed.max(1));
        let c = self.seg.compressed as usize;
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
        if n > c && pos <= i64::from(self.seg.len_units()) {
            take(prev + sp - pos);
        }
        best.map(|t| u32::try_from(t).unwrap_or(u32::MAX))
    }

    /// Проверка инвариантов (для тестов и загруженных файлов).
    pub(crate) fn validate(&self) -> Result<(), String> {
        let seg = self.seg;
        if seg.tiles.is_empty() || seg.tiles.len() > MAX_TILES_U16 {
            return Err("segment length out of range".into());
        }
        if self.slots.len() != capacity_for(seg.tiles.len())
            || (seg.head + seg.len) as usize > self.slots.len()
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
        if sum > u64::from(seg.len_units()) || sum != u64::from(seg.tail_pos) {
            return Err("segment item positions inconsistent".into());
        }
        if wide != seg.wide {
            return Err(format!("wide-gap counter {} != actual {wide}", seg.wide));
        }
        if seg.compressed > seg.len {
            return Err("segment compressed prefix out of range".into());
        }
        Ok(())
    }

    /// Равенство по наблюдаемому состоянию (без учёта места в пуле).
    fn same(&self, o: &SegRef<'_>) -> bool {
        let (a, b) = (self.seg, o.seg);
        a.tiles == b.tiles
            && a.out_dir == b.out_dir
            && a.speed == b.speed
            && self.window() == o.window()
            && a.compressed == b.compressed
            && a.tail_pos == b.tail_pos
            && a.wide == b.wide
            && a.waiters == b.waiters
            && a.link == b.link
            && a.link_epoch == b.link_epoch
    }

    fn repr(&self) -> SegmentReprRef<'a> {
        SegmentReprRef {
            tiles: &self.seg.tiles,
            out_dir: self.seg.out_dir,
            speed: self.seg.speed,
            items: ItemsSer(self.window()),
            compressed: self.seg.compressed,
            waiters: &self.seg.waiters,
            link: self.seg.link,
            link_epoch: self.seg.link_epoch,
        }
    }
}

impl SegMut<'_> {
    /// Вид только для чтения.
    #[inline(always)]
    pub(crate) fn r(&self) -> SegRef<'_> {
        SegRef {
            seg: self.seg,
            slots: self.slots,
        }
    }

    /// Свободное место перед первым предметом.
    pub(crate) fn front_free(&self) -> u32 {
        self.r().front_free()
    }

    /// Нижняя оценка тиков до возможности вставки (см. [`SegRef::earliest_insert`]).
    pub(crate) fn earliest_insert(&self, pos: u32) -> Option<u32> {
        self.r().earliest_insert(pos)
    }

    /// Проверка инвариантов.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.r().validate()
    }

    #[inline(always)]
    fn window_mut(&mut self) -> &mut [u32] {
        &mut self.slots[self.seg.head as usize..(self.seg.head + self.seg.len) as usize]
    }

    /// Переносит окно в начало диапазона, освобождая место в конце.
    fn compact(&mut self) {
        let (h, n) = (self.seg.head as usize, self.seg.len as usize);
        if h > 0 {
            self.slots.copy_within(h..h + n, 0);
            self.slots[n..h + n].fill(0);
            self.seg.head = 0;
        }
    }

    /// Освобождает ячейку после конца окна; `false`, если диапазон полон.
    fn reserve_back(&mut self) -> bool {
        if (self.seg.len as usize) >= self.slots.len() {
            return false;
        }
        if (self.seg.head + self.seg.len) as usize == self.slots.len() {
            self.compact();
        }
        true
    }

    pub(crate) fn pop_front(&mut self) -> Option<ItemId> {
        if self.seg.len == 0 {
            return None;
        }
        let h = self.seg.head as usize;
        let first = self.slots[h];
        self.slots[h] = 0;
        self.seg.len -= 1;
        if self.seg.len == 0 {
            self.seg.head = 0;
            self.seg.tail_pos = 0;
        } else {
            self.seg.head += 1;
            let next = self.slots[h + 1];
            // бывший второй предмет становится первым: его зазор больше не «внутренний»
            if is_wide(1, gap_of(next)) {
                self.seg.wide -= 1;
            }
            self.slots[h + 1] = with_gap(next, gap_of(next) + gap_of(first));
        }
        self.seg.compressed = 0;
        Some(item_of(first))
    }

    fn push_back(&mut self, item: ItemId, gap: u32) -> bool {
        if !self.reserve_back() {
            return false;
        }
        self.slots[(self.seg.head + self.seg.len) as usize] = pack(item, gap);
        self.seg.len += 1;
        true
    }

    /// Вставка в окно перед индексом `i` (сдвигает хвост на одну ячейку).
    fn insert_at_index(&mut self, i: usize, slot: u32) -> bool {
        if !self.reserve_back() {
            return false;
        }
        let (h, n) = (self.seg.head as usize, self.seg.len as usize);
        if i == 0 && h > 0 {
            self.seg.head -= 1;
            self.slots[h - 1] = slot;
        } else {
            self.slots.copy_within(h + i..h + n, h + i + 1);
            self.slots[h + i] = slot;
        }
        self.seg.len += 1;
        true
    }

    /// Вставка предмета на расстоянии `pos` от конца сегмента.
    pub(crate) fn try_insert_at(&mut self, item: ItemId, pos: u32) -> bool {
        if pos > self.seg.len_units() {
            return false;
        }
        if self.seg.len == 0 {
            if !self.push_back(item, pos) {
                return false;
            }
            self.seg.tail_pos = pos;
            self.seg.compressed = 0;
            return true;
        }
        let front_gap = gap_of(self.slots[self.seg.head as usize]);
        // быстрый путь: за последним предметом
        if pos >= self.seg.tail_pos + ITEM_SPACING {
            let gap = pos - self.seg.tail_pos;
            let idx = self.seg.len as usize;
            if !self.push_back(item, gap) {
                return false;
            }
            self.seg.wide += u32::from(is_wide(idx, gap));
            self.seg.tail_pos = pos;
            return true;
        }
        // быстрый путь: перед первым предметом
        if pos + ITEM_SPACING <= front_gap {
            let rest = front_gap - pos;
            if !self.insert_at_index(0, pack(item, pos)) {
                return false;
            }
            let h = self.seg.head as usize;
            self.slots[h + 1] = with_gap(self.slots[h + 1], rest);
            self.seg.wide += u32::from(is_wide(1, rest));
            self.seg.compressed = 0;
            return true;
        }
        // внутри: возможно только при наличии широкого зазора
        if self.seg.wide == 0 {
            return false;
        }
        let Some((i, prev, next)) = self.r().locate(pos) else {
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
        let h = self.seg.head as usize;
        self.slots[h + i + 1] = with_gap(self.slots[h + i + 1], b);
        self.seg.wide = self.seg.wide - u32::from(is_wide(i, gap))
            + u32::from(is_wide(i, a))
            + u32::from(is_wide(i + 1, b));
        self.seg.compressed = self.seg.compressed.min(i as u32);
        true
    }

    /// Вставка на входе тайла `k` (сзади, по направлению движения).
    pub(crate) fn try_insert_behind(&mut self, item: ItemId, k: usize) -> bool {
        let l = self.seg.tiles.len();
        if k >= l {
            return false;
        }
        self.try_insert_at(item, (l - k) as u32 * TILE_LEN)
    }

    /// Вставка в центр тайла `k` (сбоку).
    pub(crate) fn try_insert_side(&mut self, item: ItemId, k: usize) -> bool {
        let l = self.seg.tiles.len();
        if k >= l {
            return false;
        }
        self.try_insert_at(item, (l - k) as u32 * TILE_LEN - TILE_LEN / 2)
    }

    /// Сдвигает все предметы на `speed` с учётом блокировки и сжатия.
    /// Возвращает `true`, если сдвинулся хотя бы один предмет.
    #[inline]
    pub(crate) fn advance(&mut self) -> bool {
        let d = self.seg.speed;
        let start = self.seg.compressed as usize;
        let mut compressed = self.seg.compressed;
        let mut wide = self.seg.wide;
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
        self.seg.compressed = compressed;
        self.seg.wide = wide;
        self.seg.tail_pos -= if all_full { d } else { prev_m };
        moved
    }

    /// Разбирает сегмент на предметы с привязкой к тайлам: (индекс тайла, предмет, смещение от конца тайла).
    pub(crate) fn drain_by_tile(&mut self, mut f: impl FnMut(usize, ItemId, u32)) {
        let l = self.seg.tiles.len();
        let mut pos = 0u32;
        for &s in self.r().window() {
            pos += gap_of(s);
            let from_end = ((pos / TILE_LEN) as usize).min(l.saturating_sub(1));
            let k = l - 1 - from_end;
            f(k, item_of(s), pos - from_end as u32 * TILE_LEN);
        }
        self.slots.fill(0);
        self.seg.head = 0;
        self.seg.len = 0;
        self.seg.tail_pos = 0;
        self.seg.compressed = 0;
        self.seg.wide = 0;
    }

    /// Заполняет пустой сегмент предметами, отсортированными по расстоянию от конца; раздвигает
    /// слишком близкие. Возвращает число не поместившихся предметов.
    pub(crate) fn fill_sorted(&mut self, sorted: impl Iterator<Item = (ItemId, u32)>) -> u32 {
        debug_assert_eq!(self.seg.len, 0);
        let len = self.seg.len_units();
        let mut lost = 0;
        let mut prev: Option<u32> = None;
        for (item, pos) in sorted {
            let pos = match prev {
                Some(p) => pos.max(p + ITEM_SPACING),
                None => pos,
            };
            let gap = pos - prev.unwrap_or(0);
            let idx = self.seg.len as usize;
            if pos > len || !self.push_back(item, gap) {
                lost += 1;
                continue;
            }
            self.seg.wide += u32::from(is_wide(idx, gap));
            prev = Some(pos);
        }
        self.seg.tail_pos = prev.unwrap_or(0);
        self.seg.compressed = 0;
        lost
    }
}

/// Хранилище сегментов (P-03): записи по индексам и **один общий пул предметов**.
///
/// Каждый сегмент владеет диапазоном пула `[base, base + cap)`. Новый сегмент получает диапазон
/// в конце пула; освобождённые диапазоны копятся как «дыры», и когда их больше четверти пула,
/// пул уплотняется **в порядке индексов сегментов**. Тик обходит сегменты по возрастанию
/// индекса, поэтому предметы читаются почти последовательно — их предвыбирает процессор,
/// в отличие от отдельных буферов в случайных местах кучи (−11% времени тика B3-100k в
/// эксперименте). Раскладка ненаблюдаема: сохранение и сравнение миров идут по содержимому.
#[derive(Debug, Clone, Default)]
pub struct Segments {
    list: Vec<Option<Segment>>,
    pool: Vec<u32>,
    /// Ячеек пула в освобождённых диапазонах.
    holes: usize,
}

/// Уплотнять пул, когда дыр больше четверти и больше этого числа ячеек.
const COMPACT_MIN_HOLES: usize = 4096;

impl Segments {
    /// Число индексов (включая пустые).
    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }

    /// Есть ли сегмент с этим индексом.
    pub(crate) fn is_live(&self, i: usize) -> bool {
        self.list.get(i).is_some_and(Option::is_some)
    }

    /// Сегмент для чтения.
    #[inline]
    pub(crate) fn get(&self, i: usize) -> Option<SegRef<'_>> {
        let seg = self.list.get(i)?.as_ref()?;
        Some(SegRef {
            seg,
            slots: &self.pool[seg.range()],
        })
    }

    /// Сегмент для изменения.
    #[inline]
    pub(crate) fn get_mut(&mut self, i: usize) -> Option<SegMut<'_>> {
        let seg = self.list.get_mut(i)?.as_mut()?;
        let range = seg.range();
        Some(SegMut {
            seg,
            slots: &mut self.pool[range],
        })
    }

    /// Все сегменты в порядке индексов.
    pub(crate) fn iter(&self) -> impl Iterator<Item = SegRef<'_>> {
        let pool = &self.pool;
        self.list.iter().flatten().map(move |seg| SegRef {
            seg,
            slots: &pool[seg.range()],
        })
    }

    /// Все сегменты с индексами.
    pub(crate) fn enumerate(&self) -> impl Iterator<Item = (usize, SegRef<'_>)> {
        let pool = &self.pool;
        self.list.iter().enumerate().filter_map(move |(i, s)| {
            s.as_ref().map(|seg| {
                (
                    i,
                    SegRef {
                        seg,
                        slots: &pool[seg.range()],
                    },
                )
            })
        })
    }

    /// Применить `f` к каждому сегменту.
    pub(crate) fn for_each_mut(&mut self, mut f: impl FnMut(SegMut<'_>)) {
        let pool = &mut self.pool;
        for seg in self.list.iter_mut().flatten() {
            let range = seg.range();
            f(SegMut {
                seg,
                slots: &mut pool[range],
            });
        }
    }

    /// Длина пула в ячейках (для тестов: после уплотнения уменьшается).
    #[cfg(test)]
    pub(crate) fn pool_len(&self) -> usize {
        self.pool.len()
    }

    /// Записи сегментов (без предметов) — для служебных правок кэшей.
    #[cfg(test)]
    pub(crate) fn records_mut(&mut self) -> impl Iterator<Item = &mut Segment> {
        self.list.iter_mut().flatten()
    }

    /// Поместить сегмент на свободный индекс `i` (или в конец при `i == len`): получает
    /// пустой диапазон в конце пула.
    pub(crate) fn place(&mut self, i: usize, mut seg: Segment) {
        seg.base = u32::try_from(self.pool.len()).unwrap_or(u32::MAX);
        seg.head = 0;
        self.pool.resize(self.pool.len() + seg.cap as usize, 0);
        if i == self.list.len() {
            self.list.push(Some(seg));
        } else if let Some(slot) = self.list.get_mut(i) {
            debug_assert!(slot.is_none(), "segment slot {i} is occupied");
            *slot = Some(seg);
        }
    }

    /// Убрать сегмент; его диапазон становится дырой (при избытке дыр пул уплотняется).
    pub(crate) fn remove(&mut self, i: usize) -> Option<Segment> {
        let seg = self.list.get_mut(i)?.take()?;
        self.pool[seg.range()].fill(0);
        self.holes += seg.cap as usize;
        if self.holes > COMPACT_MIN_HOLES && self.holes * 4 > self.pool.len() {
            self.compact();
        }
        Some(seg)
    }

    /// Уплотнить пул: диапазоны живых сегментов подряд в порядке индексов.
    fn compact(&mut self) {
        let mut pool = Vec::with_capacity(self.pool.len() - self.holes);
        for seg in self.list.iter_mut().flatten() {
            let r = seg.range();
            seg.base = u32::try_from(pool.len()).unwrap_or(u32::MAX);
            pool.extend_from_slice(&self.pool[r]);
        }
        self.pool = pool;
        self.holes = 0;
    }
}

impl PartialEq for Segments {
    fn eq(&self, o: &Self) -> bool {
        self.list.len() == o.list.len()
            && (0..self.list.len()).all(|i| match (self.get(i), o.get(i)) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same(&b),
                _ => false,
            })
    }
}

impl Eq for Segments {}

impl Serialize for Segments {
    /// Как прежний `Vec<Option<Segment>>`: байты сохранения не зависят от раскладки пула.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(
            self.list
                .iter()
                .map(|o| o.as_ref().map(|seg| self.view(seg).repr())),
        )
    }
}

impl Segments {
    fn view<'a>(&'a self, seg: &'a Segment) -> SegRef<'a> {
        SegRef {
            seg,
            slots: &self.pool[seg.range()],
        }
    }
}

impl<'de> Deserialize<'de> for Segments {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let reprs: Vec<Option<SegmentRepr>> = Vec::deserialize(d)?;
        let mut out = Segments::default();
        for (i, r) in reprs.into_iter().enumerate() {
            match r {
                None => out.list.push(None),
                Some(r) => out.load(i, r).map_err(serde::de::Error::custom)?,
            }
        }
        Ok(out)
    }
}

impl Segments {
    /// Восстановить сегмент из сохранения (с проверкой инвариантов).
    fn load(&mut self, i: usize, r: SegmentRepr) -> Result<(), String> {
        if r.tiles.is_empty() || r.tiles.len() > MAX_TILES_U16 {
            return Err("segment length out of range".into());
        }
        let seg = Segment::new(r.tiles, r.out_dir, r.speed);
        if r.items.len() > seg.cap as usize {
            return Err("too many items on segment".into());
        }
        let len_units = seg.len_units();
        self.place(i, seg);
        let Some(m) = self.get_mut(i) else {
            return Err("segment slot unavailable".into());
        };
        for (k, &(item, gap)) in r.items.iter().enumerate() {
            if gap > len_units {
                return Err("item gap out of range".into());
            }
            m.slots[k] = pack(item, gap);
            m.seg.tail_pos = m.seg.tail_pos.saturating_add(gap);
            m.seg.wide += u32::from(is_wide(k, gap));
        }
        m.seg.len = r.items.len() as u32;
        m.seg.compressed = r.compressed;
        m.seg.waiters = r.waiters;
        m.seg.link = r.link;
        m.seg.link_epoch = r.link_epoch;
        m.validate()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const IRON: ItemId = ItemId(0);

    /// Отдельный сегмент со своим пулом — для тестов логики сегмента.
    #[derive(Clone)]
    struct Owned(Segments);

    impl Owned {
        fn v(&mut self) -> SegMut<'_> {
            self.0.get_mut(0).unwrap()
        }

        fn r(&self) -> SegRef<'_> {
            self.0.get(0).unwrap()
        }
    }

    impl std::ops::Deref for Owned {
        type Target = Segment;
        fn deref(&self) -> &Segment {
            self.0.list[0].as_ref().unwrap()
        }
    }

    fn seg(len: i32, speed: u32) -> Owned {
        let mut store = Segments::default();
        store.place(
            0,
            Segment::new(
                (0..len).map(|x| TilePos::new(x, 0)).collect(),
                Dir::Right,
                speed,
            ),
        );
        Owned(store)
    }

    #[test]
    fn item_travels_full_length() {
        let mut s = seg(4, 16);
        assert!(s.v().try_insert_behind(IRON, 0));
        let mut ticks = 0;
        while s.r().ready_front().is_none() {
            s.v().advance();
            ticks += 1;
            s.r().validate().unwrap();
        }
        assert_eq!(ticks, 4 * TILE_LEN / 16);
    }

    #[test]
    fn blocked_line_compresses_to_spacing() {
        let mut s = seg(2, 16);
        for _ in 0..200 {
            s.v().try_insert_behind(IRON, 0);
            s.v().advance();
            s.r().validate().unwrap();
        }
        // выход заблокирован: 2 тайла по 4 предмета + один на самом входе
        assert_eq!(s.item_count(), 9);
        let positions: Vec<u32> = s.r().item_positions().map(|(_, p)| p).collect();
        assert_eq!(positions[0], 0);
        for w in positions.windows(2) {
            assert_eq!(w[1] - w[0], ITEM_SPACING);
        }
    }

    #[test]
    fn unblocking_resumes_motion() {
        let mut s = seg(2, 16);
        for _ in 0..200 {
            s.v().try_insert_behind(IRON, 0);
            s.v().advance();
        }
        let mut out = 0;
        for _ in 0..1000 {
            if s.r().ready_front().is_some() {
                s.v().pop_front();
                out += 1;
            }
            s.v().advance();
            s.r().validate().unwrap();
        }
        assert_eq!(out, 9);
        assert_eq!(s.item_count(), 0);
    }

    #[test]
    fn side_insert_respects_spacing() {
        let mut s = seg(3, 16);
        assert!(s.v().try_insert_side(IRON, 1));
        assert!(!s.v().try_insert_side(IRON, 1));
        assert!(s.v().try_insert_side(IRON, 0));
        assert!(s.v().try_insert_side(IRON, 2));
        s.r().validate().unwrap();
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
                        if s.r().ready_front().is_some() {
                            s.v().pop_front();
                            count -= 1;
                        }
                    }
                    1 => {
                        count += usize::from(
                            s.v()
                                .try_insert_behind(IRON, rng.below(len as u32) as usize),
                        )
                    }
                    2 => {
                        count +=
                            usize::from(s.v().try_insert_side(IRON, rng.below(len as u32) as usize))
                    }
                    3 => {
                        let p = rng.below(s.len_units() + 1);
                        count += usize::from(s.v().try_insert_at(IRON, p))
                    }
                    _ => {
                        s.v().advance();
                    }
                }
                s.r().validate().unwrap();
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
                        s.v().try_insert_at(IRON, p);
                    }
                    _ => {
                        s.v().advance();
                    }
                }
            }
            let pos = rng.below(s.len_units() + 1);
            let mut probe = s.clone();
            if probe.v().try_insert_at(IRON, pos) {
                continue; // вставка возможна уже сейчас
            }
            let predicted = s.r().earliest_insert(pos);
            // имитация: выход заблокирован (ничего не выдаётся), только движение
            let horizon = 2 * s.len_units() / s.speed + 2;
            let mut sim = s.clone();
            for t in 1..=horizon {
                sim.v().advance();
                let mut probe = sim.clone();
                if probe.v().try_insert_at(IRON, pos) {
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
        s.v().try_insert_side(IRON, 0);
        s.v().try_insert_side(IRON, 2);
        s.v().advance();
        let before: Vec<_> = s.r().item_positions().collect();
        let mut items = Vec::new();
        s.v()
            .drain_by_tile(|k, item, off| items.push((item, (2 - k as u32) * TILE_LEN + off)));
        items.sort_by_key(|&(_, p)| p);
        assert_eq!(s.v().fill_sorted(items.into_iter()), 0);
        let after: Vec<_> = s.r().item_positions().collect();
        assert_eq!(before, after);
    }

    /// Уплотнение пула не меняет содержимого сегментов и раскладывает их по индексам.
    #[test]
    fn pool_compaction_keeps_contents() {
        let mut rng = crate::rng::Rng::new(5);
        let mut store = Segments::default();
        let mut free: Vec<usize> = Vec::new();
        for step in 0..4000 {
            if !free.is_empty() && rng.below(3) == 0 || store.len() < 8 {
                let len = 1 + rng.below(32) as i32;
                let i = free.pop().unwrap_or(store.len());
                store.place(
                    i,
                    Segment::new(
                        (0..len).map(|x| TilePos::new(x, step)).collect(),
                        Dir::Right,
                        1 + rng.below(32),
                    ),
                );
                let mut m = store.get_mut(i).unwrap();
                for _ in 0..rng.below(40) {
                    let p = rng.below(m.len_units() + 1);
                    m.try_insert_at(IRON, p);
                    m.advance();
                }
            } else if rng.below(2) == 0 {
                let i = rng.below(store.len() as u32) as usize;
                if store.remove(i).is_some() {
                    free.push(i);
                }
            }
            let before: Vec<(usize, Vec<(ItemId, u32)>)> = store
                .enumerate()
                .map(|(i, s)| (i, s.item_positions().collect()))
                .collect();
            for (i, s) in store.enumerate() {
                s.validate().unwrap_or_else(|e| panic!("segment {i}: {e}"));
            }
            assert!(store.holes <= store.pool.len());
            let copy = store.clone();
            store.compact();
            assert!(store == copy, "compaction changed observable state");
            let after: Vec<(usize, Vec<(ItemId, u32)>)> = store
                .enumerate()
                .map(|(i, s)| (i, s.item_positions().collect()))
                .collect();
            assert_eq!(before, after);
            // после уплотнения диапазоны идут подряд по возрастанию индексов
            let bases: Vec<u32> = store.list.iter().flatten().map(|s| s.base).collect();
            assert!(bases.windows(2).all(|w| w[0] < w[1]));
        }
    }
}
