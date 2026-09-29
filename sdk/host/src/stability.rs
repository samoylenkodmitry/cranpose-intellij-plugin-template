//! Debounced analysis and inline editor badges, implemented in Rust.
use crate::{
    jvm::{self, A, J, O},
    model,
    project::{self, Project},
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Deserialize)]
pub struct Badge {
    pub start: i32,
    pub end: i32,
    pub label: String,
    pub tone: String,
    pub detail: String,
}
struct Placed {
    inlay: O,
    id: i64,
    badge: Badge,
}
struct Document {
    object: O,
    path: String,
    stamp: i64,
}
struct Analysis {
    revision: u64,
    documents: Vec<Document>,
    files: BTreeMap<String, Vec<Badge>>,
}
pub struct State {
    revision: Arc<AtomicU64>,
    due: Option<Instant>,
    receiver: Option<mpsc::Receiver<Analysis>>,
    placed: Vec<Placed>,
    tooltip: Option<O>,
    hovered: Option<i64>,
    pub enabled: bool,
    pub show_stable: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            revision: Arc::new(AtomicU64::new(1)),
            due: Some(Instant::now() + Duration::from_millis(450)),
            receiver: None,
            placed: vec![],
            tooltip: None,
            hovered: None,
            enabled: true,
            show_stable: true,
        }
    }
}
impl State {
    pub fn schedule(&mut self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.due = Some(Instant::now() + Duration::from_millis(450));
    }
}
pub fn install(project: &Arc<Project>, j: &mut J<'_>, multicaster: &O) -> Result<()> {
    let enabled = project.property(j, "cranpose.stability.enabled")? != "false";
    let stable = project.property(j, "cranpose.stability.stable")? != "false";
    {
        let mut state = project.stability.lock().expect("stability");
        state.enabled = enabled;
        state.show_stable = stable;
    }
    let weak = Arc::downgrade(project);
    let id = project.scope.register(move |j, op, args| {
        if let Some(project) = weak.upgrade() {
            if op == "Callback.mouseExited" {
                hide_tooltip(&project, j)?;
            } else if !args.is_empty() {
                tooltip(&project, j, &args[0], op == "Callback.mouseClicked")?;
            }
        }
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    j.void(multicaster,"addEditorMouseMotionListener","(Lcom/intellij/openapi/editor/event/EditorMouseMotionListener;Lcom/intellij/openapi/Disposable;)V",&[A::O(&callback),A::O(&project.object)])?;
    j.void(multicaster,"addEditorMouseListener","(Lcom/intellij/openapi/editor/event/EditorMouseListener;Lcom/intellij/openapi/Disposable;)V",&[A::O(&callback),A::O(&project.object)])?;
    Ok(())
}
pub fn tick(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    let result = {
        let mut state = project.stability.lock().expect("stability");
        let result = state.receiver.as_ref().and_then(|r| r.try_recv().ok());
        if result.is_some() {
            state.receiver = None;
        }
        result
    };
    if let Some(result) = result {
        let (revision, enabled, show_stable) = {
            let state = project.stability.lock().expect("stability");
            (
                state.revision.load(Ordering::Acquire),
                state.enabled,
                state.show_stable,
            )
        };
        if revision == result.revision && enabled {
            let mut fresh = true;
            for document in &result.documents {
                if j.long(&document.object, "getModificationStamp")? != document.stamp {
                    fresh = false;
                    break;
                }
            }
            if fresh {
                clear(project, j)?;
                for editor in editors(j)? {
                    if j.bool(&editor, "isDisposed")? {
                        continue;
                    }
                    let owner = j.obj(
                        &editor,
                        "getProject",
                        "()Lcom/intellij/openapi/project/Project;",
                        &[],
                    )?;
                    if !j.same(&owner, &project.object)? {
                        continue;
                    }
                    let document = j.obj(
                        &editor,
                        "getDocument",
                        "()Lcom/intellij/openapi/editor/Document;",
                        &[],
                    )?;
                    for expected in &result.documents {
                        if j.same(&document, &expected.object)? {
                            let inlay_model = j.obj(
                                &editor,
                                "getInlayModel",
                                "()Lcom/intellij/openapi/editor/InlayModel;",
                                &[],
                            )?;
                            let length = j.int(&document, "getTextLength")?;
                            for badge in result.files.get(&expected.path).into_iter().flatten() {
                                if (!show_stable && badge.tone == "stable")
                                    || badge.start < 0
                                    || badge.end < badge.start
                                    || badge.end > length
                                {
                                    continue;
                                }
                                let badge = badge.clone();
                                let captured = badge.clone();
                                let id = jvm::register(move |j, op, args| {
                                    if op == "Badge.calcWidthInPixels" {
                                        let width = crate::glyphs::badge_width(
                                            j,
                                            &args[0],
                                            &captured.label,
                                        )?;
                                        j.boxed_int(width)
                                    } else {
                                        crate::glyphs::badge(
                                            j,
                                            args,
                                            &captured.label,
                                            &captured.tone,
                                        )?;
                                        j.null()
                                    }
                                });
                                let renderer =
                                    j.new("dev/cranpose/rust/Badge", "(J)V", &[A::J(id)])?;
                                let inlay=j.obj(&inlay_model,"addInlineElement","(IZLcom/intellij/openapi/editor/EditorCustomElementRenderer;)Lcom/intellij/openapi/editor/Inlay;",&[A::I(badge.end),A::Z(true),A::O(&renderer)])?;
                                if inlay.is_null() {
                                    jvm::unregister(id);
                                } else {
                                    project
                                        .stability
                                        .lock()
                                        .expect("stability")
                                        .placed
                                        .push(Placed { inlay, id, badge });
                                    crate::authoring::invalidate(project);
                                }
                            }
                        }
                    }
                }
            } else {
                project.stability.lock().expect("stability").schedule();
            }
        }
    }
    let start = {
        let mut state = project.stability.lock().expect("stability");
        if state.enabled && state.due.is_some_and(|d| Instant::now() >= d) {
            state.due = None;
            Some((
                state.revision.clone(),
                state.revision.load(Ordering::Acquire),
            ))
        } else {
            None
        }
    };
    if let Some((revision, expected)) = start {
        let manager = j.static_obj(
            "com/intellij/openapi/fileEditor/FileDocumentManager",
            "getInstance",
            "()Lcom/intellij/openapi/fileEditor/FileDocumentManager;",
            &[],
        )?;
        let mut documents = vec![];
        let mut overlays = BTreeMap::new();
        for editor in editors(j)? {
            if j.bool(&editor, "isDisposed")? {
                continue;
            }
            let owner = j.obj(
                &editor,
                "getProject",
                "()Lcom/intellij/openapi/project/Project;",
                &[],
            )?;
            if !j.same(&owner, &project.object)? {
                continue;
            }
            let document = j.obj(
                &editor,
                "getDocument",
                "()Lcom/intellij/openapi/editor/Document;",
                &[],
            )?;
            if let Some(path) = document_path(project, j, &manager, &document)? {
                let source = j.text(&document, "getText")?;
                let absolute = project.root.join(&path);
                overlays.insert(absolute.to_string_lossy().into_owned(), source);
                if !documents.iter().any(|d: &Document| d.path == path) {
                    documents.push(Document {
                        stamp: j.long(&document, "getModificationStamp")?,
                        object: document,
                        path,
                    });
                }
            }
        }
        if documents.is_empty() {
            return Ok(());
        }
        let unsaved = j.obj(
            &manager,
            "getUnsavedDocuments",
            "()[Lcom/intellij/openapi/editor/Document;",
            &[],
        )?;
        for document in j.elements(&unsaved)? {
            if let Some(path) = document_path(project, j, &manager, &document)? {
                overlays
                    .entry(project.root.join(path).to_string_lossy().into_owned())
                    .or_insert(j.text(&document, "getText")?);
            }
        }
        let request = json!({"root":project.root,"only":documents.iter().map(|d|&d.path).collect::<Vec<_>>(),"overlays":overlays.into_iter().map(|(path,source)|json!({"path":path,"source":source})).collect::<Vec<_>>()});
        let binary = project::binary(j)?;
        let weak = Arc::downgrade(project);
        let (sender, receiver) = mpsc::channel();
        project.stability.lock().expect("stability").receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = (|| -> Result<BTreeMap<String, Vec<Badge>>> {
                let mut command = Command::new(binary);
                command.arg("--stability");
                let output = crate::process::capture(
                    command,
                    Some(serde_json::to_vec(&request)?),
                    Duration::from_secs(30),
                    || {
                        revision.load(Ordering::Acquire) != expected
                            || weak
                                .upgrade()
                                .is_none_or(|p| p.closed.load(Ordering::Acquire))
                    },
                )?;
                ensure!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                parse(&String::from_utf8(output.stdout)?)
            })();
            if revision.load(Ordering::Acquire) != expected {
                return;
            }
            let files = result.unwrap_or_else(|error| {
                documents
                    .iter()
                    .map(|d| {
                        (
                            d.path.clone(),
                            vec![Badge {
                                start: 0,
                                end: 0,
                                label: "analysis unavailable".into(),
                                tone: "muted".into(),
                                detail: format!("{error:#}"),
                            }],
                        )
                    })
                    .collect()
            });
            let _ = sender.send(Analysis {
                revision: expected,
                documents,
                files,
            });
        });
    }
    Ok(())
}
pub fn parse(source: &str) -> Result<BTreeMap<String, Vec<Badge>>> {
    let value: Value = serde_json::from_str(source)?;
    ensure!(
        value["schemaVersion"] == 1,
        "Unsupported stability report version"
    );
    if let Some(error) = value["error"].as_str() {
        anyhow::bail!("{error}");
    }
    let mut files = BTreeMap::new();
    for file in value["files"].as_array().into_iter().flatten() {
        files.insert(
            model::s(file, "path"),
            serde_json::from_value::<Vec<Badge>>(file["badges"].clone())?,
        );
    }
    for diagnostic in value["diagnostics"].as_array().into_iter().flatten() {
        if diagnostic["rule"] == "CP000" {
            let offset = diagnostic["location"]["utf16Start"].as_i64().unwrap_or(0) as i32;
            files
                .entry(model::s(diagnostic, "path"))
                .or_insert_with(Vec::new)
                .push(Badge {
                    start: offset,
                    end: offset,
                    label: "syntax error".into(),
                    tone: "danger".into(),
                    detail: model::s(diagnostic, "message"),
                });
        }
    }
    Ok(files)
}
fn editors(j: &mut J<'_>) -> Result<Vec<O>> {
    let factory = j.static_obj(
        "com/intellij/openapi/editor/EditorFactory",
        "getInstance",
        "()Lcom/intellij/openapi/editor/EditorFactory;",
        &[],
    )?;
    let editors = j.obj(
        &factory,
        "getAllEditors",
        "()[Lcom/intellij/openapi/editor/Editor;",
        &[],
    )?;
    j.elements(&editors)
}
fn document_path(
    project: &Project,
    j: &mut J<'_>,
    manager: &O,
    document: &O,
) -> Result<Option<String>> {
    let file = j.obj(
        manager,
        "getFile",
        "(Lcom/intellij/openapi/editor/Document;)Lcom/intellij/openapi/vfs/VirtualFile;",
        &[A::O(document)],
    )?;
    if file.is_null() {
        return Ok(None);
    }
    let path = PathBuf::from(j.text(&file, "getPath")?);
    if path.extension().is_none_or(|s| s != "rs") {
        return Ok(None);
    }
    Ok(path
        .strip_prefix(&project.root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/")))
}
pub fn clear(project: &Project, j: &mut J<'_>) -> Result<()> {
    crate::authoring::invalidate(project);
    hide_tooltip(project, j)?;
    let placed = std::mem::take(&mut project.stability.lock().expect("stability").placed);
    for placed in placed {
        if j.bool(&placed.inlay, "isValid")? {
            j.void(&placed.inlay, "dispose", "()V", &[])?;
        }
        jvm::unregister(placed.id);
    }
    Ok(())
}
fn hide_tooltip(project: &Project, j: &mut J<'_>) -> Result<()> {
    let tooltip = {
        let mut state = project.stability.lock().expect("stability");
        state.hovered = None;
        state.tooltip.take()
    };
    if let Some(tooltip) = tooltip {
        j.void(&tooltip, "hide", "()V", &[])?;
    }
    Ok(())
}
fn tooltip(project: &Project, j: &mut J<'_>, event: &O, immediate: bool) -> Result<()> {
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
    if immediate && j.int(&mouse, "getButton")? != 1 {
        return Ok(());
    }
    let point = j.obj(&mouse, "getPoint", "()Ljava/awt/Point;", &[])?;
    let model = j.obj(
        &editor,
        "getInlayModel",
        "()Lcom/intellij/openapi/editor/InlayModel;",
        &[],
    )?;
    let class = j.class("dev/cranpose/rust/Badge")?;
    let inlay = j.obj(
        &model,
        "getElementAt",
        "(Ljava/awt/Point;Ljava/lang/Class;)Lcom/intellij/openapi/editor/Inlay;",
        &[A::O(&point), A::O(&class)],
    )?;
    let badge = if inlay.is_null() {
        None
    } else {
        let renderer = j.obj(
            &inlay,
            "getRenderer",
            "()Lcom/intellij/openapi/editor/EditorCustomElementRenderer;",
            &[],
        )?;
        let id = j.id(&renderer)?;
        let state = project.stability.lock().expect("stability");
        if !immediate && state.hovered == Some(id) {
            return Ok(());
        }
        state
            .placed
            .iter()
            .find(|p| p.id == id)
            .map(|p| (id, p.badge.clone()))
    };
    hide_tooltip(project, j)?;
    let Some((id, badge)) = badge else {
        return Ok(());
    };
    let tooltip = badge_tooltip(j, &editor, &point, &badge.detail)?;
    let manager = j.static_obj(
        "com/intellij/ide/IdeTooltipManager",
        "getInstance",
        "()Lcom/intellij/ide/IdeTooltipManager;",
        &[],
    )?;
    j.obj(
        &manager,
        "show",
        "(Lcom/intellij/ide/IdeTooltip;Z)Lcom/intellij/ide/IdeTooltip;",
        &[A::O(&tooltip), A::Z(immediate)],
    )?;
    let mut state = project.stability.lock().expect("stability");
    state.tooltip = Some(tooltip);
    state.hovered = Some(id);
    Ok(())
}
fn badge_tooltip(j: &mut J<'_>, editor: &O, point: &O, detail: &str) -> Result<O> {
    let detail = escape(detail).replace('\n', "<br>");
    let label = j.new(
        "javax/swing/JLabel",
        "(Ljava/lang/String;)V",
        &[A::S(&format!(
            "<html><body style='width: 360px'>{detail}</body></html>"
        ))],
    )?;
    let component = j.obj(
        editor,
        "getContentComponent",
        "()Ljavax/swing/JComponent;",
        &[],
    )?;
    // Java varargs must be an array even when there are no equality keys.
    let keys = j.array("java/lang/Object", &[])?;
    j.new(
        "com/intellij/ide/IdeTooltip",
        "(Ljava/awt/Component;Ljava/awt/Point;Ljavax/swing/JComponent;[Ljava/lang/Object;)V",
        &[A::O(&component), A::O(point), A::O(&label), A::O(&keys)],
    )
}
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
#[cfg(feature = "ide-tests")]
pub(crate) fn integration_test(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    document: &O,
) -> Result<()> {
    let mut previous = None;
    for (revision_delta, stamp_delta, label, count) in [
        (0, 0, "stable", 1),
        (-1, 0, "stale", 1),
        (0, -1, "stale document", 1),
        (0, 0, "updated", 1),
    ] {
        let (sender, receiver) = mpsc::channel();
        let stamp = j.long(document, "getModificationStamp")?;
        {
            let mut state = project.stability.lock().expect("state");
            state.enabled = true;
            state.due = None;
            state.receiver = Some(receiver);
            let revision = (state.revision.load(Ordering::Acquire) as i64 + revision_delta) as u64;
            sender.send(Analysis {
                revision,
                documents: vec![Document {
                    object: document.clone(),
                    path: "test.rs".into(),
                    stamp: stamp + stamp_delta,
                }],
                files: BTreeMap::from([(
                    "test.rs".into(),
                    vec![Badge {
                        start: 8,
                        end: 13,
                        label: label.into(),
                        tone: "stable".into(),
                        detail: "Tracked by value".into(),
                    }],
                )]),
            })?;
        }
        tick(project, j)?;
        let state = project.stability.lock().expect("state");
        ensure!(state.placed.len() == count, "Inline badge count");
        let inlay = state.placed[0].inlay.clone();
        drop(state);
        ensure!(j.bool(&inlay, "isValid")?, "Inlay disposed early");
        ensure!(
            j.int(&inlay, "getWidthInPixels")? > 0,
            "Invisible stability badge"
        );
        ensure!(
            crate::glyphs::rendered(j, &inlay)?.len() > 40,
            "Stability badge did not paint in the editor"
        );
        if revision_delta < 0 || stamp_delta < 0 {
            ensure!(
                j.same(
                    &inlay,
                    previous
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("previous"))?
                )?,
                "Stale result replaced live badge"
            );
        }
        previous = Some(inlay);
    }
    clear(project, j)?;
    ensure!(
        !j.bool(
            previous.as_ref().ok_or_else(|| anyhow::anyhow!("inlay"))?,
            "isValid"
        )?,
        "Badge disposal"
    );
    let point = j.new("java/awt/Point", "(II)V", &[A::I(10), A::I(10)])?;
    let tip = badge_tooltip(
        j,
        editor,
        &point,
        "Callback <Fn()> & explanation\nSuppression reason",
    )?;
    ensure!(!tip.is_null(), "Stability explanation tooltip");
    j.void(&tip, "hide", "()V", &[])?;
    project.stability.lock().expect("state").enabled = false;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_errors_become_visible_badges() {
        let report = json!({"schemaVersion":1,"files":[],"diagnostics":[{"rule":"CP000","path":"src/a.rs","location":{"utf16Start":3},"message":"expected }"}]});
        let result = parse(&report.to_string()).expect("parse");
        assert_eq!(result["src/a.rs"][0].label, "syntax error");
        assert_eq!(result["src/a.rs"][0].end, 3);
    }
    #[test]
    fn unsupported_reports_fail() {
        assert!(parse(r#"{"schemaVersion":2}"#).is_err());
        assert!(parse(r#"{"schemaVersion":1,"error":"invalid config"}"#).is_err());
    }
}
