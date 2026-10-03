//! One bounded mailbox and one sleeping render worker per native window.
//! Geometry changes preserve animation clocks. No CPU frame image is allocated.
use anyhow::{Context, Result};
use cranpose::{composable, rememberMutableStateOf};
use cranpose_app_shell::{AppShell, default_root_key};
use cranpose_core::MutableState;
pub use cranpose_plugin_authoring_ui::decorations::{CounterFrame, Frame, LightningFrame};
use cranpose_plugin_gpu::Presentation;
pub use cranpose_plugin_gpu::{Bounds, Target};
use cranpose_render_wgpu::{WgpuRenderer, WgpuTextSystem};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Pending {
    frame: Option<(Bounds, Frame)>,
    bounds: Option<Bounds>,
    generation: u64,
    stopped: bool,
    wake: bool,
    error: Option<String>,
    presented: u64,
}
struct Mailbox {
    notify: Arc<dyn Fn() + Send + Sync>,
    pending: Mutex<Pending>,
    changed: Condvar,
}

/// Host-facing handle. Dropping it stops the worker without blocking the EDT.
pub struct Controller {
    mailbox: Arc<Mailbox>,
}
impl Controller {
    /// Starts GPU initialization on a dedicated worker. The target starts hidden.
    pub fn new(target: Target) -> Result<Self> {
        Self::with_waker(target, || {})
    }
    /// Notifies the host after the first presentation or an asynchronous failure.
    pub fn with_waker(target: Target, notify: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let mailbox = Arc::new(Mailbox {
            pending: Mutex::default(),
            changed: Condvar::new(),
            notify: Arc::new(notify),
        });
        mailbox.pending.lock().expect("GPU mailbox").bounds = Some(target.bounds());
        let worker_mailbox = mailbox.clone();
        let worker_target = target.clone();
        thread::Builder::new()
            .name("Cranpose editor GPU".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(worker_target.clone(), &worker_mailbox)
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("GPU rendering worker panicked")));
                let _ = worker_target.set_visible(false);
                if let Err(error) = result {
                    worker_mailbox.pending.lock().expect("GPU mailbox").error =
                        Some(format!("{error:#}"));
                    (worker_mailbox.notify)();
                }
            })?;
        Ok(Self { mailbox })
    }
    /// Replaces the pending scene. Slow GPU frames never accumulate a queue of
    /// old counts or scrolling positions.
    pub fn update(&self, bounds: Bounds, frame: Frame) -> Result<()> {
        let mut pending = self.mailbox.pending.lock().expect("GPU mailbox");
        if let Some(error) = &pending.error {
            anyhow::bail!("{error}");
        }
        pending.bounds = Some(bounds);
        pending.frame = Some((bounds, frame));
        pending.generation += 1;
        self.mailbox.changed.notify_one();
        Ok(())
    }
    /// Clears the next scene and lets the worker go idle without blocking the EDT.
    pub fn hide(&self) -> Result<()> {
        let mut pending = self.mailbox.pending.lock().expect("GPU mailbox");
        let bounds = pending.bounds.context("GPU bounds unavailable")?;
        pending.frame = Some((bounds, Frame::default()));
        pending.generation += 1;
        self.mailbox.changed.notify_one();
        Ok(())
    }
    /// Surfaces asynchronous device/renderer failures to the host.
    pub fn error(&self) -> Option<String> {
        self.mailbox
            .pending
            .lock()
            .expect("GPU mailbox")
            .error
            .clone()
    }
    /// Number of frames actually submitted; useful for verifying idle behavior.
    pub fn frames_presented(&self) -> u64 {
        self.mailbox.pending.lock().expect("GPU mailbox").presented
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        let mut pending = self.mailbox.pending.lock().expect("GPU mailbox");
        pending.stopped = true;
        self.mailbox.changed.notify_one();
    }
}

type Slot = Rc<RefCell<Option<MutableState<Frame>>>>;
#[composable]
fn Root(slot: Slot) {
    let state = rememberMutableStateOf(Frame::default);
    *slot.borrow_mut() = Some(state);
    cranpose_plugin_authoring_ui::decorations::Decorations(state.get());
}

