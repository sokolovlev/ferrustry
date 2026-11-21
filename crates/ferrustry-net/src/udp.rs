//! Сетевой транспорт поверх UDP: renet (D-08) — надёжный упорядоченный канал сообщений с
//! нарезкой больших сообщений (снимок мира), `renet_netcode` — соединения и рукопожатие.
//!
//! Режим без шифрования и токенов (`Unsecure`): игра по прямому адресу в локальной сети или
//! через интернет без сервера авторизации. Совместимость версий проверяют и netcode (номер
//! протокола), и наш `Hello` (версия игры и отпечаток контента).

use std::cell::RefCell;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use renet::{
    ChannelConfig, ClientId, ConnectionConfig, RenetClient, RenetServer, SendType,
    ServerEvent as RenetEvent,
};
use renet_netcode::{
    ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication,
    ServerConfig as NetcodeConfig,
};

use crate::{Link, MAX_MESSAGE, NetError, PROTOCOL_VERSION};

/// Порт по умолчанию.
pub const DEFAULT_PORT: u16 = 27_960;

/// Номер протокола для netcode: другая версия протокола не соединится вовсе.
const NETCODE_PROTOCOL: u64 = 0x4645_5252_5553_0000 | PROTOCOL_VERSION as u64;
const CHANNEL: u8 = 0;

fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn connection_config() -> ConnectionConfig {
    let channel = |max: usize| {
        vec![ChannelConfig {
            channel_id: CHANNEL,
            max_memory_usage_bytes: max,
            send_type: SendType::ReliableOrdered {
                resend_time: Duration::from_millis(150),
            },
        }]
    };
    ConnectionConfig {
        // ~7,5 МБ/с при 30 обновлениях в секунду: снимок мегабазы (1,6 МБ) — за ~0,2 с
        available_bytes_per_tick: 250_000,
        server_channels_config: channel(MAX_MESSAGE * 2),
        client_channels_config: channel(4 * 1024 * 1024),
    }
}

struct HubInner {
    server: RenetServer,
    transport: NetcodeServerTransport,
    fresh: Vec<ClientId>,
}

/// Серверная сторона: UDP-сокет и все соединения. Новые соединения отдаются как [`UdpPeer`].
#[derive(Clone)]
pub struct UdpHub(Rc<RefCell<HubInner>>);

impl UdpHub {
    /// Слушать адрес (порт 0 — любой свободный).
    pub fn bind(addr: SocketAddr, max_clients: usize) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        let local = socket.local_addr()?;
        let config = NetcodeConfig {
            current_time: now(),
            max_clients,
            protocol_id: NETCODE_PROTOCOL,
            public_addresses: vec![local],
            authentication: ServerAuthentication::Unsecure,
        };
        let transport = NetcodeServerTransport::new(config, socket)?;
        Ok(Self(Rc::new(RefCell::new(HubInner {
            server: RenetServer::new(connection_config()),
            transport,
            fresh: Vec::new(),
        }))))
    }

    /// Адрес, который слушает сервер.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.0.borrow().transport.addresses().first().copied()
    }

    /// Принять пакеты из сети (раз в кадр сервера).
    pub fn update(&self, dt: Duration) -> Result<(), NetError> {
        let mut h = self.0.borrow_mut();
        let HubInner {
            server,
            transport,
            fresh,
        } = &mut *h;
        server.update(dt);
        transport
            .update(dt, server)
            .map_err(|e| NetError::Protocol(e.to_string()))?;
        while let Some(e) = server.get_event() {
            if let RenetEvent::ClientConnected { client_id } = e {
                fresh.push(client_id);
            }
        }
        Ok(())
    }

    /// Новые соединения с прошлого вызова.
    pub fn accept(&self) -> Vec<UdpPeer> {
        let fresh = std::mem::take(&mut self.0.borrow_mut().fresh);
        fresh
            .into_iter()
            .map(|id| UdpPeer {
                hub: self.0.clone(),
                id,
            })
            .collect()
    }

    /// Отправить накопленное в сеть (раз в кадр сервера, после рассылки).
    pub fn flush(&self) {
        let mut h = self.0.borrow_mut();
        let HubInner {
            server, transport, ..
        } = &mut *h;
        transport.send_packets(server);
    }

    /// Подключено клиентов (на уровне транспорта).
    pub fn connected(&self) -> usize {
        self.0.borrow().server.connected_clients()
    }
}

