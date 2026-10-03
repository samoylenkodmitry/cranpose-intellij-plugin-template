//! GPU surfaces composited by the OS above an existing AWT window.
//!
//! No frame crosses JNI or passes through a CPU image. Call [`Target::new`] on
//! the AWT event thread, keep the owning window alive, and drop the presentation
//! before destroying its AWT peer. Native input remains with the host window.
use anyhow::{Context, Result, ensure};
use jni::{
    JNIEnv,
    objects::{GlobalRef, JObject, JString, JValue},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

/// A rectangle in AWT logical coordinates, relative to the owning window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}
impl Bounds {
    fn validate(self) -> Result<Self> {
        ensure!(
            self.x.is_finite()
                && self.y.is_finite()
                && self.scale.is_finite()
                && self.scale > 0.0
                && self.scale <= 8.0
                && self.width > 0
                && self.height > 0,
            "Invalid GPU layer bounds"
        );
        let [w, h] = self.pixels();
        ensure!(
            w <= 16384 && h <= 16384 && u64::from(w) * u64::from(h) <= 16_777_216,
            "GPU layer exceeds the 16 megapixel budget"
        );
        Ok(self)
    }
    /// Physical texture dimensions, rounded up to cover the logical rectangle.
    pub fn pixels(self) -> [u32; 2] {
        [
            (f64::from(self.width) * self.scale).ceil() as u32,
            (f64::from(self.height) * self.scale).ceil() as u32,
        ]
    }
}

/// Retained native target. Clones share one layer and serialize geometry edits.
#[derive(Clone)]
pub struct Target(Arc<Mutex<Inner>>);
struct Inner {
    native: platform::Layer,
    bounds: Bounds,
    _window: GlobalRef,
}
impl Target {
    /// Creates a transparent, input-transparent layer on the AWT event thread.
    pub fn new(env: &mut JNIEnv<'_>, window: &JObject<'_>, bounds: Bounds) -> Result<Self> {
        let bounds = bounds.validate()?;
        ensure!(
            env.call_static_method("java/awt/EventQueue", "isDispatchThread", "()Z", &[])?
                .z()?,
            "GPU layer creation requires the AWT event thread"
        );
        ensure!(
            env.call_method(window, "isDisplayable", "()Z", &[])?.z()?,
            "AWT window has no native peer"
        );
        let mut native = match platform::Layer::new(env, window) {
            Ok(native) => native,
            Err(error) => {
                // Optional JBR accessors may be absent. Clear only the exception
                // raised by this attempted backend before the host logs/falls back.
                if env.exception_check()? {
                    env.exception_clear()?;
                }
                return Err(error);
            }
        };
        native.place(bounds)?;
        Ok(Self(Arc::new(Mutex::new(Inner {
            native,
            bounds,
            _window: env.new_global_ref(window)?,
        }))))
    }
    /// Moves/resizes the native layer without changing its animation state.
    pub fn place(&self, bounds: Bounds) -> Result<()> {
        let bounds = bounds.validate()?;
        let mut inner = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("GPU target poisoned"))?;
        inner.native.place(bounds)?;
        inner.bounds = bounds;
        Ok(())
    }
    /// Hides the layer without allocating or presenting a frame.
    pub fn set_visible(&self, visible: bool) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| anyhow::anyhow!("GPU target poisoned"))?
            .native
            .set_visible(visible)
    }
    /// Current logical bounds and density.
    pub fn bounds(&self) -> Bounds {
        self.0.lock().expect("GPU target").bounds
    }
    fn commit(&self) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| anyhow::anyhow!("GPU target poisoned"))?
            .native
            .commit()
    }
}

