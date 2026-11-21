//! Сетевая партия (этап 6): игра подключается к выделенному серверу и считает мир по кадрам
//! команд (D-18). Пауза, скорость и загрузка сохранений в сетевой партии недоступны —
//! временем управляет сервер.

use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use ferrustry_net::{Client, ClientEvent, DEFAULT_PORT, UdpLink};
use ferrustry_render::atlas::palette;
use ferrustry_render::{Atlas, Hud, SpriteTable, rgba};
use ferrustry_sim::TICKS_PER_SECOND;

use super::{Game, Screen};
use crate::ui::{Rect, Ui, text_width};

/// Запас кадров, сверх которого клиент догоняет (D-19: 2–4 тика).
const FRAME_BUFFER: usize = 4;
/// Наибольшее число тиков за кадр при догоне.
const MAX_CATCH_UP: usize = 60;

/// Подключение к серверу.
pub struct NetPlay {
    client: Client<UdpLink>,
    addr: String,
    player: Option<u8>,
    acc: f64,
}

/// Адрес сервера: `host`, `host:port`, `ip:port`; порт по умолчанию — [`DEFAULT_PORT`].
pub fn resolve(addr: &str) -> Result<SocketAddr, String> {
    if let Ok(a) = addr.parse::<SocketAddr>() {
        return Ok(a);
    }
    if let Ok(ip) = addr.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    // имя хоста с портом или без
    let host_port = match addr.rsplit_once(':') {
        Some((_, port)) if port.parse::<u16>().is_ok() => addr.to_owned(),
        _ => format!("{addr}:{DEFAULT_PORT}"),
    };
    host_port
        .to_socket_addrs()
        .map_err(|e| format!("{addr}: {e}"))?
        .next()
        .ok_or_else(|| format!("{addr}: no address"))
}

impl Game {
    /// Подключиться к серверу по адресу `addr` под именем `name`.
    pub fn connect(&mut self, addr: &str, name: &str) {
        let res = resolve(addr).and_then(|a| {
            let link = UdpLink::connect(a).map_err(|e| e.to_string())?;
            Client::connect(link, &self.reg, name).map_err(|e| e.to_string())
        });
        match res {
            Ok(client) => {
                self.net = Some(NetPlay {
                    client,
                    addr: addr.to_owned(),
                    player: None,
                    acc: 0.0,
                });
                self.paused = false;
                self.speed = 1;
                self.screen = Screen::Connecting;
            }
            Err(e) => self.notify(format!("CANNOT CONNECT: {e}").to_uppercase()),
        }
    }

