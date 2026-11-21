//! Сервер партии: принимает игроков и команды, считает мир, рассылает кадры и контрольные суммы.

use ferrustry_content::ContentRegistry;
use ferrustry_sim::{Command, CommandError, SIM_VERSION, TICKS_PER_SECOND, World};

use crate::{ClientMsg, Link, PROTOCOL_VERSION, PlayerId, Rolling, ServerMsg, decode, encode};

/// Настройки сервера.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Как часто рассылать контрольную сумму, в тиках (D-20: раз в 5 с).
    pub checksum_every: u64,
    /// Наибольшее число игроков (D-22: 8).
    pub max_players: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            checksum_every: 5 * u64::from(TICKS_PER_SECOND),
            max_players: 8,
        }
    }
}

/// Что произошло на сервере (для журнала и интерфейса).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    /// Игрок вошёл.
    Joined {
        /// Номер.
        player: PlayerId,
        /// Имя.
        name: String,
    },
    /// Подключение отклонено.
    Rejected(String),
    /// Игрок отключился.
    Left(PlayerId),
    /// Клиент прислал непонятное или недопустимое сообщение (отключён).
    Misbehaved(String),
    /// Клиент разошёлся с сервером и получил свежий снимок.
    Resynced(PlayerId),
}

struct Peer<L> {
    link: L,
    player: Option<PlayerId>,
    alive: bool,
}

/// Сервер.
pub struct Server<L: Link> {
    world: World,
    peers: Vec<Peer<L>>,
    pending: Vec<(PlayerId, Command)>,
    config: ServerConfig,
    next_player: PlayerId,
    rolling: Rolling,
}

impl<L: Link> Server<L> {
    /// Сервер для мира `world`.
    pub fn new(world: World, config: ServerConfig) -> Self {
        Self {
            world,
            peers: Vec::new(),
            pending: Vec::new(),
            rolling: Rolling::new(config.checksum_every),
            config,
            next_player: 1,
        }
    }

    /// Мир сервера (единственный источник истины).
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Новое подключение (ждёт `Hello`).
    pub fn accept(&mut self, link: L) {
        self.peers.push(Peer {
            link,
            player: None,
            alive: true,
        });
    }

    /// Игроков в партии.
    pub fn players(&self) -> usize {
        self.peers
            .iter()
            .filter(|p| p.alive && p.player.is_some())
            .count()
    }

