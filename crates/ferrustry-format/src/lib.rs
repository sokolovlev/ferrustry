//! Контейнерный формат файлов Ferrustry (см. `docs/compatibility.md`).
//!
//! ```text
//! magic    : 8 байт "FERRUSTR"
//! kind     : u8
//! version  : u16 major, u16 minor (little-endian)
//! flags    : u32
//! sections : повторяются до конца файла: id u32, len u64, crc32 u32, data[len]
//! ```
//! Данные секций сжаты zstd. Неизвестные секции пропускаются.

#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::io::Read;

mod replay;
mod schematic;

pub use schematic::{read_schematic, write_schematic};

pub use replay::{
    Checkpoint, PlayOutcome, Recorder, Replay, TickInput, play, read_replay, write_replay,
};

use ferrustry_content::{ContentRegistry, NameTable};
use ferrustry_sim::World;

/// Сигнатура файла.
pub const MAGIC: &[u8; 8] = b"FERRUSTR";
/// Текущая версия формата.
pub const VERSION: (u16, u16) = (3, 0);
/// Предел размера распакованной секции (защита от «zip-бомб»).
pub const MAX_SECTION: u64 = 512 * 1024 * 1024;

const SECTION_NAMES: u32 = 1;
const SECTION_WORLD: u32 = 2;
const HEADER_LEN: usize = 8 + 1 + 2 + 2 + 4;

/// Вид файла.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FileKind {
    /// Карта.
    Map = 1,
    /// Сохранение.
    Save = 2,
    /// Схема.
    Schematic = 3,
    /// Повтор.
    Replay = 4,
}

impl FileKind {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Map,
            2 => Self::Save,
            3 => Self::Schematic,
            4 => Self::Replay,
            _ => return None,
        })
    }
}

/// Ошибка чтения или записи.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    /// Файл не является файлом Ferrustry.
    #[error("not a Ferrustry file")]
    BadMagic,
    /// Файл обрезан.
    #[error("file is truncated")]
    Truncated,
    /// Неподдерживаемая версия.
    #[error("unsupported format version {0}.{1}")]
    UnsupportedVersion(u16, u16),
    /// Неожиданный вид файла.
    #[error("expected {expected:?} file, found kind {found}")]
    WrongKind {
        /// Ожидаемый вид.
        expected: FileKind,
        /// Найденный код вида.
        found: u8,
    },
    /// Контрольная сумма секции не совпала.
    #[error("section {0} checksum mismatch")]
    Checksum(u32),
    /// Обязательная секция отсутствует.
    #[error("missing section {0}")]
    MissingSection(u32),
    /// Секция больше допустимого.
    #[error("section {0} is too large")]
    TooLarge(u32),
    /// Сохранение сделано с другим контентом. Переназначение ID пока не реализовано.
    #[error("save was made with different content; ID remapping is not implemented yet")]
    ContentMismatch,
    /// Состояние мира в файле не прошло проверку целостности.
    #[error("invalid world state: {0}")]
    InvalidWorld(String),
    /// Replay записан другой версией симуляции.
    #[error("replay was recorded by simulation {file}, this is {current}")]
    SimVersionMismatch {
        /// Версия в файле.
        file: String,
        /// Текущая версия.
        current: String,
    },
    /// Ошибка кодирования.
    #[error("encoding error: {0}")]
    Encoding(#[from] postcard::Error),
    /// Ошибка ввода-вывода (сжатие).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new(kind: FileKind) -> Self {
        let mut buf = Vec::with_capacity(1024);
        buf.extend_from_slice(MAGIC);
        buf.push(kind as u8);
        buf.extend_from_slice(&VERSION.0.to_le_bytes());
        buf.extend_from_slice(&VERSION.1.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        Self { buf }
    }

    fn section(&mut self, id: u32, raw: &[u8]) -> Result<(), FormatError> {
        let data = zstd::bulk::compress(raw, 3)?;
        self.buf.extend_from_slice(&id.to_le_bytes());
        self.buf
            .extend_from_slice(&(data.len() as u64).to_le_bytes());
        self.buf
            .extend_from_slice(&crc32fast::hash(&data).to_le_bytes());
        self.buf.extend_from_slice(&data);
        Ok(())
    }
}

struct Sections<'a> {
    rest: &'a [u8],
}

fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], FormatError> {
    if rest.len() < n {
        return Err(FormatError::Truncated);
    }
    let (a, b) = rest.split_at(n);
    *rest = b;
    Ok(a)
}

fn le_u32(b: &[u8]) -> u32 {
    let mut a = [0; 4];
    a.copy_from_slice(&b[..4]);
    u32::from_le_bytes(a)
}

impl<'a> Sections<'a> {
    fn next(&mut self) -> Result<Option<(u32, &'a [u8])>, FormatError> {
        if self.rest.is_empty() {
            return Ok(None);
        }
        let id = le_u32(take(&mut self.rest, 4)?);
        let mut len = [0; 8];
        len.copy_from_slice(take(&mut self.rest, 8)?);
        let len = usize::try_from(u64::from_le_bytes(len)).map_err(|_| FormatError::Truncated)?;
        let crc = le_u32(take(&mut self.rest, 4)?);
        let data = take(&mut self.rest, len)?;
        if crc32fast::hash(data) != crc {
            return Err(FormatError::Checksum(id));
        }
        Ok(Some((id, data)))
    }
}

