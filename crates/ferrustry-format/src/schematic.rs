//! Файл схемы (`.fschem`): постройки по **именам** контента, а не по ID — схемы переживают
//! изменение порядка контента и переносятся между версиями игры.

use ferrustry_content::{BlockId, ContentRegistry, ItemId};
use ferrustry_sim::Dir;
use ferrustry_sim::schematic::{SchemEntry, Schematic};
use serde::{Deserialize, Serialize};

use crate::{FileKind, FormatError, Sections, Writer, decompress, read_header};

const SECTION_SCHEM: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Entry {
    block: String,
    dx: i32,
    dy: i32,
    rot: Dir,
    filter: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Repr {
    width: i32,
    height: i32,
    entries: Vec<Entry>,
}

/// Записывает схему.
pub fn write_schematic(s: &Schematic, reg: &ContentRegistry) -> Result<Vec<u8>, FormatError> {
    let repr = Repr {
        width: s.width,
        height: s.height,
        entries: s
            .entries
            .iter()
            .map(|e| Entry {
                block: reg.block(e.block).name.clone(),
                dx: e.dx,
                dy: e.dy,
                rot: e.rot,
                filter: e.filter.map(|i| reg.item(i).name.clone()),
            })
            .collect(),
    };
    let mut w = Writer::new(FileKind::Schematic);
    w.section(SECTION_SCHEM, &postcard::to_allocvec(&repr)?)?;
    Ok(w.buf)
}

/// Читает схему, сопоставляя имена с текущим контентом.
pub fn read_schematic(bytes: &[u8], reg: &ContentRegistry) -> Result<Schematic, FormatError> {
    let mut sections = Sections {
        rest: read_header(bytes, FileKind::Schematic)?,
    };
    let mut repr: Option<Repr> = None;
    while let Some((id, data)) = sections.next()? {
        if id == SECTION_SCHEM {
            repr = Some(postcard::from_bytes(&decompress(id, data)?)?);
        }
    }
    let repr = repr.ok_or(FormatError::MissingSection(SECTION_SCHEM))?;
    if !(1..=512).contains(&repr.width)
        || !(1..=512).contains(&repr.height)
        || repr.entries.len() > 100_000
    {
        return Err(FormatError::InvalidWorld(
            "schematic size out of range".into(),
        ));
    }
    let mut entries = Vec::with_capacity(repr.entries.len());
    for e in repr.entries {
        let block: BlockId = reg.block_by_name(&e.block).ok_or_else(|| {
            FormatError::InvalidWorld(format!("unknown block `{}` in schematic", e.block))
        })?;
        let filter: Option<ItemId> = match e.filter {
            Some(n) => Some(reg.item_by_name(&n).ok_or_else(|| {
                FormatError::InvalidWorld(format!("unknown item `{n}` in schematic"))
            })?),
            None => None,
        };
        let s = i32::from(reg.block(block).size);
        if e.dx < 0 || e.dy < 0 || e.dx + s > repr.width || e.dy + s > repr.height {
            return Err(FormatError::InvalidWorld(
                "schematic entry outside its bounds".into(),
            ));
        }
        entries.push(SchemEntry {
            block,
            dx: e.dx,
            dy: e.dy,
            rot: e.rot,
            filter,
        });
    }
    Ok(Schematic {
        width: repr.width,
        height: repr.height,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrustry_sim::scenario;

    #[test]
    fn schematic_roundtrip_and_name_remapping() {
        let reg = ContentRegistry::base().unwrap();
        let (w, _) = scenario::vertical_slice(&reg).unwrap();
        let s = w
            .copy_schematic(
                &reg,
                ferrustry_sim::TilePos::new(0, 0),
                ferrustry_sim::TilePos::new(30, 10),
            )
            .unwrap();
        let bytes = write_schematic(&s, &reg).unwrap();
        assert_eq!(read_schematic(&bytes, &reg).unwrap(), s);
        // другой порядок контента: ID меняются, схема — нет (по именам)
        let shuffled = ferrustry_content::BASE_CONTENT.replacen(
            "name: \"conveyor\"",
            "name: \"conveyor_tmp\"",
            1,
        );
        let other = ContentRegistry::from_ron(&shuffled).unwrap();
        assert!(
            read_schematic(&bytes, &other).is_err(),
            "missing block is reported, not guessed"
        );
        // файл сохранения не читается как схема
        let save = crate::write_save(&w, &reg).unwrap();
        assert!(matches!(
            read_schematic(&save, &reg),
            Err(FormatError::WrongKind { .. })
        ));
    }
}
