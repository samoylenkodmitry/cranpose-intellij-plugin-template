//! Bounded charging and completion effects for confirmed live value updates.
use crate::{
    jvm::{A, J, O},
    project::Project,
    surface::{Panel, Surface},
};
use anyhow::Result;
use cranpose_plugin_authoring::{Catalog, feedback::ViewTarget};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct State {
    edit: Option<Edit>,
    overlay: Option<(Weak<Panel>, Weak<Surface>)>,
    showing: Option<Instant>,
    charging: bool,
}
#[derive(Clone)]
pub struct Edit {
    path: String,
    stamp: i64,
    offset: i32,
    started: Instant,
}
#[derive(Clone)]
pub struct Trace {
    edit: Edit,
    file: String,
    target: ViewTarget,
    revision: u64,
    schema: String,
    generation: Option<u64>,
    composed: Option<Instant>,
    request: Option<u64>,
    attempts: usize,
}
impl Trace {
    pub fn follows(&self, frame: Instant) -> bool {
        self.composed.is_some_and(|composed| frame >= composed)
    }
    /// Ignore stale acknowledgments, rejected source schemas and composition
    /// events from another update. A frame alone is not proof of live values.
    pub fn message(&mut self, channel: &str, payload: &str) -> bool {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            return false;
        };
        if channel == "cranpose.dev.values.result"
            && value["revision"].as_u64() == Some(self.revision)
            && value["file"].as_str() == Some(&self.file)
            && value["schema"].as_str() == Some(&self.schema)
        {
            if value["accepted"] != true || value["changed"] != true {
                return true;
            }
            self.generation = value["generation"].as_u64();
        } else if channel == "cranpose.dev.composed"
            && self.generation.is_some()
            && value["generation"].as_u64() == self.generation
        {
            self.composed = Some(Instant::now());
        }
        false
    }
    pub fn expired(&self) -> bool {
        self.edit.started.elapsed() > Duration::from_secs(10)
    }
    pub fn request(&mut self) -> Option<u64> {
        if self.expired() || self.composed.is_none() || self.request.is_some() || self.attempts >= 8
        {
            return None;
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 63);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.request = Some(id);
        self.attempts += 1;
        Some(id)
    }
    pub fn reply(&mut self, payload: &str) -> Option<[f64; 4]> {
        let id = request_id(payload)?;
        if self.request != Some(id) || self.expired() {
            return None;
        }
        self.request = None;
        self.target.bounds(&self.file, payload, id)
    }
}
pub fn is_reply(payload: &str) -> bool {
    request_id(payload).is_some_and(|id| id & (1 << 63) != 0)
}
fn request_id(payload: &str) -> Option<u64> {
    // Ignore node payloads without allocating another JSON tree on regular
    // inspector replies. Reserved IDs never enter Studio's inspection sequence.
    #[derive(serde::Deserialize)]
    struct Header {
        #[serde(rename = "requestId")]
        request: u64,
    }
    if payload.len() > 8 * 1024 * 1024 {
        return None;
    }
    serde_json::from_str::<Header>(payload)
        .ok()
        .map(|h| h.request)
}
pub fn document_changed(project: &Arc<Project>, j: &mut J<'_>, event: &O) -> Result<()> {
    let (Some(file), Some(editor)) = project.selected(j)? else {
        return Ok(());
    };
    let path = j.text(&file, "getPath")?;
    if !path.ends_with(".rs") || !Path::new(&path).starts_with(&project.root) {
        return Ok(());
    }
    let document = j.obj(
        event,
        "getDocument",
        "()Lcom/intellij/openapi/editor/Document;",
        &[],
    )?;
    let selected = j.obj(
        &editor,
        "getDocument",
        "()Lcom/intellij/openapi/editor/Document;",
        &[],
    )?;
    if !j.same(&document, &selected)? {
        return Ok(());
    }
    cancel(project, j)?;
    let offset = j.int(event, "getOffset")?;
    let length = j.int(event, "getNewLength")?;
    project.feedback.lock().expect("feedback").edit = Some(Edit {
        path,
        stamp: j.long(&document, "getModificationStamp")?,
        offset: offset + length.saturating_sub(1),
        started: Instant::now(),
    });
    // A new edit invalidates an older in-flight match, even if temporarily invalid Rust.
    for workspace in project
        .workspaces
        .lock()
        .expect("workspaces")
        .iter()
        .filter_map(Weak::upgrade)
    {
        workspace.trace(None);
    }
    Ok(())
}
pub fn parsed(project: &Arc<Project>, path: &str, stamp: i64, catalog: &Catalog, revision: u64) {
    let edit = project.feedback.lock().expect("feedback").edit.clone();
    let Some(edit) = edit.filter(|e| e.path == path && e.stamp == stamp) else {
        return;
    };
    let Some(target) = ViewTarget::at(catalog, edit.offset as usize) else {
        project.feedback.lock().expect("feedback").edit = None;
        return;
    };
    let Ok(file) = Path::new(path).strip_prefix(&project.root) else {
        return;
    };
    let trace = Trace {
        edit,
        file: file.to_string_lossy().replace('\\', "/"),
        target,
        revision,
        schema: catalog.schema.clone(),
        generation: None,
        composed: None,
        request: None,
        attempts: 0,
    };
    for workspace in project
        .workspaces
        .lock()
        .expect("workspaces")
        .iter()
        .filter_map(Weak::upgrade)
    {
        workspace.trace(Some(trace.clone()));
    }
}
pub fn attach(project: &Project, panel: &Arc<Panel>, surface: &Arc<Surface>) {
    project.feedback.lock().expect("feedback").overlay =
        Some((Arc::downgrade(panel), Arc::downgrade(surface)));
}
pub fn tick(project: &Project, j: &mut J<'_>) -> Result<()> {
    let (expired, pending) = {
        let state = project.feedback.lock().expect("feedback");
        (
            (state.charging && state.edit.is_none())
                || state
                    .edit
                    .as_ref()
                    .is_some_and(|e| e.started.elapsed() > Duration::from_secs(10))
                || (!state.charging
                    && state
                        .showing
                        .is_some_and(|t| t.elapsed() > Duration::from_millis(1100))),
            state.edit.clone().filter(|_| state.showing.is_none()),
        )
    };
    if expired {
        stop(project, j)?;
    } else if let Some(edit) = pending {
        let active = project
            .workspaces
            .lock()
            .expect("workspaces")
            .iter()
            .filter_map(Weak::upgrade)
            .any(|w| w.has_preview());
        if active {
            show(project, j, &edit, None)?;
        }
    }
    Ok(())
}
pub fn stop(project: &Project, j: &mut J<'_>) -> Result<()> {
    project.feedback.lock().expect("feedback").edit = None;
    cancel(project, j)
}
fn cancel(project: &Project, j: &mut J<'_>) -> Result<()> {
    let overlay = {
        let mut state = project.feedback.lock().expect("feedback");
        state.charging = false;
        state.showing.take().and_then(|_| state.overlay.clone())
    };
    if let Some((panel, surface)) = overlay {
        if let Some(panel) = panel.upgrade() {
            panel.message("ide.authoring.bolt", "{\"clear\":true}");
        }
        let Some(surface) = surface.upgrade() else {
            return Ok(());
        };
        j.void(surface.component(), "setVisible", "(Z)V", &[A::Z(false)])?;
        surface.clear_frame(j)?;
    }
    Ok(())
}
fn point(j: &mut J<'_>, from: &O, x: i32, y: i32, to: &O) -> Result<[i32; 2]> {
    let p = j.static_obj(
        "javax/swing/SwingUtilities",
        "convertPoint",
        "(Ljava/awt/Component;IILjava/awt/Component;)Ljava/awt/Point;",
        &[A::O(from), A::I(x), A::I(y), A::O(to)],
    )?;
    Ok([j.field_int(&p, "x")?, j.field_int(&p, "y")?])
}
pub fn flash(
    project: &Project,
    j: &mut J<'_>,
    trace: &Trace,
    preview: &O,
    bounds: [f64; 4],
    scale: f64,
) -> Result<()> {
    if trace.expired() {
        return Ok(());
    }
    if !show(project, j, &trace.edit, Some((preview, bounds, scale)))? {
        return stop(project, j);
    }
    project.feedback.lock().expect("feedback").edit = None;
    if j.static_call(
        "java/lang/Boolean",
        "getBoolean",
        "(Ljava/lang/String;)Z",
        &[A::S("cranpose.trace.edits")],
    )?
    .z()?
    {
        let message = format!(
            "CRANPOSE_EDIT_PRESENTED {}",
            json!({"editToMatchedFrameMs":trace.edit.started.elapsed().as_secs_f64()*1000.0,"snapshotRequests":trace.attempts,"file":trace.file,"lines":trace.target.lines})
        );
        let logger = j.static_obj(
            "com/intellij/openapi/diagnostic/Logger",
            "getInstance",
            "(Ljava/lang/String;)Lcom/intellij/openapi/diagnostic/Logger;",
            &[A::S("dev.cranpose.feedback")],
        )?;
        j.void(&logger, "info", "(Ljava/lang/String;)V", &[A::S(&message)])?;
    }
    Ok(())
}
fn show(
    project: &Project,
    j: &mut J<'_>,
    edit: &Edit,
    destination: Option<(&O, [f64; 4], f64)>,
) -> Result<bool> {
    let (Some(file), Some(editor)) = project.selected(j)? else {
        return Ok(false);
    };
    if j.text(&file, "getPath")? != edit.path {
        return Ok(false);
    }
    let doc = j.obj(
        &editor,
        "getDocument",
        "()Lcom/intellij/openapi/editor/Document;",
        &[],
    )?;
    if j.long(&doc, "getModificationStamp")? != edit.stamp {
        return Ok(false);
    }
    let content = j.obj(
        &editor,
        "getContentComponent",
        "()Ljavax/swing/JComponent;",
        &[],
    )?;
    if !j.bool(&content, "isShowing")? {
        return Ok(false);
    }
    let root = j.static_obj(
        "javax/swing/SwingUtilities",
        "getRootPane",
        "(Ljava/awt/Component;)Ljavax/swing/JRootPane;",
        &[A::O(&content)],
    )?;
    if root.is_null() {
        return Ok(false);
    }
    if let Some((preview, _, _)) = destination {
        if !j.bool(preview, "isShowing")? {
            return Ok(false);
        }
        let other = j.static_obj(
            "javax/swing/SwingUtilities",
            "getRootPane",
            "(Ljava/awt/Component;)Ljavax/swing/JRootPane;",
            &[A::O(preview)],
        )?;
        if !j.same(&root, &other)? {
            return Ok(false);
        }
    }
    let layer = j.obj(&root, "getLayeredPane", "()Ljavax/swing/JLayeredPane;", &[])?;
    let origin = j.obj(
        &editor,
        "offsetToXY",
        "(I)Ljava/awt/Point;",
        &[A::I(edit.offset)],
    )?;
    let x = j.field_int(&origin, "x")?;
    let y = j.field_int(&origin, "y")? + j.int(&editor, "getLineHeight")?;
    let visible = j.obj(&content, "getVisibleRect", "()Ljava/awt/Rectangle;", &[])?;
    if !j
        .call(&visible, "contains", "(II)Z", &[A::I(x), A::I(y - 1)])?
        .z()?
    {
        return Ok(false);
    }
    let from = point(j, &content, x, y, &layer)?;
    let (target, bw, bh) = if let Some((preview, bounds, scale)) = destination {
        let [bx, by, bw, bh] = bounds.map(|v| v * scale);
        let visible = j.obj(preview, "getVisibleRect", "()Ljava/awt/Rectangle;", &[])?;
        if !j
            .call(
                &visible,
                "contains",
                "(II)Z",
                &[A::I((bx + bw / 2.0) as i32), A::I((by + bh / 2.0) as i32)],
            )?
            .z()?
        {
            return Ok(false);
        }
        let target = point(j, preview, bx as i32, by as i32, &layer)?;
        (target, bw, bh)
    } else {
        ([from[0] - 32, from[1] - 24], 64.0, 48.0)
    };
    let (panel, surface) = {
        let state = project.feedback.lock().expect("feedback");
        let Some((p, s)) = &state.overlay else {
            return Ok(false);
        };
        let (Some(p), Some(s)) = (p.upgrade(), s.upgrade()) else {
            return Ok(false);
        };
        (p, s)
    };
    let parent = j.obj(
        surface.component(),
        "getParent",
        "()Ljava/awt/Container;",
        &[],
    )?;
    if !j.same(&parent, &layer)? {
        if !parent.is_null() {
            j.void(
                &parent,
                "remove",
                "(Ljava/awt/Component;)V",
                &[A::O(surface.component())],
            )?;
        }
        j.obj(
            &layer,
            "add",
            "(Ljava/awt/Component;)Ljava/awt/Component;",
            &[A::O(surface.component())],
        )?;
        j.void(
            &layer,
            "setLayer",
            "(Ljava/awt/Component;I)V",
            &[A::O(surface.component()), A::I(450)],
        )?;
    }
    // Render only the arc's bounding rectangle, not the entire IDE window.
    let left = (from[0].min(target[0]) - 40).max(0);
    let top = (from[1].min(target[1]) - 48).max(0);
    let right = (from[0].max(target[0] + bw.ceil() as i32) + 40).min(j.int(&layer, "getWidth")?);
    let bottom = (from[1].max(target[1] + bh.ceil() as i32) + 48).min(j.int(&layer, "getHeight")?);
    if right <= left || bottom <= top {
        return Ok(false);
    }
    surface.clear_frame(j)?;
    j.void(
        surface.component(),
        "setBounds",
        "(IIII)V",
        &[
            A::I(left),
            A::I(top),
            A::I(right - left),
            A::I(bottom - top),
        ],
    )?;
    surface.size(j)?;
    j.void(surface.component(), "setVisible", "(Z)V", &[A::Z(true)])?;
    panel.send(crate::protocol::Packet::new(13).int(surface.id).byte(1));
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    panel.message("ide.authoring.bolt",&json!({"request":NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed),"phase":if destination.is_some(){"ready"}else{"pending"},"from":[from[0]-left,from[1]-top],"target":[target[0]-left,target[1]-top,bw,bh],"size":[right-left,bottom-top]}).to_string());
    let mut state = project.feedback.lock().expect("feedback");
    state.showing = Some(Instant::now());
    state.charging = destination.is_none();
    Ok(true)
}

