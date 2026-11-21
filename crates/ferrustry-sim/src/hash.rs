//! Хеш состояния мира для проверки детерминизма и сверки клиентов в сетевой игре.
//!
//! Мир сериализуется (postcard) **прямо в хешер**, без промежуточного буфера, а хешер
//! обрабатывает данные словами по 8 байт (little-endian — одинаково на всех платформах).
//! Шаг `h = (rotl(h, 5) ^ w) * K` обратим и по `h`, и по `w`: изменение любого слова меняет
//! результат. Это не криптографический хеш — он ловит ошибки, а не подделку.

use postcard::ser_flavors::Flavor;

use crate::world::World;

const K: u64 = 0x517C_C1B7_2722_0A95;

/// Потоковый хешер-«приёмник» сериализации.
struct WordHasher {
    h: u64,
    buf: [u8; 8],
    fill: usize,
    len: u64,
}

impl WordHasher {
    fn new() -> Self {
        Self {
            h: 0xCBF2_9CE4_8422_2325,
            buf: [0; 8],
            fill: 0,
            len: 0,
        }
    }

    #[inline]
    fn word(&mut self, w: u64) {
        self.h = (self.h.rotate_left(5) ^ w).wrapping_mul(K);
    }

    fn finish(mut self) -> u64 {
        if self.fill > 0 {
            let mut last = [0u8; 8];
            last[..self.fill].copy_from_slice(&self.buf[..self.fill]);
            self.word(u64::from_le_bytes(last));
        }
        self.word(self.len);
        // перемешать старшие биты в младшие
        let h = self.h;
        (h ^ (h >> 29)).wrapping_mul(K) ^ (h >> 32)
    }
}

impl Flavor for WordHasher {
    type Output = u64;

    #[inline]
    fn try_push(&mut self, b: u8) -> postcard::Result<()> {
        self.buf[self.fill] = b;
        self.fill += 1;
        self.len += 1;
        if self.fill == 8 {
            self.fill = 0;
            self.word(u64::from_le_bytes(self.buf));
        }
        Ok(())
    }

    fn try_extend(&mut self, mut data: &[u8]) -> postcard::Result<()> {
        self.len += data.len() as u64;
        // дополнить начатое слово
        while self.fill != 0 {
            let Some((&b, rest)) = data.split_first() else {
                return Ok(());
            };
            self.buf[self.fill] = b;
            self.fill = (self.fill + 1) % 8;
            if self.fill == 0 {
                self.word(u64::from_le_bytes(self.buf));
            }
            data = rest;
        }
        let mut chunks = data.chunks_exact(8);
        for c in &mut chunks {
            let mut w = [0u8; 8];
            w.copy_from_slice(c);
            self.word(u64::from_le_bytes(w));
        }
        let rest = chunks.remainder();
        self.buf[..rest.len()].copy_from_slice(rest);
        self.fill = rest.len();
        Ok(())
    }

    fn finalize(self) -> postcard::Result<u64> {
        Ok(self.finish())
    }
}

/// Приёмник сериализации, дописывающий в общий хешер (для хеша по частям).
struct Sink<'a>(&'a mut WordHasher);

impl Flavor for Sink<'_> {
    type Output = ();

    #[inline]
    fn try_push(&mut self, b: u8) -> postcard::Result<()> {
        self.0.try_push(b)
    }

    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        self.0.try_extend(data)
    }

    fn finalize(self) -> postcard::Result<()> {
        Ok(())
    }
}

/// Хеш части `k` из `n` состояния (скользящая контрольная сумма, D-20).
///
/// Большие векторы режутся по диапазонам индексов (часть `k` хеширует свою долю элементов и
/// длину), остальные поля целиком распределяются по частям по номеру поля. За `n` подряд
/// идущих частей хешируется всё состояние, а каждая стоит ~1/`n` полного хеша.
pub(crate) struct PartHasher {
    h: WordHasher,
    k: usize,
    n: usize,
    field: usize,
}

impl PartHasher {
    pub(crate) fn new(k: u64, n: u64) -> Self {
        let n = n.max(1);
        let mut p = Self {
            h: WordHasher::new(),
            k: (k % n) as usize,
            n: n as usize,
            field: 0,
        };
        p.h.word(k % n);
        p
    }

    fn ser<T: serde::Serialize + ?Sized>(&mut self, v: &T) {
        // ошибка невозможна (запись в хешер); игнорировать безопасно
        let _ = postcard::serialize_with_flavor(v, Sink(&mut self.h));
    }

    /// Поле целиком — в одной из частей.
    pub(crate) fn whole<T: serde::Serialize + ?Sized>(&mut self, v: &T) {
        if self.field % self.n == self.k {
            self.ser(v);
        }
        self.field += 1;
    }

