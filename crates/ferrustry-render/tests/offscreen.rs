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

fn wgpu_format() -> ferrustry_render::gpu::Format {
    ferrustry_render::gpu::Format::Rgba8UnormSrgb
}
