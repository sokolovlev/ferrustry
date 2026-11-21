//! B7: выделенный сервер под нагрузкой ботов-клиентов по настоящему UDP (127.0.0.1).
//!
//! Боты проходят обычное рукопожатие и вход (получают снимок), присылают команды стройки
//! (по умолчанию 5 в секунду каждый) и читают всё, что шлёт сервер, но мир не считают: меряется
//! сервер, а не процессор ботов. Сервер идёт в реальном времени, 30 тиков в секунду.

use std::time::{Duration, Instant};

use ferrustry_content::ContentRegistry;
use ferrustry_net::{
    ClientMsg, Link, PROTOCOL_VERSION, Server, ServerConfig, ServerMsg, UdpHub, UdpLink, UdpPeer,
    decode, encode,
};
use ferrustry_sim::rng::Rng;
use ferrustry_sim::{Command, Dir, SIM_VERSION, TICKS_PER_SECOND, TilePos, scenario};
use serde::Serialize;

/// Отчёт B7.
#[derive(Debug, Serialize)]
pub struct NetBenchReport {
    clients: u32,
    conveyors: u32,
    buildings: usize,
    seconds: f64,
    commands_per_client_per_sec: u32,
    /// Время от подключения до получения снимка у последнего бота.
    join_all_ms: f64,
    snapshot_bytes: usize,
    /// Тик мира без сети (тот же мир, до запуска сервера).
    plain_tick_mean_us: f64,
    /// Полный кадр сервера: приём пакетов, команды, тик, рассылка, отправка.
    server_frame_mean_us: f64,
    server_frame_p99_us: f64,
    server_frame_max_us: f64,
    /// Сетевая часть кадра (кадр минус тик мира).
    network_mean_us: f64,
    ticks: u64,
    commands_sent: u64,
    /// Полезная нагрузка протокола сервер → клиент (кадры и суммы), КБ/с на клиента.
    payload_kb_per_client: f64,
    /// Трафик с заголовками и подтверждениями (оценка renet, после выхода снимка из окна).
    sent_kb_per_client: f64,
    recv_kb_per_client: f64,
    rtt_ms: f64,
    frames_received_per_client: f64,
}

struct Bot {
    link: UdpLink,
    joined: bool,
    frames: u64,
    /// Байт сообщений протокола получено за замер (без заголовков транспорта).
    payload: u64,
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p) as usize]
}

