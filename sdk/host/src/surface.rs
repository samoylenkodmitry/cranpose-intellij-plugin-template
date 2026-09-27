//! Cranpose pixels and input on IntelliJ-owned AWT surfaces.
use crate::{
    jvm::{self, A, J, O, Scope},
    protocol::{Event, Frame, Packet, Window},
    session::{Options, Session, SessionEvent},
};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};
pub type Message = dyn for<'a> Fn(&mut J<'a>, &Arc<Panel>, &str, &str) -> Result<()> + Send + Sync;
pub type Lifecycle =
    dyn for<'a> Fn(&mut J<'a>, &Arc<Panel>, bool, &str) -> Result<()> + Send + Sync;
pub type Pointer =
    dyn for<'a> Fn(&mut J<'a>, f64, f64, bool, bool, f64) -> Result<bool> + Send + Sync;
pub type Presented = dyn for<'a> Fn(&mut J<'a>, &Arc<Panel>, u32) -> Result<()> + Send + Sync;
pub struct Panel {
    pub primary: Arc<Surface>,
    scope: Scope,
    timer: OnceLock<O>,
    state: Mutex<PanelState>,
    pub on_message: Mutex<Option<Arc<Message>>>,
    pub on_lifecycle: Mutex<Option<Arc<Lifecycle>>>,
    pub on_pointer: Mutex<Option<Arc<Pointer>>>,
    pub on_presented: Mutex<Option<Arc<Presented>>>,
}
struct PanelState {
    session: Option<Session>,
    options: Options,
    watcher: Option<crate::watcher::Watcher>,
    children: HashMap<u32, Arc<Surface>>,
    closed: bool,
    connected: bool,
    dark: bool,
}
pub struct Surface {
    pub id: u32,
    pub view: OnceLock<O>,
    panel: Weak<Panel>,
    scope: Scope,
    state: Mutex<SurfaceState>,
}
#[derive(Clone)]
struct Image {
    object: O,
    pixels: O,
    width: u32,
    height: u32,
}
struct SurfaceState {
    image: Option<Image>,
    window: Option<O>,
    overlay: bool,
    transparent: bool,
    content_scale: f64,
    screen_scale: f64,
    pressed: bool,
    press_screen: (i32, i32),
    gesture: Option<Gesture>,
    high_surrogate: Option<u16>,
    status: String,
    selection: Option<[f64; 4]>,
    relative: bool,
}
#[derive(Clone)]
struct Gesture {
    edge: Option<u8>,
    bounds: [i32; 4],
    from: (i32, i32),
    travelled: bool,
}