#[derive(Default)]
struct Animation {
    counters: BTreeMap<u64, (String, Instant)>,
    bolt: Option<((u64, bool), Instant)>,
}
impl Animation {
    fn accept(&mut self, frame: &Frame, now: Instant) {
        self.counters
            .retain(|id, _| frame.counters.iter().any(|c| c.id == *id));
        for counter in &frame.counters {
            let entry = self
                .counters
                .entry(counter.id)
                .or_insert_with(|| (counter.label.clone(), now - Duration::from_secs(1)));
            if entry.0 != counter.label {
                *entry = (counter.label.clone(), now);
            }
        }
        match &frame.lightning {
            Some(bolt) => {
                let key = (bolt.request, bolt.pending);
                if self.bolt.as_ref().is_none_or(|(old, _)| *old != key) {
                    self.bolt = Some((key, now));
                }
            }
            None => self.bolt = None,
        }
    }
    fn frame(&self, scene: &Frame, now: Instant) -> (Frame, bool) {
        let mut frame = scene.clone();
        let mut active = false;
        for counter in &mut frame.counters {
            counter.progress = self.counters.get(&counter.id).map_or(1.0, |(_, at)| {
                (now.duration_since(*at).as_secs_f32() / 0.7).min(1.0)
            });
            active |= counter.progress < 1.0;
        }
        if let (Some(bolt), Some((_, at))) = (&mut frame.lightning, self.bolt) {
            bolt.progress = (now.duration_since(at).as_secs_f32()
                / if bolt.pending { 10.0 } else { 0.85 })
            .min(1.0);
            if bolt.progress >= 1.0 {
                frame.lightning = None;
            } else {
                active = true;
            }
        }
        (frame, active)
    }
}

