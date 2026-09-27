//! Document-based preview markers and native Cranpose value controls.
//! Parsing runs off the EDT; unchanged documents and viewports do no work.
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
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub struct State {
    pub generation: Option<mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    key: Option<(String, i64)>,
    current: Option<Parsed>,
    receiver: Option<mpsc::Receiver<Parsed>>,
    placed: Vec<Placed>,
    panel: Option<Arc<Panel>>,
    popup: Option<(O, Arc<Panel>)>,
    geometry: String,
    overlay_id: Option<u32>,
    pub overlay_ready: bool,
    watched: Option<(O, O, i64)>,
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
    let id = project.scope.register(move |j, op, args| {
        if op == "Callback.mouseClicked"
            && let Some(project) = weak.upgrade()
        {
            click(&project, j, &args[0])?;
        }
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    j.void(multicaster, "addEditorMouseListener", "(Lcom/intellij/openapi/editor/event/EditorMouseListener;Lcom/intellij/openapi/Disposable;)V", &[A::O(&callback), A::O(&project.object)])
}

pub fn tick(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    crate::wizard::tick(project, j)?;
    let popup = project
        .authoring
        .lock()
        .expect("authoring")
        .popup
        .as_ref()
        .map(|(o, _)| o.clone());
    if let Some(popup) = popup
        && j.bool(&popup, "isDisposed")?
        && let Some((_, panel)) = project.authoring.lock().expect("authoring").popup.take()
    {
        panel.close(j)?;
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
            clear_placed(project, j)?;
            if let Some(catalog) = &result.catalog {
                place(project, j, &editor, &path, catalog)?;
                let relative = Path::new(&path)
                    .strip_prefix(&project.root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let payload = serde_json::to_string(&Update {
                    file: relative,
                    schema: catalog.schema.clone(),
                    revision: SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros() as u64,
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
        project.authoring.lock().expect("authoring").receiver = Some(receiver);
        std::thread::spawn(move || {
            let catalog = Catalog::parse(&source).ok();
            let _ = sender.send(Parsed {
                path,
                stamp,
                source,
                catalog,
            });
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
    unwatch_editor(project, j)?;
    clear_placed(project, j)?;
    let (panel, popup) = {
        let mut s = project.authoring.lock().expect("authoring");
        (s.panel.take(), s.popup.take())
    };
    if let Some(panel) = panel {
        panel.close(j)?;
    }
    if let Some((popup, panel)) = popup {
        j.void(&popup, "cancel", "()V", &[])?;
        panel.close(j)?;
    }
    Ok(())
}

fn place(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    path: &str,
    catalog: &Catalog,
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
    for literal in catalog.literals.iter().take(512) {
        let id = jvm::register(|j, op, _| {
            if op == "ValueGlyph.calcWidthInPixels" {
                j.boxed_int(14)
            } else {
                j.null()
            }
        });
        let renderer = j.new("dev/cranpose/rust/ValueGlyph", "(J)V", &[A::J(id)])?;
        let inlay=j.obj(&model,"addInlineElement","(IZLcom/intellij/openapi/editor/EditorCustomElementRenderer;)Lcom/intellij/openapi/editor/Inlay;",&[A::I(literal.range.end_utf16 as i32),A::Z(true),A::O(&renderer)])?;
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
                    literal: Some(literal.id),
                });
        }
    }
    Ok(())
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
            .is_none_or(|c| c.functions.is_empty())
    {
        return Ok(());
    }
    let mut items = vec![];
    for placed in &state.placed {
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
            items.push(json!({"kind":"value","x":j.field_int(&rect,"x")?-x,"y":top,"width":14,"height":height,"label":"◆","id":id}));
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
    state.geometry = key;
    drop(state);
    let mut badges = crate::stability::decorations(project, j, editor, x, y, h)?;
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

fn click(project: &Arc<Project>, j: &mut J<'_>, event: &O) -> Result<()> {
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
    if j.int(&mouse, "getButton")? != 1 {
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
    if inlay.is_null() {
        return Ok(());
    }
    let state = project.authoring.lock().expect("authoring");
    let literal = state
        .placed
        .iter()
        .find_map(|p| j.same(&p.object, &inlay).ok().filter(|v| *v).and(p.literal));
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
    open_control(project, j, &editor, &point, id, catalog, source)
}

fn open_control(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    point: &O,
    id: usize,
    catalog: Catalog,
    source: String,
) -> Result<()> {
    if let Some((popup, panel)) = project.authoring.lock().expect("authoring").popup.take() {
        j.void(&popup, "cancel", "()V", &[])?;
        panel.close(j)?;
    }
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
    let mut options = project::options(j, false)?;
    options
        .environment
        .insert("CRANPOSE_AUTHORING".into(), "value".into());
    let panel = Panel::new(j, options)?;
    let init = json!({"literal":literal}).to_string();
    *panel.on_lifecycle.lock().expect("lifecycle") =
        Some(Arc::new(move |_, panel, connected, _| {
            if connected {
                panel.message("ide.authoring.control", &init);
            }
            Ok(())
        }));
    let weak = Arc::downgrade(project);
    let expected = Arc::new(std::sync::Mutex::new((catalog, source)));
    *panel.on_message.lock().expect("message") = Some(Arc::new(
        move |j, panel, channel, payload| {
            if channel != "ide.authoring.edit" {
                return Ok(());
            }
            let Some(project) = weak.upgrade() else {
                return Ok(());
            };
            let payload = payload.to_owned();
            let panel = panel.clone();
            let document = document.clone();
            let expected = expected.clone();
            jvm::later(j, move |j| {
                if project.closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(());
                }
                let result = (|| -> Result<()> {
                    let request: Value = serde_json::from_str(&payload)?;
                    let value = request["value"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing value"))?;
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
                    let result=j.static_void("com/intellij/openapi/command/WriteCommandAction","runWriteCommandAction","(Lcom/intellij/openapi/project/Project;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Runnable;[Lcom/intellij/psi/PsiFile;)V",&[A::O(&project.object),A::S("Tune Cranpose value"),A::Null,A::O(&callback),A::O(&files)]);
                    jvm::unregister(callback_id);
                    result?;
                    *expected = (next_catalog, next);
                    Ok(())
                })();
                panel.message(
                    "ide.authoring.result",
                    &match result {
                        Ok(()) => json!({"ok":true}),
                        Err(e) => json!({"ok":false,"error":e.to_string()}),
                    }
                    .to_string(),
                );
                Ok(())
            })
        },
    ));
    let size = j.new("java/awt/Dimension", "(II)V", &[A::I(340), A::I(240)])?;
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
        &[A::Z(true)],
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
    let relative = j.new(
        "com/intellij/ui/awt/RelativePoint",
        "(Ljava/awt/Component;Ljava/awt/Point;)V",
        &[A::O(&component), A::O(point)],
    )?;
    j.void(
        &popup,
        "show",
        "(Lcom/intellij/ui/awt/RelativePoint;)V",
        &[A::O(&relative)],
    )?;
    project.attach(j, &panel)?;
    panel.start(j)?;
    project.authoring.lock().expect("authoring").popup = Some((popup, panel));
    Ok(())
}

#[cfg(feature = "ide-tests")]
pub fn integration_test(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    let source = "// 🦀\n#[composable]\nfn Card() { Text(\"Café 🦀\"); Space(12); }";
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
            items.len() == 3,
            "Expected one preview marker and two live-value glyphs"
        );
        let mut retired = Vec::new();
        for (object, literal) in items {
            if let Some(id) = literal {
                let token = ["\"Café 🦀\"", "12"][id];
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
        Ok(())
    })();
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
