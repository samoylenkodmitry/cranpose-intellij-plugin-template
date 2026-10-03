use super::*;
use jawt::macos::SurfaceLayers;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAMetalLayer, CATransaction};

define_class! {
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct CranposeGpuOverlayView;
    unsafe impl NSObjectProtocol for CranposeGpuOverlayView {}
    impl CranposeGpuOverlayView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView { std::ptr::null_mut() }
    }
}
struct View {
    metal: Retained<CAMetalLayer>,
    parent: Retained<NSView>,
    view: Retained<CranposeGpuOverlayView>,
}
// SAFETY: view access and destruction are restricted to AppKit via main_thread.
unsafe impl Send for View {}
pub struct Layer {
    backing: Retained<CALayer>,
    native: Option<View>,
    bounds: Option<Bounds>,
}
// SAFETY: all NSView access is dispatched to AppKit's main queue. Target
// serializes operations and retains the native view until wgpu releases it.
unsafe impl Send for Layer {}
fn main_thread(f: impl FnOnce() + Send) {
    if MainThreadMarker::new().is_some() {
        f();
    } else {
        dispatch2::DispatchQueue::main().exec_sync(f);
    }
}
impl Layer {
    pub fn new(env: &mut JNIEnv<'_>, window: &JObject<'_>) -> Result<Self> {
        let backing = with_awt(env, window, |info| {
            info.window_layer()
                .superlayer()
                .context("AWT window layer is not attached")
        })?;
        // Capture only on the EDT. Creating an AppKit view can ask Java's
        // accessibility subsystem about focus, so the EDT must remain free.
        Ok(Self {
            backing,
            native: None,
            bounds: None,
        })
    }
    pub fn surface_target(&mut self) -> Result<wgpu::SurfaceTargetUnsafe> {
        if self.native.is_none() {
            let backing = Retained::as_ptr(&self.backing) as usize;
            let mut result = None;
            main_thread(|| unsafe {
                let backing = &*(backing as *const CALayer);
                let parent: *mut NSView = msg_send![backing, delegate];
                if parent.is_null() {
                    return;
                }
                let is_view: bool = msg_send![parent, isKindOfClass: NSView::class()];
                if !is_view {
                    return;
                }
                let Some(parent) = Retained::retain(parent) else {
                    return;
                };
                let Some(mtm) = MainThreadMarker::new() else {
                    return;
                };
                let view: Retained<CranposeGpuOverlayView> = msg_send![CranposeGpuOverlayView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(1.0,1.0))];
                let metal = CAMetalLayer::new();
                metal.setOpaque(false);
                metal.setGeometryFlipped(true);
                metal.setContentsGravity(objc2_quartz_core::kCAGravityTopLeft);
                metal.setMasksToBounds(true);
                view.setLayer(Some(&metal));
                view.setWantsLayer(true);
                view.setHidden(true);
                parent.addSubview(&view);
                result = Some(View {
                    metal,
                    parent,
                    view,
                });
            });
            self.native = Some(result.context("AWT backing layer has no native view")?);
            if let Some(bounds) = self.bounds {
                self.place(bounds)?;
            }
        }
        let native = self.native.as_ref().context("Native view unavailable")?;
        Ok(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
            Retained::as_ptr(&native.metal) as *mut _,
        ))
    }
    pub fn place(&mut self, b: Bounds) -> Result<()> {
        self.bounds = Some(b);
        let Some(native) = &self.native else {
            return Ok(());
        };
        let view = Retained::as_ptr(&native.view) as usize;
        let parent = Retained::as_ptr(&native.parent) as usize;
        let metal = Retained::as_ptr(&native.metal) as usize;
        main_thread(move || unsafe {
            let view = &*(view as *const CranposeGpuOverlayView);
            let parent = &*(parent as *const NSView);
            let metal = &*(metal as *const CAMetalLayer);
            let y = if parent.isFlipped() {
                b.y
            } else {
                parent.bounds().size.height - b.y - f64::from(b.height)
            };
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            view.setFrame(NSRect::new(
                NSPoint::new(b.x, y),
                NSSize::new(f64::from(b.width), f64::from(b.height)),
            ));
            metal.setContentsScale(b.scale);
            CATransaction::commit();
        });
        Ok(())
    }
    pub fn set_visible(&mut self, visible: bool) -> Result<()> {
        let Some(native) = &self.native else {
            return Ok(());
        };
        let view = Retained::as_ptr(&native.view) as usize;
        main_thread(move || unsafe {
            (&*(view as *const CranposeGpuOverlayView)).setHidden(!visible);
        });
        Ok(())
    }
    pub fn commit(&mut self) -> Result<()> {
        CATransaction::flush();
        Ok(())
    }
    #[cfg(feature = "probe")]
    pub fn reference(&mut self, color: Option<[f64; 4]>) -> Result<()> {
        let native = self.native.as_ref().context("Native view unavailable")?;
        let color = color.map(|[r, g, b, a]| objc2_core_graphics::CGColor::new_srgb(r, g, b, a));
        native.metal.setBackgroundColor(color.as_deref());
        CATransaction::flush();
        Ok(())
    }
}
impl Drop for Layer {
    fn drop(&mut self) {
        if let Some(native) = self.native.take() {
            main_thread(move || {
                native.view.removeFromSuperview();
                drop(native);
            });
        }
    }
}
