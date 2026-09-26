//! Persistent Cargo configurations and a Cranpose configuration editor.
use crate::{
    jvm::{self, A, J, O},
    model::Target,
    project::{self, Project},
    surface::Panel,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub manifest: String,
    pub package: String,
    pub target: String,
    pub kind: String,
    pub command: String,
    pub arguments: String,
    pub features: String,
    pub directory: String,
    pub environment: String,
    pub no_default_features: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            manifest: String::new(),
            package: String::new(),
            target: String::new(),
            kind: "bin".into(),
            command: "run".into(),
            arguments: String::new(),
            features: String::new(),
            directory: String::new(),
            environment: String::new(),
            no_default_features: false,
        }
    }
}
impl Config {
    pub fn target(target: &Target) -> Self {
        Self {
            manifest: target.manifest.clone(),
            package: target.package_name.clone(),
            target: target.name.clone(),
            kind: target.kind.clone(),
            features: target.features.join(","),
            directory: Path::new(&target.manifest)
                .parent()
                .unwrap_or(Path::new(""))
                .to_string_lossy()
                .into_owned(),
            ..Self::default()
        }
    }
    pub fn environment(&self) -> Result<BTreeMap<String, String>> {
        self.environment
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let (name, value) = line
                    .split_once('=')
                    .context("Use one NAME=value environment variable per line.")?;
                ensure!(
                    name.bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                    "Invalid environment variable name: {name}"
                );
                Ok((name.into(), value.into()))
            })
            .collect()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            Path::new(&self.manifest).is_file(),
            "Select a Cargo.toml file."
        );
        ensure!(
            !self.package.is_empty() && !self.target.is_empty(),
            "Select a Cranpose target."
        );
        ensure!(
            matches!(self.command.as_str(), "run" | "check" | "test"),
            "Choose run, check or test."
        );
        ensure!(
            matches!(self.kind.as_str(), "bin" | "example"),
            "Choose a binary or example target."
        );
        ensure!(
            self.directory.is_empty() || Path::new(&self.directory).is_dir(),
            "The working directory does not exist."
        );
        ensure!(
            self.command != "check" || self.arguments.trim().is_empty(),
            "Cargo check does not take application arguments."
        );
        self.environment()?;
        Ok(())
    }
    pub(crate) fn args(&self, j: &mut J<'_>) -> Result<Vec<String>> {
        let mut args = vec![
            self.command.clone(),
            "--manifest-path".into(),
            self.manifest.clone(),
            "--package".into(),
            self.package.clone(),
        ];
        if self.command != "test" {
            args.extend([format!("--{}", self.kind), self.target.clone()]);
        }
        let features = self
            .features
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !features.is_empty() {
            args.extend(["--features".into(), features.join(",")]);
        }
        if self.no_default_features {
            args.push("--no-default-features".into());
        }
        if !self.arguments.trim().is_empty() {
            let parsed = j.static_obj(
                "com/intellij/util/execution/ParametersListUtil",
                "parse",
                "(Ljava/lang/String;)Ljava/util/List;",
                &[A::S(&self.arguments)],
            )?;
            let array = j.obj(&parsed, "toArray", "()[Ljava/lang/Object;", &[])?;
            args.push("--".into());
            for arg in j.elements(&array)? {
                args.push(j.read_string(&arg)?);
            }
        }
        Ok(args)
    }
}
pub(crate) fn config(j: &mut J<'_>, object: &O) -> Result<Config> {
    let json = j.field_object(object, "configuration", "Ljava/lang/String;")?;
    let text = j.read_string(&json)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}
