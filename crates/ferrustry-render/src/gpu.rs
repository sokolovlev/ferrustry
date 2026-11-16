//! GPU-часть: конвейер instanced-спрайтов на wgpu, рендер в окно и вне экрана.

use wgpu::util::DeviceExt;

/// Формат текстур (реэкспорт для клиентов без прямой зависимости от wgpu).
pub use wgpu::TextureFormat as Format;

use crate::atlas::{Atlas, palette};
use crate::batch::{INSTANCE_FLOATS, SpriteBatch};
use crate::camera::Camera;

/// Ошибка инициализации или рендера.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// Нет подходящего графического адаптера.
    #[error("no suitable GPU adapter: {0}")]
    NoAdapter(String),
    /// Не удалось создать устройство.
    #[error("cannot create GPU device: {0}")]
    Device(String),
    /// Ошибка чтения кадра из GPU.
    #[error("frame readback failed: {0}")]
    Readback(String),
    /// Ошибка записи PNG.
    #[error("png: {0}")]
    Png(String),
}

const SHADER: &str = r#"
struct View { scale: vec2<f32>, offset: vec2<f32> };
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_smp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uvr: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) quarter_turns: f32,
) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, -0.5), vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, 0.5), vec2<f32>(-0.5, 0.5),
    );
    let c = corners[vi];
    let a = quarter_turns * 1.5707964;
    let cs = cos(a);
    let sn = sin(a);
    let local = c * size;
    let r = vec2<f32>(local.x * cs - local.y * sn, local.x * sn + local.y * cs);
    let world = center + r;
    var out: VOut;
    out.pos = vec4<f32>(world * view.scale + view.offset, 0.0, 1.0);
    let t = c + vec2<f32>(0.5, 0.5);
    // текстура хранится сверху вниз, мир — снизу вверх
    out.uv = vec2<f32>(mix(uvr.x, uvr.z, t.x), mix(uvr.w, uvr.y, t.y));
    out.color = color;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return textureSample(atlas_tex, atlas_smp, in.uv) * in.color;
}
"#;

const STRIDE: u64 = (INSTANCE_FLOATS * 4) as u64;

fn srgb_to_linear(c: u8) -> f64 {
    let v = f64::from(c) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Цвет фона (Deep Space) с учётом формата цели.
fn clear_color(format: wgpu::TextureFormat) -> wgpu::Color {
    let c = palette::DEEP_SPACE;
    let conv = |v: u8| {
        if format.is_srgb() {
            srgb_to_linear(v)
        } else {
            f64::from(v) / 255.0
        }
    };
    wgpu::Color {
        r: conv(c[0]),
        g: conv(c[1]),
        b: conv(c[2]),
        a: 1.0,
    }
}

/// Устройство и очередь GPU.
pub struct Gpu {
    /// Экземпляр wgpu.
    pub instance: wgpu::Instance,
    /// Адаптер.
    pub adapter: wgpu::Adapter,
    /// Устройство.
    pub device: wgpu::Device,
    /// Очередь.
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Создаёт устройство для экземпляра и (необязательно) совместимой поверхности.
    pub fn new(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, RenderError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ferrustry"),
            required_limits:
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Устройство без окна (тесты, скриншоты, CI).
    pub fn headless() -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        Self::new(instance, None)
    }

    /// Имя адаптера и бэкенд (для журнала и отчётов).
    pub fn adapter_name(&self) -> String {
        let i = self.adapter.get_info();
        format!("{} ({:?})", i.name, i.backend)
    }
}

/// Рендерер спрайтов.
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    world_view: wgpu::Buffer,
    hud_view: wgpu::Buffer,
    world_bg: wgpu::BindGroup,
    hud_bg: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: u64,
    format: wgpu::TextureFormat,
    scratch: Vec<f32>,
}

impl Renderer {
    /// Создаёт конвейер и загружает атлас.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        atlas: &Atlas,
    ) -> Self {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.width * 4),
                rows_per_image: Some(atlas.height),
            },
            wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: 1,
            },
        );
        let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprites"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let view_buf = |label| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&[0f32; 4]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        };
        let world_view = view_buf("world view");
        let hud_view = view_buf("hud view");
        let bind = |buf: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sprites"),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&tex_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let world_bg = bind(&world_view);
        let hud_bg = bind(&hud_view);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprites"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprites"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: STRIDE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 64 * 1024 * STRIDE;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            world_view,
            hud_view,
            world_bg,
            hud_bg,
            instances,
            capacity,
            format,
            scratch: Vec::new(),
        }
    }

    /// Формат цели.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Рисует слой мира и слой интерфейса в `target`.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        camera: &Camera,
        world: &SpriteBatch,
        hud: &SpriteBatch,
    ) {
        let (s, o) = camera.clip_transform();
        queue.write_buffer(
            &self.world_view,
            0,
            bytemuck::cast_slice(&[s[0], s[1], o[0], o[1]]),
        );
        let (vw, vh) = (camera.viewport[0].max(1.0), camera.viewport[1].max(1.0));
        // интерфейс: пиксели, Y вверх от нижнего края
        queue.write_buffer(
            &self.hud_view,
            0,
            bytemuck::cast_slice(&[2.0 / vw, 2.0 / vh, -1.0, -1.0]),
        );

        self.scratch.clear();
        self.scratch.extend_from_slice(world.floats());
        self.scratch.extend_from_slice(hud.floats());
        let bytes = (self.scratch.len() * 4) as u64;
        if bytes > self.capacity {
            self.capacity = bytes.next_power_of_two();
            self.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if bytes > 0 {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.scratch));
        }

        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color(self.format)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            let n_world = world.len() as u32;
            let n_hud = hud.len() as u32;
            if n_world > 0 {
                pass.set_bind_group(0, &self.world_bg, &[]);
                pass.draw(0..6, 0..n_world);
            }
            if n_hud > 0 {
                pass.set_bind_group(0, &self.hud_bg, &[]);
                pass.draw(0..6, n_world..n_world + n_hud);
            }
        }
        queue.submit(Some(enc.finish()));
    }
}

/// Рендер одного кадра вне экрана; возвращает RGBA8 (sRGB) построчно сверху вниз.
pub fn render_offscreen(
    gpu: &Gpu,
    renderer: &mut Renderer,
    camera: &Camera,
    world: &SpriteBatch,
    hud: &SpriteBatch,
) -> Result<Vec<u8>, RenderError> {
    let (w, h) = (camera.viewport[0] as u32, camera.viewport[1] as u32);
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: renderer.format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render(&gpu.device, &gpu.queue, &view, camera, world, hud);

    let row = (w * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(enc.finish()));
    let (tx, rx) = std::sync::mpsc::channel();
    buf.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    rx.recv()
        .map_err(|e| RenderError::Readback(e.to_string()))?
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    let data = buf
        .slice(..)
        .get_mapped_range()
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        out.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    drop(data);
    buf.unmap();
    if matches!(
        renderer.format(),
        wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
    ) {
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    Ok(out)
}

/// Записывает RGBA8 в PNG.
pub fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), RenderError> {
    let file = std::fs::File::create(path).map_err(|e| RenderError::Png(e.to_string()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc
        .write_header()
        .map_err(|e| RenderError::Png(e.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| RenderError::Png(e.to_string()))
}
