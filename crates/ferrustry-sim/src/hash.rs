//! Хеш состояния мира для проверки детерминизма.

use crate::world::World;

/// FNV-1a 64 поверх сериализованного состояния.
pub fn state_hash(world: &World) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    // Сериализация в память не может завершиться ошибкой для этих типов; на всякий случай
    // ошибка превращается в отличимое значение, а не панику.
    match postcard::to_allocvec(world) {
        Ok(bytes) => {
            for b in bytes {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01B3);
            }
            h
        }
        Err(_) => 0,
    }
}