/// One native swap chain. Construct and use it on the rendering worker.
/// Field order keeps the native target alive until the wgpu surface is released.
pub struct Presentation {
    surface: wgpu::Surface<'static>,
    instance: wgpu::Instance,
    target: Target,
    pub adapter: wgpu::Adapter,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    config: wgpu::SurfaceConfiguration,
}
impl Presentation {
    /// Clears a GPU frame for native presentation validation. This does not map
    /// a buffer or copy an image through JNI.
    #[cfg(feature = "probe")]
    pub fn clear(&mut self, [r, g, b, a]: [f64; 4]) -> Result<()> {
        self.target.set_visible(true)?;
        let color = wgpu::Color { r, g, b, a };
        let frame = self.acquire()?.context("Probe surface is occluded")?;
        let view = frame.texture.create_view(&Default::default());
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU presentation probe"),
            source: wgpu::ShaderSource::Wgsl(format!(
                "@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {{ let p=array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0)); return vec4(p[i],0.0,1.0); }} @fragment fn fs()->@location(0) vec4<f32> {{ return vec4<f32>({r},{g},{b},{a}); }}"
            ).into()),
        });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("GPU presentation probe"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.config.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(2)),
        })?;
        self.commit()?;
        Ok(())
    }
    /// Chooses a native GPU backend with premultiplied transparency.
    pub fn new(target: Target) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = {
            let mut inner = target
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("GPU target poisoned"))?;
            // SAFETY: Target retains all native handles and the AWT window for
            // the complete lifetime of the surface. Native mutations use its lock.
            unsafe { instance.create_surface_unsafe(inner.native.surface_target()?)? }
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .context("Native surface lacks an 8-bit linear color format")?;
        let alpha_mode = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
        ]
        .into_iter()
        .find(|mode| caps.alpha_modes.contains(mode))
        // wgpu's Metal backend labels CAMetalLayer's nonopaque mode as
        // PostMultiplied. Core Animation still consumes premultiplied pixels.
        .or_else(|| {
            (adapter.get_info().backend == wgpu::Backend::Metal
                && caps
                    .alpha_modes
                    .contains(&wgpu::CompositeAlphaMode::PostMultiplied))
            .then_some(wgpu::CompositeAlphaMode::PostMultiplied)
        })
        .context("Native surface lacks premultiplied transparency")?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("Cranpose editor decorations"),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            }))?;
        let [width, height] = target.bounds().pixels();
        ensure!(
            width <= device.limits().max_texture_dimension_2d
                && height <= device.limits().max_texture_dimension_2d,
            "GPU layer exceeds the device texture limit"
        );
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Srgb,
        };
        surface.configure(&device, &config);
        target.commit()?;
        Ok(Self {
            surface,
            instance,
            target,
            adapter,
            device: Arc::new(device),
            queue: Arc::new(queue),
            config,
        })
    }
    /// Native swap-chain color format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
    /// Obtains the next GPU texture, reconfiguring after resize or surface loss.
    pub fn acquire(&mut self) -> Result<Option<wgpu::SurfaceTexture>> {
        let [w, h] = self.target.bounds().pixels();
        ensure!(
            w <= self.device.limits().max_texture_dimension_2d
                && h <= self.device.limits().max_texture_dimension_2d,
            "GPU layer exceeds the device texture limit"
        );
        if [self.config.width, self.config.height] != [w, h] {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
            self.target.commit()?;
        }
        for _ in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => return Ok(Some(frame)),
                wgpu::CurrentSurfaceTexture::Lost => {
                    let mut inner = self
                        .target
                        .0
                        .lock()
                        .map_err(|_| anyhow::anyhow!("GPU target poisoned"))?;
                    // SAFETY: the retained target outlives both swap chains.
                    self.surface = unsafe {
                        self.instance
                            .create_surface_unsafe(inner.native.surface_target()?)?
                    };
                    self.surface.configure(&self.device, &self.config);
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    self.surface.configure(&self.device, &self.config);
                    self.target.commit()?;
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(None);
                }
                other => anyhow::bail!("GPU surface unavailable: {other:?}"),
            }
        }
        Ok(None)
    }

    /// Commits any native compositor changes after submitting/presenting a frame.
    pub fn commit(&self) -> Result<()> {
        self.target.commit()
    }
    /// Independent Core Animation reference for color-managed compositor tests.
    #[cfg(all(feature = "probe", target_os = "macos"))]
    pub fn reference(&self, color: Option<[f64; 4]>) -> Result<()> {
        self.target
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("GPU target poisoned"))?
            .native
            .reference(color)
    }
}

// Load JAWT from the running JVM, rather than another installed JDK. Keep the
// library loaded until every API call and drawing-surface guard has completed.
fn with_awt<T>(
    env: &mut JNIEnv<'_>,
    window: &JObject<'_>,
    read: impl FnOnce(&jawt::DrawingSurfacePlatformInfo) -> Result<T>,
) -> Result<T> {
    let key = env.new_string("java.home")?;
    let home = env
        .call_static_method(
            "java/lang/System",
            "getProperty",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&key)],
        )?
        .l()?;
    let home = PathBuf::from(String::from(env.get_string(&JString::from(home))?));
    let library = home.join(if cfg!(windows) {
        "bin/jawt.dll"
    } else if cfg!(target_os = "macos") {
        "lib/libjawt.dylib"
    } else {
        "lib/libjawt.so"
    });
    // SAFETY: This is the current JVM's documented JAWT ABI. The zeroed struct
    // contains only nullable function pointers; JAWT_GetAWT fills it on success.
    unsafe {
        let library = libloading::Library::new(library)?;
        let get: libloading::Symbol<
            unsafe extern "system" fn(
                *mut jni::sys::JNIEnv,
                *mut jawt::sys::JAWT,
            ) -> jni::sys::jboolean,
        > = library.get(b"JAWT_GetAWT\0")?;
        let mut raw: jawt::sys::JAWT = std::mem::zeroed();
        raw.version = jawt::sys::JAWT_VERSION_9;
        ensure!(
            get(env.get_native_interface(), &mut raw) != 0,
            "JAWT 9 unavailable"
        );
        let awt = jawt::Awt::from_inner(raw);
        let local = env.new_local_ref(window)?;
        let mut drawing = awt
            .drawing_surface(env, local)
            .context("AWT drawing surface unavailable")?;
        let (_, mut guard) = drawing.lock().context("AWT drawing surface lock failed")?;
        let info = guard
            .drawing_surface_info()
            .context("AWT drawing surface information unavailable")?;
        read(info.platform_info())
    }
}
