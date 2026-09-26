//! Runs only in a separate test IDE, never linked into production releases.
use crate::{
    jvm::{A, J, O},
    project::{self, Project},
    protocol::{Frame, Packet},
    run_configuration::{self, Config},
    surface::Panel,
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
pub fn dispatch(j: &mut J<'_>, operation: &str) -> Result<O> {
    match operation {
        "SelfTest.isHeadless" => j.boxed_bool(true),
        "SelfTest.getRequiredModality" => j.boxed_int(1),
        "SelfTest.main" => {
            let output = property(j, "cranpose.test.output")?;
            std::fs::create_dir_all(&output)?;
            let mut results = vec![];
            for (name, test) in [
                (
                    "project lifecycle",
                    services as fn(&mut J<'_>) -> Result<()>,
                ),
                ("native surface pixels and disposal", surface),
                ("persistent Cargo configuration", configuration),
                ("native Cranpose rendering", native_ui),
                ("preview UI reconnection", reconnect),
                ("inline stability badges", inlays),
            ] {
                if (matches!(
                    name,
                    "persistent Cargo configuration" | "preview UI reconnection"
                ) && !crate::features().cargo)
                    || (name == "inline stability badges" && !crate::features().stability)
                {
                    continue;
                }
                let result = test(j);
                if j.env.exception_check()? {
                    j.env.exception_describe()?;
                    j.env.exception_clear()?;
                }
                results.push(json!({"name":name,"passed":result.is_ok(),"error":result.err().map(|e|format!("{e:#}"))}));
            }
            let passed = results.iter().all(|r| r["passed"] == true);
            let report = json!({"passed":passed,"tests":results});
            std::fs::write(
                std::path::Path::new(&output).join("results.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            println!("CRANPOSE_IDE_TESTS {report}");
            j.static_void(
                "java/lang/System",
                "exit",
                "(I)V",
                &[A::I(if passed { 0 } else { 1 })],
            )?;
            j.null()
        }
        _ => anyhow::bail!("Unknown self test: {operation}"),
    }
}
fn property(j: &mut J<'_>, key: &str) -> Result<String> {
    let value = j.static_obj(
        "java/lang/System",
        "getProperty",
        "(Ljava/lang/String;)Ljava/lang/String;",
        &[A::S(key)],
    )?;
    j.read_string(&value)
}
fn project(j: &mut J<'_>) -> Result<Arc<Project>> {
    let manager = j.static_obj(
        "com/intellij/openapi/project/ProjectManager",
        "getInstance",
        "()Lcom/intellij/openapi/project/ProjectManager;",
        &[],
    )?;
    let object = j.obj(
        &manager,
        "getDefaultProject",
        "()Lcom/intellij/openapi/project/Project;",
        &[],
    )?;
    let project = Project::get(j, &object)?;
    project.stability.lock().expect("state").enabled = false;
    Ok(project)
}
fn services(j: &mut J<'_>) -> Result<()> {
    let p = project(j)?;
    ensure!(
        Arc::ptr_eq(&p, &Project::get(j, &p.object)?),
        "Duplicate project service"
    );
    p.set_property(j, "cranpose.test.property", "🦀 λ")?;
    ensure!(
        p.property(j, "cranpose.test.property")? == "🦀 λ",
        "Property round trip"
    );
    let options = project::options(j, false)?;
    let panel = Panel::new(j, options)?;
    p.theme(j, &panel)?;
    panel.close(j)?;
    Ok(())
}
fn surface(j: &mut J<'_>) -> Result<()> {
    let options = project::options(j, false)?;
    let panel = Panel::new(j, options)?;
    let frame = Frame {
        surface: 0,
        id: 1,
        buffer_width: 4,
        buffer_height: 4,
        x: 0,
        y: 0,
        width: 4,
        height: 4,
        pixels: vec![0xff123456u32 as i32; 16],
    };
    panel.primary.frame(j, &frame)?;
    let patch = Frame {
        x: 1,
        y: 2,
        width: 2,
        height: 1,
        pixels: vec![0xffabcdefu32 as i32; 2],
        ..frame
    };
    panel.primary.frame(j, &patch)?;
    let image = panel.primary.snapshot().context("Image")?;
    ensure!(
        j.call(&image, "getRGB", "(II)I", &[A::I(0), A::I(0)])?
            .i()?
            == 0xff123456u32 as i32,
        "Untouched pixel"
    );
    ensure!(
        j.call(&image, "getRGB", "(II)I", &[A::I(1), A::I(2)])?
            .i()?
            == 0xffabcdefu32 as i32,
        "Dirty rectangle"
    );
    panel.primary.set_scale(j, 2.0)?;
    panel.primary.set_scale(j, f64::NAN)?;
    panel.close(j)?;
    panel.close(j)?;
    Ok(())
}
fn configuration(j: &mut J<'_>) -> Result<()> {
    let p = project(j)?;
    let kind = j.new("dev/cranpose/rust/ConfigurationType", "()V", &[])?;
    let factories = j.obj(
        &kind,
        "getConfigurationFactories",
        "()[Lcom/intellij/execution/configurations/ConfigurationFactory;",
        &[],
    )?;
    let factory = j
        .elements(&factories)?
        .into_iter()
        .next()
        .context("Factory")?;
    let create = |j: &mut J<'_>| {
        j.obj(&factory,"createTemplateConfiguration","(Lcom/intellij/openapi/project/Project;)Lcom/intellij/execution/configurations/RunConfiguration;",&[A::O(&p.object)])
    };
    let original = create(j)?;
    let config = Config {
        manifest: format!(
            "{}/samples/counter/Cargo.toml",
            property(j, "cranpose.test.workspace")?
        ),
        package: "cranpose-counter".into(),
        target: "cranpose-counter".into(),
        arguments: "\"two words\" 🦀".into(),
        environment: "A=one=two\nB=λ".into(),
        no_default_features: true,
        ..Config::default()
    };
    p.snapshot.lock().expect("snapshot").targets = vec![crate::model::Target {
        package_name: config.package.clone(),
        name: config.target.clone(),
        kind: "bin".into(),
        manifest: config.manifest.clone(),
        source: "src/main.rs".into(),
        features: vec![],
        cranpose_dependency: Some("cranpose".into()),
        id: "counter".into(),
        label: "cranpose-counter".into(),
    }];
    run_configuration::store(j, &original, &config)?;
    let xml = j.new(
        "org/jdom/Element",
        "(Ljava/lang/String;)V",
        &[A::S("configuration")],
    )?;
    j.void(
        &original,
        "writeExternal",
        "(Lorg/jdom/Element;)V",
        &[A::O(&xml)],
    )?;
    let restored = create(j)?;
    j.void(
        &restored,
        "readExternal",
        "(Lorg/jdom/Element;)V",
        &[A::O(&xml)],
    )?;
    let value = run_configuration::config(j, &restored)?;
    ensure!(
        value.arguments == config.arguments
            && value.environment == config.environment
            && value.no_default_features,
        "Configuration round trip"
    );
    ensure!(
        value.args(j)?.ends_with(&["two words".into(), "🦀".into()]),
        "Argument quoting"
    );
    let cloned = j.obj(
        &restored,
        "clone",
        "()Lcom/intellij/execution/configurations/RunConfiguration;",
        &[],
    )?;
    ensure!(
        run_configuration::config(j, &cloned)?.arguments == config.arguments,
        "Configuration clone"
    );
    let editor = j.obj(
        &restored,
        "getConfigurationEditor",
        "()Lcom/intellij/openapi/options/SettingsEditor;",
        &[],
    )?;
    let component = j.obj(&editor, "getComponent", "()Ljavax/swing/JComponent;", &[])?;
    let size = j.obj(
        &component,
        "getPreferredSize",
        "()Ljava/awt/Dimension;",
        &[],
    )?;
    ensure!(
        j.field_int(&size, "height")? > 400,
        "Configuration editor collapsed"
    );
    j.void(
        &editor,
        "resetFrom",
        "(Ljava/lang/Object;)V",
        &[A::O(&restored)],
    )?;
    let panel = p
        .live_panels()
        .into_iter()
        .find(|panel| {
            j.same(panel.primary.component(), &component)
                .unwrap_or(false)
        })
        .context("Settings panel")?;
    j.void(&component, "setSize", "(II)V", &[A::I(660), A::I(520)])?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while panel.primary.snapshot().is_none() && Instant::now() < deadline {
        panel.tick(j)?;
        panel.send(Packet::new(1).int(0).int(660).int(520).float(2.).float(60.));
        panel.send(Packet::new(13).int(0).byte(1));
        std::thread::sleep(Duration::from_millis(20));
    }
    ensure!(
        panel.primary.snapshot().is_some(),
        "Cranpose settings did not render"
    );
    // The platform may reset an editor repeatedly while opening a modal dialog.
    // Keep the native UI alive beyond its first frame and exercise those resets.
    for _ in 0..3 {
        j.void(
            &editor,
            "resetFrom",
            "(Ljava/lang/Object;)V",
            &[A::O(&restored)],
        )?;
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            panel.tick(j)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        ensure!(
            panel.connected(),
            "Cranpose settings exited after resetting a populated form"
        );
    }
    j.void(
        &editor,
        "applyTo",
        "(Ljava/lang/Object;)V",
        &[A::O(&restored)],
    )?;
    let applied = run_configuration::config(j, &restored)?;
    ensure!(
        applied.manifest == config.manifest
            && applied.arguments == config.arguments
            && applied.environment == config.environment
            && applied.no_default_features,
        "Cranpose settings lost values during initialization or Apply"
    );
    j.void(&editor, "dispose", "()V", &[])?;
    Ok(())
}
fn native_ui(j: &mut J<'_>) -> Result<()> {
    let options = project::options(j, false)?;
    let panel = Panel::new(j, options)?;
    j.void(
        panel.primary.component(),
        "setSize",
        "(II)V",
        &[A::I(680), A::I(480)],
    )?;
    panel.start(j)?;
    let result = (|| -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while panel.primary.snapshot().is_none() && Instant::now() < deadline {
            panel.tick(j)?;
            panel.send(Packet::new(1).int(0).int(680).int(480).float(1.).float(60.));
            panel.send(Packet::new(13).int(0).byte(1));
            std::thread::sleep(Duration::from_millis(20));
        }
        let image = panel
            .primary
            .snapshot()
            .context("Native Cranpose process did not render")?;
        let output = property(j, "cranpose.test.output")?;
        let file = j.new(
            "java/io/File",
            "(Ljava/lang/String;)V",
            &[A::S(&format!("{output}/native-ui.png"))],
        )?;
        ensure!(
            j.static_call(
                "javax/imageio/ImageIO",
                "write",
                "(Ljava/awt/image/RenderedImage;Ljava/lang/String;Ljava/io/File;)Z",
                &[A::O(&image), A::S("png"), A::O(&file)]
            )?
            .z()?,
            "PNG encoder"
        );
        ensure!(j.int(&image, "getWidth")? == 680, "Native viewport size");
        Ok(())
    })();
    panel.close(j)?;
    result
}
fn inlays(j: &mut J<'_>) -> Result<()> {
    let p = project(j)?;
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
        &[A::S("fn Card(value: i32) {}")],
    )?;
    let editor=j.obj(&factory,"createEditor","(Lcom/intellij/openapi/editor/Document;Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/editor/Editor;",&[A::O(&document),A::O(&p.object)])?;
    let result = crate::stability::integration_test(&p, j, &editor, &document);
    j.void(
        &factory,
        "releaseEditor",
        "(Lcom/intellij/openapi/editor/Editor;)V",
        &[A::O(&editor)],
    )?;
    result
}

fn reconnect(j: &mut J<'_>) -> Result<()> {
    let p = project(j)?;
    crate::workspace::integration_test(p, j)
}
