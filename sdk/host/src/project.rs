//! IntelliJ project services, authored in Rust.
use crate::{
    jvm::{self, A, J, O, Scope},
    model::{self, Snapshot},
    session::Options,
    surface::Panel,
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
static PROJECTS: OnceLock<Mutex<Vec<Arc<Project>>>> = OnceLock::new();
fn projects() -> &'static Mutex<Vec<Arc<Project>>> {
    PROJECTS.get_or_init(Mutex::default)
}
pub struct Project {
    pub object: O,
    pub root: PathBuf,
    pub scope: Scope,
    pub snapshot: Mutex<Snapshot>,
    pub panels: Mutex<Vec<Weak<Panel>>>,
    pub workspaces: Mutex<Vec<Weak<crate::workspace::Workspace>>>,
    pub stability: Mutex<crate::stability::State>,
    pub closed: AtomicBool,
    timer: OnceLock<O>,
    metadata: Mutex<Option<mpsc::Receiver<Result<Snapshot, String>>>>,
    pub process: Mutex<Option<O>>,
    editor_stamp: Mutex<String>,
    editor_key: Mutex<Option<(String, i64)>>,
    pub editor: Mutex<crate::editor::State>,
    pub authoring: Mutex<crate::authoring::State>,
}
impl Project {
    pub fn get(j: &mut J<'_>, object: &O) -> Result<Arc<Self>> {
        let mut registry = projects().lock().expect("projects");
        for project in registry.iter() {
            if j.same(&project.object, object)? {
                return Ok(project.clone());
            }
        }
        let root = PathBuf::from(j.text(object, "getBasePath")?);
        let project = Arc::new(Self {
            object: object.clone(),
            root,
            scope: Scope::default(),
            snapshot: Mutex::new(Snapshot::default()),
            panels: Mutex::new(vec![]),
            workspaces: Mutex::new(vec![]),
            stability: Mutex::new(crate::stability::State::default()),
            closed: AtomicBool::new(false),
            timer: OnceLock::new(),
            metadata: Mutex::new(None),
            process: Mutex::new(None),
            editor_stamp: Mutex::new(String::new()),
            editor_key: Mutex::new(None),
            editor: Mutex::new(crate::editor::State::default()),
            authoring: Mutex::new(crate::authoring::State::default()),
        });
        registry.push(project.clone());
        drop(registry);
        let app = j.application()?;
        if j.bool(&app, "isDispatchThread")? {
            project.initialize(j)?;
        } else {
            let pending = project.clone();
            jvm::later(j, move |j| pending.initialize(j))?;
        }
        Ok(project)
    }
    fn initialize(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        if j.bool(&self.object, "isDisposed")? {
            self.closed.store(true, Ordering::Release);
            return Ok(());
        }
        let weak = Arc::downgrade(self);
        let id = self.scope.register(move |j, op, args| {
            if let Some(project) = weak.upgrade() {
                match op {
                    "Callback.dispose" => project.dispose(j)?,
                    "Callback.actionPerformed" => project.tick(j)?,
                    "Callback.documentChanged"
                    | "Callback.editorCreated"
                    | "Callback.editorReleased"
                    | "Callback.after" => project.stability.lock().expect("stability").schedule(),
                    "Callback.selectionChanged" => {
                        project.send_editor(j)?;
                        project.stability.lock().expect("stability").schedule();
                    }
                    "Callback.lookAndFeelChanged" => {
                        let panels = project.live_panels();
                        for panel in panels {
                            project.theme(j, &panel)?;
                        }
                    }
                    "Callback.caretPositionChanged" => project.caret(j, &args[0])?,
                    _ => {}
                }
            }
            j.null()
        });
        let callback = jvm::callback(j, id)?;
        j.static_void(
            "com/intellij/openapi/util/Disposer",
            "register",
            "(Lcom/intellij/openapi/Disposable;Lcom/intellij/openapi/Disposable;)V",
            &[A::O(&self.object), A::O(&callback)],
        )?;
        let timer = j.new(
            "javax/swing/Timer",
            "(ILjava/awt/event/ActionListener;)V",
            &[A::I(100), A::O(&callback)],
        )?;
        self.timer.set(timer.clone()).ok();
        j.void(&timer, "start", "()V", &[])?;
        let factory = j.static_obj(
            "com/intellij/openapi/editor/EditorFactory",
            "getInstance",
            "()Lcom/intellij/openapi/editor/EditorFactory;",
            &[],
        )?;
        let multicaster = j.obj(
            &factory,
            "getEventMulticaster",
            "()Lcom/intellij/openapi/editor/event/EditorEventMulticaster;",
            &[],
        )?;
        j.void(&multicaster,"addDocumentListener","(Lcom/intellij/openapi/editor/event/DocumentListener;Lcom/intellij/openapi/Disposable;)V",&[A::O(&callback),A::O(&self.object)])?;
        j.void(
            &multicaster,
            "addCaretListener",
            "(Lcom/intellij/openapi/editor/event/CaretListener;Lcom/intellij/openapi/Disposable;)V",
            &[A::O(&callback), A::O(&self.object)],
        )?;
        j.void(&factory,"addEditorFactoryListener","(Lcom/intellij/openapi/editor/event/EditorFactoryListener;Lcom/intellij/openapi/Disposable;)V",&[A::O(&callback),A::O(&self.object)])?;
        let bus = j.obj(
            &self.object,
            "getMessageBus",
            "()Lcom/intellij/util/messages/MessageBus;",
            &[],
        )?;
        subscribe(
            j,
            &bus,
            &self.object,
            &callback,
            "com/intellij/openapi/fileEditor/FileEditorManagerListener",
            "FILE_EDITOR_MANAGER",
        )?;
        subscribe(
            j,
            &bus,
            &self.object,
            &callback,
            "com/intellij/openapi/vfs/VirtualFileManager",
            "VFS_CHANGES",
        )?;
        let app = j.application()?;
        let bus = j.obj(
            &app,
            "getMessageBus",
            "()Lcom/intellij/util/messages/MessageBus;",
            &[],
        )?;
        subscribe(
            j,
            &bus,
            &self.object,
            &callback,
            "com/intellij/ide/ui/LafManagerListener",
            "TOPIC",
        )?;
        if crate::features().stability {
            crate::stability::install(self, j, &multicaster)?;
        }
        crate::authoring::install(self, j, &multicaster)?;
        Ok(())
    }
    fn tick(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = self
            .metadata
            .lock()
            .expect("metadata")
            .as_ref()
            .and_then(|r| r.try_recv().ok());
        if let Some(result) = result {
            self.metadata.lock().expect("metadata").take();
            let snapshot = match result {
                Ok(mut snapshot) => {
                    let selected = self.snapshot.lock().expect("snapshot").selected.clone();
                    if snapshot.targets.iter().any(|t| t.id == selected) {
                        snapshot.selected = selected;
                    }
                    snapshot
                }
                Err(error) => Snapshot {
                    status: error,
                    ..Snapshot::default()
                },
            };
            *self.snapshot.lock().expect("snapshot") = snapshot;
            self.publish();
        }
        self.update_editor(j, false)?;
        crate::editor::fit_overlays(self, j)?;
        crate::authoring::tick(self, j)?;
        if crate::features().stability {
            crate::stability::tick(self, j)?;
        }
        Ok(())
    }
    pub fn live_panels(&self) -> Vec<Arc<Panel>> {
        let mut panels = self.panels.lock().expect("panels");
        panels.retain(|p| p.strong_count() > 0);
        panels.iter().filter_map(Weak::upgrade).collect()
    }
    pub fn attach(self: &Arc<Self>, j: &mut J<'_>, panel: &Arc<Panel>) -> Result<()> {
        self.panels
            .lock()
            .expect("panels")
            .push(Arc::downgrade(panel));
        self.theme(j, panel)?;
        Ok(())
    }
    pub fn publish(&self) {
        let json =
            serde_json::to_string(&*self.snapshot.lock().expect("snapshot")).unwrap_or_default();
        for panel in self.live_panels() {
            panel.message("cranpose.project", &json);
        }
    }
    pub fn trusted(&self, j: &mut J<'_>) -> Result<bool> {
        let trusted = j
            .static_call(
                "com/intellij/ide/impl/TrustedProjects",
                "isTrusted",
                "(Lcom/intellij/openapi/project/Project;)Z",
                &[A::O(&self.object)],
            )?
            .z()?;
        if !trusted {
            self.snapshot.lock().expect("snapshot").status =
                "Trust this project in the IDE before running Cargo or previews.".into();
            self.publish();
        }
        Ok(trusted)
    }
    pub fn refresh(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        if !self.trusted(j)? || self.snapshot.lock().expect("snapshot").busy {
            return Ok(());
        }
        {
            let mut snapshot = self.snapshot.lock().expect("snapshot");
            snapshot.busy = true;
            snapshot.status = "Reading Cargo workspace…".into();
        }
        self.publish();
        let root = self.root.clone();
        let (sender, receiver) = mpsc::channel();
        *self.metadata.lock().expect("metadata") = Some(receiver);
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            let result = (|| -> Result<Snapshot> {
                ensure!(
                    root.join("Cargo.toml").is_file(),
                    "No Cargo.toml at the project root. Open the Cargo workspace directory."
                );
                let mut command = Command::new(model::cargo());
                command
                    .args([
                        "metadata",
                        "--format-version",
                        "1",
                        "--no-deps",
                        "--manifest-path",
                    ])
                    .arg(root.join("Cargo.toml"))
                    .current_dir(&root);
                let output =
                    crate::process::capture(command, None, Duration::from_secs(60), || {
                        weak.upgrade()
                            .is_none_or(|p| p.closed.load(Ordering::Acquire))
                    })?;
                ensure!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                model::metadata(&String::from_utf8(output.stdout)?)
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        Ok(())
    }
    pub fn select(&self, id: &str) {
        let mut snapshot = self.snapshot.lock().expect("snapshot");
        if snapshot.targets.iter().any(|t| t.id == id) {
            snapshot.selected = id.into();
        }
        drop(snapshot);
        self.publish();
    }
    pub fn execute(self: &Arc<Self>, j: &mut J<'_>, task: &str) -> Result<()> {
        if !self.trusted(j)? {
            return Ok(());
        }
        let target = {
            let snapshot = self.snapshot.lock().expect("snapshot");
            snapshot
                .targets
                .iter()
                .find(|t| t.id == snapshot.selected)
                .cloned()
        };
        let Some(target) = target else {
            return Ok(());
        };
        if task == "preview" {
            crate::workspace::show(self, j, &target.source, None, Some(&target))?;
            return Ok(());
        }
        if self.snapshot.lock().expect("snapshot").busy {
            return Ok(());
        }
        let manager = j.static_obj(
            "com/intellij/openapi/fileEditor/FileDocumentManager",
            "getInstance",
            "()Lcom/intellij/openapi/fileEditor/FileDocumentManager;",
            &[],
        )?;
        j.void(&manager, "saveAllDocuments", "()V", &[])?;
        let arguments = model::arguments(task, &target);
        let command = command(j, &self.root, &arguments, &BTreeMap::new())?;
        let handler = j.new(
            "com/intellij/execution/process/KillableColoredProcessHandler",
            "(Lcom/intellij/execution/configurations/GeneralCommandLine;)V",
            &[A::O(&command)],
        )?;
        let factory = j.static_obj(
            "com/intellij/execution/filters/TextConsoleBuilderFactory",
            "getInstance",
            "()Lcom/intellij/execution/filters/TextConsoleBuilderFactory;",
            &[],
        )?;
        let builder=j.obj(&factory,"createBuilder","(Lcom/intellij/openapi/project/Project;)Lcom/intellij/execution/filters/TextConsoleBuilder;",&[A::O(&self.object)])?;
        let console = j.obj(
            &builder,
            "getConsole",
            "()Lcom/intellij/execution/ui/ConsoleView;",
            &[],
        )?;
        let checking = task == "check";
        if !checking {
            j.void(
                &console,
                "attachToProcess",
                "(Lcom/intellij/execution/process/ProcessHandler;)V",
                &[A::O(&handler)],
            )?;
        }
        let output = console.clone();
        let callback_id = Arc::new(std::sync::atomic::AtomicI64::new(0));
        let retained_id = callback_id.clone();
        let weak = Arc::downgrade(self);
        let buffer = Mutex::new(String::new());
        let label = target.label.clone();
        let id = self.scope.register(move |j, op, args| {
            let Some(project) = weak.upgrade() else {
                return j.null();
            };
            if op == "Callback.onTextAvailable" {
                let text = j.text(&args[0], "getText")?;
                if !checking {
                    return j.null();
                }
                let stderr = j.constant(
                    "com/intellij/execution/process/ProcessOutputTypes",
                    "STDERR",
                    "Lcom/intellij/openapi/util/Key;",
                )?;
                if j.same(&args[1], &stderr)? {
                    print_console(j, &output, &text, "ERROR_OUTPUT")?;
                    return j.null();
                }
                let mut buffer = buffer.lock().expect("cargo lines");
                if buffer.len() + text.len() > 1_048_576 {
                    print_console(j, &output, &buffer, "NORMAL_OUTPUT")?;
                    buffer.clear();
                }
                buffer.push_str(&text);
                while let Some(end) = buffer.find('\n') {
                    let line = buffer[..end].to_owned();
                    buffer.drain(..=end);
                    cargo_line(&project, j, &output, &line)?;
                }
            } else if op == "Callback.processTerminated" {
                let code = j.int(&args[0], "getExitCode")?;
                if checking {
                    let tail = std::mem::take(&mut *buffer.lock().expect("cargo lines"));
                    if !tail.is_empty() {
                        cargo_line(&project, j, &output, &tail)?;
                    }
                    print_console(
                        j,
                        &output,
                        &format!("\nCargo exited with code {code}\n"),
                        "SYSTEM_OUTPUT",
                    )?;
                }
                jvm::unregister(retained_id.load(Ordering::Acquire));
                project.process.lock().expect("process").take();
                {
                    let mut snapshot = project.snapshot.lock().expect("snapshot");
                    snapshot.busy = false;
                    snapshot.status = if code == 0 {
                        format!("Finished: {label}")
                    } else {
                        format!("Cargo failed ({code}). See the Run console.")
                    };
                }
                project.publish();
            }
            j.null()
        });
        callback_id.store(id, Ordering::Release);
        let callback = jvm::callback(j, id)?;
        j.void(
            &handler,
            "addProcessListener",
            "(Lcom/intellij/execution/process/ProcessListener;)V",
            &[A::O(&callback)],
        )?;
        let component = j.obj(&console, "getComponent", "()Ljavax/swing/JComponent;", &[])?;
        let descriptor=j.new("com/intellij/execution/ui/RunContentDescriptor","(Lcom/intellij/execution/ui/ExecutionConsole;Lcom/intellij/execution/process/ProcessHandler;Ljavax/swing/JComponent;Ljava/lang/String;)V",&[A::O(&console),A::O(&handler),A::O(&component),A::S(&format!("Cranpose: {task}"))])?;
        let manager = j.static_obj(
            "com/intellij/execution/ui/RunContentManager",
            "getInstance",
            "(Lcom/intellij/openapi/project/Project;)Lcom/intellij/execution/ui/RunContentManager;",
            &[A::O(&self.object)],
        )?;
        let executor = j.static_obj(
            "com/intellij/execution/executors/DefaultRunExecutor",
            "getRunExecutorInstance",
            "()Lcom/intellij/execution/Executor;",
            &[],
        )?;
        j.void(
            &manager,
            "showRunContent",
            "(Lcom/intellij/execution/Executor;Lcom/intellij/execution/ui/RunContentDescriptor;)V",
            &[A::O(&executor), A::O(&descriptor)],
        )?;
        *self.process.lock().expect("process") = Some(handler.clone());
        {
            let mut snapshot = self.snapshot.lock().expect("snapshot");
            snapshot.busy = true;
            snapshot.status = format!("{task}: {}", target.label);
            snapshot.diagnostics.clear();
        }
        self.publish();
        j.void(&handler, "startNotify", "()V", &[])
    }
    pub fn stop(&self, j: &mut J<'_>) -> Result<()> {
        let process = self.process.lock().expect("process").clone();
        if let Some(process) = process {
            j.void(&process, "destroyProcess", "()V", &[])?;
        }
        Ok(())
    }
    pub fn theme(&self, j: &mut J<'_>, panel: &Panel) -> Result<()> {
        let dark = !j
            .static_call("com/intellij/ui/JBColor", "isBright", "()Z", &[])?
            .z()?;
        let background = j.static_obj(
            "com/intellij/util/ui/UIUtil",
            "getPanelBackground",
            "()Ljava/awt/Color;",
            &[],
        )?;
        let text = j.static_obj(
            "com/intellij/util/ui/UIUtil",
            "getLabelForeground",
            "()Ljava/awt/Color;",
            &[],
        )?;
        let accent = j.static_obj(
            "com/intellij/util/ui/JBUI$CurrentTheme$Focus",
            "focusColor",
            "()Ljava/awt/Color;",
            &[],
        )?;
        let bg = rgb(j, &background)?;
        let raised = bg.map(|v| {
            (v as f64
                + ((if dark { 255.0 } else { 0.0 }) - v as f64) * if dark { 0.07 } else { 0.04 })
                as i32
        });
        panel.set_theme(dark);
        panel.message("ide.theme",&json!({"dark":dark,"background":hex(bg),"surface":hex(raised),"text":hex(rgb(j,&text)?),"muted":if dark{"#969daa"}else{"#6a6e78"},"accent":hex(rgb(j,&accent)?)}).to_string());
        Ok(())
    }
    pub fn editor_manager(&self, j: &mut J<'_>) -> Result<O> {
        j.static_obj("com/intellij/openapi/fileEditor/FileEditorManager","getInstance","(Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/fileEditor/FileEditorManager;",&[A::O(&self.object)])
    }
    pub fn selected(&self, j: &mut J<'_>) -> Result<(Option<O>, Option<O>)> {
        let manager = self.editor_manager(j)?;
        let files = j.obj(
            &manager,
            "getSelectedFiles",
            "()[Lcom/intellij/openapi/vfs/VirtualFile;",
            &[],
        )?;
        let file = j.elements(&files)?.into_iter().next();
        let editor = j.obj(
            &manager,
            "getSelectedTextEditor",
            "()Lcom/intellij/openapi/editor/Editor;",
            &[],
        )?;
        Ok((file, (!editor.is_null()).then_some(editor)))
    }
    pub fn send_editor(&self, j: &mut J<'_>) -> Result<()> {
        self.update_editor(j, true)
    }
    fn update_editor(&self, j: &mut J<'_>, force: bool) -> Result<()> {
        let (file, editor) = self.selected(j)?;
        let path = if let Some(file) = &file {
            j.text(file, "getPath")?
        } else {
            String::new()
        };
        let text = if let Some(editor) = editor {
            let document = j.obj(
                &editor,
                "getDocument",
                "()Lcom/intellij/openapi/editor/Document;",
                &[],
            )?;
            let stamp = j.long(&document, "getModificationStamp")?;
            let mut key = self.editor_key.lock().expect("editor key");
            if key.as_ref() == Some(&(path.clone(), stamp)) {
                drop(key);
                if force {
                    let payload = self.editor_stamp.lock().expect("editor").clone();
                    for panel in self.live_panels() {
                        panel.message("cranpose.editor", &payload);
                        panel.message("ide.editor", &payload);
                    }
                }
                return Ok(());
            }
            *key = Some((path.clone(), stamp));
            j.text(&document, "getText")?
        } else {
            String::new()
        };
        let payload=json!({"path":path,"name":Path::new(&path).file_name().unwrap_or_default().to_string_lossy(),"composables":if path.ends_with(".rs"){crate::source::symbols(&text)}else{vec![]}}).to_string();
        let changed = {
            let mut stamp = self.editor_stamp.lock().expect("editor");
            let changed = *stamp != payload;
            if changed {
                *stamp = payload.clone();
            }
            changed
        };
        if changed {
            for panel in self.live_panels() {
                panel.message("cranpose.editor", &payload);
                panel.message("ide.editor", &payload);
            }
        }
        Ok(())
    }
    fn caret(&self, j: &mut J<'_>, event: &O) -> Result<()> {
        crate::editor::caret(self, j, event)
    }
    pub fn property(&self, j: &mut J<'_>, key: &str) -> Result<String> {
        let properties = self.properties(j)?;
        let value = j.obj(
            &properties,
            "getValue",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[A::S(key)],
        )?;
        j.read_string(&value)
    }
    pub fn set_property(&self, j: &mut J<'_>, key: &str, value: &str) -> Result<()> {
        let properties = self.properties(j)?;
        j.void(
            &properties,
            "setValue",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[A::S(key), A::S(value)],
        )
    }
    fn properties(&self, j: &mut J<'_>) -> Result<O> {
        j.static_obj(
            "com/intellij/ide/util/PropertiesComponent",
            "getInstance",
            "(Lcom/intellij/openapi/project/Project;)Lcom/intellij/ide/util/PropertiesComponent;",
            &[A::O(&self.object)],
        )
    }
    pub fn dispose(&self, j: &mut J<'_>) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.stop(j)?;
        if let Some(timer) = self.timer.get() {
            j.void(timer, "stop", "()V", &[])?;
        }
        crate::stability::clear(self, j)?;
        crate::authoring::dispose(self, j)?;
        let workspaces = self
            .workspaces
            .lock()
            .expect("workspaces")
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for workspace in workspaces {
            workspace.dispose(j)?;
        }
        for panel in self.live_panels() {
            panel.close(j)?;
        }
        self.scope.clear();
        projects()
            .lock()
            .expect("projects")
            .retain(|p| !std::ptr::eq(Arc::as_ptr(p), self));
        Ok(())
    }
    pub fn create_starter(self: &Arc<Self>, j: &mut J<'_>) -> Result<()> {
        if self.root.join("Cargo.toml").exists() || self.root.join("src").exists() {
            return self.info(
                j,
                "Create a starter in an empty project directory. Cargo.toml or src already exists.",
            );
        }
        let name=j.static_obj("com/intellij/openapi/ui/Messages","showInputDialog","(Lcom/intellij/openapi/project/Project;Ljava/lang/String;Ljava/lang/String;Ljavax/swing/Icon;Ljava/lang/String;Lcom/intellij/openapi/ui/InputValidator;)Ljava/lang/String;",&[A::O(&self.object),A::S("Cargo package name"),A::S("Create Cranpose App"),A::Null,A::S("cranpose-app"),A::Null])?;
        if name.is_null() {
            return Ok(());
        }
        let name = j.read_string(&name)?;
        let manifest = match model::starter_manifest(&name) {
            Ok(s) => s,
            Err(e) => return self.info(j, &e.to_string()),
        };
        let fs = j.static_obj(
            "com/intellij/openapi/vfs/LocalFileSystem",
            "getInstance",
            "()Lcom/intellij/openapi/vfs/LocalFileSystem;",
            &[],
        )?;
        let root = j.obj(
            &fs,
            "refreshAndFindFileByPath",
            "(Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
            &[A::S(&self.root.to_string_lossy())],
        )?;
        ensure!(!root.is_null(), "Project directory is unavailable");
        let scope = jvm::Scope::default();
        let requestor = self.object.clone();
        let id = scope.register(move |j, _, _| {
            for name in ["Cargo.toml", "src"] {
                let child = j.obj(
                    &root,
                    "findChild",
                    "(Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
                    &[A::S(name)],
                )?;
                ensure!(child.is_null(), "{name} already exists");
            }
            let directory = j.obj(
                &root,
                "createChildDirectory",
                "(Ljava/lang/Object;Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
                &[A::O(&requestor), A::S("src")],
            )?;
            for (parent, name, text) in [
                (&root, "Cargo.toml", manifest.as_str()),
                (&directory, "main.rs", crate::source::STARTER),
            ] {
                let file = j.obj(
                    parent,
                    "createChildData",
                    "(Ljava/lang/Object;Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
                    &[A::O(&requestor), A::S(name)],
                )?;
                j.static_void(
                    "com/intellij/openapi/vfs/VfsUtil",
                    "saveText",
                    "(Lcom/intellij/openapi/vfs/VirtualFile;Ljava/lang/String;)V",
                    &[A::O(&file), A::S(text)],
                )?;
            }
            j.null()
        });
        let callback = jvm::callback(j, id)?;
        let array =
            j.env
                .new_object_array(0, "com/intellij/psi/PsiFile", jni::objects::JObject::null())?;
        let files = j.global(array)?;
        j.static_void("com/intellij/openapi/command/WriteCommandAction","runWriteCommandAction","(Lcom/intellij/openapi/project/Project;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Runnable;[Lcom/intellij/psi/PsiFile;)V",&[A::O(&self.object),A::S("Create Cranpose application"),A::Null,A::O(&callback),A::O(&files)])?;
        self.refresh(j)
    }
    pub fn info(&self, j: &mut J<'_>, text: &str) -> Result<()> {
        j.static_void(
            "com/intellij/openapi/ui/Messages",
            "showInfoMessage",
            "(Lcom/intellij/openapi/project/Project;Ljava/lang/String;Ljava/lang/String;)V",
            &[A::O(&self.object), A::S(text), A::S("Cranpose")],
        )
    }
}
pub fn command(
    j: &mut J<'_>,
    root: &Path,
    args: &[String],
    environment: &BTreeMap<String, String>,
) -> Result<O> {
    let mut values = vec![j.string(&model::cargo().to_string_lossy())?];
    for arg in args {
        values.push(j.string(arg)?);
    }
    let array = j.array("java/lang/String", &values)?;
    let command = j.new(
        "com/intellij/execution/configurations/GeneralCommandLine",
        "([Ljava/lang/String;)V",
        &[A::O(&array)],
    )?;
    j.obj(
        &command,
        "withWorkDirectory",
        "(Ljava/lang/String;)Lcom/intellij/execution/configurations/GeneralCommandLine;",
        &[A::S(&root.to_string_lossy())],
    )?;
    let charset = j.constant(
        "java/nio/charset/StandardCharsets",
        "UTF_8",
        "Ljava/nio/charset/Charset;",
    )?;
    j.obj(
        &command,
        "withCharset",
        "(Ljava/nio/charset/Charset;)Lcom/intellij/execution/configurations/GeneralCommandLine;",
        &[A::O(&charset)],
    )?;
    let parent = j.constant(
        "com/intellij/execution/configurations/GeneralCommandLine$ParentEnvironmentType",
        "CONSOLE",
        "Lcom/intellij/execution/configurations/GeneralCommandLine$ParentEnvironmentType;",
    )?;
    j.obj(&command,"withParentEnvironmentType","(Lcom/intellij/execution/configurations/GeneralCommandLine$ParentEnvironmentType;)Lcom/intellij/execution/configurations/GeneralCommandLine;",&[A::O(&parent)])?;
    for (name, value) in std::iter::once(("CARGO_TERM_COLOR", "never"))
        .chain(environment.iter().map(|(k, v)| (k.as_str(), v.as_str())))
    {
        j.obj(&command,"withEnvironment","(Ljava/lang/String;Ljava/lang/String;)Lcom/intellij/execution/configurations/GeneralCommandLine;",&[A::S(name),A::S(value)])?;
    }
    Ok(command)
}
pub fn subscribe(
    j: &mut J<'_>,
    bus: &O,
    parent: &O,
    callback: &O,
    class: &str,
    field: &str,
) -> Result<()> {
    let connection = j.obj(
        bus,
        "connect",
        "(Lcom/intellij/openapi/Disposable;)Lcom/intellij/util/messages/MessageBusConnection;",
        &[A::O(parent)],
    )?;
    let topic = j.constant(class, field, "Lcom/intellij/util/messages/Topic;")?;
    j.void(
        &connection,
        "subscribe",
        "(Lcom/intellij/util/messages/Topic;Ljava/lang/Object;)V",
        &[A::O(&topic), A::O(callback)],
    )
}
pub fn binary(j: &mut J<'_>) -> Result<PathBuf> {
    for key in ["cranpose.idea.ui.binary", "cranpose.ui.binary"] {
        let property = j.static_obj(
            "java/lang/System",
            "getProperty",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[A::S(key)],
        )?;
        let property = j.read_string(&property)?;
        if !property.is_empty() {
            return Ok(property.into());
        }
    }
    for key in ["CRANPOSE_IDEA_UI_BINARY", "CRANPOSE_UI_BINARY"] {
        if let Some(path) = std::env::var_os(key) {
            return Ok(path.into());
        }
    }
    let class = j.class("dev/cranpose/rust/Native")?;
    let loader = j.obj(&class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?;
    let descriptor = j.obj(
        &loader,
        "getPluginDescriptor",
        "()Lcom/intellij/openapi/extensions/PluginDescriptor;",
        &[],
    )?;
    let path = j.obj(&descriptor, "getPluginPath", "()Ljava/nio/file/Path;", &[])?;
    let lib = PathBuf::from(j.text(&path, "toString")?).join("lib");
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    let exe = if cfg!(windows) {
        "cranpose-intellij-ui.exe"
    } else {
        "cranpose-intellij-ui"
    };
    let binary = lib
        .join("native")
        .join(format!("{os}-{}", std::env::consts::ARCH))
        .join(exe);
    ensure!(binary.is_file(), "Missing native UI: {}", binary.display());
    Ok(binary)
}
pub fn cache(j: &mut J<'_>) -> Result<PathBuf> {
    let path = j.static_obj(
        "com/intellij/openapi/application/PathManager",
        "getSystemPath",
        "()Ljava/lang/String;",
        &[],
    )?;
    Ok(PathBuf::from(j.read_string(&path)?).join("cranpose-dev"))
}
pub fn options(j: &mut J<'_>, studio: bool) -> Result<Options> {
    Ok(Options {
        command: vec![binary(j)?.to_string_lossy().into_owned()],
        directory: None,
        environment: if studio {
            BTreeMap::from([("CRANPOSE_STUDIO".into(), "1".into())])
        } else {
            BTreeMap::new()
        },
        timeout: Duration::from_secs(15),
    })
}
pub fn navigate(
    project: &Project,
    j: &mut J<'_>,
    path: &str,
    line: i32,
    column: i32,
) -> Result<()> {
    crate::workspace::reveal_source(project, j, path)?;
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
    let descriptor = j.new(
        "com/intellij/openapi/fileEditor/OpenFileDescriptor",
        "(Lcom/intellij/openapi/project/Project;Lcom/intellij/openapi/vfs/VirtualFile;II)V",
        &[
            A::O(&project.object),
            A::O(&file),
            A::I(line.max(0)),
            A::I(column.max(0)),
        ],
    )?;
    j.void(&descriptor, "navigate", "(Z)V", &[A::Z(true)])
}
fn rgb(j: &mut J<'_>, color: &O) -> Result<[i32; 3]> {
    Ok([
        j.int(color, "getRed")?,
        j.int(color, "getGreen")?,
        j.int(color, "getBlue")?,
    ])
}
fn hex([r, g, b]: [i32; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn print_console(j: &mut J<'_>, console: &O, text: &str, kind: &str) -> Result<()> {
    let kind = j.constant(
        "com/intellij/execution/ui/ConsoleViewContentType",
        kind,
        "Lcom/intellij/execution/ui/ConsoleViewContentType;",
    )?;
    j.void(
        console,
        "print",
        "(Ljava/lang/String;Lcom/intellij/execution/ui/ConsoleViewContentType;)V",
        &[A::S(text), A::O(&kind)],
    )
}
fn cargo_line(project: &Project, j: &mut J<'_>, console: &O, line: &str) -> Result<()> {
    if let Some(mut diagnostic) = model::diagnostic(line) {
        let file = diagnostic["file"].as_str().unwrap_or_default();
        if !file.is_empty() {
            let path = PathBuf::from(file);
            let path = if path.is_absolute() {
                path
            } else {
                project.root.join(path)
            };
            diagnostic["file"] = json!(path.to_string_lossy());
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
                &[A::S(&path.to_string_lossy())],
            )?;
            if !file.is_null() {
                let row = diagnostic["line"].as_i64().unwrap_or(1) as i32;
                let column = diagnostic["column"].as_i64().unwrap_or(1) as i32;
                let link=j.new("com/intellij/execution/filters/OpenFileHyperlinkInfo","(Lcom/intellij/openapi/project/Project;Lcom/intellij/openapi/vfs/VirtualFile;II)V",&[A::O(&project.object),A::O(&file),A::I((row-1).max(0)),A::I((column-1).max(0))])?;
                j.void(
                    console,
                    "printHyperlink",
                    "(Ljava/lang/String;Lcom/intellij/execution/filters/HyperlinkInfo;)V",
                    &[
                        A::S(&format!("{}:{row}:{column}\n", path.display())),
                        A::O(&link),
                    ],
                )?;
            }
        }
        let rendered = diagnostic["rendered"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| diagnostic["message"].as_str().unwrap_or_default());
        print_console(
            j,
            console,
            &format!("{rendered}\n"),
            if diagnostic["level"] == "error" {
                "ERROR_OUTPUT"
            } else {
                "NORMAL_OUTPUT"
            },
        )?;
        project
            .snapshot
            .lock()
            .expect("snapshot")
            .diagnostics
            .push(diagnostic);
    } else if serde_json::from_str::<serde_json::Value>(line).is_err() {
        print_console(j, console, &format!("{line}\n"), "NORMAL_OUTPUT")?;
    }
    Ok(())
}
