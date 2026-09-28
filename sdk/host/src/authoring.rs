//! Document-based preview markers and native Cranpose value controls.
//! Parsing runs off the EDT; unchanged documents and viewports do no work.
#[cfg(feature = "ide-tests")]
mod placement_test;
use crate::{
    editor,
    jvm::{self, A, J, O},
    project::{self, Project},
    surface::Panel,
    workspace,
};
use anyhow::{Result, ensure};
use cranpose_plugin_authoring::{
    Catalog,
    runtime::{Update, Value as LiveValue},
};
#[cfg(feature = "ide-tests")]
pub use placement_test::integration_test as placement_test;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub struct State {
    wake: Option<Arc<crate::wake::Wake>>,
    pub generation: Option<mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    key: Option<(String, i64)>,
    current: Option<Parsed>,
    receiver: Option<mpsc::Receiver<Parsed>>,
    placed: Vec<Placed>,
    panel: Option<Arc<Panel>>,
    popup: Option<(O, Arc<Panel>)>,
    control: Option<Arc<Panel>>,
    control_request: u64,
    control_init: Option<String>,
    hovered: Option<usize>,
    geometry: String,
    overlay_id: Option<u32>,
    pub overlay_ready: bool,
    watched: Option<(O, O, i64)>,
    arrival: Option<Arrival>,
}
struct Arrival {
    path: String,
    line: i32,
    request: u64,
    created: std::time::Instant,
    shown: Option<std::time::Instant>,
}
pub fn source_arrival(project: &Project, path: &str, line: i32) {
    static REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut state = project.authoring.lock().expect("authoring");
    state.arrival = Some(Arrival {
        path: path.into(),
        line,
        request: REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        created: std::time::Instant::now(),
        shown: None,
    });
    state.geometry.clear();
}
pub fn invalidate(project: &Project) {
    project
        .authoring
        .lock()
        .expect("authoring")
        .geometry
        .clear();
}
struct Parsed {
    path: String,
    stamp: i64,
    source: String,
    catalog: Option<Catalog>,
}
struct Placed {
    object: O,
    callback: i64,
    literal: Option<usize>,
}