/// Прогнать B7.
pub fn run(
    reg: &ContentRegistry,
    clients: u32,
    conveyors: u32,
    seconds: f64,
    rate: u32,
) -> Result<NetBenchReport, String> {
    let world = scenario::bench_megabase(reg, conveyors.max(1)).map_err(|e| e.to_string())?;
    // тик без сети — для сравнения
    let mut plain = world.clone();
    for _ in 0..300 {
        plain.step(reg);
    }
    let t = Instant::now();
    for _ in 0..300 {
        plain.step(reg);
    }
    let plain_tick = t.elapsed().as_secs_f64() * 1e6 / 300.0;
    drop(plain);
    let buildings = world.buildings().len();
    let snapshot_bytes = ferrustry_format::write_save(&world, reg)
        .map_err(|e| e.to_string())?
        .len();

    let hub = UdpHub::bind(
        "127.0.0.1:0".parse().map_err(|_| "addr")?,
        clients as usize + 1,
    )
    .map_err(|e| e.to_string())?;
    let addr = hub.local_addr().ok_or("no address")?;
    let config = ServerConfig {
        max_players: clients as usize,
        ..Default::default()
    };
    let mut server: Server<UdpPeer> = Server::new(world, config);
    let mut bots: Vec<Bot> = (0..clients)
        .map(|_| {
            UdpLink::connect(addr).map(|link| Bot {
                link,
                joined: false,
                frames: 0,
                payload: 0,
            })
        })
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for (i, b) in bots.iter_mut().enumerate() {
        b.link
            .send(encode(&ClientMsg::Hello {
                protocol: PROTOCOL_VERSION,
                sim_version: SIM_VERSION.to_owned(),
                content: reg.fingerprint(),
                name: format!("bot{i}"),
            }))
            .map_err(|e| e.to_string())?;
    }
    let conveyor = reg.block_by_name("conveyor").ok_or("no conveyor")?;
    let (gw, gh) = (
        server.world().grid().width(),
        server.world().grid().height(),
    );
    let mut rng = Rng::new(7);
    let step = Duration::from_secs(1) / TICKS_PER_SECOND;
    let start = Instant::now();
    let mut join_all_ms = None;
    let mut next = Instant::now();
    let mut next_bot = Instant::now();
    let mut last = Instant::now();
    let (mut frames_us, mut tick_us) = (Vec::new(), Vec::new());
    let (mut sent, mut recv, mut rtt, mut samples) = (0.0, 0.0, 0.0, 0u32);
    let mut ticks = 0u64;
    let mut commands = 0u64;
    let mut measuring_since = None;
    loop {
        if start.elapsed() > Duration::from_secs_f64(seconds + 60.0) {
            return Err("B7 timed out (bots did not join)".into());
        }
        // боты: приём и отправка 60 раз в секунду, как кадры настоящей игры
        let bot_frame = Instant::now() >= next_bot;
        if bot_frame {
            next_bot += Duration::from_secs(1) / 60;
        }
        let dt = if bot_frame {
            last.elapsed()
        } else {
            Duration::ZERO
        };
        if bot_frame {
            last = Instant::now();
        }
        for b in bots.iter_mut().filter(|_| bot_frame) {
            b.link.update(dt).map_err(|e| e.to_string())?;
            while let Some(m) = b.link.recv().map_err(|e| e.to_string())? {
                if measuring_since.is_some() {
                    b.payload += m.len() as u64;
                }
                match decode::<ServerMsg>(&m).map_err(|e| e.to_string())? {
                    ServerMsg::Welcome { .. } => b.joined = true,
                    ServerMsg::Frame { .. } => b.frames += 1,
                    ServerMsg::Reject(why) => return Err(format!("bot rejected: {why}")),
                    _ => {}
                }
            }
            b.link.flush().map_err(|e| e.to_string())?;
        }
        if join_all_ms.is_none() && bots.iter().all(|b| b.joined) {
            join_all_ms = Some(start.elapsed().as_secs_f64() * 1e3);
            measuring_since = Some(Instant::now());
            for b in &mut bots {
                b.frames = 0;
                b.payload = 0;
            }
        }
        // сервер: кадр по расписанию 30 Hz
        if Instant::now() >= next {
            let f = Instant::now();
            hub.update(step).map_err(|e| e.to_string())?;
            for p in hub.accept() {
                server.accept(p);
            }
            server.poll(reg);
            let t = Instant::now();
            server.tick(reg);
            let tick = t.elapsed();
            hub.flush();
            let frame = f.elapsed();
            next += step;
            // команды ботов: в среднем `rate` в секунду на бота (уйдут со следующей отправкой)
            for b in bots.iter_mut().filter(|b| b.joined) {
                if rng.below(TICKS_PER_SECOND) < rate {
                    let p = TilePos::new(rng.below(gw) as i32, rng.below(gh) as i32);
                    b.link
                        .send(encode(&ClientMsg::Command(Command::Place {
                            block: conveyor,
                            origin: p,
                            rot: Dir::Right,
                        })))
                        .map_err(|e| e.to_string())?;
                    if measuring_since.is_some() {
                        commands += 1;
                    }
                }
            }
            if let Some(since) = measuring_since {
                frames_us.push(frame.as_secs_f64() * 1e6);
                tick_us.push(tick.as_secs_f64() * 1e6);
                ticks += 1;
                // трафик renet — скользящее окно 6 с: ждём, пока из него выйдет снимок входа
                if since.elapsed() > Duration::from_secs(7) {
                    for (s, r, ms) in hub.traffic() {
                        sent += s;
                        recv += r;
                        rtt += ms;
                        samples += 1;
                    }
                }
                if since.elapsed().as_secs_f64() >= seconds {
                    break;
                }
            }
        } else {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
    let n = f64::from(samples.max(1));
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let frame_mean = mean(&frames_us);
    let tick_mean = mean(&tick_us);
    let frames_received =
        bots.iter().map(|b| b.frames as f64).sum::<f64>() / f64::from(clients.max(1));
    let max = frames_us.iter().copied().fold(0.0, f64::max);
    Ok(NetBenchReport {
        clients,
        conveyors,
        buildings,
        seconds,
        commands_per_client_per_sec: rate,
        join_all_ms: join_all_ms.unwrap_or(0.0),
        snapshot_bytes,
        plain_tick_mean_us: plain_tick,
        server_frame_mean_us: frame_mean,
        server_frame_p99_us: pct(&mut frames_us, 0.99),
        server_frame_max_us: max,
        network_mean_us: frame_mean - tick_mean,
        ticks,
        commands_sent: commands,
        payload_kb_per_client: bots.iter().map(|b| b.payload as f64).sum::<f64>()
            / f64::from(clients.max(1))
            / seconds
            / 1024.0,
        sent_kb_per_client: sent / n / 1024.0,
        recv_kb_per_client: recv / n / 1024.0,
        rtt_ms: rtt / n,
        frames_received_per_client: frames_received,
    })
}