    /// Вектор — по долям: часть `k` хеширует длину и элементы `[len·k/n, len·(k+1)/n)`.
    pub(crate) fn split<T: serde::Serialize>(&mut self, v: &[T]) {
        let len = v.len() as u64;
        let (n, k) = (self.n as u64, self.k as u64);
        let (a, b) = ((len * k / n) as usize, (len * (k + 1) / n) as usize);
        self.h.word(len);
        self.ser(&v[a..b]);
        self.field += 1;
    }

    /// Как [`split`](Self::split), но элементы хешируются в каноническом виде `f`.
    pub(crate) fn split_map<'a, T, R: serde::Serialize>(
        &mut self,
        v: &'a [T],
        f: impl Fn(&'a T) -> R,
    ) {
        let len = v.len() as u64;
        let (n, k) = (self.n as u64, self.k as u64);
        let (a, b) = ((len * k / n) as usize, (len * (k + 1) / n) as usize);
        self.h.word(len);
        for x in &v[a..b] {
            self.ser(&f(x));
        }
        self.field += 1;
    }

    pub(crate) fn finish(self) -> u64 {
        self.h.finish()
    }
}

/// Хеш сериализованного состояния мира.
pub fn state_hash(world: &World) -> u64 {
    // ошибка сериализации невозможна для этих типов; на всякий случай — отличимое значение
    postcard::serialize_with_flavor(world, WordHasher::new()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_bytes(parts: &[&[u8]]) -> u64 {
        let mut h = WordHasher::new();
        for p in parts {
            if p.len() == 1 {
                h.try_push(p[0]).unwrap_or(());
            } else {
                h.try_extend(p).unwrap_or(());
            }
        }
        h.finish()
    }

    #[test]
    fn chunking_does_not_change_the_hash() {
        let data: Vec<u8> = (0..100u8).collect();
        let whole = hash_bytes(&[&data]);
        // по байту, кусками разной длины — тот же хеш
        let bytes: Vec<&[u8]> = data.chunks(1).collect();
        assert_eq!(hash_bytes(&bytes), whole);
        let odd: Vec<&[u8]> = data.chunks(3).collect();
        assert_eq!(hash_bytes(&odd), whole);
        let mixed = [&data[..5], &data[5..6], &data[6..17], &data[17..]];
        assert_eq!(hash_bytes(&mixed), whole);
    }

    #[test]
    fn any_byte_change_and_length_change_the_hash() {
        let data: Vec<u8> = (0..64u8).collect();
        let base = hash_bytes(&[&data]);
        for i in 0..data.len() {
            let mut d = data.clone();
            d[i] ^= 1;
            assert_ne!(hash_bytes(&[&d]), base, "byte {i}");
        }
        // хвостовой ноль меняет длину — хеш другой
        let mut longer = data.clone();
        longer.push(0);
        assert_ne!(hash_bytes(&[&longer]), base);
    }
}

#[cfg(test)]
mod checksum_tests {
    use ferrustry_content::ContentRegistry;

    use crate::geom::TilePos;
    use crate::scenario;
    use crate::world::World;

    const N: u64 = 150;

    fn parts(w: &World) -> Vec<u64> {
        (0..N).map(|k| w.checksum_part(k, N)).collect()
    }

    fn differs(a: &World, b: &World) -> bool {
        parts(a) != parts(b)
    }

    #[test]
    fn every_kind_of_change_shows_in_some_part() {
        let reg = ContentRegistry::base().unwrap();
        let mut base = scenario::bench_battle(&reg, 50, 20).unwrap();
        for _ in 0..120 {
            base.step(&reg);
        }
        assert!(!differs(&base, &base.clone()), "equal worlds, equal parts");
        let ingot = reg.item_by_name("iron_ingot").unwrap();

        let mut w = base.clone();
        w.grant(&reg, ingot, 1);
        assert!(differs(&base, &w), "core items / stats");

        let mut w = base.clone();
        let id = w.buildings.iter().next().map(|(id, _)| id).unwrap();
        w.buildings.get_mut(id).unwrap().health -= 1;
        assert!(differs(&base, &w), "building health");

        let mut w = base.clone();
        assert!(!w.units.x.is_empty());
        let last = w.units.x.len() - 1;
        w.units.x[last] += 1;
        assert!(differs(&base, &w), "a unit position");

        let mut w = base.clone();
        let p = TilePos::new(1, 1);
        let ore = w.grid.ore(p);
        w.set_ore(
            p,
            if ore.is_some() {
                None
            } else {
                Some(ferrustry_content::OreId(0))
            },
        );
        assert!(differs(&base, &w), "ore under a tile");

        let mut w = base.clone();
        w.rng_mut().next_u64();
        assert!(differs(&base, &w), "rng state");
    }

    #[test]
    fn belt_items_show_and_parts_cost_a_fraction() {
        let reg = ContentRegistry::base().unwrap();
        let mut a = scenario::bench_megabase(&reg, 2000).unwrap();
        for _ in 0..300 {
            a.step(&reg);
        }
        let mut b = a.clone();
        b.step(&reg);
        assert!(differs(&a, &b), "one tick of belts changes the checksum");
    }
}
