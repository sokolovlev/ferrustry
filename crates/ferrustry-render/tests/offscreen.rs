//! Рендер вне экрана на реальном (или программном) GPU. Если адаптера нет — тест пропускается
//! с сообщением (в CI на Linux ставится lavapipe, поэтому там он выполняется).
#![allow(clippy::unwrap_used, missing_docs)]

use ferrustry_content::ContentRegistry;
use ferrustry_render::atlas::palette;
use ferrustry_render::{
    Atlas, Camera, Gpu, Hud, SpriteBatch, SpriteTable, build_world, render_offscreen, rgba,
    write_png,
};
use ferrustry_sim::scenario;

#[test]
fn renders_vertical_slice() {
    let gpu = match Gpu::headless() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("SKIPPED: no GPU adapter ({e})");
            return;
        }
    };
    eprintln!("adapter: {}", gpu.adapter_name());
    let reg = ContentRegistry::base().unwrap();
    let atlas = Atlas::build(&reg);
    let table = SpriteTable::new(&atlas, &reg);
    let (mut world, _) = scenario::vertical_slice(&reg).unwrap();
    for _ in 0..600 {
        world.step(&reg);
    }
    let (w, h) = (640u32, 360u32);
    // центр на ядре (18..21, 2..5), 32 px на тайл
    let cam = Camera::new([12.0, 4.5], 32.0, [w as f32, h as f32]);
    let mut batch = SpriteBatch::default();
    let mut dirs = Vec::new();
    build_world(&mut batch, &table, &reg, &world, &cam, &[], &mut dirs);
    let mut hud = Hud::default();
    hud.begin(h as f32);
    hud.text(
        &atlas,
        8.0,
        8.0,
        2.0,
        "FERRUSTRY",
        rgba(palette::FORGE_ORANGE),
    );
    let mut renderer =
        ferrustry_render::Renderer::new(&gpu.device, &gpu.queue, wgpu_format(), &atlas);
    let px = render_offscreen(&gpu, &mut renderer, &cam, &batch, &hud.batch).unwrap();
    assert_eq!(px.len(), (w * h * 4) as usize);

    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("vertical_slice.png");
    write_png(&out, w, h, &px).unwrap();
    eprintln!("screenshot: {}", out.display());

    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let close = |a: [u8; 3], b: [u8; 4], tol: i32| {
        a.iter()
            .zip(b)
            .all(|(&x, y)| (i32::from(x) - i32::from(y)).abs() <= tol)
    };
    // «F» на ядре: центр ядра (19.5, 3.5) → пиксель; глиф рисуется оранжевым
    // ножка «F» в спрайте ядра: x = 38..44 из 96 px → мир 18 + 41/32 ≈ 19.28
    let core_px = cam.world_to_screen([19.28, 3.5]);
    let probe = at(core_px[0] as u32, core_px[1] as u32);
    assert!(
        close(probe, palette::FORGE_ORANGE, 12),
        "core emblem pixel {probe:?}"
    );
    // надпись в левом верхнем углу — оранжевая
    let mut orange = 0;
    for y in 8..22 {
        for x in 8..120 {
            orange += u32::from(close(at(x, y), palette::FORGE_ORANGE, 12));
        }
    }
    assert!(orange > 100, "hud text pixels: {orange}");
}

/// Оборона: склад, топливный генератор, дуговая турель в момент разряда, новые враги.
#[test]
fn renders_defense_scene_with_arc_discharge() {
    use ferrustry_sim::{Command, Dir, TilePos, World};
    let gpu = match Gpu::headless() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("SKIPPED: no GPU adapter ({e})");
            return;
        }
    };
    let reg = ContentRegistry::base().unwrap();
    let atlas = Atlas::build(&reg);
    let table = SpriteTable::new(&atlas, &reg);
    let ids = scenario::BaseIds::resolve(&reg).unwrap();
    let b = |n: &str| reg.block_by_name(n).unwrap();
    let place = |block, x, y, rot| Command::Place {
        block,
        origin: TilePos::new(x, y),
        rot,
    };
    let mut world = World::new(&reg, 30, 14, ids.basalt, 1);
    let mut cmds = vec![
        place(ids.core, 24, 5, Dir::Right),
        place(b("arc_tower"), 16, 8, Dir::Right),
        place(b("gun_turret"), 16, 3, Dir::Right),
        place(b("burner"), 19, 8, Dir::Right),
        place(b("depot"), 19, 2, Dir::Right),
        place(ids.conveyor, 21, 2, Dir::Right),
        place(ids.conveyor, 22, 2, Dir::Right),
        place(ids.conveyor, 23, 2, Dir::Up),
        place(ids.conveyor, 18, 8, Dir::Right),
    ];
    cmds.extend((2..12).map(|y| place(b("iron_wall"), 12, y, Dir::Right)));
    scenario::build(&mut world, &reg, &cmds).unwrap();
    world.debug_fill_turrets(&reg);
    world.debug_fill_conveyors(reg.item_by_name("carbon").unwrap(), 64);
    for (k, name) in ["spitter", "shellback", "crawler", "spitter"]
        .iter()
        .enumerate()
    {
        let x = 14 * 256 + 128 + (k as i32 % 2) * 300;
        let y = (4 + 2 * k as i32) * 256 + 128;
        world.spawn_unit(&reg, reg.unit_by_name(name).unwrap(), x, y);
    }
    // дождаться видимого разряда
    let arc_at = TilePos::new(16, 8);
    let arc = |w: &World| {
        let id = w.grid().building(arc_at)?;
        w.arc_beam(w.buildings().get(id)?)
    };
    let mut flashed = false;
    for _ in 0..300 {
        world.step(&reg);
        if arc(&world).is_some() {
            flashed = true;
            break;
        }
    }
    assert!(flashed, "arc tower never fired");
    let (w, h) = (640u32, 360u32);
    let cam = Camera::new([17.0, 7.0], 26.0, [w as f32, h as f32]);
    let mut batch = SpriteBatch::default();
    let mut dirs = Vec::new();
    build_world(&mut batch, &table, &reg, &world, &cam, &[], &mut dirs);
    let hud = Hud::default();
    let mut renderer =
        ferrustry_render::Renderer::new(&gpu.device, &gpu.queue, wgpu_format(), &atlas);
    let px = render_offscreen(&gpu, &mut renderer, &cam, &batch, &hud.batch).unwrap();
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("defense_scene.png");
    write_png(&out, w, h, &px).unwrap();
    eprintln!("screenshot: {}", out.display());
    // голубое свечение разряда присутствует
    let blue = px
        .chunks_exact(4)
        .filter(|p| p[2] > 200 && p[0] < 170 && p[1] > 120)
        .count();
    assert!(blue > 20, "arc glow pixels: {blue}");
}

fn wgpu_format() -> ferrustry_render::gpu::Format {
    ferrustry_render::gpu::Format::Rgba8UnormSrgb
}