    /// Идёт ли сетевая партия.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn online(&self) -> bool {
        self.net.is_some()
    }

    fn leave_net(&mut self, why: impl Into<String>) {
        self.net = None;
        if matches!(
            self.screen,
            Screen::Connecting | Screen::Playing | Screen::PauseMenu
        ) {
            self.screen = Screen::MainMenu;
        }
        self.message = Some((why.into(), 8.0));
    }

    /// Шаг сетевой партии: приём, отправка команд, расчёт тиков по кадрам. Возвращает число тиков.
    pub(super) fn update_net(&mut self, dt: f64) -> u32 {
        if !matches!(
            self.screen,
            Screen::Connecting | Screen::Playing | Screen::PauseMenu
        ) {
            self.leave_net("LEFT THE MULTIPLAYER GAME");
            return 0;
        }
        let Some(net) = self.net.as_mut() else {
            return 0;
        };
        let mut failure = None;
        if let Err(e) = net.client.link_mut().update(Duration::from_secs_f64(dt)) {
            failure = Some(e.to_string());
        }
        let events = net.client.poll(&self.reg);
        // снимки — до расчёта тиков: считать надо уже новый мир
        for e in events {
            match e {
                ClientEvent::Joined(p, w) => {
                    self.begin_play(*w);
                    if let Some(net) = self.net.as_mut() {
                        net.player = Some(p);
                    }
                    self.message = Some((format!("CONNECTED AS PLAYER {p}"), 5.0));
                }
                ClientEvent::Resynced(w) => {
                    self.world = *w;
                    self.notify("RESYNCED WITH THE SERVER");
                }
                ClientEvent::Desync { tick, .. } => {
                    self.notify(format!("DESYNC AT TICK {tick}: RELOADING FROM THE SERVER"));
                }
                ClientEvent::Rejected(why) | ClientEvent::Disconnected(why) => {
                    failure = Some(why);
                }
                ClientEvent::Results(_) => {}
            }
        }
        let Some(net) = self.net.as_mut() else {
            return 0;
        };
        for c in self.pending.drain(..) {
            if let Err(e) = net.client.send(c) {
                failure = Some(e.to_string());
            }
        }
        let mut ticks = 0;
        if net.client.joined() {
            let step = 1.0 / f64::from(TICKS_PER_SECOND);
            net.acc = (net.acc + dt).min(step * FRAME_BUFFER as f64);
            let mut n = (net.acc / step) as usize;
            net.acc -= n as f64 * step;
            let buffered = net.client.buffered();
            if buffered > n + FRAME_BUFFER {
                n = (buffered - FRAME_BUFFER / 2).min(MAX_CATCH_UP);
            }
            let (done, events) = net.client.advance(&self.reg, &mut self.world, n);
            ticks = done as u32;
            for e in events {
                match e {
                    // почему не встала постройка — как в одиночной игре
                    ClientEvent::Results(r) => self.report_failures(&r),
                    ClientEvent::Desync { tick, .. } => {
                        self.message = Some((
                            format!("DESYNC AT TICK {tick}: RELOADING FROM THE SERVER"),
                            5.0,
                        ));
                    }
                    ClientEvent::Disconnected(why) => failure = Some(why),
                    _ => {}
                }
            }
        }
        if let Some(net) = self.net.as_mut()
            && let Err(e) = net.client.link_mut().flush()
        {
            failure = Some(e.to_string());
        }
        if let Some(f) = failure {
            self.leave_net(format!("DISCONNECTED: {f}").to_uppercase());
        }
        ticks
    }

    /// Строка состояния сети для HUD.
    pub(super) fn net_status(&self) -> Option<String> {
        let net = self.net.as_ref()?;
        let rtt = net.client.link().rtt_ms();
        Some(match net.player {
            Some(p) => format!("ONLINE  PLAYER {p}  PING {rtt:.0} MS"),
            None => "CONNECTING".to_owned(),
        })
    }

    /// Прямоугольник кнопки отмены на экране подключения.
    pub fn connecting_cancel_rect(&self) -> Rect {
        self.menu_rects(1, self.camera.viewport[1] * 0.55)[0]
    }

    pub(super) fn connecting_screen(
        &mut self,
        ui: &mut Ui,
        atlas: &Atlas,
        table: &SpriteTable,
        hud: &mut Hud,
    ) {
        let u = self.u();
        let (vw, vh) = (self.camera.viewport[0], self.camera.viewport[1]);
        let white = table.white;
        hud.rect(atlas, white, [0.0, 0.0, vw, vh], [0.0, 0.0, 0.0, 0.8]);
        let addr = self.net.as_ref().map_or("", |n| n.addr.as_str());
        let t = format!("CONNECTING TO {addr}").to_uppercase();
        hud.text(
            atlas,
            (vw - text_width(&t, 3.0 * u)) / 2.0,
            vh * 0.4,
            3.0 * u,
            &t,
            rgba(palette::FORGE_ORANGE),
        );
        if ui.button(
            hud,
            atlas,
            white,
            self.connecting_cancel_rect(),
            "CANCEL",
            3.0 * u,
            true,
        ) {
            self.leave_net("CONNECTION CANCELLED");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::game::Key;
    use ferrustry_content::ContentRegistry;
    use ferrustry_net::{Server, ServerConfig, UdpHub, UdpPeer};
    use ferrustry_sim::mapgen::{self, Difficulty, MapSettings, MapSize};
    use ferrustry_sim::{Command, Dir, TilePos, scenario, state_hash};
    use std::time::Instant;

    #[test]
    fn game_joins_a_udp_server_builds_and_matches_it() {
        let reg = ContentRegistry::base().unwrap();
        let world = mapgen::generate(
            &reg,
            &MapSettings {
                size: MapSize::Small,
                difficulty: Difficulty::Normal,
                seed: 8,
            },
        )
        .unwrap();
        let hub = UdpHub::bind("127.0.0.1:0".parse().unwrap(), 8).unwrap();
        let addr = hub.local_addr().unwrap().to_string();
        let mut server: Server<UdpPeer> = Server::new(world, ServerConfig::default());
        let demo = scenario::demo_map(&reg, 1).unwrap();
        let mut g = Game::new(reg.clone(), demo, [1280.0, 720.0]);
        g.connect(&addr, "tester");
        assert_eq!(g.screen(), Screen::Connecting);
        assert!(g.online());
        let wall = reg.block_by_name("iron_wall").unwrap();
        let at = TilePos::new(10, 10);
        let start = Instant::now();
        let mut sent = false;
        let pump = |g: &mut Game, server: &mut Server<UdpPeer>, tick: bool| {
            hub.update(Duration::from_millis(10)).unwrap();
            for p in hub.accept() {
                server.accept(p);
            }
            server.poll(&reg);
            if tick {
                server.tick(&reg);
            }
            hub.flush();
            g.update(1.0 / 30.0);
            std::thread::sleep(Duration::from_millis(2));
        };
        while server.world().tick_count() < 120 {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out");
            pump(&mut g, &mut server, true);
            if g.screen() == Screen::Playing && !sent {
                // пауза и скорость в сетевой партии недоступны
                g.handle(crate::game::Input::Key(Key::Pause, true));
                assert!(!g.paused);
                g.queue(vec![Command::Place {
                    block: wall,
                    origin: at,
                    rot: Dir::Right,
                }]);
                sent = true;
            }
        }
        assert!(sent, "joined");
        // сервер стоит, игра догоняет
        while g.world.tick_count() < server.world().tick_count() {
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "catch-up timed out"
            );
            pump(&mut g, &mut server, false);
        }
        assert!(
            server.world().grid().building(at).is_some(),
            "command reached the server"
        );
        assert_eq!(state_hash(&g.world), state_hash(server.world()));
        // та же клетка ещё раз — отказ сервера виден игроку
        g.queue(vec![Command::Place {
            block: wall,
            origin: at,
            rot: Dir::Right,
        }]);
        let target = server.world().tick_count() + 30;
        while g.world.tick_count() < target {
            assert!(start.elapsed() < Duration::from_secs(60), "retry timed out");
            let tick = server.world().tick_count() < target;
            pump(&mut g, &mut server, tick);
        }
        assert!(
            g.message
                .as_ref()
                .is_some_and(|(m, _)| m.contains("OCCUPIED")),
            "{:?}",
            g.message
        );
        assert!(g.net_status().unwrap().contains("PLAYER 1"));
        // выход в главное меню — отключение
        g.screen = Screen::MainMenu;
        g.update(1.0 / 30.0);
        assert!(!g.online());
    }

    #[test]
    fn addresses_get_the_default_port() {
        assert_eq!(resolve("127.0.0.1").unwrap().port(), DEFAULT_PORT);
        assert_eq!(resolve("127.0.0.1:4000").unwrap().port(), 4000);
        assert_eq!(resolve("[::1]:4001").unwrap().port(), 4001);
        assert_eq!(resolve("localhost:4002").unwrap().port(), 4002);
        assert!(resolve("no such host!").is_err());
    }
}