    /// Принять входящие сообщения: подключения и команды. Команды попадут в следующий кадр.
    pub fn poll(&mut self, reg: &ContentRegistry) -> Vec<ServerEvent> {
        let mut events = Vec::new();
        for i in 0..self.peers.len() {
            while self.peers[i].alive {
                let msg = match self.peers[i].link.recv() {
                    Ok(Some(m)) => m,
                    Ok(None) => break,
                    Err(_) => {
                        self.drop_peer(i, &mut events);
                        break;
                    }
                };
                match decode::<ClientMsg>(&msg) {
                    Ok(ClientMsg::Hello {
                        protocol,
                        sim_version,
                        content,
                        name,
                    }) if self.peers[i].player.is_none() => {
                        let why = if protocol != PROTOCOL_VERSION {
                            Some(format!(
                                "protocol {protocol}, server speaks {PROTOCOL_VERSION}"
                            ))
                        } else if sim_version != SIM_VERSION {
                            Some(format!(
                                "game version {sim_version}, server runs {SIM_VERSION}"
                            ))
                        } else if content != reg.fingerprint() {
                            Some("game content differs from the server".to_owned())
                        } else if self.players() >= self.config.max_players {
                            Some("the server is full".to_owned())
                        } else {
                            None
                        };
                        if let Some(why) = why {
                            let _ = self.peers[i]
                                .link
                                .send(encode(&ServerMsg::Reject(why.clone())));
                            self.peers[i].alive = false;
                            events.push(ServerEvent::Rejected(why));
                            continue;
                        }
                        let snapshot = match ferrustry_format::write_save(&self.world, reg) {
                            Ok(s) => s,
                            Err(e) => {
                                events.push(ServerEvent::Rejected(format!("snapshot failed: {e}")));
                                self.peers[i].alive = false;
                                continue;
                            }
                        };
                        let player = self.next_player;
                        self.next_player = self.next_player.wrapping_add(1).max(1);
                        let welcome = ServerMsg::Welcome {
                            player,
                            checksum_every: self.config.checksum_every.max(1),
                            snapshot,
                        };
                        if self.peers[i].link.send(encode(&welcome)).is_err() {
                            self.peers[i].alive = false;
                            continue;
                        }
                        self.peers[i].player = Some(player);
                        events.push(ServerEvent::Joined { player, name });
                    }
                    Ok(ClientMsg::Resync) if self.peers[i].player.is_some() => {
                        let sent = ferrustry_format::write_save(&self.world, reg)
                            .map(|s| self.peers[i].link.send(encode(&ServerMsg::Snapshot(s))));
                        match (sent, self.peers[i].player) {
                            (Ok(Ok(())), Some(p)) => events.push(ServerEvent::Resynced(p)),
                            _ => self.drop_peer(i, &mut events),
                        }
                    }
                    Ok(ClientMsg::Command(cmd)) if self.peers[i].player.is_some() => {
                        match valid(reg, &cmd) {
                            Ok(()) => {
                                if let Some(p) = self.peers[i].player {
                                    self.pending.push((p, cmd));
                                }
                            }
                            Err(why) => {
                                events.push(ServerEvent::Misbehaved(why));
                                self.drop_peer(i, &mut events);
                            }
                        }
                    }
                    Ok(other) => {
                        events.push(ServerEvent::Misbehaved(format!("unexpected {other:?}")));
                        self.drop_peer(i, &mut events);
                    }
                    Err(e) => {
                        events.push(ServerEvent::Misbehaved(e.to_string()));
                        self.drop_peer(i, &mut events);
                    }
                }
            }
        }
        self.peers.retain(|p| p.alive);
        events
    }

    fn drop_peer(&mut self, i: usize, events: &mut Vec<ServerEvent>) {
        if self.peers[i].alive {
            self.peers[i].alive = false;
            if let Some(p) = self.peers[i].player {
                events.push(ServerEvent::Left(p));
            }
        }
    }

    /// Один тик: накопленные команды → кадр, расчёт, рассылка кадра (и контрольной суммы).
    pub fn tick(&mut self, reg: &ContentRegistry) -> Vec<Result<(), CommandError>> {
        let commands = std::mem::take(&mut self.pending);
        let tick = self.world.tick_count();
        let cmds: Vec<Command> = commands.iter().map(|(_, c)| c.clone()).collect();
        let results = self.world.tick(reg, &cmds);
        let frame = encode(&ServerMsg::Frame { tick, commands });
        let check = self
            .rolling
            .after_tick(&self.world)
            .map(|(tick, hash)| encode(&ServerMsg::Checksum { tick, hash }));
        for p in &mut self.peers {
            if !p.alive || p.player.is_none() {
                continue;
            }
            let mut ok = p.link.send(frame.clone()).is_ok();
            if let Some(c) = &check {
                ok &= p.link.send(c.clone()).is_ok();
            }
            p.alive = ok;
        }
        results
    }
}

/// Проверка команды от недоверенного клиента (ID контента в пределах реестра).
fn valid(reg: &ContentRegistry, cmd: &Command) -> Result<(), String> {
    match cmd {
        Command::Place { block, .. } if usize::from(block.0) >= reg.blocks().len() => {
            Err(format!("unknown block id {}", block.0))
        }
        Command::Configure { item: Some(i), .. } if usize::from(i.0) >= reg.items().len() => {
            Err(format!("unknown item id {}", i.0))
        }
        _ => Ok(()),
    }
}
