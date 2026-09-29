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
            j.string("A Rust application based on the Cranpose Showcase sample")
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
        let result = starter::generate(&root, &cache, || {
            weak.upgrade()
                .is_none_or(|p| p.closed.load(Ordering::Acquire))
        })
        .map(|()| root);
        let _ = sender.send(result.map_err(|e| format!("{e:#}")));
    });
    Ok(())
}
pub fn tick(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    if !j.bool(&project.object, "isInitialized")? {
        return Ok(());
    }
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
                jvm::later_non_modal(j, move |j| {
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
                    let entry = ["src/main.rs", "src/lib.rs", "Cargo.toml"]
                        .into_iter()
                        .map(|s| root.join(s))
                        .find(|p| p.is_file())
                        .context("Starter entry file")?;
                    j.obj(
                        &fs,
                        "refreshAndFindFileByPath",
                        "(Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
                        &[A::S(&entry.to_string_lossy())],
                    )?;
                    project::navigate(&project, j, &entry.to_string_lossy(), 0, 0)?;
                    if crate::features().cargo {
                        let target = prepare_desktop(&project, j, &root)?;
                        crate::workspace::show(&project, j, &target.source, None, Some(&target))?;
                    }
                    attach_cargo(&project, j, &fs, &root)?;
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

fn prepare_desktop(
    project: &Project,
    j: &mut J<'_>,
    root: &std::path::Path,
) -> Result<crate::model::Target> {
    let target = starter::desktop_target(root);
    crate::run_configuration::ensure_target(project, j, &target)?;
    // Metadata must not be a prerequisite: first-run machines may have no Rust,
    // and dependency downloads can take minutes. The preview owns build progress.
    project.initialize_starter(target.clone(), root);
    Ok(target)
}

// Ask the installed Rust plugin to discover the newly generated manifest. Using
// its import provider's loader keeps IDEA without Rust support a valid host.
fn attach_cargo(project: &Project, j: &mut J<'_>, fs: &O, root: &std::path::Path) -> Result<()> {
    let manifest = j.obj(
        fs,
        "refreshAndFindFileByPath",
        "(Ljava/lang/String;)Lcom/intellij/openapi/vfs/VirtualFile;",
        &[A::S(&root.join("Cargo.toml").to_string_lossy())],
    )?;
    if manifest.is_null() {
        return Ok(());
    }
    let provider = j.static_obj(
        "com/intellij/projectImport/ProjectOpenProcessor",
        "getImportProvider",
        "(Lcom/intellij/openapi/vfs/VirtualFile;)Lcom/intellij/projectImport/ProjectOpenProcessor;",
        &[A::O(&manifest)],
    )?;
    if provider.is_null() || j.text(&provider, "getName")? != "Cargo" {
        return Ok(());
    }
    let class = j.obj(&provider, "getClass", "()Ljava/lang/Class;", &[])?;
    let loader = j.obj(&class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?;
    let service_class = j.obj(
        &loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[A::S("org.rust.cargo.project.model.CargoProjectsService")],
    )?;
    let service = j.obj(
        &project.object,
        "getService",
        "(Ljava/lang/Class;)Ljava/lang/Object;",
        &[A::O(&service_class)],
    )?;
    j.obj(
        &service,
        "discoverAndRefresh",
        "()Ljava/util/concurrent/CompletableFuture;",
        &[],
    )?;
    Ok(())
}

#[cfg(feature = "ide-tests")]
pub fn integration_test(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
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
    // Exercise the real wizard's asynchronous worker as well as its entry/card.
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("Offline showcase 🦀");
    std::fs::create_dir_all(root.join(".idea"))?;
    generate(project, j, root.clone())?;
    let receiver = project
        .authoring
        .lock()
        .expect("authoring")
        .generation
        .take()
        .context("Wizard worker")?;
    let generated = receiver
        .recv_timeout(std::time::Duration::from_secs(15))?
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(generated == root, "Wizard destination");
    anyhow::ensure!(
        root.join("src/main.rs").is_file(),
        "Wizard application entry"
    );
    anyhow::ensure!(root.join("assets/app-icon.png").is_file(), "Wizard assets");
    anyhow::ensure!(
        root.join("LICENSE").is_file() && root.join(".idea").is_dir(),
        "License and IDE metadata"
    );
    anyhow::ensure!(!root.join(".git").exists(), "No template history");
    if crate::features().cargo {
        let saved = project.snapshot.lock().expect("snapshot").clone();
        let target = prepare_desktop(project, j, &root)?;
        let settings = crate::run_configuration::ensure_target(project, j, &target)?;
        let configuration = j.obj(
            &settings,
            "getConfiguration",
            "()Lcom/intellij/execution/configurations/RunConfiguration;",
            &[],
        )?;
        let mut value = crate::run_configuration::config(j, &configuration)?;
        value.validate()?;
        anyhow::ensure!(
            value.target == "cranpose-showcase" && value.features == "desktop",
            "Desktop run target"
        );
        anyhow::ensure!(
            !j.bool(&settings, "isTemporary")?,
            "Wizard configuration must persist"
        );
        value.arguments = "user argument".into();
        crate::run_configuration::store(j, &configuration, &value)?;
        prepare_desktop(project, j, &root)?;
        let again = crate::run_configuration::ensure_target(project, j, &target)?;
        anyhow::ensure!(
            j.same(&settings, &again)?,
            "Wizard must not duplicate configurations"
        );
        anyhow::ensure!(
            crate::run_configuration::config(j, &configuration)?.arguments == "user argument",
            "Preserve customized configuration"
        );
        let manager = j.static_obj(
            "com/intellij/execution/RunManager",
            "getInstance",
            "(Lcom/intellij/openapi/project/Project;)Lcom/intellij/execution/RunManager;",
            &[A::O(&project.object)],
        )?;
        let selected = j.obj(
            &manager,
            "getSelectedConfiguration",
            "()Lcom/intellij/execution/RunnerAndConfigurationSettings;",
            &[],
        )?;
        anyhow::ensure!(
            j.same(&settings, &selected)?,
            "Select the saved run configuration"
        );
        let snapshot = project.snapshot.lock().expect("snapshot").clone();
        anyhow::ensure!(
            snapshot.targets.len() == 1 && snapshot.selected == target.id && !snapshot.busy,
            "Ready target without metadata or compilation"
        );
        j.void(
            &manager,
            "removeConfiguration",
            "(Lcom/intellij/execution/RunnerAndConfigurationSettings;)V",
            &[A::O(&settings)],
        )?;
        *project.snapshot.lock().expect("snapshot") = saved;
    }
    Ok(())
}
