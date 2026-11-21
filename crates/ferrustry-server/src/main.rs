//! Выделенный сервер Ferrustry: партия без графики, игроки подключаются по UDP (этап 6).
//!
//! ```text
//! ferrustry-server [--port 27960] [--bind 0.0.0.0]
//!                  [--map FILE.fmap | --load FILE.fsave | --size small|medium|large
//!                   --difficulty easy|normal|hard --seed S]
//!                  [--max-players 8] [--run-empty] [--ticks N]
//! ```
//!
//! Пока в партии никого нет, симуляция стоит (иначе волны снесут базу без игроков);
//! `--run-empty` — считать всегда. `--ticks N` — выйти после N тиков (проверки, CI).

use std::process::ExitCode;
use std::time::{Duration, Instant};

use ferrustry_content::ContentRegistry;
use ferrustry_net::{DEFAULT_PORT, Server, ServerConfig, ServerEvent, UdpHub, UdpPeer};
use ferrustry_sim::mapgen::{self, Difficulty, MapSettings, MapSize};
use ferrustry_sim::{TICKS_PER_SECOND, World};

struct Options {
    bind: String,
    port: u16,
    map: Option<String>,
    load: Option<String>,
    size: MapSize,
    difficulty: Difficulty,
    seed: u64,
    max_players: usize,
    run_empty: bool,
    ticks: Option<u64>,
}

fn parse() -> Result<Options, String> {
    let mut o = Options {
        bind: "0.0.0.0".into(),
        port: DEFAULT_PORT,
        map: None,
        load: None,
        size: MapSize::Medium,
        difficulty: Difficulty::Normal,
        seed: 1,
        max_players: 8,
        run_empty: false,
        ticks: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or(format!("{a} requires a value"));
        match a.as_str() {
            "--bind" => o.bind = val()?,
            "--port" => o.port = val()?.parse().map_err(|_| "bad --port")?,
            "--map" => o.map = Some(val()?),
            "--load" => o.load = Some(val()?),
            "--size" => {
                o.size = match val()?.as_str() {
                    "small" => MapSize::Small,
                    "medium" => MapSize::Medium,
                    "large" => MapSize::Large,
                    other => return Err(format!("unknown size `{other}`")),
                }
            }
            "--difficulty" => {
                o.difficulty = match val()?.as_str() {
                    "easy" => Difficulty::Easy,
                    "normal" => Difficulty::Normal,
                    "hard" => Difficulty::Hard,
                    other => return Err(format!("unknown difficulty `{other}`")),
                }
            }
            "--seed" => o.seed = val()?.parse().map_err(|_| "bad --seed")?,
            "--max-players" => {
                o.max_players = val()?.parse().map_err(|_| "bad --max-players")?;
            }
            "--run-empty" => o.run_empty = true,
            "--ticks" => o.ticks = Some(val()?.parse().map_err(|_| "bad --ticks")?),
            "-h" | "--help" => {
                println!(
                    "ferrustry-server [--port P] [--bind ADDR] [--map FILE.fmap | --load FILE.fsave | \
                     --size S --difficulty D --seed N] [--max-players N] [--run-empty] [--ticks N]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(o)
}

fn world(o: &Options, reg: &ContentRegistry) -> Result<World, String> {
    if let Some(p) = &o.map {
        let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        let def = ferrustry_format::read_map(&bytes, reg).map_err(|e| format!("{p}: {e}"))?;
        return def
            .instantiate(reg, o.seed)
            .map_err(|e| format!("{p}: {e}"));
    }
    if let Some(p) = &o.load {
        let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        return ferrustry_format::read_save(&bytes, reg).map_err(|e| format!("{p}: {e}"));
    }
    let settings = MapSettings {
        size: o.size,
        difficulty: o.difficulty,
        seed: o.seed,
    };
    mapgen::generate(reg, &settings).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let o = parse()?;
    let reg = ContentRegistry::base().map_err(|e| e.to_string())?;
    let world = world(&o, &reg)?;
    let addr = format!("{}:{}", o.bind, o.port)
        .parse()
        .map_err(|_| format!("bad address {}:{}", o.bind, o.port))?;
    let hub = UdpHub::bind(addr, o.max_players).map_err(|e| format!("{addr}: {e}"))?;
    let local = hub
        .local_addr()
        .map_or_else(|| addr.to_string(), |a| a.to_string());
    println!(
        "ferrustry-server {}: listening on {local} (UDP), map {}x{}, tick {}",
        ferrustry_sim::SIM_VERSION,
        world.grid().width(),
        world.grid().height(),
        world.tick_count()
    );
    let config = ServerConfig {
        max_players: o.max_players,
        ..Default::default()
    };
    let mut server: Server<UdpPeer> = Server::new(world, config);

    let step = Duration::from_secs(1) / TICKS_PER_SECOND;
    let mut next = Instant::now();
    let mut last = Instant::now();
    let mut ran = 0u64;
    let (mut busy, mut busy_ticks, mut report) = (Duration::ZERO, 0u32, Instant::now());
    loop {
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
            continue;
        }
        let dt = last.elapsed();
        last = Instant::now();
        hub.update(dt).map_err(|e| e.to_string())?;
        for p in hub.accept() {
            server.accept(p);
        }
        for e in server.poll(&reg) {
            match e {
                ServerEvent::Joined { player, name } => println!("player {player} `{name}` joined"),
                ServerEvent::Left(p) => println!("player {p} left"),
                ServerEvent::Rejected(why) => println!("connection rejected: {why}"),
                ServerEvent::Misbehaved(why) => println!("client dropped: {why}"),
                ServerEvent::Resynced(p) => println!("player {p} desynced, sent a fresh snapshot"),
            }
        }
        // догон при отставании — не больше 8 тиков за проход (как в клиенте)
        let mut n = 0;
        while next <= Instant::now() && n < 8 {
            if o.run_empty || server.players() > 0 {
                let t = Instant::now();
                server.tick(&reg);
                busy += t.elapsed();
                busy_ticks += 1;
                ran += 1;
            }
            next += step;
            n += 1;
        }
        if n == 8 {
            next = Instant::now();
        }
        hub.flush();
        if report.elapsed() >= Duration::from_secs(10) {
            let mean = busy.as_secs_f64() * 1e6 / f64::from(busy_ticks.max(1));
            println!(
                "tick {}, players {}, mean tick {mean:.0} us",
                server.world().tick_count(),
                server.players()
            );
            (busy, busy_ticks, report) = (Duration::ZERO, 0, Instant::now());
        }
        if o.ticks.is_some_and(|t| ran >= t) {
            println!(
                "stopping after {ran} ticks at tick {}",
                server.world().tick_count()
            );
            return Ok(());
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