pub(crate) fn store(j: &mut J<'_>, object: &O, value: &Config) -> Result<()> {
    let text = j.string(&serde_json::to_string(value)?)?;
    j.env.set_field(
        object,
        "configuration",
        "Ljava/lang/String;",
        jni::objects::JValue::Object(text.as_obj()),
    )?;
    Ok(())
}
pub fn dispatch(j: &mut J<'_>, operation: &str, receiver: &O, args: &[O]) -> Result<O> {
    match operation {
        "ConfigurationType.getDisplayName"|"ConfigurationType.getId"=>j.string("Cranpose"),
        "ConfigurationType.getConfigurationTypeDescription"=>j.string("Run, check or test a Cranpose Cargo target"),
        "ConfigurationType.getIcon"=>crate::editor::icon(j),
        "ConfigurationType.getConfigurationFactories"=>{
            let existing=j.field_object(receiver,"factories","[Lcom/intellij/execution/configurations/ConfigurationFactory;")?;
            if !existing.is_null(){return Ok(existing);}
            let factory=j.new("dev/cranpose/rust/ConfigurationFactory","(Lcom/intellij/execution/configurations/ConfigurationType;)V",&[A::O(receiver)])?;
            let array=j.array("com/intellij/execution/configurations/ConfigurationFactory",&[factory])?;
            j.env.set_field(receiver,"factories","[Lcom/intellij/execution/configurations/ConfigurationFactory;",jni::objects::JValue::Object(array.as_obj()))?;Ok(array)
        }
        "ConfigurationFactory.getId"=>j.string("Cranpose Cargo"),
        "ConfigurationFactory.createTemplateConfiguration"=>j.new("dev/cranpose/rust/RunConfiguration","(Lcom/intellij/openapi/project/Project;Lcom/intellij/execution/configurations/ConfigurationFactory;Ljava/lang/String;)V",&[A::O(&args[0]),A::O(receiver),A::S("Cranpose")]),
        "RunConfiguration.<init>"=>{store(j,receiver,&Config::default())?;j.null()}
        "RunConfiguration.checkConfiguration"=>{
            if let Err(error)=config(j,receiver)?.validate(){j.env.throw_new("com/intellij/execution/configurations/RuntimeConfigurationError",error.to_string())?;}j.null()
        }
        "RunConfiguration.getConfigurationEditor"=>{
            let project=j.obj(receiver,"getProject","()Lcom/intellij/openapi/project/Project;",&[])?;let project=Project::get(j,&project)?;settings(&project,j)
        }
        "RunConfiguration.getState"=>j.new("dev/cranpose/rust/RunState","(Lcom/intellij/execution/runners/ExecutionEnvironment;Ljava/lang/Object;)V",&[A::O(&args[1]),A::O(receiver)]),
        "RunConfiguration.readExternal"=>{read(j,receiver,&args[0])?;j.null()}
        "RunConfiguration.writeExternal"=>{write(j,receiver,&args[0])?;j.null()}
        "RunState.<init>"=>{j.env.set_field(receiver,"configuration","Ljava/lang/Object;",jni::objects::JValue::Object(args[1].as_obj()))?;j.null()}
        "RunState.startProcess"=>{
            let object=j.field_object(receiver,"configuration","Ljava/lang/Object;")?;let config=config(j,&object)?;config.validate()?;
            let project=j.obj(&object,"getProject","()Lcom/intellij/openapi/project/Project;",&[])?;let project=Project::get(j,&project)?;
            ensure!(project.trusted(j)?,"Trust this project before running Cargo.");
            let directory=if config.directory.is_empty(){Path::new(&config.manifest).parent().unwrap_or(Path::new("."))}else{Path::new(&config.directory)};
            let args=config.args(j)?;let command=project::command(j,directory,&args,&config.environment()?)?;
            j.new("com/intellij/execution/process/KillableColoredProcessHandler","(Lcom/intellij/execution/configurations/GeneralCommandLine;)V",&[A::O(&command)])
        }
        _=>anyhow::bail!("Unknown run configuration operation: {operation}"),
    }
}
fn read(j: &mut J<'_>, receiver: &O, element: &O) -> Result<()> {
    let child = j.obj(
        element,
        "getChild",
        "(Ljava/lang/String;)Lorg/jdom/Element;",
        &[A::S("cranpose")],
    )?;
    if child.is_null() {
        return Ok(());
    }
    let mut value = serde_json::to_value(Config::default())?;
    for key in [
        "manifest",
        "package",
        "target",
        "kind",
        "command",
        "noDefaultFeatures",
    ] {
        let text = j.obj(
            &child,
            "getAttributeValue",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[A::S(key)],
        )?;
        if !text.is_null() {
            let text = j.read_string(&text)?;
            value[key] = if key == "noDefaultFeatures" {
                json!(text == "true")
            } else {
                json!(text)
            };
        }
    }
    for key in ["arguments", "features", "directory", "environment"] {
        let text = j.obj(
            &child,
            "getChildText",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[A::S(key)],
        )?;
        value[key] = json!(j.read_string(&text)?);
    }
    store(j, receiver, &serde_json::from_value(value)?)
}
fn write(j: &mut J<'_>, receiver: &O, element: &O) -> Result<()> {
    let value = serde_json::to_value(config(j, receiver)?)?;
    j.call(
        element,
        "removeChildren",
        "(Ljava/lang/String;)Z",
        &[A::S("cranpose")],
    )?;
    let child = j.new(
        "org/jdom/Element",
        "(Ljava/lang/String;)V",
        &[A::S("cranpose")],
    )?;
    for key in [
        "manifest",
        "package",
        "target",
        "kind",
        "command",
        "noDefaultFeatures",
    ] {
        let text = value[key]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value[key].to_string());
        j.obj(
            &child,
            "setAttribute",
            "(Ljava/lang/String;Ljava/lang/String;)Lorg/jdom/Element;",
            &[A::S(key), A::S(&text)],
        )?;
    }
    for key in ["arguments", "features", "directory", "environment"] {
        let field = j.new("org/jdom/Element", "(Ljava/lang/String;)V", &[A::S(key)])?;
        j.obj(
            &field,
            "setText",
            "(Ljava/lang/String;)Lorg/jdom/Element;",
            &[A::S(value[key].as_str().unwrap_or_default())],
        )?;
        j.obj(
            &child,
            "addContent",
            "(Lorg/jdom/Content;)Lorg/jdom/Element;",
            &[A::O(&field)],
        )?;
    }
    j.obj(
        element,
        "addContent",
        "(Lorg/jdom/Content;)Lorg/jdom/Element;",
        &[A::O(&child)],
    )?;
    Ok(())
}
struct EditorState {
    value: Config,
    synced: String,
    ready: bool,
}
fn settings(project: &Arc<Project>, j: &mut J<'_>) -> Result<O> {
    let mut options = project::options(j, false)?;
    options
        .environment
        .insert("CRANPOSE_RUN_SETTINGS".into(), "1".into());
    let panel = Panel::new(j, options)?;
    let size = j.new("java/awt/Dimension", "(II)V", &[A::I(660), A::I(520)])?;
    j.void(
        panel.primary.component(),
        "setPreferredSize",
        "(Ljava/awt/Dimension;)V",
        &[A::O(&size)],
    )?;
    let settings_object = Arc::new(OnceLock::<O>::new());
    let changed_editor = settings_object.clone();
    let state = Arc::new(Mutex::new(EditorState {
        value: Config::default(),
        synced: String::new(),
        ready: false,
    }));
    let captured = state.clone();
    *panel.on_message.lock().expect("callback") = Some(Arc::new(move |j, _, channel, payload| {
        if channel == "runSettings.changed" {
            captured.lock().expect("settings").value = serde_json::from_str(payload)?;
            if let Some(editor) = changed_editor.get() {
                j.void(editor, "fireEditorStateChanged", "()V", &[])?;
            }
        } else if channel == "runSettings.synced" {
            let value: Value = serde_json::from_str(payload)?;
            let mut state = captured.lock().expect("settings");
            state.value = serde_json::from_value(value["value"].clone())?;
            state.synced = value["token"].as_str().unwrap_or_default().into();
        }
        Ok(())
    }));
    let captured = state.clone();
    let owner = Arc::downgrade(project);
    *panel.on_lifecycle.lock().expect("callback") =
        Some(Arc::new(move |j, panel, connected, _| {
            captured.lock().expect("settings").ready = connected;
            if connected && let Some(project) = owner.upgrade() {
                project.theme(j, panel)?;
                let value = captured.lock().expect("settings").value.clone();
                send_settings(&project, panel, &value);
            }
            Ok(())
        }));
    let ids = Arc::new(AtomicI64::new(0));
    let callback_id = ids.clone();
    let owner = project.clone();
    let captured_panel = panel.clone();
    let id = project.scope.register(move |j, op, args| match op {
        "RunSettings.createEditor" => {
            captured_panel.start(j)?;
            Ok(captured_panel.primary.component().clone())
        }
        "RunSettings.resetEditorFrom" => {
            let value = config(j, &args[0])?;
            state.lock().expect("settings").value = value.clone();
            send_settings(&owner, &captured_panel, &value);
            j.null()
        }
        "RunSettings.applyEditorTo" => {
            if state.lock().expect("settings").ready {
                let token = format!("sync-{:?}", Instant::now());
                captured_panel.message("runSettings.sync", &token);
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline {
                    captured_panel.tick(j)?;
                    if state.lock().expect("settings").synced == token {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                ensure!(
                    state.lock().expect("settings").synced == token,
                    "Cranpose settings did not respond. Try Apply again."
                );
            }
            store(j, &args[0], &state.lock().expect("settings").value)?;
            j.null()
        }
        "RunSettings.disposeEditor" => {
            captured_panel.close(j)?;
            jvm::unregister(callback_id.load(Ordering::Acquire));
            j.null()
        }
        _ => j.null(),
    });
    ids.store(id, Ordering::Release);
    let editor = j.new("dev/cranpose/rust/RunSettings", "(J)V", &[A::J(id)])?;
    settings_object.set(editor.clone()).ok();
    project.attach(j, &panel)?;
    Ok(editor)
}
fn send_settings(project: &Project, panel: &Panel, value: &Config) {
    let targets = project.snapshot.lock().expect("snapshot").targets.clone();
    panel.message(
        "runSettings.init",
        &json!({"value":value,"targets":targets}).to_string(),
    );
}
pub fn create_selected(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    let target = {
        let snapshot = project.snapshot.lock().expect("snapshot");
        snapshot
            .targets
            .iter()
            .find(|t| t.id == snapshot.selected)
            .cloned()
    };
    let Some(target) = target else {
        return Ok(());
    };
    let class = j.class("dev/cranpose/rust/ConfigurationType")?;
    let kind = j.static_obj(
        "com/intellij/execution/configurations/ConfigurationTypeUtil",
        "findConfigurationType",
        "(Ljava/lang/Class;)Lcom/intellij/execution/configurations/ConfigurationType;",
        &[A::O(&class)],
    )?;
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
        .context("Configuration factory")?;
    let manager = j.static_obj(
        "com/intellij/execution/RunManager",
        "getInstance",
        "(Lcom/intellij/openapi/project/Project;)Lcom/intellij/execution/RunManager;",
        &[A::O(&project.object)],
    )?;
    let settings=j.obj(&manager,"createConfiguration","(Ljava/lang/String;Lcom/intellij/execution/configurations/ConfigurationFactory;)Lcom/intellij/execution/RunnerAndConfigurationSettings;",&[A::S(&target.name),A::O(&factory)])?;
    let configuration = j.obj(
        &settings,
        "getConfiguration",
        "()Lcom/intellij/execution/configurations/RunConfiguration;",
        &[],
    )?;
    store(j, &configuration, &Config::target(&target))?;
    if j.static_call("com/intellij/execution/impl/RunDialog","editConfiguration","(Lcom/intellij/openapi/project/Project;Lcom/intellij/execution/RunnerAndConfigurationSettings;Ljava/lang/String;)Z",&[A::O(&project.object),A::O(&settings),A::S("Save Cranpose Configuration")])?.z()?{
        j.void(&manager,"addConfiguration","(Lcom/intellij/execution/RunnerAndConfigurationSettings;)V",&[A::O(&settings)])?;
        j.void(&manager,"setSelectedConfiguration","(Lcom/intellij/execution/RunnerAndConfigurationSettings;)V",&[A::O(&settings)])?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_preserves_equals_and_rejects_invalid_names() {
        let c = Config {
            environment: "TOKEN=a=b\nEMPTY=\n".into(),
            ..Config::default()
        };
        assert_eq!(c.environment().expect("environment")["TOKEN"], "a=b");
        assert!(
            Config {
                environment: "BAD NAME=value".into(),
                ..Config::default()
            }
            .environment()
            .is_err()
        );
    }
    #[test]
    fn missing_legacy_fields_have_defaults() {
        let c: Config =
            serde_json::from_str(r#"{"manifest":"Cargo.toml"}"#).expect("configuration");
        assert_eq!(c.command, "run");
        assert_eq!(c.kind, "bin");
        assert!(!c.no_default_features);
    }
}
