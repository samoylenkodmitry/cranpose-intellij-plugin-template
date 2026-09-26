//! Preview workspace placement. Controls, inspector and preview models run in Cranpose.
use crate::{
    jvm::{self, A, J, O, Scope},
    model::{self, Target},
    project::{self, Project},
    session::Options,
    surface::Panel,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub struct Workspace {
    pub project: Arc<Project>,
    pub source: O,
    pub source_path: String,
    pub studio: Arc<Panel>,
    pub component: OnceLock<O>,
    viewport: O,
    scope: Scope,
    state: Mutex<State>,
    closed: AtomicBool,
}
struct State {
    ready: bool,
    pending: Vec<String>,
    active: Option<(i64, Arc<Panel>)>,
    candidate: Option<(i64, Arc<Panel>)>,
    placement: Value,
    checkpoint: Value,
    viewport: [i32; 2],
}
impl Workspace {
    pub fn new(project: Arc<Project>, j: &mut J<'_>, source: O) -> Result<Arc<Self>> {
        let options = project::options(j, true)?;
        let studio = Panel::new(j, options)?;
        let viewport = j.new(
            "javax/swing/JPanel",
            "(Ljava/awt/LayoutManager;)V",
            &[A::Null],
        )?;
        j.void(&viewport, "setOpaque", "(Z)V", &[A::Z(false)])?;
        j.void(&viewport, "setVisible", "(Z)V", &[A::Z(false)])?;
        let source_path = if source.is_null() {
            String::new()
        } else {
            j.text(&source, "getPath")?
        };
        let workspace = Arc::new(Self {
            project: project.clone(),
            source,
            source_path,
            studio: studio.clone(),
            component: OnceLock::new(),
            viewport,
            scope: Scope::default(),
            state: Mutex::new(State {
                ready: false,
                pending: vec![],
                active: None,
                candidate: None,
                placement: Value::Null,
                checkpoint: Value::Null,
                viewport: [0, 0],
            }),
            closed: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&workspace);
        let id = workspace.scope.register(move |j, op, _| {
            if let Some(workspace) = weak.upgrade() {
                match op {
                    "Workspace.doLayout" => workspace.layout(j)?,
                    "Workspace.dispose" => workspace.dispose(j)?,
                    "Callback.hierarchyChanged"
                        if j.bool(workspace.component(), "isShowing")? =>
                    {
                        workspace.studio.start(j)?;
                    }
                    _ => {}
                }
            }
            j.null()
        });
        let component = j.new("dev/cranpose/rust/Workspace", "(J)V", &[A::J(id)])?;
        workspace.component.set(component.clone()).ok();
        let minimum = j.new("java/awt/Dimension", "(II)V", &[A::I(300), A::I(240)])?;
        j.void(
            &component,
            "setMinimumSize",
            "(Ljava/awt/Dimension;)V",
            &[A::O(&minimum)],
        )?;
        j.obj(
            &component,
            "add",
            "(Ljava/awt/Component;)Ljava/awt/Component;",
            &[A::O(studio.primary.component())],
        )?;
        j.obj(
            &component,
            "add",
            "(Ljava/awt/Component;)Ljava/awt/Component;",
            &[A::O(&workspace.viewport)],
        )?;
        j.void(
            &component,
            "setLayer",
            "(Ljava/awt/Component;I)V",
            &[A::O(&workspace.viewport), A::I(100)],
        )?;
        let listener = jvm::callback(j, id)?;
        j.void(
            &component,
            "addHierarchyListener",
            "(Ljava/awt/event/HierarchyListener;)V",
            &[A::O(&listener)],
        )?;
        let weak = Arc::downgrade(&workspace);
        *studio.on_lifecycle.lock().expect("callback") =
            Some(Arc::new(move |j, _, connected, _| {
                if let Some(workspace) = weak.upgrade() {
                    if connected {
                        workspace.connected(j)?;
                    } else {
                        workspace.state.lock().expect("workspace").ready = false;
                        j.void(&workspace.viewport, "setVisible", "(Z)V", &[A::Z(false)])?;
                    }
                }
                Ok(())
            }));
        let weak = Arc::downgrade(&workspace);
        *studio.on_message.lock().expect("callback") =
            Some(Arc::new(move |j, _, channel, payload| {
                if let Some(workspace) = weak.upgrade() {
                    if channel == "studio.host" {
                        workspace.handle(j, payload)?;
                    } else {
                        handle_message(&workspace.project, j, channel, payload)?;
                    }
                }
                Ok(())
            }));
        project.attach(j, &studio)?;
        project
            .workspaces
            .lock()
            .expect("workspaces")
            .push(Arc::downgrade(&workspace));
        Ok(workspace)
    }
    pub fn component(&self) -> &O {
        self.component.get().expect("workspace initialized")
    }
    fn connected(&self, j: &mut J<'_>) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let pending = {
            let mut state = self.state.lock().expect("workspace");
            state.ready = true;
            state.viewport = [0, 0];
            std::mem::take(&mut state.pending)
        };
        let settings = serde_json::from_str::<Value>(&self.project.property(j, "cranpose.studio")?)
            .unwrap_or(json!({}));
        let (checkpoint, active, candidate) = {
            let state = self.state.lock().expect("workspace");
            (
                state.checkpoint.clone(),
                state.active.as_ref().map_or(0, |s| s.0),
                state.candidate.as_ref().map_or(0, |s| s.0),
            )
        };
        self.studio.message("studio.init", &json!({"root":self.project.root,"cache":project::cache(j)?,"source":self.source_path,"settings":settings,"checkpoint":checkpoint,"activeSession":active,"candidateSession":candidate}).to_string());
        self.layout(j)?;
        self.project.theme(j, &self.studio)?;
        self.project.publish();
        for payload in pending {
            self.studio.message("studio.command", &payload);
        }
        let refresh = {
            let snapshot = self.project.snapshot.lock().expect("snapshot");
            snapshot.targets.is_empty() && !snapshot.busy
        };
        if refresh {
            self.project.refresh(j)?;
        }
        Ok(())
    }
    pub fn command(&self, payload: Value) {
        let payload = payload.to_string();
        let mut state = self.state.lock().expect("workspace");
        if state.ready {
            drop(state);
            self.studio.message("studio.command", &payload);
        } else {
            state.pending.push(payload);
        }
    }
    fn handle(self: &Arc<Self>, j: &mut J<'_>, payload: &str) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let request: Value = serde_json::from_str(payload)?;
        match request["action"].as_str().unwrap_or_default() {
            "start" => self.start(j, &request)?,
            "stop" => {
                let (candidate, active) = {
                    let mut state = self.state.lock().expect("workspace");
                    (state.candidate.take(), state.active.take())
                };
                for child in [candidate, active].into_iter().flatten() {
                    self.close_child(j, &child.1)?;
                }
                self.layout(j)?;
            }
            "layout" => {
                self.state.lock().expect("workspace").placement = request;
                self.layout(j)?;
            }
            "message" => {
                let child = self.state.lock().expect("workspace").active.clone();
                if let Some((id, panel)) = child
                    && request["session"].as_i64() == Some(id)
                {
                    panel.message(
                        &model::s(&request, "channel"),
                        &model::s(&request, "payload"),
                    );
                }
            }
            "checkpoint" => {
                let checkpoint = request["value"].clone();
                if checkpoint.is_object() && checkpoint["settings"].is_object() {
                    let changed = {
                        let mut state = self.state.lock().expect("workspace");
                        let changed = state.checkpoint["settings"] != checkpoint["settings"];
                        state.checkpoint = checkpoint.clone();
                        changed
                    };
                    if changed {
                        self.project.set_property(
                            j,
                            "cranpose.studio",
                            &checkpoint["settings"].to_string(),
                        )?;
                    }
                }
            }
            "settings" => {
                self.project
                    .set_property(j, "cranpose.studio", &request["value"].to_string())?
            }
            "navigate" => project::navigate(
                &self.project,
                j,
                &model::s(&request, "file"),
                request["line"].as_i64().unwrap_or(1) as i32 - 1,
                0,
            )?,
            "configure" => crate::run_configuration::create_selected(&self.project, j)?,
            "export" => self.export(j)?,
            _ => {}
        }
        Ok(())
    }
    fn start(self: &Arc<Self>, j: &mut J<'_>, request: &Value) -> Result<()> {
        let id = request["session"].as_i64().context("Preview session")?;
        if !self.project.trusted(j)? {
            self.event(
                id,
                "stopped",
                json!({"message":"Trust this project before running its code"}),
            );
            return Ok(());
        }
        let mut options = request["options"].clone();
        let valid = {
            let snapshot = self.project.snapshot.lock().expect("snapshot");
            snapshot.targets.iter().any(|t| {
                Some(t.package_name.as_str()) == options["package"].as_str()
                    && Some(t.name.as_str()) == options["target"].as_str()
                    && Some(t.kind.as_str()) == options["kind"].as_str()
            })
        };
        if !valid {
            self.event(
                id,
                "stopped",
                json!({"message":"Refresh the Cargo workspace to select this target"}),
            );
            return Ok(());
        }
        options["root"] = json!(self.project.root);
        options["cache"] = json!(project::cache(j)?);
        let old = self.state.lock().expect("workspace").candidate.take();
        if let Some((_, panel)) = old {
            self.close_child(j, &panel)?;
        }
        let preview = model::s(request, "preview");
        let binary = project::binary(j)?;
        let panel = Panel::new(
            j,
            Options {
                command: vec![
                    binary.to_string_lossy().into_owned(),
                    "--dev-run".into(),
                    options.to_string(),
                ],
                directory: Some(self.project.root.clone()),
                environment: if preview.is_empty() {
                    BTreeMap::new()
                } else {
                    BTreeMap::from([("CRANPOSE_PREVIEW".into(), preview)])
                },
                timeout: Duration::from_secs(1200),
            },
        )?;
        self.state.lock().expect("workspace").candidate = Some((id, panel.clone()));
        let weak = Arc::downgrade(self);
        *panel.on_lifecycle.lock().expect("callback") =
            Some(Arc::new(move |j, panel, connected, message| {
                let Some(workspace) = weak.upgrade() else {
                    return panel.close(j);
                };
                if connected {
                    let (current, old) = {
                        let mut state = workspace.state.lock().expect("workspace");
                        if state.candidate.as_ref().is_some_and(|c| c.0 == id) {
                            let old = state.active.take();
                            state.active = state.candidate.take();
                            (true, old)
                        } else {
                            (false, None)
                        }
                    };
                    if !current {
                        return panel.close(j);
                    }
                    if let Some((_, old)) = old {
                        workspace.close_child(j, &old)?;
                    }
                    j.obj(
                        &workspace.viewport,
                        "add",
                        "(Ljava/awt/Component;)Ljava/awt/Component;",
                        &[A::O(panel.primary.component())],
                    )?;
                    workspace.layout(j)?;
                    workspace.event(id, "connected", json!({}));
                } else {
                    let event = {
                        let mut state = workspace.state.lock().expect("workspace");
                        if state.candidate.as_ref().is_some_and(|c| c.0 == id) {
                            state.candidate = None;
                            Some(("failed", state.active.as_ref().map(|c| c.0).unwrap_or(0)))
                        } else if state.active.as_ref().is_some_and(|c| c.0 == id) {
                            state.active = None;
                            Some(("stopped", 0))
                        } else {
                            None
                        }
                    };
                    if let Some((event, fallback)) = event {
                        workspace.close_child(j, panel)?;
                        workspace.event(
                            id,
                            event,
                            json!({"message":message,"fallbackSession":fallback}),
                        );
                        workspace.layout(j)?;
                    }
                }
                Ok(())
            }));
        let weak = Arc::downgrade(self);
        *panel.on_message.lock().expect("callback") =
            Some(Arc::new(move |j, panel, channel, payload| {
                if let Some(workspace) = weak.upgrade() {
                    if channel == "host.log" {
                        workspace.event(id, "log", json!({"line":payload}));
                    } else if channel == "host.overlay" {
                        crate::editor::attach_overlay(&workspace.project, j, panel, payload)?;
                    } else {
                        workspace.event(
                            id,
                            "message",
                            json!({"channel":channel,"payload":payload}),
                        );
                    }
                }
                Ok(())
            }));
        let weak = Arc::downgrade(self);
        *panel.on_pointer.lock().expect("callback") =
            Some(Arc::new(move |_, x, y, scroll, shift, pan| {
                if let Some(workspace) = weak.upgrade() {
                    if scroll && pan != 0.0 {
                        workspace.event(
                            id,
                            "pan",
                            json!({"x":if shift{pan}else{0.0},"y":if shift{0.0}else{pan}}),
                        );
                        return Ok(false);
                    }
                    if !scroll
                        && workspace.state.lock().expect("workspace").placement["pick"]
                            .as_bool()
                            .unwrap_or(false)
                    {
                        workspace.event(id, "pointer", json!({"x":x,"y":y}));
                        return Ok(false);
                    }
                }
                Ok(true)
            }));
        panel.start(j)
    }
    pub fn layout(&self, j: &mut J<'_>) -> Result<()> {
        let Some(component) = self.component.get() else {
            return Ok(());
        };
        let width = j.int(component, "getWidth")?;
        let height = j.int(component, "getHeight")?;
        let notify = {
            let mut state = self.state.lock().expect("workspace");
            let changed = state.ready && state.viewport != [width, height];
            if changed {
                state.viewport = [width, height];
            }
            changed
        };
        if notify {
            self.studio.message(
                "studio.viewport",
                &json!({"width":width,"height":height}).to_string(),
            );
        }
        bounds(j, self.studio.primary.component(), 0, 0, width, height)?;
        let (child, placement) = {
            let state = self.state.lock().expect("workspace");
            (state.active.clone(), state.placement.clone())
        };
        j.void(
            &self.viewport,
            "setVisible",
            "(Z)V",
            &[A::Z(child.is_some())],
        )?;
        let Some((_, panel)) = child else {
            return Ok(());
        };
        if placement.is_null() {
            return Ok(());
        }
        let number = |v: &Value, key: &str, default: f64| {
            v[key].as_f64().filter(|n| n.is_finite()).unwrap_or(default)
        };
        panel
            .primary
            .set_scale(j, number(&placement, "scale", 1.0))?;
        let clip_y = number(&placement["viewport"], "y", 0.0) as i32;
        bounds(
            j,
            &self.viewport,
            0,
            clip_y,
            number(&placement["viewport"], "width", width as f64).clamp(1.0, width.max(1) as f64)
                as i32,
            number(&placement["viewport"], "height", height as f64).max(1.0) as i32,
        )?;
        bounds(
            j,
            panel.primary.component(),
            number(&placement, "x", 0.0) as i32,
            number(&placement, "y", 0.0) as i32 - clip_y,
            number(&placement, "width", 480.0).max(1.0) as i32,
            number(&placement, "height", 640.0).max(1.0) as i32,
        )?;
        panel.set_theme(placement["dark"].as_bool().unwrap_or(false));
        panel
            .primary
            .select(placement["selected"].as_object().map(|_| {
                ["x", "y", "width", "height"].map(|key| number(&placement["selected"], key, 0.0))
            }));
        panel.primary.repaint(j)?;
        j.void(component, "repaint", "()V", &[])
    }
    fn event(&self, id: i64, event: &str, mut value: Value) {
        if !self.closed.load(Ordering::Acquire) {
            value["session"] = json!(id);
            value["event"] = json!(event);
            self.studio.message("studio.child", &value.to_string());
        }
    }
    fn close_child(&self, j: &mut J<'_>, panel: &Panel) -> Result<()> {
        *panel.on_lifecycle.lock().expect("callback") = None;
        panel.close(j)?;
        j.void(
            &self.viewport,
            "remove",
            "(Ljava/awt/Component;)V",
            &[A::O(panel.primary.component())],
        )?;
        j.void(&self.viewport, "revalidate", "()V", &[])?;
        Ok(())
    }
    fn export(&self, j: &mut J<'_>) -> Result<()> {
        let image = self
            .state
            .lock()
            .expect("workspace")
            .active
            .as_ref()
            .and_then(|(_, p)| p.primary.snapshot());
        let Some(image) = image else {
            return Ok(());
        };
        let factory = j.static_obj(
            "com/intellij/openapi/fileChooser/FileChooserFactory",
            "getInstance",
            "()Lcom/intellij/openapi/fileChooser/FileChooserFactory;",
            &[],
        )?;
        let png = j.string("png")?;
        let extensions = j.array("java/lang/String", &[png])?;
        let descriptor = j.new(
            "com/intellij/openapi/fileChooser/FileSaverDescriptor",
            "(Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;)V",
            &[
                A::S("Export Cranpose preview"),
                A::S("Save the rendered preview"),
                A::O(&extensions),
            ],
        )?;
        let dialog=j.obj(&factory,"createSaveFileDialog","(Lcom/intellij/openapi/fileChooser/FileSaverDescriptor;Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/fileChooser/FileSaverDialog;",&[A::O(&descriptor),A::O(&self.project.object)])?;
        let parent = if self.source.is_null() {
            j.null()?
        } else {
            j.obj(
                &self.source,
                "getParent",
                "()Lcom/intellij/openapi/vfs/VirtualFile;",
                &[],
            )?
        };
        let wrapper=j.obj(&dialog,"save","(Lcom/intellij/openapi/vfs/VirtualFile;Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFileWrapper;",&[A::O(&parent),A::S("cranpose-preview.png")])?;
        if !wrapper.is_null() {
            let file = j.obj(&wrapper, "getFile", "()Ljava/io/File;", &[])?;
            j.static_call(
                "javax/imageio/ImageIO",
                "write",
                "(Ljava/awt/image/RenderedImage;Ljava/lang/String;Ljava/io/File;)Z",
                &[A::O(&image), A::S("png"), A::O(&file)],
            )?;
        }
        Ok(())
    }
    pub fn dispose(&self, j: &mut J<'_>) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let children = {
            let mut state = self.state.lock().expect("workspace");
            state.pending.clear();
            [state.active.take(), state.candidate.take()]
        };
        for (_, panel) in children.into_iter().flatten() {
            self.close_child(j, &panel)?;
        }
        self.studio.close(j)?;
        self.scope.clear();
        Ok(())
    }
}
pub fn bounds(j: &mut J<'_>, object: &O, x: i32, y: i32, width: i32, height: i32) -> Result<()> {
    j.void(
        object,
        "setBounds",
        "(IIII)V",
        &[A::I(x), A::I(y), A::I(width), A::I(height)],
    )
}
pub fn create_editor(project: Arc<Project>, j: &mut J<'_>, file: O) -> Result<O> {
    let workspace = Workspace::new(project.clone(), j, file.clone())?;
    let id_cell = Arc::new(std::sync::atomic::AtomicI64::new(0));
    let captured = id_cell.clone();
    let id = project.scope.register(move |j, op, _| match op {
        "PreviewEditor.getComponent" | "PreviewEditor.getPreferredFocusedComponent" => {
            Ok(workspace.component().clone())
        }
        "PreviewEditor.getName" => j.string("Cranpose"),
        "PreviewEditor.getFile" => Ok(workspace.source.clone()),
        "PreviewEditor.isModified" => j.boxed_bool(false),
        "PreviewEditor.isValid" => j.boxed_bool(!workspace.closed.load(Ordering::Acquire)),
        "PreviewEditor.dispose" => {
            workspace.dispose(j)?;
            jvm::unregister(captured.load(Ordering::Acquire));
            j.null()
        }
        _ => j.null(),
    });
    id_cell.store(id, Ordering::Release);
    let preview = j.new("dev/cranpose/rust/PreviewEditor", "(J)V", &[A::J(id)])?;
    let provider = j.static_obj(
        "com/intellij/openapi/fileEditor/impl/text/TextEditorProvider",
        "getInstance",
        "()Lcom/intellij/openapi/fileEditor/impl/text/TextEditorProvider;",
        &[],
    )?;
    let text=j.obj(&provider,"createEditor","(Lcom/intellij/openapi/project/Project;Lcom/intellij/openapi/vfs/VirtualFile;)Lcom/intellij/openapi/fileEditor/FileEditor;",&[A::O(&project.object),A::O(&file)])?;
    let layout = j.constant(
        "com/intellij/openapi/fileEditor/TextEditorWithPreview$Layout",
        "SHOW_EDITOR",
        "Lcom/intellij/openapi/fileEditor/TextEditorWithPreview$Layout;",
    )?;
    j.new("com/intellij/openapi/fileEditor/TextEditorWithPreview","(Lcom/intellij/openapi/fileEditor/TextEditor;Lcom/intellij/openapi/fileEditor/FileEditor;Ljava/lang/String;Lcom/intellij/openapi/fileEditor/TextEditorWithPreview$Layout;)V",&[A::O(&text),A::O(&preview),A::S("Cranpose"),A::O(&layout)])
}
pub fn show(
    project: &Arc<Project>,
    j: &mut J<'_>,
    path: &str,
    function: Option<&str>,
    target: Option<&Target>,
) -> Result<()> {
    let fs = j.static_obj(
        "com/intellij/openapi/vfs/LocalFileSystem",
        "getInstance",
        "()Lcom/intellij/openapi/vfs/LocalFileSystem;",
        &[],
    )?;
    let file = j.obj(
        &fs,
        "findFileByPath",
        "(Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
        &[A::S(path)],
    )?;
    if file.is_null() {
        return Ok(());
    }
    let manager = project.editor_manager(j)?;
    let editors = j.obj(
        &manager,
        "openFile",
        "(Lcom/intellij/openapi/vfs/VirtualFile;Z)[Lcom/intellij/openapi/fileEditor/FileEditor;",
        &[A::O(&file), A::Z(true)],
    )?;
    for editor in j.elements(&editors)? {
        if j.env.is_instance_of(
            &editor,
            "com/intellij/openapi/fileEditor/TextEditorWithPreview",
        )? {
            j.void(
                &manager,
                "setSelectedEditor",
                "(Lcom/intellij/openapi/vfs/VirtualFile;Ljava/lang/String;)V",
                &[A::O(&file), A::S("cranpose-studio")],
            )?;
            let layout = j.constant(
                "com/intellij/openapi/fileEditor/TextEditorWithPreview$Layout",
                "SHOW_EDITOR_AND_PREVIEW",
                "Lcom/intellij/openapi/fileEditor/TextEditorWithPreview$Layout;",
            )?;
            j.void(
                &editor,
                "setLayout",
                "(Lcom/intellij/openapi/fileEditor/TextEditorWithPreview$Layout;)V",
                &[A::O(&layout)],
            )?;
        }
    }
    let workspaces = project
        .workspaces
        .lock()
        .expect("workspaces")
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .collect::<Vec<_>>();
    for workspace in workspaces {
        if workspace.source_path == path {
            if let Some(function) = function {
                workspace.command(json!({"action":"showFunction","name":function}));
            }
            if let Some(target) = target {
                workspace.command(json!({"action":"showTarget","target":target}));
            }
            workspace.studio.start(j)?;
        }
    }
    Ok(())
}
pub fn handle_message(
    project: &Arc<Project>,
    j: &mut J<'_>,
    channel: &str,
    payload: &str,
) -> Result<()> {
    if channel == "host.log" {
        return Ok(());
    }
    let request: Value = serde_json::from_str(payload)?;
    match channel {
        "cranpose.action" => match request["action"].as_str().unwrap_or_default() {
            "refresh" => project.refresh(j)?,
            "select" => project.select(&model::s(&request, "value")),
            task @ ("check" | "run" | "test" | "preview") => project.execute(j, task)?,
            "stop" => project.stop(j)?,
            "create" => project.create_starter(j)?,
            "docs" => docs(j)?,
            "configure" => crate::run_configuration::create_selected(project, j)?,
            "component" => {
                if let (Some(file), _) = project.selected(j)? {
                    let path = j.text(&file, "getPath")?;
                    show(project, j, &path, request["value"].as_str(), None)?;
                }
            }
            "navigate" => {
                if let (_, Some(editor)) = project.selected(j)?
                    && let Some(offset) = request["value"]
                        .as_str()
                        .and_then(|s| s.parse::<i32>().ok())
                {
                    let document = j.obj(
                        &editor,
                        "getDocument",
                        "()Lcom/intellij/openapi/editor/Document;",
                        &[],
                    )?;
                    if (0..=j.int(&document, "getTextLength")?).contains(&offset) {
                        let caret = j.obj(
                            &editor,
                            "getCaretModel",
                            "()Lcom/intellij/openapi/editor/CaretModel;",
                            &[],
                        )?;
                        j.void(&caret, "moveToOffset", "(I)V", &[A::I(offset)])?;
                        let scrolling = j.obj(
                            &editor,
                            "getScrollingModel",
                            "()Lcom/intellij/openapi/editor/ScrollingModel;",
                            &[],
                        )?;
                        let center = j.constant(
                            "com/intellij/openapi/editor/ScrollType",
                            "CENTER",
                            "Lcom/intellij/openapi/editor/ScrollType;",
                        )?;
                        j.void(
                            &scrolling,
                            "scrollToCaret",
                            "(Lcom/intellij/openapi/editor/ScrollType;)V",
                            &[A::O(&center)],
                        )?;
                    }
                }
            }
            "diagnostic" => {
                if let Some(index) = request["value"]
                    .as_str()
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    let diagnostic = project
                        .snapshot
                        .lock()
                        .expect("snapshot")
                        .diagnostics
                        .get(index)
                        .cloned();
                    if let Some(d) = diagnostic {
                        project::navigate(
                            project,
                            j,
                            &project.root.join(model::s(&d, "file")).to_string_lossy(),
                            d["line"].as_i64().unwrap_or(1) as i32 - 1,
                            d["column"].as_i64().unwrap_or(1) as i32 - 1,
                        )?;
                    }
                }
            }
            _ => {}
        },
        "ide.open" => project::navigate(project, j, &model::s(&request, "path"), 0, 0)?,
        "ide.notify" => {
            let manager = j.static_obj(
                "com/intellij/notification/NotificationGroupManager",
                "getInstance",
                "()Lcom/intellij/notification/NotificationGroupManager;",
                &[],
            )?;
            let group = j.obj(
                &manager,
                "getNotificationGroup",
                "(Ljava/lang/String;)Lcom/intellij/notification/NotificationGroup;",
                &[A::S("Cranpose")],
            )?;
            let kind = j.constant(
                "com/intellij/notification/NotificationType",
                "INFORMATION",
                "Lcom/intellij/notification/NotificationType;",
            )?;
            let notification=j.obj(&group,"createNotification","(Ljava/lang/String;Ljava/lang/String;Lcom/intellij/notification/NotificationType;)Lcom/intellij/notification/Notification;",&[A::S(&model::s(&request,"title")),A::S(&model::s(&request,"content")),A::O(&kind)])?;
            j.void(
                &notification,
                "notify",
                "(Lcom/intellij/openapi/project/Project;)V",
                &[A::O(&project.object)],
            )?;
        }
        _ => {}
    }
    Ok(())
}
pub fn docs(j: &mut J<'_>) -> Result<()> {
    j.static_void(
        "com/intellij/ide/BrowserUtil",
        "browse",
        "(Ljava/lang/String;)V",
        &[A::S("https://docs.rs/cranpose/latest/cranpose/")],
    )
}

