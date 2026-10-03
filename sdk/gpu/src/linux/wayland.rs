use super::*;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_compositor, wl_region, wl_registry, wl_subcompositor, wl_subsurface, wl_surface,
    },
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
struct State;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_subsurface::WlSubsurface);
delegate_noop!(State: ignore wl_region::WlRegion);
delegate_noop!(State: ignore wp_viewporter::WpViewporter);
delegate_noop!(State: ignore wp_viewport::WpViewport);
pub struct Layer {
    connection: Connection,
    queue: EventQueue<State>,
    parent: wl_surface::WlSurface,
    parent_pointer: i64,
    viewporter: Option<wp_viewporter::WpViewporter>,
    viewport: Option<wp_viewport::WpViewport>,
    child: wl_surface::WlSurface,
    sub: wl_subsurface::WlSubsurface,
    _compositor: wl_compositor::WlCompositor,
    subcompositor: wl_subcompositor::WlSubcompositor,
    peer: GlobalRef,
    vm: jni::JavaVM,
    visible: bool,
}
impl Layer {
    pub fn new(env: &mut JNIEnv<'_>, window: &JObject<'_>) -> Result<Self> {
        // JNI checks every lookup and call. These JBR accessors are optional:
        // an incompatible runtime returns an error instead of guessing pointers.
        let peer = env
            .get_field(window, "peer", "Ljava/awt/peer/ComponentPeer;")?
            .l()?;
        let surface = env
            .call_method(&peer, "getSurface", "()Lsun/awt/wl/WLMainSurface;", &[])?
            .l()?;
        let parent = env
            .call_method(surface, "getWlSurfacePtr", "()J", &[])?
            .j()?;
        let display = env
            .call_static_method(
                "sun/awt/wl/WLDisplay",
                "getInstance",
                "()Lsun/awt/wl/WLDisplay;",
                &[],
            )?
            .l()?;
        let display = env.call_method(display, "getDisplayPtr", "()J", &[])?.j()?;
        ensure!(
            parent != 0 && display != 0,
            "JBR Wayland peer is not realized"
        );
        let connection = unsafe {
            // SAFETY: JBR owns both pointers. This backend creates a private
            // event queue and never disconnects the borrowed display.
            Connection::from_backend(Backend::from_foreign_display(display as *mut _))
        };
        let parent_pointer = parent;
        let parent = unsafe {
            wl_surface::WlSurface::from_id(
                &connection,
                ObjectId::from_ptr(wl_surface::WlSurface::interface(), parent as *mut _)?,
            )?
        };
        let (globals, queue) = registry_queue_init::<State>(&connection)?;
        let handle = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&handle, 3..=4, ())?;
        let subcompositor: wl_subcompositor::WlSubcompositor = globals.bind(&handle, 1..=1, ())?;
        let child = compositor.create_surface(&handle, ());
        let viewporter: Option<wp_viewporter::WpViewporter> = globals.bind(&handle, 1..=1, ()).ok();
        let viewport = viewporter
            .as_ref()
            .map(|v| v.get_viewport(&child, &handle, ()));
        let sub = subcompositor.get_subsurface(&child, &parent, &handle, ());
        sub.set_desync();
        let empty = compositor.create_region(&handle, ());
        child.set_input_region(Some(&empty));
        empty.destroy();
        connection.flush()?;
        Ok(Self {
            connection,
            queue,
            parent,
            parent_pointer,
            viewporter,
            viewport,
            child,
            sub,
            _compositor: compositor,
            subcompositor,
            peer: env.new_global_ref(peer)?,
            vm: env.get_java_vm()?,
            visible: false,
        })
    }
    pub fn surface_target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
        Ok(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                std::ptr::NonNull::new(self.connection.backend().display_ptr().cast())
                    .context("Wayland display disappeared")?,
            ))),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(
                std::ptr::NonNull::new(self.child.id().as_ptr().cast())
                    .context("Wayland surface disappeared")?,
            )),
        })
    }
    pub fn place(&mut self, b: Bounds) -> Result<()> {
        // The parent uses JBR's surface units, which can differ from both AWT
        // logical coordinates and device pixels at fractional scale.
        let mut env = self.vm.attach_current_thread()?;
        let x = env
            .call_method(
                self.peer.as_obj(),
                "javaUnitsToSurfaceUnits",
                "(I)I",
                &[JValue::Int(b.x.round() as i32)],
            )?
            .i()?;
        let y = env
            .call_method(
                self.peer.as_obj(),
                "javaUnitsToSurfaceUnits",
                "(I)I",
                &[JValue::Int(b.y.round() as i32)],
            )?
            .i()?;
        let surface = env
            .call_method(
                self.peer.as_obj(),
                "getSurface",
                "()Lsun/awt/wl/WLMainSurface;",
                &[],
            )?
            .l()?;
        let pointer = env
            .call_method(surface, "getWlSurfacePtr", "()J", &[])?
            .j()?;
        ensure!(
            pointer == self.parent_pointer,
            "AWT Wayland peer was replaced or disposed"
        );
        self.sub.set_position(x, y);
        let mut dimensions = [0; 2];
        for (out, value) in dimensions.iter_mut().zip([b.width, b.height]) {
            *out = env
                .call_method(
                    self.peer.as_obj(),
                    "javaUnitsToSurfaceUnits",
                    "(I)I",
                    &[JValue::Int(value as i32)],
                )?
                .i()?
                .max(1);
        }
        if let Some(viewport) = &self.viewport {
            self.child.set_buffer_scale(1);
            viewport.set_destination(dimensions[0], dimensions[1]);
        } else {
            let [w, h] = b.pixels();
            let scale = (f64::from(w) / f64::from(dimensions[0])).round() as i32;
            ensure!(
                scale > 0
                    && dimensions[0] as u32 * scale as u32 == w
                    && dimensions[1] as u32 * scale as u32 == h,
                "Wayland compositor lacks fractional viewport support"
            );
            self.child.set_buffer_scale(scale);
        }
        self.parent.commit();
        drop(env);
        self.commit()
    }
    pub fn set_visible(&mut self, visible: bool) -> Result<()> {
        self.visible = visible;
        if !visible {
            self.child.attach(None, 0, 0);
            self.child.commit();
        }
        self.commit()
    }
    pub fn commit(&mut self) -> Result<()> {
        self.queue.dispatch_pending(&mut State)?;
        self.connection.flush()?;
        Ok(())
    }
}
impl Drop for Layer {
    fn drop(&mut self) {
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        if let Some(viewporter) = &self.viewporter {
            viewporter.destroy();
        }
        self.sub.destroy();
        self.child.destroy();
        self.subcompositor.destroy();
        let _ = self.connection.flush();
    }
}
