//! Сервер партии: принимает игроков и команды, считает мир, рассылает кадры и контрольные суммы.
//!
//! Вход и восстановление (S6-03): все, кто ждёт снимок, собираются в одну партию; мир копируется
//! один раз, а сериализация и сжатие идут в отдельном потоке — тик не ждёт. Пока снимок
//! готовится, кадры для этих игроков копятся в очереди и уходят следом за снимком.

use std::sync::mpsc::{Receiver, TryRecvError, channel};

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
    /// Писать снимки в отдельном потоке (иначе — сразу, в `poll`; для тестов).
    pub background_snapshots: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            checksum_every: 5 * u64::from(TICKS_PER_SECOND),
            max_players: 8,
            background_snapshots: true,
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

/// Зачем игроку снимок.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    /// Вход (`Welcome`).
    Join,
    /// Восстановление после расхождения (`Snapshot`).
    Resync,
}

/// Синхронизация игрока с миром.
enum Sync {
    /// Ждёт следующей партии снимка; кадры ему не нужны.
    Waiting(Want),
    /// В партии снимка: кадры с тика снимка копятся до его отправки.
    Batched(Want, Vec<Vec<u8>>),
    /// Получил снимок; кадры идут сразу.
    Ready,
}

struct Peer<L> {
    link: L,
    player: Option<PlayerId>,
    alive: bool,
    sync: Sync,
}

/// Сервер.
pub struct Server<L: Link> {
    world: World,
    peers: Vec<Peer<L>>,
    pending: Vec<(PlayerId, Command)>,
    config: ServerConfig,
    next_player: PlayerId,
    rolling: Rolling,
    /// Снимок, который готовится в фоне.
    snapshot: Option<Receiver<Result<Vec<u8>, String>>>,
    /// Снимков сделано (для журнала и тестов).
    snapshots: u64,
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
            snapshot: None,
            snapshots: 0,
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
            sync: Sync::Ready,
        });
    }

    /// Сколько снимков сделано (одна партия — один снимок на всех входящих).
    pub fn snapshots(&self) -> u64 {
        self.snapshots
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
                        let player = self.next_player;
                        self.next_player = self.next_player.wrapping_add(1).max(1);
                        self.peers[i].player = Some(player);
                        self.peers[i].sync = Sync::Waiting(Want::Join);
                        events.push(ServerEvent::Joined { player, name });
                    }
                    Ok(ClientMsg::Resync) if self.peers[i].player.is_some() => {
                        // повторный запрос, пока снимок уже в пути, — лишний
                        if matches!(self.peers[i].sync, Sync::Ready) {
                            self.peers[i].sync = Sync::Waiting(Want::Resync);
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
        self.service_snapshots(reg, &mut events);
        self.peers.retain(|p| p.alive);
        events
    }

    /// Отдать готовый снимок его партии и начать новую партию, если есть ждущие.
    fn service_snapshots(&mut self, reg: &ContentRegistry, events: &mut Vec<ServerEvent>) {
        if let Some(rx) = &self.snapshot {
            let res = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("snapshot thread failed".into())),
            };
            if let Some(res) = res {
                self.snapshot = None;
                self.deliver(res, events);
            }
        }
        if self.snapshot.is_some()
            || !self
                .peers
                .iter()
                .any(|p| matches!(p.sync, Sync::Waiting(_)))
        {
            return;
        }
        // новая партия: снимок мира на текущем тике для всех ждущих
        for p in &mut self.peers {
            if let Sync::Waiting(w) = p.sync {
                p.sync = Sync::Batched(w, Vec::new());
            }
        }
        self.snapshots += 1;
        if self.config.background_snapshots {
            // копия — миллисекунды в этом потоке; сериализация и сжатие — в фоне
            let (world, reg) = (self.world.clone(), reg.clone());
            let (tx, rx) = channel();
            let spawned = std::thread::Builder::new()
                .name("ferrustry-snapshot".into())
                .spawn(move || {
                    let _ = tx.send(
                        ferrustry_format::write_save(&world, &reg).map_err(|e| e.to_string()),
                    );
                });
            match spawned {
                Ok(_) => self.snapshot = Some(rx),
                Err(e) => self.deliver(Err(e.to_string()), events),
            }
        } else {
            let res = ferrustry_format::write_save(&self.world, reg).map_err(|e| e.to_string());
            self.deliver(res, events);
        }
    }

    /// Разослать снимок партии и следом — накопленные кадры.
    fn deliver(&mut self, res: Result<Vec<u8>, String>, events: &mut Vec<ServerEvent>) {
        let every = self.config.checksum_every.max(1);
        for p in &mut self.peers {
            // только участники этой партии; ждущие следующую остаются ждать
            if !matches!(p.sync, Sync::Batched(..)) {
                continue;
            }
            let Sync::Batched(want, queue) = std::mem::replace(&mut p.sync, Sync::Ready) else {
                continue;
            };
            let (Ok(snapshot), Some(player)) = (&res, p.player) else {
                p.alive = false;
                if let Err(e) = &res {
                    events.push(ServerEvent::Rejected(format!("snapshot failed: {e}")));
                }
                continue;
            };
            let first = match want {
                Want::Join => ServerMsg::Welcome {
                    player,
                    checksum_every: every,
                    snapshot: snapshot.clone(),
                },
                Want::Resync => {
                    events.push(ServerEvent::Resynced(player));
                    ServerMsg::Snapshot(snapshot.clone())
                }
            };
            let mut ok = p.link.send(encode(&first)).is_ok();
            for m in queue {
                ok &= p.link.send(m).is_ok();
            }
            if !ok {
                p.alive = false;
                events.push(ServerEvent::Left(player));
            }
        }
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
            match &mut p.sync {
                Sync::Waiting(_) => {}
                // снимок в пути: кадры — следом за ним, по порядку
                Sync::Batched(_, queue) => {
                    queue.push(frame.clone());
                    if let Some(c) = &check {
                        queue.push(c.clone());
                    }
                }
                Sync::Ready => {
                    let mut ok = p.link.send(frame.clone()).is_ok();
                    if let Some(c) = &check {
                        ok &= p.link.send(c.clone()).is_ok();
                    }
                    p.alive = ok;
                }
            }
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
