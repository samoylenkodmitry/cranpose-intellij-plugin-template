//! EDT geometry and lifecycle for the shared, native GPU decoration layer.
use crate::{
    jvm::{self, A, J, O},
    project::Project,
    wake::Wake,
};
use anyhow::Result;
use cranpose_plugin_decorations::{
    Bounds, Controller, CounterFrame, Frame, LightningFrame, Target,
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct Counter {
    pub id: u64,
    pub editor: O,
    pub inlay: O,
    pub label: String,
    pub gpu: Arc<AtomicBool>,
}
/// Lightning anchors in the host component's coordinate system.
pub(crate) struct Bolt {
    pub component: O,
    pub editor: O,
    pub frame: LightningFrame,
}
struct Window {
    object: O,
    controller: Option<Controller>,
    last: Option<(Bounds, Frame)>,
    error: Option<String>,
    callback: O,
    wake: Weak<Wake>,
    _scope: jvm::Scope,
}
struct Watch {
    editor: O,
    content: O,
    scrolling: O,
    callback: O,
    disposable: O,
    _scope: jvm::Scope,
}
#[derive(Default)]
pub(crate) struct State {
    wake: Option<Arc<Wake>>,
    windows: Vec<Window>,
    watches: Vec<Watch>,
    menu: Option<(O, O, jvm::Scope)>,
}

pub(crate) fn install(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    let weak = Arc::downgrade(project);
    let wake = Wake::new(j, move |j| {
        if let Some(project) = weak.upgrade()
            && !project.closed.load(Ordering::Acquire)
        {
            tick(&project, j)?;
        }
        Ok(())
    })?;
    let (scope, callback) = listener(j, &wake)?;
    let menu = j.static_obj(
        "javax/swing/MenuSelectionManager",
        "defaultManager",
        "()Ljavax/swing/MenuSelectionManager;",
        &[],
    )?;
    j.void(
        &menu,
        "addChangeListener",
        "(Ljavax/swing/event/ChangeListener;)V",
        &[A::O(&callback)],
    )?;
    let mut state = project.decorations.lock().expect("decorations");
    state.menu = Some((menu, callback, scope));
    state.wake = Some(wake);
    Ok(())
}
fn listener(j: &mut J<'_>, wake: &Arc<Wake>) -> Result<(jvm::Scope, O)> {
    let scope = jvm::Scope::default();
    let wake = Arc::downgrade(wake);
    let id = scope.register(move |j, _, _| {
        if let Some(wake) = wake.upgrade() {
            wake.request(j)?;
        }
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    Ok((scope, callback))
}
impl Watch {
    fn new(j: &mut J<'_>, project: &Project, editor: O, wake: &Arc<Wake>) -> Result<Self> {
        let (scope, callback) = listener(j, wake)?;
        let disposable = j.static_obj(
            "com/intellij/openapi/util/Disposer",
            "newDisposable",
            "()Lcom/intellij/openapi/Disposable;",
            &[],
        )?;
        j.static_void(
            "com/intellij/openapi/util/Disposer",
            "register",
            "(Lcom/intellij/openapi/Disposable;Lcom/intellij/openapi/Disposable;)V",
            &[A::O(&project.object), A::O(&disposable)],
        )?;
        let scrolling = j.obj(
            &editor,
            "getScrollingModel",
            "()Lcom/intellij/openapi/editor/ScrollingModel;",
            &[],
        )?;
        j.void(
            &scrolling,
            "addVisibleAreaListener",
            "(Lcom/intellij/openapi/editor/event/VisibleAreaListener;)V",
            &[A::O(&callback)],
        )?;
        for (getter, ty, listener_ty) in [
            (
                "getInlayModel",
                "com/intellij/openapi/editor/InlayModel",
                "com/intellij/openapi/editor/InlayModel$Listener",
            ),
            (
                "getFoldingModel",
                "com/intellij/openapi/editor/FoldingModel",
                "com/intellij/openapi/editor/ex/FoldingListener",
            ),
        ] {
            let model = j.obj(&editor, getter, &format!("()L{ty};"), &[])?;
            j.void(
                &model,
                "addListener",
                &format!("(L{listener_ty};Lcom/intellij/openapi/Disposable;)V"),
                &[A::O(&callback), A::O(&disposable)],
            )?;
        }
        j.void(
            &editor,
            "addPropertyChangeListener",
            "(Ljava/beans/PropertyChangeListener;Lcom/intellij/openapi/Disposable;)V",
            &[A::O(&callback), A::O(&disposable)],
        )?;
        let content = content(j, &editor)?;
        for (method, ty) in [
            ("addHierarchyListener", "HierarchyListener"),
            ("addComponentListener", "ComponentListener"),
        ] {
            j.void(
                &content,
                method,
                &format!("(Ljava/awt/event/{ty};)V"),
                &[A::O(&callback)],
            )?;
        }
        Ok(Self {
            editor,
            content,
            scrolling,
            callback,
            disposable,
            _scope: scope,
        })
    }
    fn dispose(self, j: &mut J<'_>) -> Result<()> {
        j.void(
            &self.scrolling,
            "removeVisibleAreaListener",
            "(Lcom/intellij/openapi/editor/event/VisibleAreaListener;)V",
            &[A::O(&self.callback)],
        )?;
        for (method, ty) in [
            ("removeHierarchyListener", "HierarchyListener"),
            ("removeComponentListener", "ComponentListener"),
        ] {
            j.void(
                &self.content,
                method,
                &format!("(Ljava/awt/event/{ty};)V"),
                &[A::O(&self.callback)],
            )?;
        }
        j.static_void(
            "com/intellij/openapi/util/Disposer",
            "dispose",
            "(Lcom/intellij/openapi/Disposable;)V",
            &[A::O(&self.disposable)],
        )
    }
}
impl Window {
    fn new(j: &mut J<'_>, object: O, wake: &Arc<Wake>) -> Result<Self> {
        let (scope, callback) = listener(j, wake)?;
        for (method, ty) in [
            ("addWindowListener", "WindowListener"),
            ("addComponentListener", "ComponentListener"),
        ] {
            j.void(
                &object,
                method,
                &format!("(Ljava/awt/event/{ty};)V"),
                &[A::O(&callback)],
            )?;
        }
        Ok(Self {
            object,
            wake: Arc::downgrade(wake),
            controller: None,
            last: None,
            error: None,
            callback,
            _scope: scope,
        })
    }
    fn dispose(self, j: &mut J<'_>) -> Result<()> {
        if let Some(controller) = &self.controller {
            controller.hide()?;
        }
        for (method, ty) in [
            ("removeWindowListener", "WindowListener"),
            ("removeComponentListener", "ComponentListener"),
        ] {
            j.void(
                &self.object,
                method,
                &format!("(Ljava/awt/event/{ty};)V"),
                &[A::O(&self.callback)],
            )?;
        }
        Ok(())
    }
    fn present(&mut self, j: &mut J<'_>, frame: Frame) -> Result<bool> {
        if frame.counters.is_empty() && frame.lightning.is_none()
            || !j.bool(&self.object, "isShowing")?
            || !j.bool(&self.object, "isActive")?
        {
            if self.last.take().is_some()
                && let Some(controller) = &self.controller
            {
                controller.hide()?;
            }
            return Ok(false);
        }
        if self.error.is_some() {
            return Ok(false);
        }
        let config = j.obj(
            &self.object,
            "getGraphicsConfiguration",
            "()Ljava/awt/GraphicsConfiguration;",
            &[],
        )?;
        let transform = j.obj(
            &config,
            "getDefaultTransform",
            "()Ljava/awt/geom/AffineTransform;",
            &[],
        )?;
        let scale = j.double(&transform, "getScaleX")?;
        let (bounds, frame) = crop(frame, scale);
        let result = (|| -> Result<bool> {
            if self.controller.is_none() {
                let wake = self.wake.clone();
                self.controller = Some(Controller::with_waker(
                    Target::new(&mut j.env, self.object.as_obj(), bounds)?,
                    move || {
                        if let Some(wake) = wake.upgrade() {
                            let _ = wake.request_from_worker();
                        }
                    },
                )?);
            }
            let controller = self.controller.as_ref().expect("GPU controller");
            if let Some(error) = controller.error() {
                anyhow::bail!("{error}");
            }
            if self.last.as_ref() != Some(&(bounds, frame.clone())) {
                controller.update(bounds, frame.clone())?;
                self.last = Some((bounds, frame));
            }
            Ok(controller.frames_presented() > 0)
        })();
        match result {
            Ok(ready) => Ok(ready),
            Err(error) => {
                self.controller.take();
                let message = format!(
                    "Cranpose GPU decorations unavailable; counters use static IDE text: {error:#}"
                );
                self.error = Some(message.clone());
                let logger = j.static_obj(
                    "com/intellij/openapi/diagnostic/Logger",
                    "getInstance",
                    "(Ljava/lang/String;)Lcom/intellij/openapi/diagnostic/Logger;",
                    &[A::S("dev.cranpose.gpu")],
                )?;
                j.void(&logger, "warn", "(Ljava/lang/String;)V", &[A::S(&message)])?;
                Ok(false)
            }
        }
    }
}
fn content(j: &mut J<'_>, editor: &O) -> Result<O> {
    j.obj(
        editor,
        "getContentComponent",
        "()Ljavax/swing/JComponent;",
        &[],
    )
}
fn owner(j: &mut J<'_>, component: &O) -> Result<O> {
    j.static_obj(
        "javax/swing/SwingUtilities",
        "getWindowAncestor",
        "(Ljava/awt/Component;)Ljava/awt/Window;",
        &[A::O(component)],
    )
}
fn point(j: &mut J<'_>, component: &O, window: &O, x: i32, y: i32) -> Result<[f32; 2]> {
    let p = j.static_obj(
        "javax/swing/SwingUtilities",
        "convertPoint",
        "(Ljava/awt/Component;IILjava/awt/Component;)Ljava/awt/Point;",
        &[A::O(component), A::I(x), A::I(y), A::O(window)],
    )?;
    let insets = j.obj(window, "getInsets", "()Ljava/awt/Insets;", &[])?;
    Ok([
        (j.field_int(&p, "x")? - j.field_int(&insets, "left")?) as f32,
        (j.field_int(&p, "y")? - j.field_int(&insets, "top")?) as f32,
    ])
}
fn rect(j: &mut J<'_>, r: &O) -> Result<[i32; 4]> {
    Ok([
        j.field_int(r, "x")?,
        j.field_int(r, "y")?,
        j.field_int(r, "width")?,
        j.field_int(r, "height")?,
    ])
}
fn intersects([x, y, w, h]: [i32; 4], [a, b, c, d]: [i32; 4]) -> bool {
    w > 0 && h > 0 && x < a + c && y < b + d && x + w > a && y + h > b
}
fn crop(mut frame: Frame, scale: f64) -> (Bounds, Frame) {
    let mut rects: Vec<_> = frame.counters.iter().map(|c| c.bounds).collect();
    if let Some(bolt) = &frame.lightning {
        let [x, y] = bolt.from;
        let [a, b, w, h] = bolt.target;
        rects.push([
            x.min(a) - 40.0,
            y.min(b) - 48.0,
            (x.max(a + w) - x.min(a)) + 80.0,
            (y.max(b + h) - y.min(b)) + 96.0,
        ]);
    }
    let left = rects
        .iter()
        .map(|r| r[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0);
    let top = rects
        .iter()
        .map(|r| r[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0);
    let right = rects.iter().map(|r| r[0] + r[2]).fold(0.0, f32::max).ceil();
    let bottom = rects.iter().map(|r| r[1] + r[3]).fold(0.0, f32::max).ceil();
    let width = (right - left).max(1.0) as u32;
    let height = (bottom - top).max(1.0) as u32;
    for counter in &mut frame.counters {
        counter.bounds[0] -= left;
        counter.bounds[1] -= top;
        counter.clip[0] -= left;
        counter.clip[1] -= top;
    }
    if let Some(bolt) = &mut frame.lightning {
        bolt.from[0] -= left;
        bolt.from[1] -= top;
        bolt.target[0] -= left;
        bolt.target[1] -= top;
        bolt.size = [width as f32, height as f32];
    }
    (
        Bounds {
            x: left.into(),
            y: top.into(),
            width,
            height,
            scale,
        },
        frame,
    )
}

pub(crate) fn tick(project: &Project, j: &mut J<'_>) -> Result<()> {
    if project.closed.load(Ordering::Acquire) {
        return Ok(());
    }
    let counters: Vec<_> = project
        .workspaces
        .lock()
        .expect("workspaces")
        .iter()
        .filter_map(Weak::upgrade)
        .flat_map(|w| w.counters.lock().expect("counters").anchors())
        .collect();
    let bolt = crate::feedback::decoration(project, j)?;
    let mut state = project.decorations.lock().expect("decorations");
    let Some(wake) = state.wake.clone() else {
        return Ok(());
    };
    let mut editors: Vec<O> = counters.iter().map(|c| c.editor.clone()).collect();
    if let Some(bolt) = &bolt {
        editors.push(bolt.editor.clone());
    }
    for editor in &editors {
        if j.bool(editor, "isDisposed")? {
            continue;
        }
        let mut found = false;
        for watch in &state.watches {
            found |= j.same(editor, &watch.editor)?;
        }
        if !found {
            state
                .watches
                .push(Watch::new(j, project, editor.clone(), &wake)?);
        }
    }
    for i in (0..state.watches.len()).rev() {
        let mut keep = false;
        for editor in &editors {
            keep |= j.same(editor, &state.watches[i].editor)? && !j.bool(editor, "isDisposed")?;
        }
        if !keep {
            state.watches.remove(i).dispose(j)?;
        }
    }
    let mut batches: Vec<(O, Frame, Vec<usize>)> = Vec::new();
    let tones = crate::glyphs::Tones::current(j)?;
    let color = tones.text;
    for (i, counter) in counters.iter().enumerate() {
        if j.bool(&counter.editor, "isDisposed")? || !j.bool(&counter.inlay, "isValid")? {
            continue;
        }
        let content = content(j, &counter.editor)?;
        if !j.bool(&content, "isShowing")? {
            continue;
        }
        let bounds = j.obj(&counter.inlay, "getBounds", "()Ljava/awt/Rectangle;", &[])?;
        if bounds.is_null() {
            continue;
        }
        let bounds = rect(j, &bounds)?;
        let visible = j.obj(&content, "getVisibleRect", "()Ljava/awt/Rectangle;", &[])?;
        let visible = rect(j, &visible)?;
        if !intersects(bounds, visible) {
            continue;
        }
        let window = owner(j, &content)?;
        if window.is_null() {
            continue;
        }
        let [x, y] = point(j, &content, &window, bounds[0], bounds[1])?;
        let [cx, cy] = point(j, &content, &window, visible[0], visible[1])?;
        let counter_frame = CounterFrame {
            id: counter.id,
            bounds: [x, y, bounds[2] as f32, bounds[3] as f32],
            clip: [cx, cy, visible[2] as f32, visible[3] as f32],
            label: counter.label.clone(),
            progress: 1.0,
            foreground: [
                f32::from(color.0) / 255.0,
                f32::from(color.1) / 255.0,
                f32::from(color.2) / 255.0,
                color.3,
            ],
        };
        let batch = batch(j, &mut batches, &window)?;
        batch.1.counters.push(counter_frame);
        batch.2.push(i);
    }
    if let Some(mut bolt) = bolt {
        let window = owner(j, &bolt.component)?;
        if !window.is_null() {
            let [x, y] = point(j, &bolt.component, &window, 0, 0)?;
            bolt.frame.from[0] += x;
            bolt.frame.from[1] += y;
            bolt.frame.target[0] += x;
            bolt.frame.target[1] += y;
            batch(j, &mut batches, &window)?.1.lightning = Some(bolt.frame);
        }
    }
    let menu = j.static_obj(
        "javax/swing/MenuSelectionManager",
        "defaultManager",
        "()Ljavax/swing/MenuSelectionManager;",
        &[],
    )?;
    let selection = j.obj(
        &menu,
        "getSelectedPath",
        "()[Ljavax/swing/MenuElement;",
        &[],
    )?;
    let menu_open = !j.elements(&selection)?.is_empty();
    let mut ready = vec![false; counters.len()];
    if counters.is_empty() && batches.is_empty() {
        for window in state.windows.drain(..) {
            window.dispose(j)?;
        }
    }
    for (object, _, _) in &batches {
        let mut found = false;
        for window in &state.windows {
            found |= j.same(object, &window.object)?;
        }
        if !found {
            state.windows.push(Window::new(j, object.clone(), &wake)?);
        }
    }
    for index in (0..state.windows.len()).rev() {
        if !j.bool(&state.windows[index].object, "isDisplayable")? {
            state.windows.remove(index).dispose(j)?;
            continue;
        }
        let window = &mut state.windows[index];
        let mut scene = None;
        if !menu_open {
            for (object, frame, ids) in &batches {
                if j.same(object, &window.object)? {
                    scene = Some((frame.clone(), ids));
                    break;
                }
            }
        }
        if let Some((frame, ids)) = scene {
            let healthy = window.present(j, frame)?;
            for i in ids {
                ready[*i] = healthy;
            }
        } else {
            window.present(j, Frame::default())?;
        }
    }
    for (counter, gpu) in counters.iter().zip(ready) {
        if counter.gpu.swap(gpu, Ordering::AcqRel) != gpu && j.bool(&counter.inlay, "isValid")? {
            j.void(&counter.inlay, "repaint", "()V", &[])?;
        }
    }
    Ok(())
}
fn batch<'a>(
    j: &mut J<'_>,
    batches: &'a mut Vec<(O, Frame, Vec<usize>)>,
    object: &O,
) -> Result<&'a mut (O, Frame, Vec<usize>)> {
    let mut found = None;
    for (i, (window, _, _)) in batches.iter().enumerate() {
        if j.same(window, object)? {
            found = Some(i);
            break;
        }
    }
    let i = found.unwrap_or_else(|| {
        batches.push((object.clone(), Frame::default(), Vec::new()));
        batches.len() - 1
    });
    Ok(&mut batches[i])
}
pub(crate) fn dispose(project: &Project, j: &mut J<'_>) -> Result<()> {
    let mut state = project.decorations.lock().expect("decorations");
    if let Some(wake) = state.wake.take() {
        wake.close();
    }
    if let Some((menu, callback, _scope)) = state.menu.take() {
        j.void(
            &menu,
            "removeChangeListener",
            "(Ljavax/swing/event/ChangeListener;)V",
            &[A::O(&callback)],
        )?;
    }
    for watch in state.watches.drain(..) {
        watch.dispose(j)?;
    }
    for window in state.windows.drain(..) {
        window.dispose(j)?;
    }
    Ok(())
}

#[cfg(feature = "ide-tests")]
pub(crate) fn test_watch(j: &mut J<'_>, project: &Arc<Project>, editor: &O) -> Result<()> {
    let wake = Wake::new(j, |_| Ok(()))?;
    let watch = Watch::new(j, project, editor.clone(), &wake)?;
    watch.dispose(j)?;
    wake.close();
    Ok(())
}

pub(crate) fn schedule(project: &Project, j: &mut J<'_>) -> Result<()> {
    let wake = project
        .decorations
        .lock()
        .expect("decorations")
        .wake
        .clone();
    if let Some(wake) = wake {
        wake.request(j)?;
    }
    Ok(())
}
