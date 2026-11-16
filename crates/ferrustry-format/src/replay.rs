//! Replay: начальное состояние мира + поток команд + контрольные хеши.
//!
//! Используется для отладки рассинхронов, регрессионных тестов и воспроизведения багов.
//! Воспроизводится только той же версией симуляции ([`ferrustry_sim::SIM_VERSION`]) и с тем
//! же контентом.

use ferrustry_content::{ContentRegistry, NameTable};
use ferrustry_sim::{Command, CommandError, SIM_VERSION, World, state_hash};
use serde::{Deserialize, Serialize};

use crate::{FileKind, FormatError, Sections, Writer, decompress, read_header};

const SECTION_NAMES: u32 = 1;
const SECTION_WORLD: u32 = 2;
const SECTION_META: u32 = 3;
const SECTION_INPUTS: u32 = 4;
const SECTION_CHECKPOINTS: u32 = 5;

/// Команды, применённые в начале тика.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickInput {
    /// Номер тика мира перед применением.
    pub tick: u64,
    /// Команды в порядке применения.
    pub commands: Vec<Command>,
}

/// Контрольная точка: хеш состояния после тика.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Номер тика мира после шага.
    pub tick: u64,
    /// [`state_hash`] в этот момент.
    pub hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Meta {
    sim_version: String,
    start_tick: u64,
    end_tick: u64,
    checkpoint_every: u64,
}

/// Записанная партия.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    meta: Meta,
    initial: World,
    inputs: Vec<TickInput>,
    checkpoints: Vec<Checkpoint>,
}

impl Replay {
    /// Число записанных тиков.
    pub fn ticks(&self) -> u64 {
        self.meta.end_tick - self.meta.start_tick
    }

    /// Контрольные точки.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Все записанные команды.
    pub fn inputs(&self) -> &[TickInput] {
        &self.inputs
    }

    /// Изменяемый доступ к командам (для тестов и инструментов «что если»).
    pub fn inputs_mut(&mut self) -> &mut Vec<TickInput> {
        &mut self.inputs
    }

    /// Начальное состояние.
    pub fn initial(&self) -> &World {
        &self.initial
    }
}

/// Запись партии во время игры.
#[derive(Debug)]
pub struct Recorder {
    replay: Replay,
}

impl Recorder {
    /// Начинает запись с текущего состояния мира; контрольная точка каждые `checkpoint_every` тиков.
    pub fn new(world: &World, checkpoint_every: u64) -> Self {
        Self {
            replay: Replay {
                meta: Meta {
                    sim_version: SIM_VERSION.to_owned(),
                    start_tick: world.tick_count(),
                    end_tick: world.tick_count(),
                    checkpoint_every: checkpoint_every.max(1),
                },
                initial: world.clone(),
                inputs: Vec::new(),
                checkpoints: Vec::new(),
            },
        }
    }

    /// Выполняет тик мира с командами и записывает его.
    pub fn tick(
        &mut self,
        world: &mut World,
        reg: &ContentRegistry,
        commands: &[Command],
    ) -> Vec<Result<(), CommandError>> {
        if !commands.is_empty() {
            self.replay.inputs.push(TickInput {
                tick: world.tick_count(),
                commands: commands.to_vec(),
            });
        }
        let results = world.tick(reg, commands);
        let t = world.tick_count();
        self.replay.meta.end_tick = t;
        if (t - self.replay.meta.start_tick) % self.replay.meta.checkpoint_every == 0 {
            self.replay.checkpoints.push(Checkpoint {
                tick: t,
                hash: state_hash(world),
            });
        }
        results
    }

    /// Завершает запись (добавляет финальную контрольную точку).
    pub fn finish(mut self, world: &World) -> Replay {
        let t = world.tick_count();
        if self.replay.checkpoints.last().is_none_or(|c| c.tick != t) {
            self.replay.checkpoints.push(Checkpoint {
                tick: t,
                hash: state_hash(world),
            });
        }
        self.replay
    }
}

