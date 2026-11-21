//! Клиент партии: снимок от сервера, расчёт тиков по кадрам, сверка скользящей контрольной
//! суммы и восстановление при расхождении (D-20).
//!
//! Мир клиент не хранит: его хранит игра (рисует, сохраняет), а клиент получает его в
//! [`Client::advance`]. Снимки приходят событиями [`ClientEvent::Joined`] и
//! [`ClientEvent::Resynced`].

use std::collections::VecDeque;

use ferrustry_content::ContentRegistry;
use ferrustry_sim::{Command, CommandError, SIM_VERSION, World};

use crate::{
    ClientMsg, Link, NetError, PROTOCOL_VERSION, PlayerId, Rolling, ServerMsg, decode, encode,
};

/// Что произошло у клиента.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// Принят в партию: номер игрока и мир со снимка (дальше его считает [`Client::advance`]).
    Joined(PlayerId, Box<World>),
    /// Сервер отказал.
    Rejected(String),
    /// Состояние разошлось с сервером в периоде, закончившемся на `tick` (D-20). Клиент сам
    /// запрашивает свежий снимок и до его прихода не считает тики.
    Desync {
        /// Тик сверки.
        tick: u64,
        /// Сумма сервера.
        server: u64,
        /// Сумма клиента.
        local: u64,
    },
    /// Свежий снимок сервера после расхождения: заменить им свой мир.
    Resynced(Box<World>),
    /// Соединение потеряно или сервер нарушил протокол.
    Disconnected(String),
    /// Результаты **своих** команд, исполненных в рассчитанных тиках (по порядку отправки):
    /// чтобы показать игроку, почему постройка не встала. Симуляция у всех одна, поэтому
    /// результат у клиента тот же, что и на сервере.
    Results(Vec<Result<(), CommandError>>),
}

/// Клиент.
pub struct Client<L: Link> {
    link: L,
    /// Тик мира после последнего снимка/кадра (`None` — ещё не вошли).
    at: Option<u64>,
    player: Option<PlayerId>,
    frames: VecDeque<(u64, Vec<(PlayerId, Command)>)>,
    server_checks: VecDeque<(u64, u64)>,
    local_checks: VecDeque<(u64, u64)>,
    rolling: Rolling,
    resyncing: bool,
    resyncs: u32,
    verified: u32,
    closed: bool,
}

impl<L: Link> Client<L> {
    /// Подключиться: отправляет `Hello`.
    pub fn connect(mut link: L, reg: &ContentRegistry, name: &str) -> Result<Self, NetError> {
        link.send(encode(&ClientMsg::Hello {
            protocol: PROTOCOL_VERSION,
            sim_version: SIM_VERSION.to_owned(),
            content: reg.fingerprint(),
            name: name.to_owned(),
        }))?;
        Ok(Self {
            link,
            at: None,
            player: None,
            frames: VecDeque::new(),
            server_checks: VecDeque::new(),
            local_checks: VecDeque::new(),
            rolling: Rolling::new(1),
            resyncing: false,
            resyncs: 0,
            verified: 0,
            closed: false,
        })
    }

    /// Вошёл ли клиент в партию.
    pub fn joined(&self) -> bool {
        self.at.is_some()
    }

    /// Транспорт (для обслуживания: `update`/`flush` у сетевого канала).
    pub fn link_mut(&mut self) -> &mut L {
        &mut self.link
    }

    /// Транспорт (например, для показа задержки).
    pub fn link(&self) -> &L {
        &self.link
    }

    /// Номер игрока.
    pub fn player(&self) -> Option<PlayerId> {
        self.player
    }

    /// Кадров получено, но не рассчитано (по нему игра выбирает темп, D-19).
    pub fn buffered(&self) -> usize {
        self.frames.len()
    }

    /// Ждёт ли клиент свежий снимок после расхождения.
    pub fn resyncing(&self) -> bool {
        self.resyncing
    }

    /// Сколько раз клиент восстанавливался по снимку.
    pub fn resyncs(&self) -> u32 {
        self.resyncs
    }

    /// Сколько периодов сверено с сервером и совпало.
    pub fn verified(&self) -> u32 {
        self.verified
    }

    /// Отправить команду серверу (исполнится в тике, который назначит сервер).
    pub fn send(&mut self, cmd: Command) -> Result<(), NetError> {
        self.link.send(encode(&ClientMsg::Command(cmd)))
    }

