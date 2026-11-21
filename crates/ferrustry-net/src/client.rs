//! Клиент партии: снимок от сервера, расчёт тиков по кадрам, сверка скользящей контрольной
//! суммы и восстановление при расхождении (D-20).

use std::collections::VecDeque;

use ferrustry_content::ContentRegistry;
use ferrustry_sim::{Command, SIM_VERSION, World};

use crate::{
    ClientMsg, Link, NetError, PROTOCOL_VERSION, PlayerId, Rolling, ServerMsg, decode, encode,
};

/// Что произошло у клиента.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// Принят в партию; мир загружен со снимка.
    Joined(PlayerId),
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
    /// Мир заменён свежим снимком сервера на тике `0`.
    Resynced(u64),
    /// Соединение потеряно или сервер нарушил протокол.
    Disconnected(String),
}

/// Клиент.
pub struct Client<L: Link> {
    link: L,
    world: Option<World>,
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
            world: None,
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

    /// Мир клиента (после входа).
    pub fn world(&self) -> Option<&World> {
        self.world.as_ref()
    }

    /// Мир клиента для изменения (только для тестов и отладки: правка мира ломает синхронность).
    #[doc(hidden)]
    pub fn world_mut(&mut self) -> Option<&mut World> {
        self.world.as_mut()
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
                }) if self.world.is_none() => match ferrustry_format::read_save(&snapshot, reg) {
                    Ok(w) => {
                        self.world = Some(w);
                        self.player = Some(player);
                        self.rolling = Rolling::new(checksum_every);
                        events.push(ClientEvent::Joined(player));
                    }
                    Err(e) => self.close(format!("bad snapshot: {e}"), &mut events),
                },
                Ok(ServerMsg::Snapshot(snapshot)) if self.world.is_some() => {
                    match ferrustry_format::read_save(&snapshot, reg) {
                        Ok(w) => {
                            let t = w.tick_count();
                            // кадры и суммы до снимка относятся к старому миру
                            self.frames.retain(|&(tick, _)| tick >= t);
                            self.server_checks.retain(|&(tick, _)| tick > t);
                            self.local_checks.clear();
                            self.rolling.reset();
                            self.world = Some(w);
                            self.resyncing = false;
                            self.resyncs += 1;
                            events.push(ClientEvent::Resynced(t));
                        }
                        Err(e) => self.close(format!("bad snapshot: {e}"), &mut events),
                    }
                }
                Ok(ServerMsg::Reject(why)) => {
                    events.push(ClientEvent::Rejected(why));
                    self.closed = true;
                }
                Ok(ServerMsg::Frame { tick, commands }) if self.world.is_some() => {
                    self.frames.push_back((tick, commands));
                }
                Ok(ServerMsg::Checksum { tick, hash }) if self.world.is_some() => {
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

    /// Рассчитать до `max` тиков по полученным кадрам. Возвращает число тиков и события.
    /// Пока клиент ждёт снимок после расхождения, тики не считаются (кадры копятся).
    pub fn advance(&mut self, reg: &ContentRegistry, max: usize) -> (usize, Vec<ClientEvent>) {
        let mut events = Vec::new();
        let mut done = 0;
        while done < max && !self.resyncing && !self.closed {
            let Some(world) = self.world.as_mut() else {
                break;
            };
            let Some((tick, commands)) = self.frames.pop_front() else {
                break;
            };
            if tick != world.tick_count() {
                let why = format!("frame for tick {tick}, world is at {}", world.tick_count());
                self.close(why, &mut events);
                break;
            }
            let cmds: Vec<Command> = commands.into_iter().map(|(_, c)| c).collect();
            world.tick(reg, &cmds);
            done += 1;
            if let Some(check) = self.rolling.after_tick(world) {
                self.local_checks.push_back(check);
                self.reconcile(&mut events);
            }
        }
        (done, events)
    }
}
