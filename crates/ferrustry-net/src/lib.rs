//! Сетевая игра Ferrustry (этап 6): авторитетный сервер + рассылка команд (D-18).
//!
//! Сервер принимает команды игроков, назначает им тик и каждый тик рассылает **кадр** —
//! номер тика и его команды. Мир считают сервер и все клиенты одной и той же детерминированной
//! симуляцией; раз в несколько секунд сервер рассылает контрольную сумму (D-20). Новый игрок
//! получает снимок мира (формат сохранений) и дальше идёт по кадрам (D-21).
//!
//! Транспорт абстрагирован ([`Link`]): надёжный упорядоченный канал сообщений. Для тестов —
//! канал в памяти ([`mem_pair`]); сетевой транспорт (renet, D-08) — задача S6-05.

mod client;
mod server;

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use ferrustry_sim::Command;
use serde::{Deserialize, Serialize};

pub use client::{Client, ClientEvent};
pub use server::{Server, ServerConfig, ServerEvent};

/// Версия протокола: несовпадение — понятный отказ при подключении.
pub const PROTOCOL_VERSION: u32 = 1;

/// Предел размера одного сообщения (снимок мегабазы — ~1,6 МБ).
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// Номер игрока в партии.
pub type PlayerId = u8;

/// Сообщение клиента серверу.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMsg {
    /// Первое сообщение: кто и с чем подключается.
    Hello {
        /// Версия протокола.
        protocol: u32,
        /// Версия симуляции (`ferrustry_sim::SIM_VERSION`).
        sim_version: String,
        /// Отпечаток контента (`ContentRegistry::fingerprint`).
        content: u64,
        /// Имя игрока.
        name: String,
    },
    /// Команда игрока.
    Command(Command),
}

/// Сообщение сервера клиенту.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    /// Принят: номер игрока и снимок мира (сохранение) на текущем тике сервера.
    Welcome {
        /// Номер игрока.
        player: PlayerId,
        /// Снимок мира.
        snapshot: Vec<u8>,
    },
    /// Отказ (причина — для показа игроку).
    Reject(String),
    /// Кадр: команды тика `tick` (тик ещё не рассчитан; кадр приходит каждый тик, даже пустой).
    Frame {
        /// Номер тика (`World::tick_count` перед расчётом).
        tick: u64,
        /// Команды тика в порядке, назначенном сервером.
        commands: Vec<(PlayerId, Command)>,
    },
    /// Контрольная сумма состояния после `tick` тиков.
    Checksum {
        /// `World::tick_count` после расчёта.
        tick: u64,
        /// Хеш состояния.
        hash: u64,
    },
}

/// Ошибка сети.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// Соединение закрыто.
    #[error("connection closed")]
    Closed,
    /// Сообщение не разобрано или слишком велико.
    #[error("bad message: {0}")]
    BadMessage(String),
    /// Нарушение протокола (например, пропущенный кадр).
    #[error("protocol violation: {0}")]
    Protocol(String),
}

/// Закодировать сообщение.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    // сериализация собственных типов в память не падает
    postcard::to_allocvec(msg).unwrap_or_default()
}

/// Разобрать сообщение (данные недоверенные: размер ограничен, ошибки — не паника).
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, NetError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(NetError::BadMessage(format!("{} bytes", bytes.len())));
    }
    postcard::from_bytes(bytes).map_err(|e| NetError::BadMessage(e.to_string()))
}

/// Надёжный упорядоченный канал сообщений.
pub trait Link {
    /// Отправить сообщение.
    fn send(&mut self, msg: Vec<u8>) -> Result<(), NetError>;
    /// Следующее сообщение, если пришло.
    fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError>;
}

/// Канал в памяти (тесты, игра на одной машине).
#[derive(Debug)]
pub struct MemLink {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
}

/// Пара связанных каналов в памяти: (сторона сервера, сторона клиента).
pub fn mem_pair() -> (MemLink, MemLink) {
    let (a_tx, b_rx) = channel();
    let (b_tx, a_rx) = channel();
    (
        MemLink { tx: a_tx, rx: a_rx },
        MemLink { tx: b_tx, rx: b_rx },
    )
}

impl Link for MemLink {
    fn send(&mut self, msg: Vec<u8>) -> Result<(), NetError> {
        self.tx.send(msg).map_err(|_| NetError::Closed)
    }

    fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        match self.rx.try_recv() {
            Ok(m) => Ok(Some(m)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(NetError::Closed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode::<ClientMsg>(&[0xFF, 0xFF, 0xFF]).is_err());
        assert!(decode::<ServerMsg>(&[]).is_err());
        // огромная заявленная длина без данных
        assert!(decode::<ServerMsg>(&[0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F]).is_err());
    }

    #[test]
    fn mem_link_reports_closed_peer() {
        let (mut a, b) = mem_pair();
        drop(b);
        assert!(matches!(a.send(vec![1]), Err(NetError::Closed)));
        assert!(matches!(a.recv(), Err(NetError::Closed)));
    }
}