fn decompress(id: u32, data: &[u8]) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    let decoder = zstd::stream::read::Decoder::new(data)?;
    decoder.take(MAX_SECTION + 1).read_to_end(&mut out)?;
    if out.len() as u64 > MAX_SECTION {
        return Err(FormatError::TooLarge(id));
    }
    Ok(out)
}

fn read_header(bytes: &[u8], expected: FileKind) -> Result<&[u8], FormatError> {
    if bytes.len() < HEADER_LEN {
        return Err(if bytes.starts_with(&MAGIC[..bytes.len().min(8)]) {
            FormatError::Truncated
        } else {
            FormatError::BadMagic
        });
    }
    if &bytes[..8] != MAGIC {
        return Err(FormatError::BadMagic);
    }
    let kind = bytes[8];
    let major = u16::from_le_bytes([bytes[9], bytes[10]]);
    let minor = u16::from_le_bytes([bytes[11], bytes[12]]);
    if major != VERSION.0 {
        return Err(FormatError::UnsupportedVersion(major, minor));
    }
    if FileKind::from_u8(kind) != Some(expected) {
        return Err(FormatError::WrongKind {
            expected,
            found: kind,
        });
    }
    Ok(&bytes[HEADER_LEN..])
}

/// Записывает сохранение.
pub fn write_save(world: &World, reg: &ContentRegistry) -> Result<Vec<u8>, FormatError> {
    let mut w = Writer::new(FileKind::Save);
    w.section(SECTION_NAMES, &postcard::to_allocvec(&reg.name_table())?)?;
    w.section(SECTION_WORLD, &postcard::to_allocvec(world)?)?;
    Ok(w.buf)
}

/// Читает сохранение и проверяет его целостность относительно контента.
pub fn read_save(bytes: &[u8], reg: &ContentRegistry) -> Result<World, FormatError> {
    let mut sections = Sections {
        rest: read_header(bytes, FileKind::Save)?,
    };
    let mut names: Option<NameTable> = None;
    let mut world: Option<World> = None;
    while let Some((id, data)) = sections.next()? {
        match id {
            SECTION_NAMES => names = Some(postcard::from_bytes(&decompress(id, data)?)?),
            SECTION_WORLD => world = Some(postcard::from_bytes(&decompress(id, data)?)?),
            _ => {} // неизвестная секция более новой minor-версии
        }
    }
    let names = names.ok_or(FormatError::MissingSection(SECTION_NAMES))?;
    let world = world.ok_or(FormatError::MissingSection(SECTION_WORLD))?;
    if names != reg.name_table() {
        return Err(FormatError::ContentMismatch);
    }
    world.validate(reg).map_err(FormatError::InvalidWorld)?;
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrustry_sim::{scenario, state_hash};

    #[test]
    fn rejects_garbage() {
        let reg = ContentRegistry::base().unwrap();
        assert!(matches!(
            read_save(b"hello", &reg),
            Err(FormatError::BadMagic)
        ));
        assert!(matches!(
            read_save(b"FERR", &reg),
            Err(FormatError::Truncated)
        ));
        let (w, _) = scenario::vertical_slice(&reg).unwrap();
        let mut bytes = write_save(&w, &reg).unwrap();
        bytes[8] = FileKind::Map as u8;
        assert!(matches!(
            read_save(&bytes, &reg),
            Err(FormatError::WrongKind { .. })
        ));
        bytes[8] = FileKind::Save as u8;
        bytes[9] = 9;
        assert!(matches!(
            read_save(&bytes, &reg),
            Err(FormatError::UnsupportedVersion(9, 0))
        ));
    }

    #[test]
    fn detects_corruption_and_truncation() {
        let reg = ContentRegistry::base().unwrap();
        let (w, _) = scenario::vertical_slice(&reg).unwrap();
        let bytes = write_save(&w, &reg).unwrap();
        let mut bad = bytes.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        assert!(matches!(
            read_save(&bad, &reg),
            Err(FormatError::Checksum(_))
        ));
        for cut in [HEADER_LEN + 3, bytes.len() - 1] {
            assert!(read_save(&bytes[..cut], &reg).is_err());
        }
    }

    #[test]
    fn content_mismatch_is_reported() {
        let reg = ContentRegistry::base().unwrap();
        let (w, _) = scenario::vertical_slice(&reg).unwrap();
        let bytes = write_save(&w, &reg).unwrap();
        let other = ContentRegistry::from_ron(
            &ferrustry_content::BASE_CONTENT.replace("\"steel\"", "\"bronze\""),
        )
        .unwrap();
        assert!(matches!(
            read_save(&bytes, &other),
            Err(FormatError::ContentMismatch)
        ));
    }

    #[test]
    fn unknown_sections_are_skipped() {
        let reg = ContentRegistry::base().unwrap();
        let (w, _) = scenario::vertical_slice(&reg).unwrap();
        let mut bytes = write_save(&w, &reg).unwrap();
        let mut extra = Writer { buf: Vec::new() };
        extra.section(999, b"future data").unwrap();
        bytes.extend_from_slice(&extra.buf);
        let loaded = read_save(&bytes, &reg).unwrap();
        assert_eq!(state_hash(&loaded), state_hash(&w));
    }
}
