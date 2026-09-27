//! IntelliJ/RustRover New Project adapters. Their content is a Cranpose surface.
use crate::{
    jvm::{self, A, J, O},
    project::{self, Project},
    starter,
    surface::Panel,
};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering, mpsc},
};

pub fn dispatch(j: &mut J<'_>, operation: &str, receiver: &O, args: &[O]) -> Result<O> {
    match operation {
        "ShowcaseGenerator.getName" | "ShowcaseBuilder.getPresentableName" => j.string("Cranpose"),
        "ShowcaseBuilder.getBuilderId" => j.string("cranpose.showcase"),
        "ShowcaseBuilder.getGroupName" => j.string("Cranpose"),
        "ShowcaseBuilder.getWeight" => j.boxed_int(85),
        "ShowcaseBuilder.isAvailable" => j.boxed_bool(true),
        "ShowcaseGenerator.getDescription" | "ShowcaseBuilder.getDescription" => {
            j.string("Native Rust UI · Showcase with liquid glass and GPU shaders")
        }
        "ShowcaseGenerator.getLogo" | "ShowcaseBuilder.getNodeIcon" => crate::editor::icon(j),
        "ShowcaseGenerator.validate" => {
            let path = PathBuf::from(j.read_string(&args[0])?);
            match starter::validate_destination(&path) {
                Ok(()) => j.constant(
                    "com/intellij/facet/ui/ValidationResult",
                    "OK",
                    "Lcom/intellij/facet/ui/ValidationResult;",
                ),
                Err(e) => j.new(
                    "com/intellij/facet/ui/ValidationResult",
                    "(Ljava/lang/String;)V",
                    &[A::S(&e.to_string())],
                ),
            }
        }
        "ShowcaseGenerator.createPeer" => peer(j, "ShowcasePeer", None),
        "ShowcaseGenerator.generateProject" => {
            let root = PathBuf::from(j.text(&args[1], "getPath")?);
            let project = Project::get(j, &args[0])?;
            generate(&project, j, root)?;
            j.null()
        }
        "ShowcaseBuilder.getModuleType" => j.constant(
            "com/intellij/openapi/module/ModuleType",
            "EMPTY",
            "Lcom/intellij/openapi/module/ModuleType;",
        ),
        "ShowcaseBuilder.modifyProjectTypeStep" => {
            let step = peer(j, "ShowcaseStep", Some(&args[0]))?;
            let component = j.obj(&step, "getComponent", "()Ljavax/swing/JComponent;", &[])?;
            j.void(
                &args[0],
                "addSettingsComponent",
                "(Ljavax/swing/JComponent;)V",
                &[A::O(&component)],
            )?;
            Ok(step)
        }
        "ShowcaseBuilder.setupRootModel" => {
            let root = PathBuf::from(j.text(receiver, "getContentEntryPath")?);
            let object = j.obj(
                &args[0],
                "getProject",
                "()Lcom/intellij/openapi/project/Project;",
                &[],
            )?;
            let project = Project::get(j, &object)?;
            j.obj(receiver,"doAddContentEntry","(Lcom/intellij/openapi/roots/ModifiableRootModel;)Lcom/intellij/openapi/roots/ContentEntry;",&[A::O(&args[0])])?;
            generate(&project, j, root)?;
            j.null()
        }
        _ => anyhow::bail!("Unknown project wizard operation: {operation}"),
    }
}
fn peer(j: &mut J<'_>, class: &str, settings: Option<&O>) -> Result<O> {
    let mut options = project::options(j, false)?;
    options
        .environment
        .insert("CRANPOSE_AUTHORING".into(), "wizard".into());
    let panel = Panel::new(j, options)?;
    let size = j.new("java/awt/Dimension", "(II)V", &[A::I(520), A::I(280)])?;
    j.void(
        panel.primary.component(),
        "setPreferredSize",
        "(Ljava/awt/Dimension;)V",
        &[A::O(&size)],
    )?;
    panel.start(j)?;
    let captured = panel.clone();
    let identity = Arc::new(std::sync::atomic::AtomicI64::new(0));
    let saved = identity.clone();
    let attached = std::sync::atomic::AtomicBool::new(false);
    let id = jvm::register(
        move |j, op, args| match op.rsplit('.').next().unwrap_or_default() {
            "getComponent" => Ok(captured.primary.component().clone()),
            "getSettings" => j.string(starter::SHOWCASE_REVISION),
            "validate" if op.starts_with("ShowcaseStep") => j.boxed_bool(true),
            "isBackgroundJobRunning" => j.boxed_bool(false),
            "buildUI" => {
                j.void(
                    &args[0],
                    "addSettingsComponent",
                    "(Ljavax/swing/JComponent;)V",
                    &[A::O(captured.primary.component())],
                )?;
                register_disposal(j, &args[0], saved.load(Ordering::Relaxed))?;
                j.null()
            }
            "hierarchyChanged" => {
                if !attached.load(Ordering::Relaxed) {
                    let window = j.static_obj(
                        "javax/swing/SwingUtilities",
                        "getWindowAncestor",
                        "(Ljava/awt/Component;)Ljava/awt/Window;",
                        &[A::O(captured.primary.component())],
                    )?;
                    if !window.is_null() {
                        let listener = jvm::callback(j, saved.load(Ordering::Relaxed))?;
                        j.void(
                            &window,
                            "addWindowListener",
                            "(Ljava/awt/event/WindowListener;)V",
                            &[A::O(&listener)],
                        )?;
                        attached.store(true, Ordering::Relaxed);
                    }
                }
                j.null()
            }
            "disposeUIResources" | "dispose" | "windowClosed" => {
                captured.close(j)?;
                jvm::unregister(saved.load(Ordering::Relaxed));
                j.null()
            }
            _ => j.null(),
        },
    );
    identity.store(id, Ordering::Relaxed);
    let listener = jvm::callback(j, id)?;
    j.void(
        panel.primary.component(),
        "addHierarchyListener",
        "(Ljava/awt/event/HierarchyListener;)V",
        &[A::O(&listener)],
    )?;
    if let Some(settings) = settings {
        register_disposal(j, settings, id)?;
    }
    j.new(&format!("dev/cranpose/rust/{class}"), "(J)V", &[A::J(id)])
}
fn register_disposal(j: &mut J<'_>, settings: &O, id: i64) -> Result<()> {
    let context = j.obj(
        settings,
        "getContext",
        "()Lcom/intellij/ide/util/projectWizard/WizardContext;",
        &[],
    )?;
    let parent = j.obj(
        &context,
        "getDisposable",
        "()Lcom/intellij/openapi/Disposable;",
        &[],
    )?;
    let callback = jvm::callback(j, id)?;
    j.static_void(
        "com/intellij/openapi/util/Disposer",
        "register",
        "(Lcom/intellij/openapi/Disposable;Lcom/intellij/openapi/Disposable;)V",
        &[A::O(&parent), A::O(&callback)],
    )
}
fn generate(project: &Arc<Project>, j: &mut J<'_>, root: PathBuf) -> Result<()> {
    starter::validate_destination(&root)?;
    let cache = project::cache(j)?.join("starters");
    let weak = Arc::downgrade(project);
    let (sender, receiver) = mpsc::sync_channel(1);
    project.authoring.lock().expect("authoring").generation = Some(receiver);
    project.snapshot.lock().expect("snapshot").status = "Creating Cranpose showcase…".into();
    project.publish();
    std::thread::spawn(move || {
        let result = starter::generate(
            &root,
            &cache,
            starter::SHOWCASE_REPOSITORY,
            starter::SHOWCASE_REVISION,
            || {
                weak.upgrade()
                    .is_none_or(|p| p.closed.load(Ordering::Acquire))
            },
        )
        .map(|()| root);
        let _ = sender.send(result.map_err(|e| format!("{e:#}")));
    });
    Ok(())
}
pub fn tick(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    let result = project
        .authoring
        .lock()
        .expect("authoring")
        .generation
        .as_ref()
        .and_then(|r| r.try_recv().ok());
    if let Some(result) = result {
        project.authoring.lock().expect("authoring").generation = None;
        match result {
            Ok(root) => {
                let project = project.clone();
                jvm::later(j, move |j| {
                    if project.closed.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let fs = j.static_obj(
                        "com/intellij/openapi/vfs/LocalFileSystem",
                        "getInstance",
                        "()Lcom/intellij/openapi/vfs/LocalFileSystem;",
                        &[],
                    )?;
                    let file = j.obj(
                        &fs,
                        "refreshAndFindFileByPath",
                        "(Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
                        &[A::S(&root.to_string_lossy())],
                    )?;
                    if !file.is_null() {
                        j.void(&file, "refresh", "(ZZ)V", &[A::Z(false), A::Z(true)])?;
                    }
                    project.refresh(j)?;
                    let entry = ["src/main.rs", "src/lib.rs", "Cargo.toml"]
                        .into_iter()
                        .map(|s| root.join(s))
                        .find(|p| p.is_file())
                        .context("Starter entry file")?;
                    project::navigate(&project, j, &entry.to_string_lossy(), 0, 0)?;
                    Ok(())
                })?;
            }
            Err(error) => {
                project.snapshot.lock().expect("snapshot").status = error.clone();
                project.publish();
                project.info(j, &error)?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "ide-tests")]
pub fn integration_test(j: &mut J<'_>) -> Result<()> {
    let generator = j.new("dev/cranpose/rust/ShowcaseGenerator", "()V", &[])?;
    anyhow::ensure!(
        j.text(&generator, "getName")? == "Cranpose",
        "New Project entry"
    );
    let peer = j.obj(
        &generator,
        "createPeer",
        "()Lcom/intellij/platform/ProjectGeneratorPeer;",
        &[],
    )?;
    let component = j.obj(&peer, "getComponent", "()Ljavax/swing/JComponent;", &[])?;
    anyhow::ensure!(
        j.env
            .is_instance_of(&component, "dev/cranpose/rust/Surface")?,
        "Wizard must use a Cranpose surface"
    );
    j.void(&peer, "dispose", "()V", &[])?;
    let builder = j.new("dev/cranpose/rust/ShowcaseBuilder", "()V", &[])?;
    anyhow::ensure!(
        j.text(&builder, "getPresentableName")? == "Cranpose",
        "IDEA wizard entry"
    );
    anyhow::ensure!(
        !j.obj(
            &builder,
            "getModuleType",
            "()Lcom/intellij/openapi/module/ModuleType;",
            &[]
        )?
        .is_null(),
        "IDEA module type"
    );
    Ok(())
}
