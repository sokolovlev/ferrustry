//! Окно (winit) и поверхность wgpu: перевод событий ОС во [`Input`], цикл кадров.

use std::sync::Arc;
use std::time::Instant;

use ferrustry_render::wgpu;
use ferrustry_render::{Atlas, Gpu, Hud, Renderer, SpriteBatch, SpriteTable};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::game::{
    Button, Game, Input, Key, LEGACY_MAP_FILE, MAPS_DIR, MapEntry, Request, next_map_path,
};
use ferrustry_content::ContentRegistry;
use ferrustry_sim::map::MapDef;

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    gpu: Gpu,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

/// Приложение: игра + графика, создаваемая при `resumed`.
pub struct App {
    game: Game,
    atlas: Atlas,
    table: SpriteTable,
    world_batch: SpriteBatch,
    hud: Hud,
    gfx: Option<Gfx>,
    last: Instant,
    frames: u64,
    exit_after: Option<u64>,
    /// Ошибка, из-за которой приложение завершилось.
    pub error: Option<String>,
}

impl App {
    /// Создаёт приложение; `exit_after` — закрыться после N кадров (дымовые тесты).
    pub fn new(game: Game, exit_after: Option<u64>) -> Self {
        let atlas = Atlas::build(&game.reg);
        let table = SpriteTable::new(&atlas, &game.reg);
        Self {
            game,
            atlas,
            table,
            world_batch: SpriteBatch::default(),
            hud: Hud::default(),
            gfx: None,
            last: Instant::now(),
            frames: 0,
            exit_after,
            error: None,
        }
    }

    /// Тик мира и число построек (для журнала при выходе и сквозных тестов).
    pub fn stats(&self) -> (u64, usize) {
        (
            self.game.world.tick_count(),
            self.game.world.buildings().len(),
        )
    }

    fn init_gfx(&mut self, el: &ActiveEventLoop) -> Result<Gfx, String> {
        let attrs = Window::default_attributes()
            .with_title("Ferrustry")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let gpu = Gpu::new(instance, Some(&surface)).map_err(|e| e.to_string())?;
        eprintln!("ferrustry: GPU {}", gpu.adapter_name());
        let size = window.inner_size();
        let caps = surface.get_capabilities(&gpu.adapter);
        // sRGB-формат обязателен для правильных цветов бренда (см. batch::rgba)
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or("surface has no formats")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: Default::default(),
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        let renderer = Renderer::new(&gpu.device, &gpu.queue, format, &self.atlas);
        self.game
            .handle(Input::Resize([config.width as f32, config.height as f32]));
        self.game.handle(Input::Scale(window.scale_factor() as f32));
        Ok(Gfx {
            window,
            surface,
            gpu,
            config,
            renderer,
        })
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.game.update(dt);
        for r in self.game.take_requests() {
            self.service(r, el);
        }
        let Some(gfx) = self.gfx.as_mut() else { return };
        self.game.frame(
            &self.atlas,
            &self.table,
            &mut self.world_batch,
            &mut self.hud,
        );
        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => Some((t, false)),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some((t, true)),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.gpu.device, &gfx.config);
                None
            }
            _ => None,
        };
        if let Some((tex, suboptimal)) = frame {
            let view = tex
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            gfx.renderer.render(
                &gfx.gpu.device,
                &gfx.gpu.queue,
                &view,
                &self.game.camera,
                &self.world_batch,
                &self.hud.batch,
            );
            gfx.window.pre_present_notify();
            gfx.gpu.queue.present(tex);
            if suboptimal {
                gfx.surface.configure(&gfx.gpu.device, &gfx.config);
            }
        }
        self.frames += 1;
        if self.exit_after.is_some_and(|n| self.frames >= n) {
            eprintln!(
                "ferrustry: exiting after {} frames, tick {}",
                self.frames,
                self.game.world.tick_count()
            );
            el.exit();
        }
    }

    fn service(&mut self, r: Request, el: &ActiveEventLoop) {
        let path = Game::quicksave_path();
        match r {
            Request::Save => {
                let res = ferrustry_format::write_save(&self.game.world, &self.game.reg)
                    .map_err(|e| e.to_string())
                    .and_then(|b| std::fs::write(path, b).map_err(|e| e.to_string()));
                if let Err(e) = &res {
                    eprintln!("ferrustry: save failed: {e}");
                }
                self.game.saved(res.is_ok());
            }
            Request::Load => {
                let res = std::fs::read(path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| {
                        ferrustry_format::read_save(&b, &self.game.reg).map_err(|e| e.to_string())
                    });
                if let Err(e) = &res {
                    eprintln!("ferrustry: load failed: {e}");
                }
                self.game.loaded(res.ok());
            }
            Request::Quit => el.exit(),
            Request::SaveSchematic(sch) => {
                let res = ferrustry_format::write_schematic(&sch, &self.game.reg)
                    .map_err(|e| e.to_string())
                    .and_then(|b| {
                        std::fs::write(Game::schematic_path(), b).map_err(|e| e.to_string())
                    });
                if let Err(e) = &res {
                    eprintln!("ferrustry: schematic save failed: {e}");
                }
                self.game.schematic_saved(res.is_ok());
            }
            Request::LoadSchematic => {
                let res = std::fs::read(Game::schematic_path())
                    .map_err(|e| e.to_string())
                    .and_then(|b| {
                        ferrustry_format::read_schematic(&b, &self.game.reg)
                            .map_err(|e| e.to_string())
                    });
                if let Err(e) = &res {
                    eprintln!("ferrustry: schematic load failed: {e}");
                }
                self.game.schematic_loaded(res.ok());
            }
            Request::SaveMap { def, path } => {
                let res = save_map(&def, path, &self.game.reg);
                if let Err(e) = &res {
                    eprintln!("ferrustry: map save failed: {e}");
                }
                self.game.map_saved(res);
            }
            Request::ListMaps => {
                let maps = list_maps(&self.game.reg);
                self.game.maps_listed(maps);
            }
            Request::LoadMap { path, purpose } => {
                let res = read_map_file(&path, &self.game.reg);
                if let Err(e) = &res {
                    eprintln!("ferrustry: map load failed: {e}");
                }
                self.game.map_loaded(&path, purpose, res);
            }
        }
    }
}

