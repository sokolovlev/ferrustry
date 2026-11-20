//! Клиент партии: снимок от сервера, расчёт тиков по кадрам, сверка контрольных сумм.

use std::collections::VecDeque;

use ferrustry_content::ContentRegistry;
use ferrustry_sim::{Command, SIM_VERSION, World, state_hash};

use crate::{ClientMsg, Link, NetError, PROTOCOL_VERSION, PlayerId, ServerMsg, decode, encode};

/// Что произошло у клиента.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// Принят в партию; мир загружен со снимка.
    Joined(PlayerId),
    /// Сервер отказал.
    Rejected(String),
    /// Состояние разошлось с сервером после `tick` тиков (D-20).
    Desync {
        /// Тик сверки.
        tick: u64,
        /// Хеш сервера.
        server: u64,
        /// Хеш клиента.
        local: u64,
    },
    /// Соединение потеряно или сервер нарушил протокол.
    Disconnected(String),
}

/// Клиент.
pub struct Client<L: Link> {
    link: L,
    world: Option<World>,
    player: Option<PlayerId>,
    frames: VecDeque<(u64, Vec<(PlayerId, Command)>)>,
    checks: VecDeque<(u64, u64)>,
    desynced: bool,
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
            checks: VecDeque::new(),
            desynced: false,
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

    /// Было ли расхождение с сервером.
    pub fn desynced(&self) -> bool {
        self.desynced
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
                Ok(ServerMsg::Welcome { player, snapshot }) if self.world.is_none() => {
                    match ferrustry_format::read_save(&snapshot, reg) {
                        Ok(w) => {
                            self.world = Some(w);
                            self.player = Some(player);
                            events.push(ClientEvent::Joined(player));
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
                    self.checks.push_back((tick, hash));
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

    /// Рассчитать до `max` тиков по полученным кадрам. Возвращает число тиков и события
    /// (расхождение контрольной суммы, нарушение порядка кадров).
    pub fn advance(&mut self, reg: &ContentRegistry, max: usize) -> (usize, Vec<ClientEvent>) {
        let mut events = Vec::new();
        let Some(world) = self.world.as_mut() else {
            return (0, events);
        };
        let mut done = 0;
        while done < max {
            let Some((tick, commands)) = self.frames.pop_front() else {
                break;
            };
            if tick != world.tick_count() {
                let why = format!("frame for tick {tick}, world is at {}", world.tick_count());
                self.closed = true;
                events.push(ClientEvent::Disconnected(why));
                break;
            }
            let cmds: Vec<Command> = commands.into_iter().map(|(_, c)| c).collect();
            world.tick(reg, &cmds);
            done += 1;
            let now = world.tick_count();
            while let Some(&(t, hash)) = self.checks.front() {
                if t > now {
                    break;
                }
                self.checks.pop_front();
                if t == now {
                    let local = state_hash(world);
                    if local != hash && !self.desynced {
                        self.desynced = true;
                        events.push(ClientEvent::Desync {
                            tick: t,
                            server: hash,
                            local,
                        });
                    }
                }
            }
        }
        (done, events)
    }
}