/// Соединение с одним клиентом на сервере.
pub struct UdpPeer {
    hub: Rc<RefCell<HubInner>>,
    id: ClientId,
}

impl Link for UdpPeer {
    fn send(&mut self, msg: Vec<u8>) -> Result<(), NetError> {
        let mut h = self.hub.borrow_mut();
        if !h.server.is_connected(self.id) {
            return Err(NetError::Closed);
        }
        h.server.send_message(self.id, CHANNEL, msg);
        Ok(())
    }

    fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        let mut h = self.hub.borrow_mut();
        if !h.server.is_connected(self.id) {
            return Err(NetError::Closed);
        }
        Ok(h.server
            .receive_message(self.id, CHANNEL)
            .map(|b| b.to_vec()))
    }
}

impl Drop for UdpPeer {
    fn drop(&mut self) {
        // сервер отключил игрока — закрыть соединение и на уровне транспорта
        if let Ok(mut h) = self.hub.try_borrow_mut() {
            h.server.disconnect(self.id);
        }
    }
}

/// Клиентская сторона: соединение с сервером.
pub struct UdpLink {
    client: RenetClient,
    transport: NetcodeClientTransport,
}

impl UdpLink {
    /// Начать соединение с сервером (рукопожатие идёт в [`update`](Self::update)).
    pub fn connect(server: SocketAddr) -> Result<Self, NetError> {
        let bind: SocketAddr = if server.is_ipv6() {
            "[::]:0".parse().map_err(|_| NetError::Closed)?
        } else {
            "0.0.0.0:0".parse().map_err(|_| NetError::Closed)?
        };
        let socket = UdpSocket::bind(bind).map_err(|e| NetError::Protocol(e.to_string()))?;
        // без сервера авторизации номер клиента выбирает сам клиент: время + адрес сокета
        let t = now();
        let port = socket.local_addr().map_or(0, |a| a.port());
        let client_id = (t.as_nanos() as u64).rotate_left(17) ^ u64::from(port);
        let auth = ClientAuthentication::Unsecure {
            protocol_id: NETCODE_PROTOCOL,
            client_id,
            server_addr: server,
            user_data: None,
        };
        let transport = NetcodeClientTransport::new(t, auth, socket)
            .map_err(|e| NetError::Protocol(e.to_string()))?;
        Ok(Self {
            client: RenetClient::new(connection_config()),
            transport,
        })
    }

    /// Принять пакеты из сети (раз в кадр).
    pub fn update(&mut self, dt: Duration) -> Result<(), NetError> {
        self.client.update(dt);
        self.transport
            .update(dt, &mut self.client)
            .map_err(|e| NetError::Protocol(e.to_string()))
    }

    /// Отправить накопленное в сеть (раз в кадр, после отправки команд).
    pub fn flush(&mut self) -> Result<(), NetError> {
        if self.client.is_disconnected() {
            return Err(NetError::Closed);
        }
        if !self.client.is_connected() {
            return Ok(()); // рукопожатие ещё идёт: сообщения ждут в очереди
        }
        self.transport
            .send_packets(&mut self.client)
            .map_err(|e| NetError::Protocol(e.to_string()))
    }

    /// Соединение установлено.
    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    /// Время «туда и обратно», мс.
    pub fn rtt_ms(&self) -> f64 {
        self.client.rtt() * 1000.0
    }
}

impl Link for UdpLink {
    fn send(&mut self, msg: Vec<u8>) -> Result<(), NetError> {
        if self.client.is_disconnected() {
            return Err(NetError::Closed);
        }
        self.client.send_message(CHANNEL, msg);
        Ok(())
    }

    fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        if self.client.is_disconnected() {
            return Err(NetError::Closed);
        }
        Ok(self.client.receive_message(CHANNEL).map(|b| b.to_vec()))
    }
}

impl Drop for UdpLink {
    fn drop(&mut self) {
        // сообщить серверу сразу, а не ждать таймаута соединения
        self.transport.disconnect();
    }
}
