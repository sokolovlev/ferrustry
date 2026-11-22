//! Клиент Ferrustry.
//!
//! ```text
//! ferrustry                          окно, демо-карта
//! ferrustry --load FILE              окно, загрузить сохранение
//! ferrustry --connect ADDR [--name N]  сетевая партия на выделенном сервере (порт по умолчанию 27960)
//! ferrustry --screenshot FILE.png    отрисовать кадр без окна (CI, отчёты)
//!           [--size 1280x720] [--ticks N] [--zoom PX] [--center X,Y] [--menu | --editor]
//! ferrustry --frames N               закрыться после N кадров (дымовой тест окна)
//! ```

mod app;
mod game;
mod ui;

use std::process::ExitCode;

use ferrustry_content::ContentRegistry;
use ferrustry_render::{
    Atlas, Gpu, Hud, Renderer, SpriteBatch, SpriteTable, render_offscreen, write_png,
};
use ferrustry_sim::scenario;
use winit::event_loop::EventLoop;

use crate::game::Game;

struct Opts {
    screenshot: Option<String>,
    size: [u32; 2],
    ticks: u64,
    zoom: Option<f32>,
    center: Option<[f32; 2]>,
    load: Option<String>,
    frames: Option<u64>,
    seed: u64,
    menu: bool,
    editor: bool,
    connect: Option<String>,
    name: String,
}

fn parse() -> Result<Opts, String> {
    let mut o = Opts {
        screenshot: None,
        size: [1280, 720],
        ticks: 0,
        zoom: None,
        center: None,
        load: None,
        frames: None,
        seed: 1,
        menu: false,
        editor: false,
        connect: None,
        name: "player".into(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or(format!("{a} requires a value"));
        match a.as_str() {
            "--screenshot" => o.screenshot = Some(val()?),
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').ok_or("--size expects WxH")?;
                o.size = [
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                ];
            }
            "--ticks" => o.ticks = val()?.parse().map_err(|_| "bad --ticks")?,
            "--zoom" => o.zoom = Some(val()?.parse().map_err(|_| "bad --zoom")?),
            "--center" => {
                let v = val()?;
                let (x, y) = v.split_once(',').ok_or("--center expects X,Y")?;
                o.center = Some([
                    x.parse().map_err(|_| "bad x")?,
                    y.parse().map_err(|_| "bad y")?,
                ]);
            }
            "--load" => o.load = Some(val()?),
            "--frames" => o.frames = Some(val()?.parse().map_err(|_| "bad --frames")?),
            "--seed" => o.seed = val()?.parse().map_err(|_| "bad --seed")?,
            "--menu" => o.menu = true,
            "--editor" => o.editor = true,
            "--connect" => o.connect = Some(val()?),
            "--name" => o.name = val()?,
            "-h" | "--help" => {
                println!(
                    "see `ferrustry` docs: --screenshot FILE --size WxH --ticks N --zoom PX --center X,Y --load FILE --frames N --seed S --menu --editor --connect ADDR --name NAME"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(o)
}

fn main() -> ExitCode {
    #[cfg(target_arch = "x86_64")]
    if !std::arch::is_x86_feature_detected!("avx2") {
        eprintln!(
            "error: Ferrustry requires a CPU with AVX2 (Intel Haswell 2013+ / AMD Zen or newer)"
        );
        return ExitCode::FAILURE;
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let o = parse()?;
    let reg = ContentRegistry::base().map_err(|e| e.to_string())?;
    let world = match &o.load {
        Some(p) => {
            let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
            ferrustry_format::read_save(&bytes, &reg).map_err(|e| e.to_string())?
        }
        None => scenario::demo_map(&reg, o.seed).map_err(|e| e.to_string())?,
    };
    let mut game = Game::new(reg, world, [o.size[0] as f32, o.size[1] as f32]);
    if let Ok(text) = std::fs::read_to_string(Game::campaign_file())
        && let Ok(done) = text.trim().parse::<usize>()
    {
        game.set_campaign_progress(done);
    }
    if let Ok(text) = std::fs::read_to_string(Game::server_file()) {
        let mut lines = text.lines();
        let (addr, name) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""));
        game.set_join_defaults(addr.trim(), name.trim());
    }
    if let Some(addr) = &o.connect {
        game.connect(addr, &o.name);
    } else if o.editor {
        game.open_editor();
    } else if o.load.is_some() || (o.screenshot.is_some() && !o.menu) {
        game.start_playing();
    }
    if let Some(z) = o.zoom {
        game.camera.zoom = z;
    }
    if let Some(c) = o.center {
        game.camera.center = c;
    }
    for _ in 0..o.ticks {
        game.world.step(&game.reg);
    }

    if let Some(path) = o.screenshot {
        return screenshot(&mut game, &path, o.size);
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = app::App::new(game, o.frames);
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    let (tick, buildings) = app.stats();
    eprintln!("ferrustry: exit at tick {tick}, buildings {buildings}");
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn screenshot(game: &mut Game, path: &str, size: [u32; 2]) -> Result<(), String> {
    let gpu = Gpu::headless().map_err(|e| e.to_string())?;
    eprintln!("ferrustry: GPU {}", gpu.adapter_name());
    let atlas = Atlas::build(&game.reg);
    let table = SpriteTable::new(&atlas, &game.reg);
    let (mut wb, mut hud) = (SpriteBatch::default(), Hud::default());
    game.update(0.0);
    game.frame(&atlas, &table, &mut wb, &mut hud);
    let mut renderer = Renderer::new(
        &gpu.device,
        &gpu.queue,
        ferrustry_render::gpu::Format::Rgba8UnormSrgb,
        &atlas,
    );
    let px = render_offscreen(&gpu, &mut renderer, &game.camera, &wb, &hud.batch)
        .map_err(|e| e.to_string())?;
    write_png(std::path::Path::new(path), size[0], size[1], &px).map_err(|e| e.to_string())?;
    eprintln!(
        "ferrustry: wrote {path} ({}x{}, {} world sprites)",
        size[0],
        size[1],
        wb.len()
    );
    Ok(())
}