fn run(target: Target, mailbox: &Arc<Mailbox>) -> Result<()> {
    let mut presentation = Presentation::new(target.clone())?;
    let mut renderer = WgpuRenderer::with_text_system(WgpuTextSystem::from_fonts(&[]));
    renderer.set_transparent_background(true);
    let mut bounds = target.bounds();
    renderer.set_root_scale(bounds.scale as f32);
    renderer.init_gpu(
        presentation.device.clone(),
        presentation.queue.clone(),
        presentation.format(),
        presentation.adapter.get_info().backend,
        presentation.adapter.get_downlevel_capabilities().flags,
    );
    let slot = Rc::new(RefCell::new(None));
    let root_slot = slot.clone();
    let [w, h] = bounds.pixels();
    let mut shell = AppShell::new_with_size_and_density(
        renderer,
        default_root_key(),
        move || Root(root_slot.clone()),
        (w, h),
        (bounds.width as f32, bounds.height as f32),
        bounds.scale as f32,
    );
    let waker = Arc::downgrade(mailbox);
    shell.set_frame_waker(move || {
        if let Some(mailbox) = waker.upgrade() {
            mailbox.pending.lock().expect("GPU mailbox").wake = true;
            mailbox.changed.notify_one();
        }
    });
    shell.update();
    let state = slot
        .borrow()
        .context("Decoration composition did not initialize")?;
    let mut scene = Frame::default();
    let mut animation = Animation::default();
    let mut active = false;
    let mut next_frame = Instant::now();
    loop {
        let mut pending = mailbox.pending.lock().expect("GPU mailbox");
        while !pending.stopped {
            if pending.frame.is_none() && !pending.wake && !active {
                pending = mailbox.changed.wait(pending).expect("GPU mailbox");
            } else if let Some(delay) = next_frame.checked_duration_since(Instant::now()) {
                pending = mailbox
                    .changed
                    .wait_timeout(pending, delay)
                    .expect("GPU mailbox")
                    .0;
            } else {
                break;
            }
        }
        if pending.stopped {
            break;
        }
        next_frame = Instant::now() + Duration::from_nanos(16_666_667);
        pending.wake = false;
        let generation = pending.generation;
        let mut next_bounds = bounds;
        if let Some((updated_bounds, frame)) = pending.frame.take() {
            next_bounds = updated_bounds;
            animation.accept(&frame, Instant::now());
            scene = frame;
        }
        drop(pending);
        if bounds != next_bounds {
            bounds = next_bounds;
            target.set_visible(false)?;
            target.place(bounds)?;
            let [w, h] = bounds.pixels();
            shell.set_density(bounds.scale as f32);
            shell.renderer().set_root_scale(bounds.scale as f32);
            shell.set_buffer_size(w, h);
            shell.set_viewport(bounds.width as f32, bounds.height as f32);
        }
        let (frame, animating) = animation.frame(&scene, Instant::now());
        active = animating;
        if frame.counters.is_empty() && frame.lightning.is_none() {
            target.set_visible(false)?;
            continue;
        }
        // Discard a scene superseded while the last GPU frame was rendering.
        {
            let pending = mailbox.pending.lock().expect("GPU mailbox");
            if pending.stopped || pending.generation != generation {
                continue;
            }
        }
        state.set(frame);
        shell.update();
        #[cfg(target_os = "linux")]
        target.set_visible(true)?;
        let Some(texture) = presentation.acquire()? else {
            // Geometry/visibility events wake occluded idle surfaces. A finite
            // animation may retry, but an occluded static counter must sleep.
            continue;
        };
        let view = texture.texture.create_view(&Default::default());
        let [w, h] = bounds.pixels();
        shell
            .renderer()
            .render_surface_texture(&texture.texture, &view, w, h)
            .map_err(|error| anyhow::anyhow!("Cranpose GPU render: {error:?}"))?;
        shell.renderer().present(texture);
        presentation.commit()?;
        target.set_visible(true)?;
        let first = {
            let mut pending = mailbox.pending.lock().expect("GPU mailbox");
            pending.presented += 1;
            pending.presented == 1
        };
        if first {
            (mailbox.notify)();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrolling_preserves_clocks_and_both_effects_stop() {
        let now = Instant::now();
        let mut frame = Frame {
            counters: vec![CounterFrame {
                id: 7,
                bounds: [0.0, 0.0, 200.0, 20.0],
                clip: [0.0, 0.0, 500.0, 300.0],
                label: "7".into(),
                progress: 1.0,
                foreground: [1.0; 4],
            }],
            lightning: Some(LightningFrame {
                from: [10.0, 20.0],
                target: [100.0, 20.0, 30.0, 30.0],
                size: [200.0, 200.0],
                request: 9,
                pending: true,
                progress: 0.0,
            }),
        };
        let mut animation = Animation::default();
        animation.accept(&frame, now);
        frame.counters[0].label = "8".into();
        animation.accept(&frame, now + Duration::from_millis(100));
        let at = now + Duration::from_millis(300);
        let before = animation.frame(&frame, at).0;
        frame.counters[0].bounds[1] += 40.0;
        frame.lightning.as_mut().expect("bolt").from[1] += 40.0;
        animation.accept(&frame, at);
        let after = animation.frame(&frame, at).0;
        assert_eq!(before.counters[0].progress, after.counters[0].progress);
        assert_eq!(
            before.lightning.expect("bolt").progress,
            after.lightning.expect("bolt").progress
        );
        frame.lightning.as_mut().expect("bolt").pending = false;
        animation.accept(&frame, at);
        assert_eq!(
            animation
                .frame(&frame, at)
                .0
                .lightning
                .expect("bolt")
                .progress,
            0.0
        );
        let (settled, active) = animation.frame(&frame, at + Duration::from_secs(1));
        assert!(!active);
        assert!(settled.lightning.is_none());
        assert_eq!(settled.counters[0].progress, 1.0);
        animation.accept(&frame, at + Duration::from_secs(2));
        assert!(
            !animation.frame(&frame, at + Duration::from_secs(2)).1,
            "Geometry must not replay an expired bolt"
        );
    }
}