#[cfg(feature = "ide-tests")]
pub fn integration_test(project: Arc<Project>, j: &mut J<'_>) -> Result<()> {
    use crate::protocol::Packet;
    use anyhow::ensure;
    use std::time::Instant;
    let saved = project.property(j, "cranpose.studio")?;
    project.set_property(
        j,
        "cranpose.studio",
        r#"{"width":612,"height":520,"inspect":true}"#,
    )?;
    let workspace = Workspace::new(project.clone(), j, j.null()?)?;
    bounds(j, workspace.component(), 0, 0, 720, 620)?;
    workspace.layout(j)?;
    workspace.studio.start(j)?;
    let result = (|| -> Result<()> {
        let wait_checkpoint = |j: &mut J<'_>| -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                workspace.studio.tick(j)?;
                workspace
                    .studio
                    .send(Packet::new(1).int(0).int(720).int(620).float(1.).float(60.));
                workspace.studio.send(Packet::new(13).int(0).byte(1));
                if workspace.state.lock().expect("workspace").checkpoint["settings"]["width"] == 612
                {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            anyhow::bail!("Cranpose controller did not return its initialized checkpoint");
        };
        wait_checkpoint(j)?;
        {
            let mut state = workspace.state.lock().expect("workspace");
            state.checkpoint["selected"] = json!("retained-selection");
            state.checkpoint["settings"]["width"] = json!(614);
        }
        workspace.studio.restart(j)?;
        // Persisted width remains authoritative; other controller state comes from the checkpoint.
        wait_checkpoint(j)?;
        for (width, expected_width, expected_top) in [(1024, 614.4, 88.0), (480, 480.0, 126.0)] {
            bounds(j, workspace.component(), 0, 0, width, 620)?;
            workspace.layout(j)?;
            workspace.studio.primary.size(j)?;
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                workspace.studio.tick(j)?;
                workspace.studio.send(Packet::new(13).int(0).byte(1));
                let placement = workspace.state.lock().expect("workspace").placement.clone();
                let actual_width = placement["viewport"]["width"].as_f64().unwrap_or_default();
                let actual_top = placement["viewport"]["y"].as_f64().unwrap_or_default();
                if (actual_width - expected_width).abs() < 1.0 && actual_top == expected_top {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "Controller did not reflow at {width}px: {placement}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let state = workspace.state.lock().expect("workspace");
        ensure!(
            state.checkpoint["selected"] == "retained-selection",
            "Controller checkpoint was lost while restarting native UI"
        );
        ensure!(
            state.active.is_none() && state.candidate.is_none(),
            "Controller restart unexpectedly launched an application"
        );
        Ok(())
    })();
    workspace.dispose(j)?;
    project.set_property(j, "cranpose.studio", &saved)?;
    result
}