#[cfg(feature = "ide-tests")]
pub fn integration_test(j: &mut J<'_>) -> Result<()> {
    // IntelliJ editors and preview viewports use different nested coordinate
    // systems. Exercise the actual Swing conversion used by the window overlay.
    let root = j.new("javax/swing/JLayeredPane", "()V", &[])?;
    let source = j.new(
        "javax/swing/JPanel",
        "(Ljava/awt/LayoutManager;)V",
        &[A::Null],
    )?;
    let viewport = j.new(
        "javax/swing/JPanel",
        "(Ljava/awt/LayoutManager;)V",
        &[A::Null],
    )?;
    let preview = j.new(
        "javax/swing/JPanel",
        "(Ljava/awt/LayoutManager;)V",
        &[A::Null],
    )?;
    for (parent, child) in [(&root, &source), (&root, &viewport), (&viewport, &preview)] {
        j.obj(
            parent,
            "add",
            "(Ljava/awt/Component;)Ljava/awt/Component;",
            &[A::O(child)],
        )?;
    }
    for (component, bounds) in [
        (&source, [20, 40, 600, 700]),
        (&viewport, [650, 100, 500, 600]),
        (&preview, [-30, -20, 400, 500]),
    ] {
        j.void(component, "setBounds", "(IIII)V", &bounds.map(A::I))?;
    }
    anyhow::ensure!(
        point(j, &source, 120, 200, &root)? == [140, 240],
        "Source offset conversion"
    );
    anyhow::ensure!(
        point(j, &preview, 80, 50, &root)? == [700, 130],
        "Panned preview conversion"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn trace() -> Trace {
        let c = Catalog::parse("#[composable]\nfn App(){ Text(\"new\"); }").expect("catalog");
        Trace {
            edit: Edit {
                path: "src/main.rs".into(),
                stamp: 1,
                offset: 0,
                started: Instant::now(),
            },
            file: "src/main.rs".into(),
            target: ViewTarget::at(&c, c.literals[0].range.start_utf16).expect("text"),
            schema: c.schema,
            revision: 42,
            generation: None,
            composed: None,
            request: None,
            attempts: 0,
        }
    }
    #[test]
    fn isolates_replies_bounds_requests_and_expires() {
        let mut t = trace();
        assert!(!t.follows(t.edit.started - Duration::from_millis(1)));
        assert!(!t.follows(Instant::now()));
        assert!(t.request().is_none());
        let ack = json!({"file":t.file,"schema":t.schema,"revision":42,"accepted":true,"changed":true,"generation":7}).to_string();
        t.message("cranpose.dev.composed", "{\"generation\":7}");
        assert!(
            t.composed.is_none(),
            "Unacknowledged composition is unrelated"
        );
        t.message("cranpose.dev.values.result", &ack.replace("42", "41"));
        assert!(t.generation.is_none());
        t.message("cranpose.dev.values.result", &ack);
        assert!(t.request().is_none(), "Acceptance is not composition");
        t.message("cranpose.dev.composed", "{\"generation\":6}");
        assert!(
            t.request().is_none(),
            "Old composition cannot complete the edit"
        );
        t.message("cranpose.dev.composed", "{\"generation\":7}");
        assert!(t.follows(Instant::now()));
        for _ in 0..8 {
            let id = t.request().expect("request");
            assert!(t.request().is_none(), "Only one snapshot in flight");
            assert!(t.reply(&json!({"requestId":id-1}).to_string()).is_none());
            assert_eq!(
                t.request,
                Some(id),
                "Stale reply cannot unblock this request"
            );
            let reply = json!({"schema":2,"requestId":id,"truncated":false,"nodes":[]}).to_string();
            assert!(is_reply(&reply));
            assert!(t.reply(&reply).is_none());
            assert!(t.request.is_none());
        }
        assert!(
            t.request().is_none(),
            "Bound snapshot capture costs even if view never matches"
        );
        assert!(!is_reply("{\"requestId\":7}"));
        let mut t = trace();
        t.edit.started = Instant::now() - Duration::from_secs(11);
        assert!(t.request().is_none());
    }
    #[test]
    fn rejected_and_unchanged_updates_do_not_cast() {
        for accepted in [true, false] {
            let mut t = trace();
            let ack = json!({"file":t.file,"schema":t.schema,"revision":42,"accepted":accepted,"changed":false}).to_string();
            assert!(t.message("cranpose.dev.values.result", &ack));
            assert!(t.request().is_none());
        }
    }
}