    /// Принять входящие сообщения.
    pub fn poll(&mut self, reg: &ContentRegistry) -> Vec<ClientEvent> {
        let mut events = Vec::new();
        while !self.closed {
            let msg = match self.link.recv() {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(e) => {
                    self.close(e.to_string(), &mut events);
                    break;
                }
            };
            match decode::<ServerMsg>(&msg) {
                Ok(ServerMsg::Welcome {
                    player,
                    checksum_every,
                    snapshot,
                }) if self.at.is_none() => match ferrustry_format::read_save(&snapshot, reg) {
                    Ok(w) => {
                        self.at = Some(w.tick_count());
                        self.player = Some(player);
                        self.rolling = Rolling::new(checksum_every);
                        events.push(ClientEvent::Joined(player, Box::new(w)));
                    }
                    Err(e) => self.close(format!("bad snapshot: {e}"), &mut events),
                },
                Ok(ServerMsg::Snapshot(snapshot)) if self.at.is_some() => {
                    match ferrustry_format::read_save(&snapshot, reg) {
                        Ok(w) => {
                            let t = w.tick_count();
                            // кадры и суммы до снимка относятся к старому миру
                            self.frames.retain(|&(tick, _)| tick >= t);
                            self.server_checks.retain(|&(tick, _)| tick > t);
                            self.local_checks.clear();
                            self.rolling.reset();
                            self.at = Some(t);
                            self.resyncing = false;
                            self.resyncs += 1;
                            events.push(ClientEvent::Resynced(Box::new(w)));
                        }
                        Err(e) => self.close(format!("bad snapshot: {e}"), &mut events),
                    }
                }
                Ok(ServerMsg::Reject(why)) => {
                    events.push(ClientEvent::Rejected(why));
                    self.closed = true;
                }
                Ok(ServerMsg::Frame { tick, commands }) if self.at.is_some() => {
                    self.frames.push_back((tick, commands));
                }
                Ok(ServerMsg::Checksum { tick, hash }) if self.at.is_some() => {
                    self.server_checks.push_back((tick, hash));
                    self.reconcile(&mut events);
                }
                Ok(other) => self.close(format!("unexpected {other:?}"), &mut events),
                Err(e) => self.close(e.to_string(), &mut events),
            }
        }
        events
    }

    fn close(&mut self, why: String, events: &mut Vec<ClientEvent>) {
        self.closed = true;
        events.push(ClientEvent::Disconnected(why));
    }

    /// Сравнить суммы сервера и свои за одинаковые периоды.
    fn reconcile(&mut self, events: &mut Vec<ClientEvent>) {
        while let (Some(&(st, sh)), Some(&(lt, lh))) =
            (self.server_checks.front(), self.local_checks.front())
        {
            if st < lt {
                self.server_checks.pop_front(); // период до нашего входа
                continue;
            }
            if lt < st {
                self.local_checks.pop_front();
                continue;
            }
            self.server_checks.pop_front();
            self.local_checks.pop_front();
            if sh == lh {
                self.verified += 1;
            } else if !self.resyncing {
                events.push(ClientEvent::Desync {
                    tick: st,
                    server: sh,
                    local: lh,
                });
                self.resyncing = true;
                if let Err(e) = self.link.send(encode(&ClientMsg::Resync)) {
                    self.close(e.to_string(), events);
                }
            }
        }
    }

    /// Рассчитать до `max` тиков мира `world` (того, что пришёл в `Joined`/`Resynced`) по
    /// полученным кадрам. Возвращает число тиков и события. Пока клиент ждёт снимок после
    /// расхождения, тики не считаются (кадры копятся).
    pub fn advance(
        &mut self,
        reg: &ContentRegistry,
        world: &mut World,
        max: usize,
    ) -> (usize, Vec<ClientEvent>) {
        let mut events = Vec::new();
        let mut own = Vec::new();
        let mut done = 0;
        while done < max && !self.resyncing && !self.closed && self.at.is_some() {
            let Some((tick, commands)) = self.frames.pop_front() else {
                break;
            };
            if tick != world.tick_count() {
                let why = format!("frame for tick {tick}, world is at {}", world.tick_count());
                self.close(why, &mut events);
                break;
            }
            let (players, cmds): (Vec<PlayerId>, Vec<Command>) = commands.into_iter().unzip();
            let results = world.tick(reg, &cmds);
            if let Some(me) = self.player {
                own.extend(
                    players
                        .iter()
                        .zip(results)
                        .filter(|&(&p, _)| p == me)
                        .map(|(_, r)| r),
                );
            }
            self.at = Some(world.tick_count());
            done += 1;
            if let Some(check) = self.rolling.after_tick(world) {
                self.local_checks.push_back(check);
                self.reconcile(&mut events);
            }
        }
        if !own.is_empty() {
            events.push(ClientEvent::Results(own));
        }
        (done, events)
    }
}