/// Результат проверки replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayOutcome {
    /// Все контрольные точки совпали.
    Verified {
        /// Проверено контрольных точек.
        checkpoints: usize,
        /// Итоговый хеш.
        final_hash: u64,
    },
    /// Рассинхрон: первая несовпавшая контрольная точка.
    Diverged {
        /// Тик, на котором хеш разошёлся.
        tick: u64,
        /// Записанный хеш.
        expected: u64,
        /// Полученный хеш.
        actual: u64,
        /// Последний тик, на котором состояние ещё совпадало (`None` — расхождение с первой точки).
        last_good_tick: Option<u64>,
    },
}

/// Воспроизводит партию и сверяет контрольные точки. Возвращает итог и мир в конечном состоянии
/// (или в момент рассинхрона).
pub fn play(replay: &Replay, reg: &ContentRegistry) -> (PlayOutcome, World) {
    let mut w = replay.initial.clone();
    let mut inputs = replay.inputs.iter().peekable();
    let mut checks = replay.checkpoints.iter().peekable();
    let mut last_good = None;
    let mut verified = 0;
    let empty: Vec<Command> = Vec::new();
    while w.tick_count() < replay.meta.end_tick {
        let cmds = match inputs.peek() {
            Some(i) if i.tick == w.tick_count() => inputs.next().map_or(&empty, |i| &i.commands),
            _ => &empty,
        };
        w.tick(reg, cmds);
        let t = w.tick_count();
        while let Some(c) = checks.peek() {
            if c.tick > t {
                break;
            }
            if c.tick == t {
                let actual = state_hash(&w);
                if actual != c.hash {
                    return (
                        PlayOutcome::Diverged {
                            tick: t,
                            expected: c.hash,
                            actual,
                            last_good_tick: last_good,
                        },
                        w,
                    );
                }
                verified += 1;
                last_good = Some(t);
            }
            checks.next();
        }
    }
    let final_hash = state_hash(&w);
    (
        PlayOutcome::Verified {
            checkpoints: verified,
            final_hash,
        },
        w,
    )
}

/// Записывает replay в файл формата Ferrustry.
pub fn write_replay(replay: &Replay, reg: &ContentRegistry) -> Result<Vec<u8>, FormatError> {
    let mut w = Writer::new(FileKind::Replay);
    w.section(SECTION_NAMES, &postcard::to_allocvec(&reg.name_table())?)?;
    w.section(SECTION_META, &postcard::to_allocvec(&replay.meta)?)?;
    w.section(SECTION_WORLD, &postcard::to_allocvec(&replay.initial)?)?;
    w.section(SECTION_INPUTS, &postcard::to_allocvec(&replay.inputs)?)?;
    w.section(
        SECTION_CHECKPOINTS,
        &postcard::to_allocvec(&replay.checkpoints)?,
    )?;
    Ok(w.buf)
}