impl Panel {
    pub fn new(j: &mut J<'_>, options: Options) -> Result<Arc<Self>> {
        let panel = Arc::new_cyclic(|weak| Self {
            primary: Arc::new(Surface::uninitialized(0, weak.clone(), false)),
            scope: Scope::default(),
            timer: OnceLock::new(),
            state: Mutex::new(PanelState {
                session: None,
                watcher: (options.command.len() == 1)
                    .then(|| crate::watcher::Watcher::new(options.command[0].clone().into())),
                options,
                children: HashMap::new(),
                closed: false,
                connected: false,
                dark: false,
            }),
            on_message: Mutex::new(None),
            on_lifecycle: Mutex::new(None),
            on_pointer: Mutex::new(None),
            on_presented: Mutex::new(None),
        });
        panel.primary.initialize(j)?;
        let weak = Arc::downgrade(&panel);
        let id = panel.scope.register(move |j, _, _| {
            if let Some(panel) = weak.upgrade() {
                panel.tick(j)?;
            }
            j.null()
        });
        let listener = jvm::callback(j, id)?;
        let timer = j.new(
            "javax/swing/Timer",
            "(ILjava/awt/event/ActionListener;)V",
            &[A::I(16), A::O(&listener)],
        )?;
        j.void(&timer, "setCoalesce", "(Z)V", &[A::Z(true)])?;
        panel.timer.set(timer).ok();
        Ok(panel)
    }
    pub fn start(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        let options = {
            let state = self.state.lock().expect("panel");
            if state.closed || state.session.is_some() {
                return Ok(());
            }
            state.options.clone()
        };
        let session = Session::start(options)?;
        self.state.lock().expect("panel").session = Some(session);
        j.void(self.timer.get().context("timer")?, "start", "()V", &[])?;
        Ok(())
    }
    pub fn send(&self, packet: Packet) {
        if let Some(session) = &self.state.lock().expect("panel").session {
            session.send(packet);
        }
    }
    pub fn message(&self, channel: &str, payload: &str) {
        self.send(Packet::message(channel, payload));
    }
    #[cfg(feature = "ide-tests")]
    pub fn connected(&self) -> bool {
        self.state.lock().expect("panel").connected
    }
    pub fn set_theme(&self, dark: bool) {
        let changed = {
            let mut state = self.state.lock().expect("panel");
            let changed = state.dark != dark;
            state.dark = dark;
            changed
        };
        if changed {
            self.send(Packet::new(9).byte(u8::from(dark)));
        }
    }
    pub fn close(&self, j: &mut J<'_>) -> Result<()> {
        let children = {
            let mut state = self.state.lock().expect("panel");
            if state.closed {
                return Ok(());
            }
            state.closed = true;
            state.watcher.take();
            state.session.take();
            state.children.drain().map(|(_, v)| v).collect::<Vec<_>>()
        };
        if let Some(timer) = self.timer.get() {
            j.void(timer, "stop", "()V", &[])?;
        }
        for child in children {
            child.close(j)?;
        }
        self.primary.scope.clear();
        self.scope.clear();
        Ok(())
    }
    pub(crate) fn restart(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        let children = {
            let mut state = self.state.lock().expect("panel");
            state.session.take();
            state.connected = false;
            state.children.drain().map(|(_, v)| v).collect::<Vec<_>>()
        };
        for child in children {
            child.close(j)?;
        }
        let callback = self.on_lifecycle.lock().expect("callback").clone();
        if let Some(callback) = callback {
            callback(j, self, false, "Rebuilding Cranpose UI")?;
        }
        self.start(j)?;
        Ok(())
    }
    pub fn tick(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        let restart = {
            let state = self.state.lock().expect("panel");
            !state.closed && state.watcher.as_ref().is_some_and(|w| w.changed())
        };
        if restart {
            self.restart(j)?;
        }
        let events = {
            let state = self.state.lock().expect("panel");
            state
                .session
                .as_ref()
                .map(|s| s.events.try_iter().take(32).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        for event in events {
            match event {
                SessionEvent::Connected => {
                    let dark = {
                        let mut state = self.state.lock().expect("panel");
                        state.connected = true;
                        state.dark
                    };
                    self.send(Packet::new(9).byte(u8::from(dark)));
                    self.primary.size(j)?;
                    self.primary.visibility(j)?;
                    let callback = self.on_lifecycle.lock().expect("callback").clone();
                    if let Some(callback) = callback {
                        callback(j, self, true, "")?;
                    }
                }
                SessionEvent::Data(Event::Frame(frame)) => {
                    let view = self.surface(frame.surface);
                    if let Some(view) = view {
                        view.frame(j, &frame)?;
                    }
                    self.send(Packet::new(11).int(frame.surface).int(frame.id));
                    let callback = self.on_presented.lock().expect("callback").clone();
                    if let Some(callback) = callback {
                        callback(j, self, frame.surface)?;
                    }
                }
                SessionEvent::Data(Event::Message(channel, payload)) => {
                    let callback = self.on_message.lock().expect("callback").clone();
                    if let Some(callback) = callback {
                        callback(j, self, &channel, &payload)?;
                    }
                }
                SessionEvent::Data(Event::Cursor(id, name)) => {
                    if let Some(surface) = self.surface(id) {
                        surface.cursor(j, &name)?;
                    }
                }
                SessionEvent::Data(Event::Window(spec)) => self.window(j, spec)?,
                SessionEvent::Data(Event::Overlay(id, anchor)) => self.overlay(j, id, &anchor)?,
                SessionEvent::Data(Event::Close(id)) => {
                    let child = self.state.lock().expect("panel").children.remove(&id);
                    if let Some(child) = child {
                        child.close(j)?;
                    }
                }
                SessionEvent::Data(Event::Move(id)) => {
                    if let Some(surface) = self.surface(id) {
                        surface.begin(j, None)?;
                    }
                }
                SessionEvent::Data(Event::Resize(id, edge)) => {
                    if let Some(surface) = self.surface(id) {
                        surface.begin(j, Some(edge))?;
                    }
                }
                SessionEvent::Log(line) => {
                    let logger = j.static_obj(
                        "com/intellij/openapi/diagnostic/Logger",
                        "getInstance",
                        "(Ljava/lang/String;)Lcom/intellij/openapi/diagnostic/Logger;",
                        &[A::S("dev.cranpose.native")],
                    )?;
                    j.void(&logger, "info", "(Ljava/lang/String;)V", &[A::S(&line)])?;
                    let callback = self.on_message.lock().expect("callback").clone();
                    if let Some(callback) = callback {
                        callback(j, self, "host.log", &line)?;
                    }
                }
                SessionEvent::Stopped(message) => {
                    let children = {
                        let mut state = self.state.lock().expect("panel");
                        state.connected = false;
                        state.session.take();
                        state
                            .children
                            .drain()
                            .map(|(_, surface)| surface)
                            .collect::<Vec<_>>()
                    };
                    for surface in children {
                        surface.close(j)?;
                    }
                    self.primary.state.lock().expect("surface").status = message.clone();
                    self.primary.repaint(j)?;
                    let callback = self.on_lifecycle.lock().expect("callback").clone();
                    if let Some(callback) = callback {
                        callback(j, self, false, &message)?;
                    }
                    if self.state.lock().expect("panel").watcher.is_none()
                        && let Some(timer) = self.timer.get()
                    {
                        j.void(timer, "stop", "()V", &[])?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn surface(&self, id: u32) -> Option<Arc<Surface>> {
        if id == 0 {
            Some(self.primary.clone())
        } else {
            self.state.lock().expect("panel").children.get(&id).cloned()
        }
    }
    fn window(self: &Arc<Self>, j: &mut J<'_>, spec: Window) -> Result<()> {
        if spec.surface == 0 {
            return Ok(());
        }
        // Recreate native window when its declaration changes, retaining its surface pixels.
        let old = self
            .state
            .lock()
            .expect("panel")
            .children
            .remove(&spec.surface);
        if let Some(old) = old {
            old.close(j)?;
        }
        let surface = Arc::new(Surface::uninitialized(
            spec.surface,
            Arc::downgrade(self),
            false,
        ));
        surface.initialize(j)?;
        let owner = j.static_obj(
            "javax/swing/SwingUtilities",
            "getWindowAncestor",
            "(Ljava/awt/Component;)Ljava/awt/Window;",
            &[A::O(self.primary.component())],
        )?;
        let decorated = spec.flags & 1 != 0;
        let window = if decorated {
            j.new(
                "javax/swing/JDialog",
                "(Ljava/awt/Window;Ljava/lang/String;)V",
                &[A::O(&owner), A::S(&spec.title)],
            )?
        } else {
            j.new(
                "javax/swing/JWindow",
                "(Ljava/awt/Window;)V",
                &[A::O(&owner)],
            )?
        };
        if decorated {
            j.void(
                &window,
                "setResizable",
                "(Z)V",
                &[A::Z(spec.flags & 4 != 0)],
            )?;
            j.void(&window, "setDefaultCloseOperation", "(I)V", &[A::I(0)])?;
        }
        j.void(
            &window,
            "setContentPane",
            "(Ljava/awt/Container;)V",
            &[A::O(surface.component())],
        )?;
        let transparent = !decorated && spec.flags & 2 != 0;
        if transparent {
            let configuration = j.obj(
                &window,
                "getGraphicsConfiguration",
                "()Ljava/awt/GraphicsConfiguration;",
                &[],
            )?;
            let device = j.obj(
                &configuration,
                "getDevice",
                "()Ljava/awt/GraphicsDevice;",
                &[],
            )?;
            let mode = j.constant(
                "java/awt/GraphicsDevice$WindowTranslucency",
                "PERPIXEL_TRANSLUCENT",
                "Ljava/awt/GraphicsDevice$WindowTranslucency;",
            )?;
            if j.call(
                &device,
                "isWindowTranslucencySupported",
                "(Ljava/awt/GraphicsDevice$WindowTranslucency;)Z",
                &[A::O(&mode)],
            )?
            .z()?
            {
                let color = j.new(
                    "java/awt/Color",
                    "(IIII)V",
                    &[A::I(0), A::I(0), A::I(0), A::I(0)],
                )?;
                j.void(
                    &window,
                    "setBackground",
                    "(Ljava/awt/Color;)V",
                    &[A::O(&color)],
                )?;
                j.void(surface.component(), "setOpaque", "(Z)V", &[A::Z(false)])?;
                surface.state.lock().expect("surface").transparent = true;
            }
        }
        let root = j.obj(&window, "getRootPane", "()Ljavax/swing/JRootPane;", &[])?;
        j.void(&root, "setDoubleBuffered", "(Z)V", &[A::Z(!transparent)])?;
        let shadow = j.boxed_bool(spec.flags & 16 != 0)?;
        j.void(
            &root,
            "putClientProperty",
            "(Ljava/lang/Object;Ljava/lang/Object;)V",
            &[A::S("Window.shadow"), A::O(&shadow)],
        )?;
        j.void(
            &window,
            "setAlwaysOnTop",
            "(Z)V",
            &[A::Z(spec.flags & 8 != 0)],
        )?;
        j.void(
            &window,
            "setFocusableWindowState",
            "(Z)V",
            &[A::Z(spec.flags & 32 != 0)],
        )?;
        let dimension = j.new(
            "java/awt/Dimension",
            "(II)V",
            &[
                A::I(spec.width.max(1.0).round() as i32),
                A::I(spec.height.max(1.0).round() as i32),
            ],
        )?;
        j.void(
            surface.component(),
            "setPreferredSize",
            "(Ljava/awt/Dimension;)V",
            &[A::O(&dimension)],
        )?;
        j.void(&window, "pack", "()V", &[])?;
        if spec.x.is_finite() && spec.y.is_finite() {
            let origin = if spec.relative {
                self.primary.screen_position(j)?
            } else {
                (0, 0)
            };
            j.void(
                &window,
                "setLocation",
                "(II)V",
                &[
                    A::I(origin.0 + spec.x.round() as i32),
                    A::I(origin.1 + spec.y.round() as i32),
                ],
            )?;
        } else {
            j.void(
                &window,
                "setLocationRelativeTo",
                "(Ljava/awt/Component;)V",
                &[A::O(&owner)],
            )?;
        }
        {
            let mut state = surface.state.lock().expect("surface");
            state.window = Some(window.clone());
            state.relative = spec.relative;
        }
        let weak = Arc::downgrade(&surface);
        let id = surface.scope.register(move |j, op, _| {
            if let Some(surface) = weak.upgrade() {
                if op.ends_with(".windowClosing") {
                    surface.send(Packet::new(15).int(surface.id));
                }
                if op.ends_with(".componentMoved") {
                    let mut location = surface.screen_position(j)?;
                    let relative = surface.state.lock().expect("surface").relative;
                    if relative && let Some(panel) = surface.panel.upgrade() {
                        let origin = panel.primary.screen_position(j)?;
                        location.0 -= origin.0;
                        location.1 -= origin.1;
                    }
                    surface.send(
                        Packet::new(14)
                            .int(surface.id)
                            .float(location.0 as f32)
                            .float(location.1 as f32),
                    );
                }
            }
            j.null()
        });
        let listener = jvm::callback(j, id)?;
        j.void(
            &window,
            "addWindowListener",
            "(Ljava/awt/event/WindowListener;)V",
            &[A::O(&listener)],
        )?;
        j.void(
            &window,
            "addComponentListener",
            "(Ljava/awt/event/ComponentListener;)V",
            &[A::O(&listener)],
        )?;
        self.state
            .lock()
            .expect("panel")
            .children
            .insert(spec.surface, surface);
        j.void(&window, "setVisible", "(Z)V", &[A::Z(true)])?;
        Ok(())
    }
    fn overlay(self: &Arc<Self>, j: &mut J<'_>, id: u32, anchor: &str) -> Result<()> {
        if id == 0 {
            return Ok(());
        }
        let surface = Arc::new(Surface::uninitialized(id, Arc::downgrade(self), true));
        surface.initialize(j)?;
        self.state
            .lock()
            .expect("panel")
            .children
            .insert(id, surface);
        let callback = self.on_message.lock().expect("callback").clone();
        if let Some(callback) = callback {
            callback(
                j,
                self,
                "host.overlay",
                &serde_json::json!({"surface":id,"anchor":anchor}).to_string(),
            )?;
        }
        Ok(())
    }
    pub fn overlay_surface(&self, id: u32) -> Option<Arc<Surface>> {
        self.surface(id)
    }
}
impl Surface {
    /// A reused popup must never flash pixels from the preceding value.
    pub(crate) fn clear_frame(&self, j: &mut J<'_>) -> Result<()> {
        self.state.lock().expect("surface").image = None;
        j.void(self.component(), "repaint", "()V", &[])
    }
    #[cfg(feature = "ide-tests")]
    pub(crate) fn test_overlay_paint(j: &mut J<'_>) -> Result<()> {
        let options = crate::project::options(j, false)?;
        let panel = Panel::new(j, options)?;
        panel.overlay(j, 1, "editor")?;
        let surface = panel.overlay_surface(1).context("overlay")?;
        anyhow::ensure!(
            !j.call(
                surface.component(),
                "contains",
                "(II)Z",
                &[A::I(1), A::I(1)]
            )?
            .z()?,
            "Overlay must pass pointer input through to the IDE"
        );
        j.void(surface.component(), "setSize", "(II)V", &[A::I(4), A::I(4)])?;
        surface.frame(
            j,
            &Frame {
                surface: 1,
                id: 1,
                buffer_width: 4,
                buffer_height: 4,
                x: 0,
                y: 0,
                width: 4,
                height: 4,
                pixels: vec![0; 16],
            },
        )?;
        let image = j.new(
            "java/awt/image/BufferedImage",
            "(III)V",
            &[A::I(4), A::I(4), A::I(2)],
        )?;
        j.void(
            &image,
            "setRGB",
            "(III)V",
            &[A::I(1), A::I(1), A::I(0xff123456u32 as i32)],
        )?;
        let graphics = j.obj(&image, "createGraphics", "()Ljava/awt/Graphics2D;", &[])?;
        surface.paint(j, &graphics)?;
        j.void(&graphics, "dispose", "()V", &[])?;
        anyhow::ensure!(
            j.call(&image, "getRGB", "(II)I", &[A::I(1), A::I(1)])?
                .i()?
                == 0xff123456u32 as i32,
            "Transparent editor overlay erased the source beneath it"
        );
        panel.close(j)
    }
    fn uninitialized(id: u32, panel: Weak<Panel>, overlay: bool) -> Self {
        Self {
            id,
            view: OnceLock::new(),
            panel,
            scope: Scope::default(),
            state: Mutex::new(SurfaceState {
                image: None,
                window: None,
                overlay,
                transparent: overlay,
                content_scale: 1.0,
                screen_scale: 1.0,
                pressed: false,
                press_screen: (0, 0),
                gesture: None,
                high_surrogate: None,
                status: "Starting Cranpose…".into(),
                selection: None,
                relative: false,
            }),
        }
    }
    fn initialize(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        let weak = Arc::downgrade(self);
        let id = self.scope.register(move |j, op, args| {
            if let Some(surface) = weak.upgrade() {
                surface.handle(j, op, args)
            } else {
                j.null()
            }
        });
        let view = j.new("dev/cranpose/rust/Surface", "(J)V", &[A::J(id)])?;
        self.view.set(view.clone()).ok();
        let overlay = self.state.lock().expect("surface").overlay;
        j.void(&view, "setOpaque", "(Z)V", &[A::Z(!overlay)])?;
        j.void(&view, "setFocusable", "(Z)V", &[A::Z(!overlay)])?;
        j.void(
            &view,
            "setFocusTraversalKeysEnabled",
            "(Z)V",
            &[A::Z(false)],
        )?;
        let callback = jvm::callback(j, id)?;
        for (method, signature) in [
            ("addMouseListener", "(Ljava/awt/event/MouseListener;)V"),
            (
                "addMouseMotionListener",
                "(Ljava/awt/event/MouseMotionListener;)V",
            ),
            (
                "addMouseWheelListener",
                "(Ljava/awt/event/MouseWheelListener;)V",
            ),
            ("addKeyListener", "(Ljava/awt/event/KeyListener;)V"),
            (
                "addComponentListener",
                "(Ljava/awt/event/ComponentListener;)V",
            ),
            (
                "addHierarchyListener",
                "(Ljava/awt/event/HierarchyListener;)V",
            ),
            (
                "addPropertyChangeListener",
                "(Ljava/beans/PropertyChangeListener;)V",
            ),
        ] {
            j.void(&view, method, signature, &[A::O(&callback)])?;
        }
        Ok(())
    }
    pub fn component(&self) -> &O {
        self.view.get().expect("initialized surface")
    }
    pub fn set_scale(&self, j: &mut J<'_>, scale: f64) -> Result<()> {
        let scale = if scale.is_finite() {
            scale.clamp(0.05, 4.0)
        } else {
            1.0
        };
        let changed = {
            let mut state = self.state.lock().expect("surface");
            let changed = state.content_scale != scale;
            state.content_scale = scale;
            changed
        };
        if changed {
            self.size(j)?;
        }
        Ok(())
    }
    pub fn select(&self, rectangle: Option<[f64; 4]>) {
        self.state.lock().expect("surface").selection = rectangle;
    }
    pub fn repaint(&self, j: &mut J<'_>) -> Result<()> {
        j.void(self.component(), "repaint", "()V", &[])
    }
    pub fn snapshot(&self) -> Option<O> {
        self.state
            .lock()
            .expect("surface")
            .image
            .as_ref()
            .map(|image| image.object.clone())
    }
    fn send(&self, packet: Packet) {
        if let Some(panel) = self.panel.upgrade() {
            panel.send(packet);
        }
    }
    pub fn size(&self, j: &mut J<'_>) -> Result<()> {
        let width = j.int(self.component(), "getWidth")?.max(1);
        let height = j.int(self.component(), "getHeight")?.max(1);
        let graphics = j.obj(
            self.component(),
            "getGraphicsConfiguration",
            "()Ljava/awt/GraphicsConfiguration;",
            &[],
        )?;
        let mut screen = 1.0;
        let mut refresh = 60;
        if !graphics.is_null() {
            let transform = j.obj(
                &graphics,
                "getDefaultTransform",
                "()Ljava/awt/geom/AffineTransform;",
                &[],
            )?;
            screen = j.double(&transform, "getScaleX")?;
            let device = j.obj(&graphics, "getDevice", "()Ljava/awt/GraphicsDevice;", &[])?;
            let mode = j.obj(&device, "getDisplayMode", "()Ljava/awt/DisplayMode;", &[])?;
            refresh = j.int(&mode, "getRefreshRate")?.max(60);
        }
        let content = {
            let mut state = self.state.lock().expect("surface");
            state.screen_scale = screen;
            state.content_scale
        };
        self.send(
            Packet::new(1)
                .int(self.id)
                .int((width as f64 * screen).ceil() as u32)
                .int((height as f64 * screen).ceil() as u32)
                .float((screen * content) as f32)
                .float(refresh as f32),
        );
        Ok(())
    }
    fn visibility(&self, j: &mut J<'_>) -> Result<()> {
        self.send(
            Packet::new(13)
                .int(self.id)
                .byte(u8::from(j.bool(self.component(), "isShowing")?)),
        );
        Ok(())
    }
    pub(crate) fn frame(&self, j: &mut J<'_>, frame: &Frame) -> Result<()> {
        let current = self.state.lock().expect("surface").image.clone();
        let image = if let Some(image) =
            current.filter(|i| i.width == frame.buffer_width && i.height == frame.buffer_height)
        {
            image
        } else {
            let object = j.new(
                "java/awt/image/BufferedImage",
                "(III)V",
                &[
                    A::I(frame.buffer_width as i32),
                    A::I(frame.buffer_height as i32),
                    A::I(3),
                ],
            )?;
            let raster = j.obj(
                &object,
                "getRaster",
                "()Ljava/awt/image/WritableRaster;",
                &[],
            )?;
            let buffer = j.obj(
                &raster,
                "getDataBuffer",
                "()Ljava/awt/image/DataBuffer;",
                &[],
            )?;
            let pixels = j.obj(&buffer, "getData", "()[I", &[])?;
            Image {
                object,
                pixels,
                width: frame.buffer_width,
                height: frame.buffer_height,
            }
        };
        let pixels = jni::objects::JIntArray::from(j.env.new_local_ref(&image.pixels)?);
        for row in 0..frame.height {
            let start = (row * frame.width) as usize;
            j.env.set_int_array_region(
                &pixels,
                ((frame.y + row) * frame.buffer_width + frame.x) as i32,
                &frame.pixels[start..start + frame.width as usize],
            )?;
        }
        j.env.delete_local_ref(pixels)?;
        self.state.lock().expect("surface").image = Some(image);
        self.repaint(j)
    }
    fn paint(&self, j: &mut J<'_>, graphics: &O) -> Result<()> {
        let (image, screen, status, transparent, overlay, selection, content) = {
            let state = self.state.lock().expect("surface");
            (
                state.image.clone(),
                state.screen_scale,
                state.status.clone(),
                state.transparent,
                state.overlay,
                state.selection,
                state.content_scale,
            )
        };
        let g = j.obj(graphics, "create", "()Ljava/awt/Graphics;", &[])?;
        let result = (|| -> Result<()> {
            // Embedded overlays paint over the editor's existing pixels. Clearing
            // this shared graphics target would erase the source underneath.
            if transparent && !overlay {
                let clear = j.constant(
                    "java/awt/AlphaComposite",
                    "Clear",
                    "Ljava/awt/AlphaComposite;",
                )?;
                j.void(
                    &g,
                    "setComposite",
                    "(Ljava/awt/Composite;)V",
                    &[A::O(&clear)],
                )?;
            } else {
                let background = j.static_obj(
                    "com/intellij/util/ui/UIUtil",
                    "getPanelBackground",
                    "()Ljava/awt/Color;",
                    &[],
                )?;
                j.void(&g, "setColor", "(Ljava/awt/Color;)V", &[A::O(&background)])?;
            }
            let width = j.int(self.component(), "getWidth")?;
            let height = j.int(self.component(), "getHeight")?;
            if !overlay {
                j.void(
                    &g,
                    "fillRect",
                    "(IIII)V",
                    &[A::I(0), A::I(0), A::I(width), A::I(height)],
                )?;
            }
            if transparent {
                let src = j.constant(
                    "java/awt/AlphaComposite",
                    "SrcOver",
                    "Ljava/awt/AlphaComposite;",
                )?;
                j.void(&g, "setComposite", "(Ljava/awt/Composite;)V", &[A::O(&src)])?;
            }
            if let Some(image) = image {
                let transform = j.static_obj(
                    "java/awt/geom/AffineTransform",
                    "getScaleInstance",
                    "(DD)Ljava/awt/geom/AffineTransform;",
                    &[A::D(1.0 / screen), A::D(1.0 / screen)],
                )?;
                j.call(&g,"drawImage","(Ljava/awt/Image;Ljava/awt/geom/AffineTransform;Ljava/awt/image/ImageObserver;)Z",&[A::O(&image.object),A::O(&transform),A::Null])?;
            } else if !transparent {
                let color = j.static_obj(
                    "com/intellij/util/ui/UIUtil",
                    "getLabelForeground",
                    "()Ljava/awt/Color;",
                    &[],
                )?;
                j.void(&g, "setColor", "(Ljava/awt/Color;)V", &[A::O(&color)])?;
                j.void(
                    &g,
                    "drawString",
                    "(Ljava/lang/String;II)V",
                    &[A::S(&status), A::I(16), A::I(28)],
                )?;
            }
            if let Some([x, y, w, h]) = selection {
                let rectangle = j.new(
                    "java/awt/geom/Rectangle2D$Double",
                    "(DDDD)V",
                    &[
                        A::D(x * content),
                        A::D(y * content),
                        A::D(w * content),
                        A::D(h * content),
                    ],
                )?;
                for (alpha, method) in [(35, "fill"), (255, "draw")] {
                    let color = j.new(
                        "java/awt/Color",
                        "(IIII)V",
                        &[A::I(54), A::I(162), A::I(235), A::I(alpha)],
                    )?;
                    j.void(&g, "setColor", "(Ljava/awt/Color;)V", &[A::O(&color)])?;
                    j.void(&g, method, "(Ljava/awt/Shape;)V", &[A::O(&rectangle)])?;
                }
            }
            Ok(())
        })();
        let disposed = j.void(&g, "dispose", "()V", &[]);
        result.and(disposed)
    }
    fn handle(&self, j: &mut J<'_>, operation: &str, args: &[O]) -> Result<O> {
        let op = operation.split('.').next_back().unwrap_or_default();
        match op {
            "paintComponent" => self.paint(j, &args[0])?,
            "contains" => {
                let overlay = self.state.lock().expect("surface").overlay;
                return j.boxed_bool(!overlay);
            }
            "componentResized" | "componentShown" | "propertyChange" => self.size(j)?,
            "hierarchyChanged" => {
                self.visibility(j)?;
                self.size(j)?;
            }
            "mousePressed" | "mouseReleased" | "mouseMoved" | "mouseDragged" | "mouseExited"
            | "mouseWheelMoved" => self.mouse(j, op, &args[0])?,
            "keyPressed" | "keyReleased" | "keyTyped" => self.key(j, op, &args[0])?,
            _ => {}
        }
        j.null()
    }
    fn mouse(&self, j: &mut J<'_>, op: &str, event: &O) -> Result<()> {
        let scale = self.state.lock().expect("surface").content_scale;
        let x = j.int(event, "getX")? as f64;
        let y = j.int(event, "getY")? as f64;
        if op == "mousePressed" {
            if j.int(event, "getButton")? != 1 {
                return Ok(());
            }
            if self.id == 0
                && let Some(panel) = self.panel.upgrade()
            {
                let disconnected = !panel.state.lock().expect("panel").connected;
                if disconnected {
                    panel.start(j)?;
                    return Ok(());
                }
            }
            let pointer = self
                .panel
                .upgrade()
                .and_then(|p| p.on_pointer.lock().expect("callback").clone());
            if let Some(pointer) = pointer
                && !pointer(j, x / scale, y / scale, false, false, 0.0)?
            {
                return Ok(());
            }
            j.call(self.component(), "requestFocusInWindow", "()Z", &[])?;
            let sx = j.int(event, "getXOnScreen")?;
            let sy = j.int(event, "getYOnScreen")?;
            {
                let mut state = self.state.lock().expect("surface");
                state.pressed = true;
                state.press_screen = (sx, sy);
            }
            self.send(
                Packet::new(3)
                    .int(self.id)
                    .float((x / scale) as f32)
                    .float((y / scale) as f32),
            );
        } else if op == "mouseReleased" {
            if j.int(event, "getButton")? != 1 {
                return Ok(());
            }
            let (pressed, travelled) = {
                let mut state = self.state.lock().expect("surface");
                let pressed = state.pressed;
                state.pressed = false;
                let travelled = state.gesture.take().is_some_and(|g| g.travelled);
                (pressed, travelled)
            };
            if pressed {
                self.send(
                    Packet::new(4)
                        .int(self.id)
                        .float(if travelled { -1.0 } else { (x / scale) as f32 })
                        .float(if travelled { -1.0 } else { (y / scale) as f32 }),
                );
            }
        } else if op == "mouseDragged" && self.state.lock().expect("surface").gesture.is_some() {
            self.follow(j, event)?;
        } else if op == "mouseMoved" || op == "mouseDragged" {
            self.send(
                Packet::new(2)
                    .int(self.id)
                    .float((x / scale) as f32)
                    .float((y / scale) as f32),
            );
        } else if op == "mouseExited" {
            if !self.state.lock().expect("surface").pressed {
                self.send(Packet::new(5).int(self.id));
            }
        } else if op == "mouseWheelMoved" {
            let rotation = j.double(event, "getPreciseWheelRotation")?;
            let shift = j.bool(event, "isShiftDown")?;
            let alt = j.bool(event, "isAltDown")?;
            let pointer = self
                .panel
                .upgrade()
                .and_then(|p| p.on_pointer.lock().expect("callback").clone());
            if let Some(pointer) = pointer
                && !pointer(
                    j,
                    x / scale,
                    y / scale,
                    true,
                    shift,
                    if alt { -rotation * 40.0 } else { 0.0 },
                )?
            {
                j.void(event, "consume", "()V", &[])?;
                return Ok(());
            }
            let mods = modifiers(j.int(event, "getModifiersEx")?);
            let delta = (-rotation * 40.0) as f32;
            self.send(
                Packet::new(6)
                    .int(self.id)
                    .float((x / scale) as f32)
                    .float((y / scale) as f32)
                    .float(if shift { delta } else { 0.0 })
                    .float(if shift { 0.0 } else { delta })
                    .byte(mods),
            );
        }
        Ok(())
    }
    fn key(&self, j: &mut J<'_>, op: &str, event: &O) -> Result<()> {
        let mods = modifiers(j.int(event, "getModifiersEx")?);
        if op == "keyTyped" {
            let unit = j.call(event, "getKeyChar", "()C", &[])?.c()?;
            if unit == 0xffff || mods & 10 != 0 || unit < 32 || unit == 127 {
                return Ok(());
            }
            let text = {
                let mut state = self.state.lock().expect("surface");
                if (0xd800..=0xdbff).contains(&unit) {
                    state.high_surrogate = Some(unit);
                    return Ok(());
                }
                let units = state
                    .high_surrogate
                    .take()
                    .map(|high| vec![high, unit])
                    .unwrap_or_else(|| vec![unit]);
                String::from_utf16_lossy(&units)
            };
            self.send(Packet::new(8).int(self.id).text(&text));
        } else {
            let code = j.int(event, "getKeyCode")?;
            let location = j.int(event, "getKeyLocation")?;
            let Some(code) = key_code(code, location) else {
                return Ok(());
            };
            self.send(
                Packet::new(7)
                    .int(self.id)
                    .byte(u8::from(op == "keyPressed"))
                    .byte(mods)
                    .text(&code),
            );
        }
        j.void(event, "consume", "()V", &[])
    }
    fn screen_position(&self, j: &mut J<'_>) -> Result<(i32, i32)> {
        if !j.bool(self.component(), "isShowing")? {
            return Ok((0, 0));
        }
        let point = j.obj(
            self.component(),
            "getLocationOnScreen",
            "()Ljava/awt/Point;",
            &[],
        )?;
        Ok((j.field_int(&point, "x")?, j.field_int(&point, "y")?))
    }
    fn begin(&self, j: &mut J<'_>, edge: Option<u8>) -> Result<()> {
        let window = self.state.lock().expect("surface").window.clone();
        let Some(window) = window else {
            return Ok(());
        };
        let rect = j.obj(&window, "getBounds", "()Ljava/awt/Rectangle;", &[])?;
        let bounds = [
            j.field_int(&rect, "x")?,
            j.field_int(&rect, "y")?,
            j.field_int(&rect, "width")?,
            j.field_int(&rect, "height")?,
        ];
        let mut state = self.state.lock().expect("surface");
        if state.pressed {
            state.gesture = Some(Gesture {
                edge,
                bounds,
                from: state.press_screen,
                travelled: false,
            });
        }
        Ok(())
    }
    fn follow(&self, j: &mut J<'_>, event: &O) -> Result<()> {
        let (gesture, window) = {
            let state = self.state.lock().expect("surface");
            (state.gesture.clone(), state.window.clone())
        };
        let (Some(mut gesture), Some(window)) = (gesture, window) else {
            return Ok(());
        };
        let dx = j.int(event, "getXOnScreen")? - gesture.from.0;
        let dy = j.int(event, "getYOnScreen")? - gesture.from.1;
        if !gesture.travelled && dx * dx + dy * dy < 16 {
            return Ok(());
        }
        gesture.travelled = true;
        let b = resized(gesture.bounds, gesture.edge, dx, dy);
        self.state.lock().expect("surface").gesture = Some(gesture);
        j.void(
            &window,
            "setBounds",
            "(IIII)V",
            &[A::I(b[0]), A::I(b[1]), A::I(b[2]), A::I(b[3])],
        )
    }
    fn cursor(&self, j: &mut J<'_>, name: &str) -> Result<()> {
        let kind = match name {
            "pointer" => 12,
            "text" => 2,
            "crosshair" => 1,
            "move" => 13,
            "wait" | "progress" => 3,
            "e-resize" => 11,
            "w-resize" => 10,
            "n-resize" => 8,
            "s-resize" => 9,
            "ne-resize" => 7,
            "nw-resize" => 6,
            "se-resize" => 5,
            "sw-resize" => 4,
            _ => 0,
        };
        let cursor = j.static_obj(
            "java/awt/Cursor",
            "getPredefinedCursor",
            "(I)Ljava/awt/Cursor;",
            &[A::I(kind)],
        )?;
        j.void(
            self.component(),
            "setCursor",
            "(Ljava/awt/Cursor;)V",
            &[A::O(&cursor)],
        )
    }
    fn close(&self, j: &mut J<'_>) -> Result<()> {
        let window = self.state.lock().expect("surface").window.take();
        if let Some(window) = window {
            j.void(&window, "dispose", "()V", &[])?;
        }
        let parent = j.obj(self.component(), "getParent", "()Ljava/awt/Container;", &[])?;
        if !parent.is_null() {
            j.void(
                &parent,
                "remove",
                "(Ljava/awt/Component;)V",
                &[A::O(self.component())],
            )?;
            j.void(&parent, "repaint", "()V", &[])?;
        }
        self.scope.clear();
        Ok(())
    }
}
pub fn modifiers(value: i32) -> u8 {
    u8::from(value & 64 != 0)
        | u8::from(value & 128 != 0) << 1
        | u8::from(value & 512 != 0) << 2
        | u8::from(value & 256 != 0) << 3
}
pub fn key_code(code: i32, location: i32) -> Option<String> {
    if (65..=90).contains(&code) {
        return Some(format!("Key{}", char::from_u32(code as u32)?));
    }
    if (48..=57).contains(&code) {
        return Some(format!("Digit{}", code - 48));
    }
    if (112..=123).contains(&code) {
        return Some(format!("F{}", code - 111));
    }
    let name = match code {
        16 => "Shift",
        17 => "Control",
        18 => "Alt",
        157 => "Meta",
        38 => "ArrowUp",
        40 => "ArrowDown",
        37 => "ArrowLeft",
        39 => "ArrowRight",
        36 => "Home",
        35 => "End",
        33 => "PageUp",
        34 => "PageDown",
        8 => "Backspace",
        127 => "Delete",
        10 if location == 4 => "NumpadEnter",
        10 => "Enter",
        9 => "Tab",
        32 => "Space",
        27 => "Escape",
        45 => "Minus",
        61 => "Equal",
        91 => "BracketLeft",
        93 => "BracketRight",
        92 => "Backslash",
        59 => "Semicolon",
        222 => "Quote",
        44 => "Comma",
        46 => "Period",
        47 => "Slash",
        192 => "Backquote",
        _ => return None,
    };
    Some(if matches!(code, 16 | 17 | 18 | 157) {
        format!("{name}{}", if location == 3 { "Right" } else { "Left" })
    } else {
        name.into()
    })
}
pub fn resized([x, y, w, h]: [i32; 4], edge: Option<u8>, dx: i32, dy: i32) -> [i32; 4] {
    let Some(edge) = edge else {
        return [x + dx, y + dy, w, h];
    };
    let west = matches!(edge, 3 | 6 | 7);
    let east = matches!(edge, 0 | 2 | 5);
    let north = matches!(edge, 1..=3);
    let south = matches!(edge, 4..=6);
    let width = (w + if east {
        dx
    } else if west {
        -dx
    } else {
        0
    })
    .max(48);
    let height = (h + if south {
        dy
    } else if north {
        -dy
    } else {
        0
    })
    .max(48);
    [
        if west { x + w - width } else { x },
        if north { y + h - height } else { y },
        width,
        height,
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_maps_sides_and_modifiers() {
        assert_eq!(key_code(16, 3).as_deref(), Some("ShiftRight"));
        assert_eq!(key_code(10, 4).as_deref(), Some("NumpadEnter"));
        assert_eq!(modifiers(64 | 128 | 256 | 512), 15);
    }
    #[test]
    fn resize_keeps_opposite_edges_fixed() {
        assert_eq!(
            resized([10, 20, 100, 100], Some(3), 80, 80),
            [62, 72, 48, 48]
        );
        assert_eq!(resized([10, 20, 100, 100], None, 3, 4), [13, 24, 100, 100]);
    }
}