pub fn install(project: &Arc<Project>, j: &mut J<'_>, multicaster: &O) -> Result<()> {
    let weak = Arc::downgrade(project);
    let wake = crate::wake::Wake::new(j, move |j| {
        if let Some(project) = weak.upgrade()
            && !project.closed.load(std::sync::atomic::Ordering::Acquire)
        {
            tick(&project, j)?;
        }
        Ok(())
    })?;
    project.authoring.lock().expect("authoring").wake = Some(wake);
    let weak = Arc::downgrade(project);
    let id = project.scope.register(move |j, op, args| {
        if let Some(project) = weak.upgrade() {
            match op {
                "Callback.mouseClicked" => pointer(&project, j, &args[0], true)?,
                "Callback.mouseMoved" => pointer(&project, j, &args[0], false)?,
                "Callback.mouseExited" => {
                    project.authoring.lock().expect("authoring").hovered = None
                }
                _ => {}
            }
        }
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    j.void(multicaster, "addEditorMouseListener", "(Lcom/intellij/openapi/editor/event/EditorMouseListener;Lcom/intellij/openapi/Disposable;)V", &[A::O(&callback), A::O(&project.object)])?;
    j.void(multicaster, "addEditorMouseMotionListener", "(Lcom/intellij/openapi/editor/event/EditorMouseMotionListener;Lcom/intellij/openapi/Disposable;)V", &[A::O(&callback), A::O(&project.object)])
}

/// Only the editor already watched by this project can need a new parse.
/// Schedule after the document write finishes; never parse inside its listener.
pub fn document_changed(project: &Project, j: &mut J<'_>, event: &O) -> Result<()> {
    let editor = project
        .authoring
        .lock()
        .expect("authoring")
        .watched
        .as_ref()
        .map(|(editor, _, _)| editor.clone());
    if let Some(editor) = editor {
        let document = j.obj(
            event,
            "getDocument",
            "()Lcom/intellij/openapi/editor/Document;",
            &[],
        )?;
        let watched = j.obj(
            &editor,
            "getDocument",
            "()Lcom/intellij/openapi/editor/Document;",
            &[],
        )?;
        if j.same(&document, &watched)? {
            schedule(project, j)?;
        }
    }
    Ok(())
}

pub fn schedule(project: &Project, j: &mut J<'_>) -> Result<()> {
    let wake = project.authoring.lock().expect("authoring").wake.clone();
    if let Some(wake) = wake {
        wake.request(j)?;
    }
    Ok(())
}

pub fn tick(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    crate::feedback::tick(project, j)?;
    crate::wizard::tick(project, j)?;
    {
        let mut state = project.authoring.lock().expect("authoring");
        if state.arrival.as_ref().is_some_and(|a| {
            a.created.elapsed() > std::time::Duration::from_secs(5)
                || a.shown
                    .is_some_and(|t| t.elapsed() > std::time::Duration::from_millis(1100))
        }) {
            state.arrival = None;
            state.geometry.clear();
        }
    }
    let popup = project
        .authoring
        .lock()
        .expect("authoring")
        .popup
        .as_ref()
        .map(|(o, _)| o.clone());
    if let Some(popup) = popup
        && j.bool(&popup, "isDisposed")?
    {
        dismiss_control(project, j)?;
    }
    let (Some(file), Some(editor)) = project.selected(j)? else {
        return detach(project, j);
    };
    let path = j.text(&file, "getPath")?;
    if !path.ends_with(".rs") || !Path::new(&path).starts_with(&project.root) {
        return detach(project, j);
    }
    watch_editor(project, j, &editor)?;
    let document = j.obj(
        &editor,
        "getDocument",
        "()Lcom/intellij/openapi/editor/Document;",
        &[],
    )?;
    let stamp = j.long(&document, "getModificationStamp")?;
    let result = project
        .authoring
        .lock()
        .expect("authoring")
        .receiver
        .as_ref()
        .and_then(|r| r.try_recv().ok());
    if let Some(result) = result {
        project.authoring.lock().expect("authoring").receiver = None;
        if result.path == path && result.stamp == stamp {
            if let Some(catalog) = &result.catalog {
                let revision = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros() as u64;
                crate::feedback::parsed(project, &path, stamp, catalog, revision);
                refresh_placed(project, j, &editor, &path, catalog)?;
                if !catalog.literals.is_empty() {
                    ensure_control(project, j)?;
                }
                let relative = Path::new(&path)
                    .strip_prefix(&project.root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let payload = serde_json::to_string(&Update {
                    file: relative,
                    schema: catalog.schema.clone(),
                    revision,
                    values: catalog
                        .literals
                        .iter()
                        .map(|l| LiveValue {
                            id: l.id,
                            kind: l.kind.clone(),
                            value: l.value.clone(),
                        })
                        .collect(),
                })?;
                for workspace in project
                    .workspaces
                    .lock()
                    .expect("workspaces")
                    .iter()
                    .filter_map(std::sync::Weak::upgrade)
                {
                    workspace.live_values(&payload);
                }
            } else {
                clear_placed(project, j)?;
            }
            let mut state = project.authoring.lock().expect("authoring");
            state.current = Some(result);
            state.geometry.clear();
        } else {
            project.authoring.lock().expect("authoring").key = None;
        }
    }
    let start = {
        let mut state = project.authoring.lock().expect("authoring");
        if state.receiver.is_none() && state.key.as_ref() != Some(&(path.clone(), stamp)) {
            state.key = Some((path.clone(), stamp));
            true
        } else {
            false
        }
    };
    if start {
        if j.int(&document, "getTextLength")? as usize > cranpose_plugin_authoring::MAX_SOURCE_BYTES
        {
            clear_placed(project, j)?;
            return detach(project, j);
        }
        let source = j.text(&document, "getText")?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let wake = {
            let mut state = project.authoring.lock().expect("authoring");
            state.receiver = Some(receiver);
            state.wake.clone()
        };
        std::thread::spawn(move || {
            let catalog = Catalog::parse(&source).ok();
            let _ = sender.send(Parsed {
                path,
                stamp,
                source,
                catalog,
            });
            if let Some(wake) = wake {
                // The regular project tick remains a fallback if JVM delivery
                // fails. A closed wake cannot revive a disposed project.
                let _ = wake.request_from_worker();
            }
        });
    }
    geometry(project, j, &editor, stamp)
}

fn clear_placed(project: &Project, j: &mut J<'_>) -> Result<()> {
    let placed = std::mem::take(&mut project.authoring.lock().expect("authoring").placed);
    for item in placed {
        if j.bool(&item.object, "isValid")? {
            j.void(&item.object, "dispose", "()V", &[])?;
        }
        jvm::unregister(item.callback);
    }
    invalidate(project);
    if let Some(panel) = &project.authoring.lock().expect("authoring").panel {
        panel.message("ide.authoring.geometry", "{\"items\":[]}");
    }
    Ok(())
}
// Folding, parameter hints and font changes move source without changing its
// document stamp. Listen to those events rather than resampling all positions.
fn watch_editor(project: &Arc<Project>, j: &mut J<'_>, editor: &O) -> Result<()> {
    if let Some((current, _, _)) = &project.authoring.lock().expect("authoring").watched
        && j.same(current, editor)?
    {
        return Ok(());
    }
    unwatch_editor(project, j)?;
    dismiss_control(project, j)?;
    project.authoring.lock().expect("authoring").hovered = None;
    clear_placed(project, j)?;
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
    let weak = Arc::downgrade(project);
    let id = project.scope.register(move |j, _, _| {
        if let Some(project) = weak.upgrade() {
            invalidate(&project);
        }
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    let inlays = j.obj(
        editor,
        "getInlayModel",
        "()Lcom/intellij/openapi/editor/InlayModel;",
        &[],
    )?;
    j.void(
        &inlays,
        "addListener",
        "(Lcom/intellij/openapi/editor/InlayModel$Listener;Lcom/intellij/openapi/Disposable;)V",
        &[A::O(&callback), A::O(&disposable)],
    )?;
    let folds = j.obj(
        editor,
        "getFoldingModel",
        "()Lcom/intellij/openapi/editor/FoldingModel;",
        &[],
    )?;
    j.void(
        &folds,
        "addListener",
        "(Lcom/intellij/openapi/editor/ex/FoldingListener;Lcom/intellij/openapi/Disposable;)V",
        &[A::O(&callback), A::O(&disposable)],
    )?;
    j.void(
        editor,
        "addPropertyChangeListener",
        "(Ljava/beans/PropertyChangeListener;Lcom/intellij/openapi/Disposable;)V",
        &[A::O(&callback), A::O(&disposable)],
    )?;
    let mut state = project.authoring.lock().expect("authoring");
    state.watched = Some((editor.clone(), disposable, id));
    state.key = None;
    state.current = None;
    Ok(())
}
fn unwatch_editor(project: &Project, j: &mut J<'_>) -> Result<()> {
    let old = project.authoring.lock().expect("authoring").watched.take();
    if let Some((_, disposable, id)) = old {
        j.static_void(
            "com/intellij/openapi/util/Disposer",
            "dispose",
            "(Lcom/intellij/openapi/Disposable;)V",
            &[A::O(&disposable)],
        )?;
        jvm::unregister(id);
    }
    Ok(())
}
fn detach(project: &Project, j: &mut J<'_>) -> Result<()> {
    dismiss_control(project, j)?;
    unwatch_editor(project, j)?;
    {
        let state = project.authoring.lock().expect("authoring");
        if state.current.is_none() && state.key.is_none() && state.placed.is_empty() {
            return Ok(());
        }
    }
    clear_placed(project, j)?;
    let mut state = project.authoring.lock().expect("authoring");
    state.key = None;
    state.current = None;
    state.geometry.clear();
    if let Some(panel) = &state.panel {
        panel.message("ide.authoring.geometry", "{\"items\":[]}");
        panel.send(crate::protocol::Packet::new(13).int(0).byte(0));
        if let Some(id) = state.overlay_id
            && let Some(surface) = panel.overlay_surface(id)
        {
            j.void(surface.component(), "setVisible", "(Z)V", &[A::Z(false)])?;
        }
    }
    Ok(())
}
pub fn dispose(project: &Project, j: &mut J<'_>) -> Result<()> {
    if let Some(wake) = project.authoring.lock().expect("authoring").wake.take() {
        wake.close();
    }
    dismiss_control(project, j)?;
    unwatch_editor(project, j)?;
    clear_placed(project, j)?;
    let (panel, control) = {
        let mut s = project.authoring.lock().expect("authoring");
        (s.panel.take(), s.control.take())
    };
    if let Some(panel) = panel {
        panel.close(j)?;
    }
    if let Some(panel) = control {
        panel.close(j)?;
    }
    Ok(())
}

// Inline anchors already follow document edits. Retain their native callbacks
// only while the source structure, literal identities and actual IDE offsets
// still agree. Rebuild on structural edits, invalidated anchors or caret-driven
// anchor movement. The shader geometry is refreshed from the new catalog either
// way, so retained color glyphs show the current value.
fn refresh_placed(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    path: &str,
    catalog: &Catalog,
) -> Result<()> {
    if reusable_placed(project, j, path, catalog)? {
        return Ok(());
    }
    clear_placed(project, j)?;
    place(project, j, editor, path, catalog)
}

fn reusable_placed(
    project: &Project,
    j: &mut J<'_>,
    path: &str,
    catalog: &Catalog,
) -> Result<bool> {
    let state = project.authoring.lock().expect("authoring");
    let Some(previous) = state
        .current
        .as_ref()
        .filter(|p| p.path == path)
        .and_then(|p| p.catalog.as_ref())
    else {
        return Ok(false);
    };
    if previous.schema != catalog.schema {
        return Ok(false);
    }
    let previews = |c: &Catalog| c.functions.iter().filter(|f| f.preview).count();
    let expected =
        previews(catalog) + (catalog.literals.len() + catalog.references.len()).min(1024);
    if state.placed.len() != expected || previews(previous) != previews(catalog) {
        return Ok(false);
    }
    let mut targets = catalog
        .literals
        .iter()
        .map(|l| (l.id, l.range.end_utf16))
        .chain(
            catalog
                .references
                .iter()
                .map(|r| (r.literal, r.range.end_utf16)),
        )
        .take(1024);
    for item in &state.placed {
        if !j.bool(&item.object, "isValid")? {
            return Ok(false);
        }
        if let Some(id) = item.literal {
            let Some((literal, offset)) = targets.next() else {
                return Ok(false);
            };
            if id != literal || j.int(&item.object, "getOffset")? as usize != offset {
                return Ok(false);
            }
        }
    }
    Ok(targets.next().is_none())
}

fn place(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    path: &str,
    catalog: &Catalog,
) -> Result<()> {
    let batch = batch_placement(j, editor, catalog)?;
    place_with_batch(project, j, editor, path, catalog, batch)
}

fn batch_placement(j: &mut J<'_>, editor: &O, catalog: &Catalog) -> Result<bool> {
    let count = (catalog.literals.len() + catalog.references.len()).min(1024);
    if count < 256 {
        return Ok(false);
    }
    // Batch setup scans the document. A sparse 840 KB file was slower even
    // with 1,024 glyphs. Restrict it to the measured dense-file range.
    let document = j.obj(
        editor,
        "getDocument",
        "()Lcom/intellij/openapi/editor/Document;",
        &[],
    )?;
    if j.int(&document, "getTextLength")? as usize > count * 64 {
        return Ok(false);
    }
    // IntelliJ documents different visual caret behavior for a batch at the
    // caret's offset. Keep the established insertion behavior for every caret.
    // Query this before entering the batch, where caret access is forbidden.
    let model = j.obj(
        editor,
        "getCaretModel",
        "()Lcom/intellij/openapi/editor/CaretModel;",
        &[],
    )?;
    let carets = j.obj(&model, "getAllCarets", "()Ljava/util/List;", &[])?;
    for index in 0..j.int(&carets, "size")? {
        let caret = j.obj(&carets, "get", "(I)Ljava/lang/Object;", &[A::I(index)])?;
        let offset = j.int(&caret, "getOffset")? as usize;
        if catalog
            .literals
            .iter()
            .map(|l| l.range.end_utf16)
            .chain(catalog.references.iter().map(|r| r.range.end_utf16))
            .take(1024)
            .any(|end| end == offset)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn place_with_batch(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    path: &str,
    catalog: &Catalog,
    batch: bool,
) -> Result<()> {
    let markup = j.obj(
        editor,
        "getMarkupModel",
        "()Lcom/intellij/openapi/editor/markup/MarkupModel;",
        &[],
    )?;
    for function in catalog.functions.iter().filter(|f| f.preview) {
        let weak = Arc::downgrade(project);
        let path = path.to_owned();
        let name = function.name.clone();
        let identity = Arc::new(std::sync::atomic::AtomicI64::new(0));
        let captured = identity.clone();
        let id = jvm::register(move |j, op, _args| match op {
            "PreviewGutter.getTooltipText" => j.string(&format!("Preview {name} · Cranpose")),
            "PreviewGutter.getClickAction" => j.new(
                "dev/cranpose/rust/PreviewClick",
                "(J)V",
                &[A::J(captured.load(std::sync::atomic::Ordering::Relaxed))],
            ),
            "PreviewClick.actionPerformed" => {
                if let Some(p) = weak.upgrade() {
                    workspace::show(&p, j, &path, Some(&name), None)?;
                }
                j.null()
            }
            _ => j.null(),
        });
        identity.store(id, std::sync::atomic::Ordering::Relaxed);
        let renderer = j.new("dev/cranpose/rust/PreviewGutter", "(J)V", &[A::J(id)])?;
        let highlighter=j.obj(&markup,"addLineHighlighter","(IILcom/intellij/openapi/editor/markup/TextAttributes;)Lcom/intellij/openapi/editor/markup/RangeHighlighter;",&[A::I(function.range.line as i32-1),A::I(5000),A::Null])?;
        j.void(
            &highlighter,
            "setGutterIconRenderer",
            "(Lcom/intellij/openapi/editor/markup/GutterIconRenderer;)V",
            &[A::O(&renderer)],
        )?;
        project
            .authoring
            .lock()
            .expect("authoring")
            .placed
            .push(Placed {
                object: highlighter,
                callback: id,
                literal: None,
            });
    }
    let model = j.obj(
        editor,
        "getInlayModel",
        "()Lcom/intellij/openapi/editor/InlayModel;",
        &[],
    )?;
    let targets = catalog
        .literals
        .iter()
        .map(|l| (l.id, l.range.end_utf16))
        .chain(
            catalog
                .references
                .iter()
                .map(|r| (r.literal, r.range.end_utf16)),
        )
        .take(1024)
        .collect::<Vec<_>>();
    let project = project.clone();
    let batch_model = model.clone();
    crate::inlays::execute(j, &batch_model, batch, move |j| {
        for (literal, end) in targets {
            let id = jvm::register(|j, op, _| {
                if op == "ValueGlyph.calcWidthInPixels" {
                    j.boxed_int(14)
                } else {
                    j.null()
                }
            });
            let renderer = j.new("dev/cranpose/rust/ValueGlyph", "(J)V", &[A::J(id)])?;
            let inlay=j.obj(&model,"addInlineElement","(IZLcom/intellij/openapi/editor/EditorCustomElementRenderer;)Lcom/intellij/openapi/editor/Inlay;",&[A::I(end as i32),A::Z(true),A::O(&renderer)])?;
            if inlay.is_null() {
                jvm::unregister(id);
            } else {
                project
                    .authoring
                    .lock()
                    .expect("authoring")
                    .placed
                    .push(Placed {
                        object: inlay,
                        callback: id,
                        literal: Some(literal),
                    });
            }
        }
        Ok(())
    })
}

fn ensure_panel(project: &Arc<Project>, j: &mut J<'_>) -> Result<Arc<Panel>> {
    if let Some(panel) = &project.authoring.lock().expect("authoring").panel {
        return Ok(panel.clone());
    }
    let mut options = project::options(j, false)?;
    options
        .environment
        .insert("CRANPOSE_AUTHORING".into(), "overlay".into());
    let panel = Panel::new(j, options)?;
    let weak = Arc::downgrade(project);
    *panel.on_lifecycle.lock().expect("lifecycle") =
        Some(Arc::new(move |j, panel, connected, _| {
            if let Some(project) = weak.upgrade() {
                {
                    let mut state = project.authoring.lock().expect("authoring");
                    state.geometry.clear();
                    state.overlay_ready = false;
                    state.overlay_id = None;
                }
                if let (_, Some(editor)) = project.selected(j)? {
                    let component = j.obj(
                        &editor,
                        "getContentComponent",
                        "()Ljavax/swing/JComponent;",
                        &[],
                    )?;
                    // Cranpose's primary drives the composition that owns HostOverlay.
                    // It stays one pixel and unattached, but follows editor visibility.
                    if connected {
                        panel.send(
                            crate::protocol::Packet::new(13)
                                .int(0)
                                .byte(u8::from(j.bool(&component, "isShowing")?)),
                        );
                    }
                    j.void(&component, "repaint", "()V", &[])?;
                }
            }
            Ok(())
        }));
    let weak = Arc::downgrade(project);
    *panel.on_message.lock().expect("message") =
        Some(Arc::new(move |j, panel, channel, payload| {
            if let Some(p) = weak.upgrade()
                && channel == "host.overlay"
            {
                editor::attach_overlay(&p, j, panel, payload)?;
                let overlay: Value = serde_json::from_str(payload)?;
                if overlay["anchor"] == "window" {
                    if let Some(surface) = overlay["surface"]
                        .as_u64()
                        .and_then(|id| panel.overlay_surface(id as u32))
                    {
                        crate::feedback::attach(&p, panel, &surface);
                    }
                    return Ok(());
                }
                {
                    let mut state = p.authoring.lock().expect("authoring");
                    state.geometry.clear();
                    state.overlay_ready = true;
                    state.overlay_id = serde_json::from_str::<Value>(payload)?["surface"]
                        .as_u64()
                        .map(|id| id as u32);
                }
                if let (_, Some(editor)) = p.selected(j)? {
                    let component = j.obj(
                        &editor,
                        "getContentComponent",
                        "()Ljavax/swing/JComponent;",
                        &[],
                    )?;
                    j.void(&component, "repaint", "()V", &[])?;
                }
            }
            Ok(())
        }));
    project.attach(j, &panel)?;
    panel.start(j)?;
    project.authoring.lock().expect("authoring").panel = Some(panel.clone());
    Ok(panel)
}

fn geometry(project: &Arc<Project>, j: &mut J<'_>, editor: &O, stamp: i64) -> Result<()> {
    let scrolling = j.obj(
        editor,
        "getScrollingModel",
        "()Lcom/intellij/openapi/editor/ScrollingModel;",
        &[],
    )?;
    let area = j.obj(&scrolling, "getVisibleArea", "()Ljava/awt/Rectangle;", &[])?;
    let x = j.field_int(&area, "x")?;
    let y = j.field_int(&area, "y")?;
    let w = j.field_int(&area, "width")?;
    let h = j.field_int(&area, "height")?;
    let height = j.int(editor, "getLineHeight")?;
    let component = j.obj(
        editor,
        "getContentComponent",
        "()Ljavax/swing/JComponent;",
        &[],
    )?;
    let visible = j.bool(&component, "isShowing")?;
    let key = format!("{stamp}:{x}:{y}:{w}:{h}:{height}:{visible}");
    let mut state = project.authoring.lock().expect("authoring");
    if state.geometry == key {
        return Ok(());
    }
    let Some(parsed) = &state.current else {
        return Ok(());
    };
    if parsed.stamp != stamp {
        return Ok(());
    }
    if state.panel.is_none()
        && parsed
            .catalog
            .as_ref()
            .is_none_or(|c| c.functions.is_empty() && c.literals.is_empty())
    {
        return Ok(());
    }
    let mut items = vec![];
    for (anchor, placed) in state.placed.iter().enumerate() {
        if let Some(id) = placed.literal
            && j.bool(&placed.object, "isValid")?
        {
            let rect = j.obj(&placed.object, "getBounds", "()Ljava/awt/Rectangle;", &[])?;
            if rect.is_null() {
                continue;
            }
            let top = j.field_int(&rect, "y")? - y;
            if top + height < 0 || top > h {
                continue;
            }
            let literal = parsed.catalog.as_ref().and_then(|c| c.literals.get(id));
            let color = literal.filter(|v| v.kind == "color");
            items.push(json!({"kind":if color.is_some(){"color"}else{"value"},"value":color.map(|v|&v.value),"x":j.field_int(&rect,"x")?-x,"y":top,"width":14,"height":height,"label":"◆","id":id,"anchor":anchor}));
        }
    }
    let folding = j.obj(
        editor,
        "getFoldingModel",
        "()Lcom/intellij/openapi/editor/FoldingModel;",
        &[],
    )?;
    for call in parsed
        .catalog
        .iter()
        .flat_map(|c| c.calls.iter())
        .take(1024)
    {
        if j.call(
            &folding,
            "isOffsetCollapsed",
            "(I)Z",
            &[A::I(call.range.start_utf16 as i32)],
        )?
        .z()?
        {
            continue;
        }
        let point = j.obj(
            editor,
            "offsetToXY",
            "(I)Ljava/awt/Point;",
            &[A::I(call.range.start_utf16 as i32)],
        )?;
        let top = j.field_int(&point, "y")? - y;
        if top + height < 0 || top > h {
            continue;
        }
        let end = j.obj(
            editor,
            "offsetToXY",
            "(I)Ljava/awt/Point;",
            &[A::I(call.range.end_utf16 as i32)],
        )?;
        items.push(json!({"kind":"call","x":j.field_int(&point,"x")?-x,"y":top+height-2,"width":(j.field_int(&end,"x")?-j.field_int(&point,"x")?).max(1),"height":2}));
        if items.len() >= 256 {
            break;
        }
    }
    let mut arrival = None;
    if state.overlay_ready
        && let Some(Arrival {
            path,
            line,
            request,
            created,
            ..
        }) = &state.arrival
        && path == &parsed.path
        && created.elapsed() < std::time::Duration::from_secs(5)
    {
        let document = j.obj(
            editor,
            "getDocument",
            "()Lcom/intellij/openapi/editor/Document;",
            &[],
        )?;
        let line = (*line).clamp(0, j.int(&document, "getLineCount")?.saturating_sub(1));
        let offset = j
            .call(&document, "getLineStartOffset", "(I)I", &[A::I(line)])?
            .i()?;
        let point = j.obj(editor, "offsetToXY", "(I)Ljava/awt/Point;", &[A::I(offset)])?;
        let top = j.field_int(&point, "y")? - y;
        if top + height >= 0 && top < h {
            arrival = Some(
                json!({"kind":"arrival","request":request,"x":0,"y":top,"width":w,"height":height}),
            );
        }
    }
    if arrival.is_some()
        && let Some(a) = &mut state.arrival
    {
        a.shown.get_or_insert_with(std::time::Instant::now);
    }
    state.geometry = key;
    drop(state);
    let mut badges = crate::stability::decorations(project, j, editor, x, y, h)?;
    if let Some(arrival) = arrival {
        badges.insert(0, arrival);
    }
    badges.extend(items);
    let items = badges.into_iter().take(256).collect::<Vec<_>>();
    let panel = ensure_panel(project, j)?;
    if let Some(id) = project.authoring.lock().expect("authoring").overlay_id
        && let Some(surface) = panel.overlay_surface(id)
    {
        j.void(surface.component(), "setVisible", "(Z)V", &[A::Z(visible)])?;
    }
    panel.send(
        crate::protocol::Packet::new(13)
            .int(0)
            .byte(u8::from(visible)),
    );
    panel.message(
        "ide.authoring.geometry",
        &json!({"items":items}).to_string(),
    );
    Ok(())
}

fn pointer(project: &Arc<Project>, j: &mut J<'_>, event: &O, focus: bool) -> Result<()> {
    if j.bool(event, "isConsumed")? {
        return Ok(());
    }
    let area = j.obj(
        event,
        "getArea",
        "()Lcom/intellij/openapi/editor/event/EditorMouseEventArea;",
        &[],
    )?;
    if j.text(&area, "toString")? != "EDITING_AREA" {
        return Ok(());
    }
    let editor = j.obj(
        event,
        "getEditor",
        "()Lcom/intellij/openapi/editor/Editor;",
        &[],
    )?;
    let owner = j.obj(
        &editor,
        "getProject",
        "()Lcom/intellij/openapi/project/Project;",
        &[],
    )?;
    if !j.same(&owner, &project.object)? {
        return Ok(());
    }
    let mouse = j.obj(event, "getMouseEvent", "()Ljava/awt/event/MouseEvent;", &[])?;
    if (focus && j.int(&mouse, "getButton")? != 1)
        || (!focus && j.int(&mouse, "getModifiersEx")? != 0)
    {
        return Ok(());
    }
    let point = j.obj(&mouse, "getPoint", "()Ljava/awt/Point;", &[])?;
    let model = j.obj(
        &editor,
        "getInlayModel",
        "()Lcom/intellij/openapi/editor/InlayModel;",
        &[],
    )?;
    let class = j.class("dev/cranpose/rust/ValueGlyph")?;
    let inlay = j.obj(
        &model,
        "getElementAt",
        "(Ljava/awt/Point;Ljava/lang/Class;)Lcom/intellij/openapi/editor/Inlay;",
        &[A::O(&point), A::O(&class)],
    )?;
    let mut state = project.authoring.lock().expect("authoring");
    let literal = if inlay.is_null() {
        None
    } else {
        state
            .placed
            .iter()
            .find_map(|p| j.same(&p.object, &inlay).ok().filter(|v| *v).and(p.literal))
    };
    // The glyph is the control's explicit hit target. Source text belongs to
    // caret placement and selection, including a click while a popup is open.
    if literal.is_none() && focus {
        state.hovered = None;
        drop(state);
        return dismiss_control(project, j);
    }
    if !focus && state.hovered == literal {
        return Ok(());
    }
    if focus
        && literal.is_some()
        && state.hovered == literal
        && let Some((popup, panel)) = &state.popup
        && !j.bool(popup, "isDisposed")?
    {
        j.bool(panel.primary.component(), "requestFocusInWindow")?;
        return Ok(());
    }
    state.hovered = literal;
    let Some(id) = literal else {
        return Ok(());
    };
    let Some(parsed) = &state.current else {
        return Ok(());
    };
    let Some(catalog) = parsed.catalog.clone() else {
        return Ok(());
    };
    let source = parsed.source.clone();
    drop(state);
    let document = j.obj(
        &editor,
        "getDocument",
        "()Lcom/intellij/openapi/editor/Document;",
        &[],
    )?;
    // A parse can be in flight while the pointer moves. Wait for a fresh catalog.
    if j.text(&document, "getText")? != source {
        project.authoring.lock().expect("authoring").hovered = None;
        return Ok(());
    }
    open_control(project, j, &editor, &point, (id, catalog, source), focus)
}

fn dismiss_control(project: &Project, j: &mut J<'_>) -> Result<()> {
    let old = {
        let mut state = project.authoring.lock().expect("authoring");
        let old = state.popup.take();
        if old.is_some() {
            state.control_request = state.control_request.wrapping_add(1);
            state.control_init = None;
        }
        old
    };
    if let Some((popup, panel)) = old {
        if !j.bool(&popup, "isDisposed")? {
            j.void(&popup, "cancel", "()V", &[])?;
        }
        *panel.on_message.lock().expect("message") = None;
        panel.send(crate::protocol::Packet::new(13).int(0).byte(0));
        panel.primary.clear_frame(j)?;
    }
    Ok(())
}

/// One hidden, warmed renderer per project; hover never spawns another process.
fn ensure_control(project: &Arc<Project>, j: &mut J<'_>) -> Result<Arc<Panel>> {
    if let Some(panel) = &project.authoring.lock().expect("authoring").control {
        return Ok(panel.clone());
    }
    let mut options = project::options(j, false)?;
    options
        .environment
        .insert("CRANPOSE_AUTHORING".into(), "value".into());
    let panel = Panel::new(j, options)?;
    let weak = Arc::downgrade(project);
    *panel.on_lifecycle.lock().expect("lifecycle") =
        Some(Arc::new(move |_, panel, connected, _| {
            if connected && let Some(project) = weak.upgrade() {
                let init = project
                    .authoring
                    .lock()
                    .expect("authoring")
                    .control_init
                    .clone();
                if let Some(init) = init {
                    panel.message("ide.authoring.control", &init);
                } else {
                    panel.send(crate::protocol::Packet::new(13).int(0).byte(0));
                }
            }
            Ok(())
        }));
    project.attach(j, &panel)?;
    panel.start(j)?;
    project.authoring.lock().expect("authoring").control = Some(panel.clone());
    Ok(panel)
}

fn open_control(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    point: &O,
    target: (usize, Catalog, String),
    focus: bool,
) -> Result<()> {
    let (id, catalog, source) = target;
    dismiss_control(project, j)?;
    let document = j.obj(
        editor,
        "getDocument",
        "()Lcom/intellij/openapi/editor/Document;",
        &[],
    )?;
    ensure!(
        j.text(&document, "getText")? == source,
        "Source changed; reopen the live value"
    );
    let literal = catalog
        .literals
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("Live value disappeared"))?
        .clone();
    let (width, height) = match literal.kind.as_str() {
        "color" => (400, 490),
        "int" | "float" => (400, 380),
        _ => (340, 240),
    };
    let control_group = format!(
        "cranpose-control-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let panel = ensure_control(project, j)?;
    let request = {
        let mut state = project.authoring.lock().expect("authoring");
        state.control_request = state.control_request.wrapping_add(1);
        state.control_request
    };
    let binding = catalog
        .references
        .iter()
        .find(|r| r.literal == id)
        .map(|r| r.name.as_str());
    let init = json!({"literal":literal,"request":request,"binding":binding}).to_string();
    project.authoring.lock().expect("authoring").control_init = Some(init.clone());
    panel.primary.clear_frame(j)?;
    panel.message("ide.authoring.control", &init);
    let weak = Arc::downgrade(project);
    let expected = Arc::new(std::sync::Mutex::new((catalog, source)));
    let pending = Arc::new(std::sync::Mutex::new(None::<String>));
    *panel.on_message.lock().expect("message") = Some(Arc::new(
        move |j, panel, channel, payload| {
            if channel != "ide.authoring.edit" {
                return Ok(());
            }
            let Some(project) = weak.upgrade() else {
                return Ok(());
            };
            if serde_json::from_str::<Value>(payload)?["request"].as_u64() != Some(request) {
                return Ok(());
            }
            if pending
                .lock()
                .expect("pending edit")
                .replace(payload.to_owned())
                .is_some()
            {
                return Ok(());
            }
            let pending = pending.clone();
            let panel = panel.clone();
            let document = document.clone();
            let expected = expected.clone();
            let control_group = control_group.clone();
            jvm::later(j, move |j| {
                if project.closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(());
                }
                if project.authoring.lock().expect("authoring").control_request != request {
                    return Ok(());
                }
                let popup = project
                    .authoring
                    .lock()
                    .expect("authoring")
                    .popup
                    .as_ref()
                    .map(|p| p.0.clone());
                if let Some(popup) = popup
                    && j.bool(&popup, "isDisposed")?
                {
                    return Ok(());
                }
                let Some(payload) = pending.lock().expect("pending edit").take() else {
                    return Ok(());
                };
                let result = (|| -> Result<()> {
                    let request: Value = serde_json::from_str(&payload)?;
                    let value = request["value"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing value"))?;
                    let group = request["gesture"]
                        .as_u64()
                        .map(|g| format!("{control_group}-{g}"));
                    let mut expected = expected.lock().expect("control");
                    let current = j.text(&document, "getText")?;
                    ensure!(
                        current == expected.1,
                        "Source changed; reopen the live value"
                    );
                    let next = expected.0.replacement(&current, id, value)?;
                    let next_catalog = Catalog::parse(&next)?;
                    let before = &expected.0.literals[id];
                    let after = &next_catalog.literals[id];
                    let replacement = next[after.range.start..after.range.end].to_owned();
                    let (start, end) = (
                        before.range.start_utf16 as i32,
                        before.range.end_utf16 as i32,
                    );
                    let doc = document.clone();
                    let callback_id = jvm::register(move |j, _, _| {
                        j.void(
                            &doc,
                            "replaceString",
                            "(IILjava/lang/CharSequence;)V",
                            &[A::I(start), A::I(end), A::S(&replacement)],
                        )?;
                        j.null()
                    });
                    let callback = jvm::callback(j, callback_id)?;
                    let files = j.array("com/intellij/psi/PsiFile", &[])?;
                    let result=j.static_void("com/intellij/openapi/command/WriteCommandAction","runWriteCommandAction","(Lcom/intellij/openapi/project/Project;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Runnable;[Lcom/intellij/psi/PsiFile;)V",&[A::O(&project.object),A::S("Tune Cranpose value"),group.as_deref().map_or(A::Null,A::S),A::O(&callback),A::O(&files)]);
                    jvm::unregister(callback_id);
                    result?;
                    *expected = (next_catalog, next);
                    Ok(())
                })();
                panel.message(
                    "ide.authoring.result",
                    &match result {
                        Ok(()) => json!({"ok":true,"request":request}),
                        Err(e) => json!({"ok":false,"error":e.to_string(),"request":request}),
                    }
                    .to_string(),
                );
                Ok(())
            })
        },
    ));
    let size = j.new("java/awt/Dimension", "(II)V", &[A::I(width), A::I(height)])?;
    j.void(
        panel.primary.component(),
        "setPreferredSize",
        "(Ljava/awt/Dimension;)V",
        &[A::O(&size)],
    )?;
    let factory = j.static_obj(
        "com/intellij/openapi/ui/popup/JBPopupFactory",
        "getInstance",
        "()Lcom/intellij/openapi/ui/popup/JBPopupFactory;",
        &[],
    )?;
    let builder=j.obj(&factory,"createComponentPopupBuilder","(Ljavax/swing/JComponent;Ljavax/swing/JComponent;)Lcom/intellij/openapi/ui/popup/ComponentPopupBuilder;",&[A::O(panel.primary.component()),A::O(panel.primary.component())])?;
    j.obj(
        &builder,
        "setRequestFocus",
        "(Z)Lcom/intellij/openapi/ui/popup/ComponentPopupBuilder;",
        &[A::Z(focus)],
    )?;
    let popup = j.obj(
        &builder,
        "createPopup",
        "()Lcom/intellij/openapi/ui/popup/JBPopup;",
        &[],
    )?;
    let component = j.obj(
        editor,
        "getContentComponent",
        "()Ljavax/swing/JComponent;",
        &[],
    )?;
    let x = j.field_int(point, "x")?;
    let y = j.field_int(point, "y")? + j.int(editor, "getLineHeight")? / 2 + 4;
    let below = j.new("java/awt/Point", "(II)V", &[A::I(x), A::I(y)])?;
    let relative = j.new(
        "com/intellij/ui/awt/RelativePoint",
        "(Ljava/awt/Component;Ljava/awt/Point;)V",
        &[A::O(&component), A::O(&below)],
    )?;
    #[cfg(feature = "ide-tests")]
    let show = !j
        .static_call("java/awt/GraphicsEnvironment", "isHeadless", "()Z", &[])?
        .z()?;
    #[cfg(not(feature = "ide-tests"))]
    let show = true;
    if show {
        j.void(
            &popup,
            "show",
            "(Lcom/intellij/ui/awt/RelativePoint;)V",
            &[A::O(&relative)],
        )?;
    }
    panel.send(crate::protocol::Packet::new(13).int(0).byte(1));
    project.authoring.lock().expect("authoring").popup = Some((popup, panel));
    Ok(())
}

#[cfg(feature = "ide-tests")]
pub fn integration_test(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    use anyhow::Context;
    let source = "// 🦀\n#[composable]\nfn Card() { Text(\"Café 🦀\"); Space(12); }\nfn palette(){ Color(0.1,0.2,0.3,1.0); }";
    let factory = j.static_obj(
        "com/intellij/openapi/editor/EditorFactory",
        "getInstance",
        "()Lcom/intellij/openapi/editor/EditorFactory;",
        &[],
    )?;
    let document = j.obj(
        &factory,
        "createDocument",
        "(Ljava/lang/CharSequence;)Lcom/intellij/openapi/editor/Document;",
        &[A::S(source)],
    )?;
    let editor=j.obj(&factory,"createEditor","(Lcom/intellij/openapi/editor/Document;Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/editor/Editor;",&[A::O(&document),A::O(&project.object)])?;
    let result = (|| -> Result<()> {
        watch_editor(project, j, &editor)?;
        place(project, j, &editor, "test.rs", &Catalog::parse(source)?)?;
        let items = project
            .authoring
            .lock()
            .expect("authoring")
            .placed
            .iter()
            .map(|p| (p.object.clone(), p.literal))
            .collect::<Vec<_>>();
        ensure!(
            items.len() == 4,
            "Expected one preview marker, two scalar glyphs and one grouped color glyph"
        );
        let mut retired = Vec::new();
        for (object, literal) in items {
            if let Some(id) = literal {
                let token = ["\"Café 🦀\"", "12", "Color(0.1,0.2,0.3,1.0)"][id];
                let byte_end = source.find(token).expect("fixture token") + token.len();
                ensure!(
                    j.int(&object, "getOffset")? as usize
                        == source[..byte_end].encode_utf16().count(),
                    "Live glyph must follow its literal in IDE UTF-16 coordinates"
                );
                project.authoring.lock().expect("authoring").geometry = "cached".into();
                j.void(&object, "update", "()V", &[])?;
                // Some IDE versions suppress unchanged update notifications;
                // removal must always move following editor content.
                ensure!(
                    j.int(&object, "getWidthInPixels")? == 14,
                    "Live glyph must reserve editor space"
                );
                j.void(&object, "dispose", "()V", &[])?;
                ensure!(
                    project
                        .authoring
                        .lock()
                        .expect("authoring")
                        .geometry
                        .is_empty(),
                    "Removed inlays must invalidate overlay geometry"
                );
            } else {
                let renderer = j.obj(
                    &object,
                    "getGutterIconRenderer",
                    "()Lcom/intellij/openapi/editor/markup/GutterIconRenderer;",
                    &[],
                )?;
                ensure!(
                    !j.obj(&renderer, "getIcon", "()Ljavax/swing/Icon;", &[])?
                        .is_null(),
                    "Missing preview gutter icon"
                );
                ensure!(
                    !j.obj(
                        &renderer,
                        "getClickAction",
                        "()Lcom/intellij/openapi/actionSystem/AnAction;",
                        &[]
                    )?
                    .is_null(),
                    "Missing preview action"
                );
                ensure!(
                    j.text(&renderer, "getTooltipText")?.contains("Card"),
                    "Missing composable name"
                );
                ensure!(
                    j.call(
                        &renderer,
                        "equals",
                        "(Ljava/lang/Object;)Z",
                        &[A::O(&renderer)]
                    )?
                    .z()?,
                    "Gutter renderer identity"
                );
                let action = j.obj(
                    &renderer,
                    "getClickAction",
                    "()Lcom/intellij/openapi/actionSystem/AnAction;",
                    &[],
                )?;
                let hash = j.int(&renderer, "hashCode")?;
                retired.push((renderer, action, hash));
            }
        }
        clear_placed(project, j)?;
        // Model the IDE's delayed paint/action caches: native state is gone,
        // but retained Java objects must still fulfill their non-null contracts.
        for (renderer, action, hash) in retired {
            let icon = j.obj(&renderer, "getIcon", "()Ljavax/swing/Icon;", &[])?;
            ensure!(
                !icon.is_null() && j.int(&icon, "getIconWidth")? > 0,
                "Retired gutter renderer must keep a paintable icon"
            );
            ensure!(
                j.int(&renderer, "hashCode")? == hash
                    && j.call(
                        &renderer,
                        "equals",
                        "(Ljava/lang/Object;)Z",
                        &[A::O(&renderer)]
                    )?
                    .z()?
                    && j.bool(&renderer, "isNavigateAction")?,
                "Retired gutter metadata must stay stable"
            );
            ensure!(
                j.obj(&renderer, "getTooltipText", "()Ljava/lang/String;", &[])?
                    .is_null()
                    && j.obj(
                        &renderer,
                        "getClickAction",
                        "()Lcom/intellij/openapi/actionSystem/AnAction;",
                        &[]
                    )?
                    .is_null(),
                "Disposed gutter callbacks must release their native state"
            );
            ensure!(
                !j.obj(
                    &action,
                    "getActionUpdateThread",
                    "()Lcom/intellij/openapi/actionSystem/ActionUpdateThread;",
                    &[]
                )?
                .is_null(),
                "Retired action must keep its update-thread contract"
            );
            j.void(
                &action,
                "actionPerformed",
                "(Lcom/intellij/openapi/actionSystem/AnActionEvent;)V",
                &[A::Null],
            )?;
        }
        // Real Editor offsets and mouse events resolve aliases after Unicode;
        // repeated hover reuses both the popup and its one native renderer.
        let aliases = "// 🦀\n#[composable]\nfn Card(){ let tint = Color(0.19,0.42,0.31,0.5); let spacing = 24.0; Text(tint); Space(spacing); }";
        let doc = document.clone();
        let write_id = jvm::register(move |j, _, _| {
            j.void(
                &doc,
                "setText",
                "(Ljava/lang/CharSequence;)V",
                &[A::S(aliases)],
            )?;
            j.null()
        });
        let write = jvm::callback(j, write_id)?;
        let application = j.static_obj(
            "com/intellij/openapi/application/ApplicationManager",
            "getApplication",
            "()Lcom/intellij/openapi/application/Application;",
            &[],
        )?;
        let written = j.void(
            &application,
            "runWriteAction",
            "(Ljava/lang/Runnable;)V",
            &[A::O(&write)],
        );
        jvm::unregister(write_id);
        written?;
        let catalog = Catalog::parse(aliases)?;
        place(project, j, &editor, "test.rs", &catalog)?;
        let reference = catalog
            .references
            .iter()
            .find(|r| r.name == "tint" && r.range.start > aliases.find("Text(").expect("call"))
            .expect("color reference");
        let offset = reference.range.start_utf16;
        let glyph_offset = reference.range.end_utf16;
        let reference_count = project
            .authoring
            .lock()
            .expect("authoring")
            .placed
            .iter()
            .filter(|p| p.literal.is_some())
            .count();
        ensure!(
            reference_count == catalog.literals.len() + catalog.references.len(),
            "Alias glyphs missing"
        );
        let stamp = j.long(&document, "getModificationStamp")?;
        project.authoring.lock().expect("authoring").current = Some(Parsed {
            path: "test.rs".into(),
            stamp,
            source: aliases.into(),
            catalog: Some(catalog),
        });
        let component = j.obj(
            &editor,
            "getContentComponent",
            "()Ljavax/swing/JComponent;",
            &[],
        )?;
        let point = j.obj(
            &editor,
            "offsetToXY",
            "(I)Ljava/awt/Point;",
            &[A::I(offset as i32)],
        )?;
        let x = j.field_int(&point, "x")? + 2;
        let y = j.field_int(&point, "y")? + 2;
        let mouse = j.new(
            "java/awt/event/MouseEvent",
            "(Ljava/awt/Component;IJIIIIZ)V",
            &[
                A::O(&component),
                A::I(503),
                A::J(0),
                A::I(0),
                A::I(x),
                A::I(y),
                A::I(0),
                A::Z(false),
            ],
        )?;
        let area = j.constant(
            "com/intellij/openapi/editor/event/EditorMouseEventArea",
            "EDITING_AREA",
            "Lcom/intellij/openapi/editor/event/EditorMouseEventArea;",
        )?;
        let event=j.new("com/intellij/openapi/editor/event/EditorMouseEvent","(Lcom/intellij/openapi/editor/Editor;Ljava/awt/event/MouseEvent;Lcom/intellij/openapi/editor/event/EditorMouseEventArea;)V",&[A::O(&editor),A::O(&mouse),A::O(&area)])?;
        pointer(project, j, &event, false)?;
        ensure!(
            project.authoring.lock().expect("authoring").popup.is_none(),
            "Hovering value text must leave the caret area clear"
        );
        let text_event = event;
        let glyph = project
            .authoring
            .lock()
            .expect("authoring")
            .placed
            .iter()
            .filter(|p| p.literal.is_some())
            .find(|p| j.int(&p.object, "getOffset").ok() == Some(glyph_offset as i32))
            .map(|p| p.object.clone())
            .context("Alias glyph")?;
        let bounds = j.obj(&glyph, "getBounds", "()Ljava/awt/Rectangle;", &[])?;
        let glyph_x = j.field_int(&bounds, "x")? + 2;
        let glyph_y = j.field_int(&bounds, "y")? + 2;
        let mouse = j.new(
            "java/awt/event/MouseEvent",
            "(Ljava/awt/Component;IJIIIIZ)V",
            &[
                A::O(&component),
                A::I(503),
                A::J(0),
                A::I(0),
                A::I(glyph_x),
                A::I(glyph_y),
                A::I(0),
                A::Z(false),
            ],
        )?;
        let event=j.new("com/intellij/openapi/editor/event/EditorMouseEvent","(Lcom/intellij/openapi/editor/Editor;Ljava/awt/event/MouseEvent;Lcom/intellij/openapi/editor/event/EditorMouseEventArea;)V",&[A::O(&editor),A::O(&mouse),A::O(&area)])?;
        pointer(project, j, &event, false)?;
        let (popup, panel) = project
            .authoring
            .lock()
            .expect("authoring")
            .popup
            .clone()
            .context("Hover did not open a control")?;
        pointer(project, j, &event, false)?;
        let again = project
            .authoring
            .lock()
            .expect("authoring")
            .popup
            .clone()
            .context("Hover popup disappeared")?;
        ensure!(
            j.same(&popup, &again.0)? && Arc::ptr_eq(&panel, &again.1),
            "Same hover recreated popup"
        );
        let callback = panel
            .on_message
            .lock()
            .expect("message")
            .clone()
            .expect("edit callback");
        callback(
            j,
            &panel,
            "ide.authoring.edit",
            "{\"request\":0,\"value\":\"0,0,0,0\"}",
        )?;
        ensure!(
            j.text(&document, "getText")? == aliases,
            "Stale control changed source"
        );
        dismiss_control(project, j)?;
        ensure!(
            panel.on_message.lock().expect("message").is_none(),
            "Closed popup retained document callback"
        );
        ensure!(
            Arc::ptr_eq(&panel, &ensure_control(project, j)?),
            "Renderer was replaced on dismissal"
        );
        pointer(project, j, &event, false)?;
        ensure!(
            project.authoring.lock().expect("authoring").popup.is_none(),
            "Dismissed hover reopened before pointer left"
        );
        pointer(project, j, &text_event, false)?;
        pointer(project, j, &event, false)?;
        ensure!(
            project.authoring.lock().expect("authoring").popup.is_some(),
            "Glyph hover must still open immediately after leaving it"
        );
        let click = j.new(
            "java/awt/event/MouseEvent",
            "(Ljava/awt/Component;IJIIIIZI)V",
            &[
                A::O(&component),
                A::I(500),
                A::J(0),
                A::I(0),
                A::I(x),
                A::I(y),
                A::I(1),
                A::Z(false),
                A::I(1),
            ],
        )?;
        let clicked=j.new("com/intellij/openapi/editor/event/EditorMouseEvent","(Lcom/intellij/openapi/editor/Editor;Ljava/awt/event/MouseEvent;Lcom/intellij/openapi/editor/event/EditorMouseEventArea;)V",&[A::O(&editor),A::O(&click),A::O(&area)])?;
        pointer(project, j, &clicked, true)?;
        ensure!(
            project.authoring.lock().expect("authoring").popup.is_none(),
            "Clicking source must dismiss the control without stealing focus"
        );
        panel.close(j)?;
        project.authoring.lock().expect("authoring").control = None;
        Ok(())
    })();
    if result.is_err() && j.env.exception_check()? {
        j.env.exception_describe()?;
        j.env.exception_clear()?;
    }
    unwatch_editor(project, j)?;
    clear_placed(project, j)?;
    j.void(
        &factory,
        "releaseEditor",
        "(Lcom/intellij/openapi/editor/Editor;)V",
        &[A::O(&editor)],
    )?;
    result
}