/// Читает replay; отклоняет файл другой версии симуляции или с другим контентом.
pub fn read_replay(bytes: &[u8], reg: &ContentRegistry) -> Result<Replay, FormatError> {
    let mut sections = Sections {
        rest: read_header(bytes, FileKind::Replay)?,
    };
    let (mut names, mut meta, mut world, mut inputs, mut checks) = (None, None, None, None, None);
    while let Some((id, data)) = sections.next()? {
        match id {
            SECTION_NAMES => {
                names = Some(postcard::from_bytes::<NameTable>(&decompress(id, data)?)?)
            }
            SECTION_META => meta = Some(postcard::from_bytes::<Meta>(&decompress(id, data)?)?),
            SECTION_WORLD => world = Some(postcard::from_bytes::<World>(&decompress(id, data)?)?),
            SECTION_INPUTS => inputs = Some(postcard::from_bytes(&decompress(id, data)?)?),
            SECTION_CHECKPOINTS => checks = Some(postcard::from_bytes(&decompress(id, data)?)?),
            _ => {}
        }
    }
    let names = names.ok_or(FormatError::MissingSection(SECTION_NAMES))?;
    let meta = meta.ok_or(FormatError::MissingSection(SECTION_META))?;
    let initial = world.ok_or(FormatError::MissingSection(SECTION_WORLD))?;
    let inputs: Vec<TickInput> = inputs.ok_or(FormatError::MissingSection(SECTION_INPUTS))?;
    let checkpoints: Vec<Checkpoint> =
        checks.ok_or(FormatError::MissingSection(SECTION_CHECKPOINTS))?;
    if meta.sim_version != SIM_VERSION {
        return Err(FormatError::SimVersionMismatch {
            file: meta.sim_version,
            current: SIM_VERSION.to_owned(),
        });
    }
    if names != reg.name_table() {
        return Err(FormatError::ContentMismatch);
    }
    initial.validate(reg).map_err(FormatError::InvalidWorld)?;
    if initial.tick_count() != meta.start_tick || meta.end_tick < meta.start_tick {
        return Err(FormatError::InvalidWorld(
            "replay tick range inconsistent".into(),
        ));
    }
    if inputs.windows(2).any(|w| w[0].tick >= w[1].tick)
        || inputs
            .iter()
            .any(|i| i.tick < meta.start_tick || i.tick >= meta.end_tick)
    {
        return Err(FormatError::InvalidWorld(
            "replay inputs out of order".into(),
        ));
    }
    Ok(Replay {
        meta,
        initial,
        inputs,
        checkpoints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrustry_sim::scenario::RandomBuilder;

    /// Записывает партию; возвращает также тики, на которых команда была принята.
    fn record_with_accepted(seed: u64, ticks: u64) -> (Replay, ContentRegistry, Vec<u64>) {
        let reg = ContentRegistry::base().unwrap();
        let (mut w, mut b) = RandomBuilder::new(&reg, 24, seed).unwrap();
        let mut rec = Recorder::new(&w, 100);
        let mut accepted = Vec::new();
        for _ in 0..ticks {
            let t = w.tick_count();
            let cmds = b.commands(t);
            if rec.tick(&mut w, &reg, &cmds).iter().any(Result::is_ok) {
                accepted.push(t);
            }
        }
        (rec.finish(&w), reg, accepted)
    }

    fn record(seed: u64, ticks: u64) -> (Replay, ContentRegistry) {
        let (r, reg, _) = record_with_accepted(seed, ticks);
        (r, reg)
    }

    #[test]
    fn roundtrip_and_verify() {
        let (r, reg) = record(5, 1234);
        let bytes = write_replay(&r, &reg).unwrap();
        let loaded = read_replay(&bytes, &reg).unwrap();
        assert_eq!(loaded, r);
        let (outcome, _) = play(&loaded, &reg);
        assert!(
            matches!(
                outcome,
                PlayOutcome::Verified {
                    checkpoints: 13,
                    ..
                }
            ),
            "{outcome:?}"
        );
    }

    #[test]
    fn tampered_command_is_located() {
        let (mut r, reg, accepted) = record_with_accepted(6, 1000);
        // убрать принятую (изменившую мир) команду на тике ≥450 → рассинхрон на первой
        // контрольной точке после неё (отклонённая команда состояние не меняет)
        let at = *accepted.iter().find(|&&t| t >= 450).unwrap();
        let idx = r.inputs.iter().position(|i| i.tick == at).unwrap();
        let removed_at = r.inputs[idx].tick;
        r.inputs.remove(idx);
        let (outcome, _) = play(&r, &reg);
        match outcome {
            PlayOutcome::Diverged {
                tick,
                last_good_tick,
                ..
            } => {
                assert!(
                    tick > removed_at && tick <= removed_at + 100,
                    "diverged at {tick}"
                );
                assert!(last_good_tick.unwrap() <= removed_at);
            }
            other => panic!("expected divergence, got {other:?}"),
        }
    }

    #[test]
    fn version_and_kind_are_checked() {
        let (mut r, reg) = record(7, 50);
        r.meta.sim_version = "0.0.0-other".into();
        let bytes = write_replay(&r, &reg).unwrap();
        assert!(matches!(
            read_replay(&bytes, &reg),
            Err(FormatError::SimVersionMismatch { .. })
        ));
        let save = crate::write_save(r.initial(), &reg).unwrap();
        assert!(matches!(
            read_replay(&save, &reg),
            Err(FormatError::WrongKind { .. })
        ));
    }
}
