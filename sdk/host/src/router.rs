use crate::{
    jvm::{self, A, J, O},
    project::{self, Project},
    surface::Panel,
};
use anyhow::Result;
use std::sync::Arc;

pub fn dispatch(j: &mut J<'_>, operation: &str, receiver: &O, args: &[O]) -> Result<O> {
    // IntelliJ can retain a gutter renderer in a paint cache after its range
    // highlighter is disposed. Non-null properties must outlive native callbacks.
    match operation {
        "PreviewGutter.getIcon" => return crate::editor::icon(j),
        "PreviewGutter.isNavigateAction" => return j.boxed_bool(true),
        "PreviewGutter.hashCode" => {
            let id = j.id(receiver)?;
            return j.boxed_int(id as i32);
        }
        "PreviewGutter.equals" => {
            let same = !args[0].is_null()
                && j.env
                    .is_instance_of(&args[0], "dev/cranpose/rust/PreviewGutter")?
                && j.id(&args[0])? == j.id(receiver)?;
            return j.boxed_bool(same);
        }
        "PreviewClick.getActionUpdateThread" => {
            return j.constant(
                "com/intellij/openapi/actionSystem/ActionUpdateThread",
                "EDT",
                "Lcom/intellij/openapi/actionSystem/ActionUpdateThread;",
            );
        }
        _ => {}
    }
    #[cfg(feature = "ide-tests")]
    if operation.starts_with("SelfTest.") {
        return crate::ide_tests::dispatch(j, operation);
    }
    let class = operation.split('.').next().unwrap_or_default();
    if matches!(class, "ShowcaseGenerator" | "ShowcaseBuilder") {
        return crate::wizard::dispatch(j, operation, receiver, args);
    }
    if matches!(class, "ShowcasePeer" | "ShowcaseStep") {
        let id = j.id(receiver)?;
        return jvm::invoke(j, id, operation, args);
    }
    if matches!(
        class,
        "Callback"
            | "Surface"
            | "Workspace"
            | "PreviewEditor"
            | "Badge"
            | "CounterBadge"
            | "RunSettings"
            | "ValueGlyph"
            | "PreviewGutter"
            | "PreviewClick"
            | "MenuStep"
    ) {
        let id = j.id(receiver)?;
        if id == 0 {
            return if operation.ends_with(".contains") {
                j.boxed_bool(false)
            } else {
                j.null()
            };
        }
        return jvm::invoke(j, id, operation, args);
    }
    if matches!(
        class,
        "ConfigurationType" | "ConfigurationFactory" | "RunConfiguration" | "RunState"
    ) {
        return crate::run_configuration::dispatch(j, operation, receiver, args);
    }
    match operation {
        "Startup.execute" => {
            let project = args[0].clone();
            jvm::later(j, move |j| {
                Project::get(j, &project)?;
                Ok(())
            })?;
            j.constant("kotlin/Unit", "INSTANCE", "Lkotlin/Unit;")
        }
        "ToolWindow.createToolWindowContent" => tool_window(j, &args[0], &args[1]),
        "PreviewProvider.accept" => {
            let path = j.text(&args[1], "getPath")?;
            j.boxed_bool(crate::model::is_cranpose_source(std::path::Path::new(
                &path,
            )))
        }
        "PreviewProvider.createEditor" => {
            let project = Project::get(j, &args[0])?;
            crate::workspace::create_editor(project, j, args[1].clone())
        }
        "PreviewProvider.getEditorTypeId" => j.string("cranpose-studio"),
        "PreviewProvider.getPolicy" => j.constant(
            "com/intellij/openapi/fileEditor/FileEditorPolicy",
            "HIDE_DEFAULT_EDITOR",
            "Lcom/intellij/openapi/fileEditor/FileEditorPolicy;",
        ),
        "LineMarker.getLineMarkerInfo" => crate::editor::line_marker(j, &args[0]),
        "Completion.fillCompletionVariants" => {
            crate::editor::complete(j, &args[0], &args[1])?;
            j.null()
        }
        _ if operation.ends_with(".getActionUpdateThread") => j.constant(
            "com/intellij/openapi/actionSystem/ActionUpdateThread",
            "EDT",
            "Lcom/intellij/openapi/actionSystem/ActionUpdateThread;",
        ),
        _ if class.ends_with("Action") => {
            if class == "DocsAction" {
                crate::workspace::docs(j)?;
                return j.null();
            }
            let project = j.obj(
                &args[0],
                "getProject",
                "()Lcom/intellij/openapi/project/Project;",
                &[],
            )?;
            if !project.is_null() {
                let project = Project::get(j, &project)?;
                match class {
                    "RefreshAction" => project.refresh(j)?,
                    "CheckAction" => project.execute(j, "check")?,
                    "RunAction" => project.execute(j, "run")?,
                    "PreviewAction" => project.execute(j, "preview")?,
                    _ => {}
                }
            }
            j.null()
        }
        _ if class == "ToggleBadges" || class == "ToggleStable" => {
            if operation.ends_with(".getActionUpdateThread") {
                return j.constant(
                    "com/intellij/openapi/actionSystem/ActionUpdateThread",
                    "EDT",
                    "Lcom/intellij/openapi/actionSystem/ActionUpdateThread;",
                );
            }
            let project = j.obj(
                &args[0],
                "getProject",
                "()Lcom/intellij/openapi/project/Project;",
                &[],
            )?;
            if project.is_null() {
                return j.boxed_bool(false);
            }
            let project = Project::get(j, &project)?;
            if operation.ends_with(".isSelected") {
                let state = project.stability.lock().expect("stability");
                j.boxed_bool(if class == "ToggleBadges" {
                    state.enabled
                } else {
                    state.show_stable
                })
            } else {
                let value = j.bool(&args[1], "booleanValue")?;
                {
                    let mut state = project.stability.lock().expect("stability");
                    if class == "ToggleBadges" {
                        state.enabled = value;
                    } else {
                        state.show_stable = value;
                    }
                    state.schedule();
                }
                project.set_property(
                    j,
                    if class == "ToggleBadges" {
                        "cranpose.stability.enabled"
                    } else {
                        "cranpose.stability.stable"
                    },
                    if value { "true" } else { "false" },
                )?;
                if !value && class == "ToggleBadges" {
                    crate::stability::clear(&project, j)?;
                }
                j.null()
            }
        }
        _ => anyhow::bail!("Unknown IDE operation: {operation}"),
    }
}
fn tool_window(j: &mut J<'_>, object: &O, window: &O) -> Result<O> {
    let project = Project::get(j, object)?;
    let options = project::options(j, false)?;
    let panel = Panel::new(j, options)?;
    let weak = Arc::downgrade(&project);
    *panel.on_message.lock().expect("callback") =
        Some(Arc::new(move |j, panel, channel, payload| {
            if let Some(project) = weak.upgrade() {
                if let Some(handler) = crate::MESSAGE_HANDLER.get()
                    && handler(j, &project.object, channel, payload)?
                {
                    return Ok(());
                }
                if channel == "host.overlay" {
                    crate::editor::attach_overlay(&project, j, panel, payload)?;
                } else {
                    crate::workspace::handle_message(&project, j, channel, payload)?;
                }
            }
            Ok(())
        }));
    let weak = Arc::downgrade(&project);
    *panel.on_lifecycle.lock().expect("callback") =
        Some(Arc::new(move |j, panel, connected, _| {
            if connected && let Some(project) = weak.upgrade() {
                project.theme(j, panel)?;
                project.publish();
                project.send_editor(j)?;
                if crate::features().cargo
                    && project.snapshot.lock().expect("snapshot").root.is_empty()
                {
                    project.refresh(j)?;
                }
            }
            Ok(())
        }));
    let factory = j.static_obj(
        "com/intellij/ui/content/ContentFactory",
        "getInstance",
        "()Lcom/intellij/ui/content/ContentFactory;",
        &[],
    )?;
    let content = j.obj(
        &factory,
        "createContent",
        "(Ljavax/swing/JComponent;Ljava/lang/String;Z)Lcom/intellij/ui/content/Content;",
        &[A::O(panel.primary.component()), A::S(""), A::Z(false)],
    )?;
    let manager = j.obj(
        window,
        "getContentManager",
        "()Lcom/intellij/ui/content/ContentManager;",
        &[],
    )?;
    j.void(
        &manager,
        "addContent",
        "(Lcom/intellij/ui/content/Content;)V",
        &[A::O(&content)],
    )?;
    let captured = panel.clone();
    let id = project.scope.register(move |j, _, _| {
        captured.close(j)?;
        j.null()
    });
    let disposable = jvm::callback(j, id)?;
    j.static_void(
        "com/intellij/openapi/util/Disposer",
        "register",
        "(Lcom/intellij/openapi/Disposable;Lcom/intellij/openapi/Disposable;)V",
        &[A::O(&content), A::O(&disposable)],
    )?;
    project.attach(j, &panel)?;
    panel.start(j)?;
    j.null()
}