fn read_map_file(path: &str, reg: &ContentRegistry) -> Result<MapDef, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    ferrustry_format::read_map(&bytes, reg).map_err(|e| e.to_string())
}

/// Записать карту в `path` или в первый свободный файл папки карт; возвращает путь.
fn save_map(def: &MapDef, path: Option<String>, reg: &ContentRegistry) -> Result<String, String> {
    let path = match path {
        Some(p) => p,
        None => {
            std::fs::create_dir_all(MAPS_DIR).map_err(|e| format!("{MAPS_DIR}: {e}"))?;
            next_map_path(|p| std::path::Path::new(p).exists())
        }
    };
    let bytes = ferrustry_format::write_map(def, reg).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| format!("{path}: {e}"))?;
    Ok(path)
}

/// Карты папки `maps/` (по имени) и файл первой версии редактора, если он есть.
fn list_maps(reg: &ContentRegistry) -> Vec<MapEntry> {
    let mut paths: Vec<String> = std::fs::read_dir(MAPS_DIR)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "fmap"))
                .map(|p| {
                    format!(
                        "{MAPS_DIR}/{}",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    if std::path::Path::new(LEGACY_MAP_FILE).exists() {
        paths.push(LEGACY_MAP_FILE.to_owned());
    }
    paths
        .iter()
        .map(|p| MapEntry::new(p, &read_map_file(p, reg)))
        .collect()
}

fn map_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Digit1 => Key::Digit(1),
        KeyCode::Digit2 => Key::Digit(2),
        KeyCode::Digit3 => Key::Digit(3),
        KeyCode::Digit4 => Key::Digit(4),
        KeyCode::Digit5 => Key::Digit(5),
        KeyCode::Digit6 => Key::Digit(6),
        KeyCode::Digit7 => Key::Digit(7),
        KeyCode::Digit8 => Key::Digit(8),
        KeyCode::Digit9 => Key::Digit(9),
        KeyCode::Digit0 => Key::Digit(10),
        KeyCode::KeyR => Key::Rotate,
        KeyCode::Space => Key::Pause,
        KeyCode::Escape => Key::Cancel,
        KeyCode::F5 => Key::Save,
        KeyCode::F9 => Key::Load,
        KeyCode::F1 => Key::Help,
        KeyCode::Tab => Key::NextCategory,
        KeyCode::KeyC => Key::Copy,
        KeyCode::KeyZ => Key::Undo,
        KeyCode::KeyY => Key::Redo,
        KeyCode::F6 => Key::SaveSchematic,
        KeyCode::F7 => Key::LoadSchematic,
        KeyCode::Equal | KeyCode::NumpadAdd => Key::Faster,
        KeyCode::Minus | KeyCode::NumpadSubtract => Key::Slower,
        KeyCode::KeyW | KeyCode::ArrowUp => Key::PanUp,
        KeyCode::KeyS | KeyCode::ArrowDown => Key::PanDown,
        KeyCode::KeyA | KeyCode::ArrowLeft => Key::PanLeft,
        KeyCode::KeyD | KeyCode::ArrowRight => Key::PanRight,
        _ => return None,
    })
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match self.init_gfx(el) {
            Ok(g) => {
                g.window.request_redraw();
                self.gfx = Some(g);
                self.last = Instant::now();
            }
            Err(e) => {
                self.error = Some(format!("graphics initialisation failed: {e}"));
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.game.handle(Input::Scale(scale_factor as f32));
            }
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.config.width = size.width.max(1);
                    g.config.height = size.height.max(1);
                    g.surface.configure(&g.gpu.device, &g.config);
                }
                self.game.handle(Input::Resize([
                    size.width.max(1) as f32,
                    size.height.max(1) as f32,
                ]));
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.game
                    .handle(Input::CursorMoved([position.x as f32, position.y as f32]));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    MouseButton::Middle => Button::Middle,
                    _ => return,
                };
                self.game.handle(match state {
                    ElementState::Pressed => Input::MouseDown(b),
                    ElementState::Released => Input::MouseUp(b),
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
                self.game.handle(Input::Wheel(d));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(k) = map_key(code)
                    && !(event.repeat && event.state == ElementState::Pressed)
                {
                    self.game
                        .handle(Input::Key(k, event.state == ElementState::Pressed));
                }
            }
            WindowEvent::RedrawRequested => self.redraw(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }
}
