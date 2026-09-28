use crate::{BRIDGE, Class};
pub fn classes(plugin_id: &str) -> Vec<(String, Vec<u8>)> {
    let mut result = vec![(BRIDGE.into(), crate::native_bridge(Some(plugin_id)))];
    let mut add =
        |name: &str, parent: &str, interfaces: &[&str], handle: bool, methods: &[(&str, &str)]| {
            let path = format!("dev/cranpose/rust/{name}");
            let mut class = Class::new(&path, parent, interfaces);
            if handle {
                class.handle_constructor();
            } else {
                class.constructor("()V", "()V", &[], false);
            }
            for (name, signature) in methods {
                class.forward(name, signature);
            }
            result.push((path, class.finish()));
        };
    add(
        "Callback",
        "java/lang/Object",
        &[
            "java/lang/Runnable",
            "com/intellij/openapi/Disposable",
            "java/awt/event/ActionListener",
            "java/awt/event/MouseListener",
            "java/awt/event/MouseMotionListener",
            "java/awt/event/MouseWheelListener",
            "java/awt/event/KeyListener",
            "java/awt/event/ComponentListener",
            "java/awt/event/HierarchyListener",
            "java/beans/PropertyChangeListener",
            "java/awt/event/WindowListener",
            "com/intellij/openapi/editor/event/DocumentListener",
            "com/intellij/openapi/editor/event/EditorFactoryListener",
            "com/intellij/openapi/editor/event/EditorMouseListener",
            "com/intellij/openapi/editor/event/EditorMouseMotionListener",
            "com/intellij/openapi/fileEditor/FileEditorManagerListener",
            "com/intellij/ide/ui/LafManagerListener",
            "com/intellij/openapi/vfs/newvfs/BulkFileListener",
            "com/intellij/openapi/editor/event/CaretListener",
            "com/intellij/openapi/editor/event/VisibleAreaListener",
            "com/intellij/openapi/editor/InlayModel$Listener",
            "com/intellij/openapi/editor/ex/FoldingListener",
            "com/intellij/util/Function",
            "java/util/function/Supplier",
            "com/intellij/codeInsight/daemon/GutterIconNavigationHandler",
            "com/intellij/codeInsight/completion/InsertHandler",
            "com/intellij/execution/process/ProcessListener",
        ],
        true,
        &[
            ("run", "()V"),
            ("dispose", "()V"),
            ("actionPerformed", "(Ljava/awt/event/ActionEvent;)V"),
            ("mousePressed", "(Ljava/awt/event/MouseEvent;)V"),
            ("mouseReleased", "(Ljava/awt/event/MouseEvent;)V"),
            ("mouseClicked", "(Ljava/awt/event/MouseEvent;)V"),
            ("mouseEntered", "(Ljava/awt/event/MouseEvent;)V"),
            ("mouseExited", "(Ljava/awt/event/MouseEvent;)V"),
            ("mouseMoved", "(Ljava/awt/event/MouseEvent;)V"),
            ("mouseDragged", "(Ljava/awt/event/MouseEvent;)V"),
            ("mouseWheelMoved", "(Ljava/awt/event/MouseWheelEvent;)V"),
            ("keyPressed", "(Ljava/awt/event/KeyEvent;)V"),
            ("keyReleased", "(Ljava/awt/event/KeyEvent;)V"),
            ("keyTyped", "(Ljava/awt/event/KeyEvent;)V"),
            ("componentResized", "(Ljava/awt/event/ComponentEvent;)V"),
            ("componentMoved", "(Ljava/awt/event/ComponentEvent;)V"),
            ("componentShown", "(Ljava/awt/event/ComponentEvent;)V"),
            ("componentHidden", "(Ljava/awt/event/ComponentEvent;)V"),
            ("hierarchyChanged", "(Ljava/awt/event/HierarchyEvent;)V"),
            ("propertyChange", "(Ljava/beans/PropertyChangeEvent;)V"),
            ("onAdded", "(Lcom/intellij/openapi/editor/Inlay;)V"),
            ("onUpdated", "(Lcom/intellij/openapi/editor/Inlay;I)V"),
            ("onRemoved", "(Lcom/intellij/openapi/editor/Inlay;)V"),
            (
                "onBatchModeFinish",
                "(Lcom/intellij/openapi/editor/Editor;)V",
            ),
            ("onFoldProcessingEnd", "()V"),
            ("windowOpened", "(Ljava/awt/event/WindowEvent;)V"),
            ("windowClosing", "(Ljava/awt/event/WindowEvent;)V"),
            ("windowClosed", "(Ljava/awt/event/WindowEvent;)V"),
            ("windowIconified", "(Ljava/awt/event/WindowEvent;)V"),
            ("windowDeiconified", "(Ljava/awt/event/WindowEvent;)V"),
            ("windowActivated", "(Ljava/awt/event/WindowEvent;)V"),
            ("windowDeactivated", "(Ljava/awt/event/WindowEvent;)V"),
            (
                "documentChanged",
                "(Lcom/intellij/openapi/editor/event/DocumentEvent;)V",
            ),
            (
                "editorCreated",
                "(Lcom/intellij/openapi/editor/event/EditorFactoryEvent;)V",
            ),
            (
                "editorReleased",
                "(Lcom/intellij/openapi/editor/event/EditorFactoryEvent;)V",
            ),
            (
                "mouseMoved",
                "(Lcom/intellij/openapi/editor/event/EditorMouseEvent;)V",
            ),
            (
                "mouseClicked",
                "(Lcom/intellij/openapi/editor/event/EditorMouseEvent;)V",
            ),
            (
                "mouseExited",
                "(Lcom/intellij/openapi/editor/event/EditorMouseEvent;)V",
            ),
            (
                "selectionChanged",
                "(Lcom/intellij/openapi/fileEditor/FileEditorManagerEvent;)V",
            ),
            ("lookAndFeelChanged", "(Lcom/intellij/ide/ui/LafManager;)V"),
            ("after", "(Ljava/util/List;)V"),
            (
                "caretPositionChanged",
                "(Lcom/intellij/openapi/editor/event/CaretEvent;)V",
            ),
            (
                "visibleAreaChanged",
                "(Lcom/intellij/openapi/editor/event/VisibleAreaEvent;)V",
            ),
            ("fun", "(Ljava/lang/Object;)Ljava/lang/Object;"),
            ("get", "()Ljava/lang/Object;"),
            (
                "navigate",
                "(Ljava/awt/event/MouseEvent;Lcom/intellij/psi/PsiElement;)V",
            ),
            (
                "handleInsert",
                "(Lcom/intellij/codeInsight/completion/InsertionContext;Lcom/intellij/codeInsight/lookup/LookupElement;)V",
            ),
            (
                "onTextAvailable",
                "(Lcom/intellij/execution/process/ProcessEvent;Lcom/intellij/openapi/util/Key;)V",
            ),
            (
                "processTerminated",
                "(Lcom/intellij/execution/process/ProcessEvent;)V",
            ),
        ],
    );
    add(
        "Surface",
        "javax/swing/JComponent",
        &[],
        true,
        &[
            ("paintComponent", "(Ljava/awt/Graphics;)V"),
            ("contains", "(II)Z"),
        ],
    );
    add(
        "ShowcaseGenerator",
        "java/lang/Object",
        &["com/intellij/platform/DirectoryProjectGenerator"],
        false,
        &[
            ("getName", "()Ljava/lang/String;"),
            ("getDescription", "()Ljava/lang/String;"),
            ("getLogo", "()Ljavax/swing/Icon;"),
            (
                "createPeer",
                "()Lcom/intellij/platform/ProjectGeneratorPeer;",
            ),
            (
                "validate",
                "(Ljava/lang/String;)Lcom/intellij/facet/ui/ValidationResult;",
            ),
            (
                "generateProject",
                "(Lcom/intellij/openapi/project/Project;Lcom/intellij/openapi/vfs/VirtualFile;Ljava/lang/Object;Lcom/intellij/openapi/module/Module;)V",
            ),
        ],
    );
    add(
        "ShowcasePeer",
        "java/lang/Object",
        &[
            "com/intellij/platform/ProjectGeneratorPeer",
            "com/intellij/openapi/Disposable",
        ],
        true,
        &[
            ("getComponent", "()Ljavax/swing/JComponent;"),
            (
                "getComponent",
                "(Lcom/intellij/openapi/ui/TextFieldWithBrowseButton;Ljava/lang/Runnable;)Ljavax/swing/JComponent;",
            ),
            (
                "buildUI",
                "(Lcom/intellij/ide/util/projectWizard/SettingsStep;)V",
            ),
            ("getSettings", "()Ljava/lang/Object;"),
            ("validate", "()Lcom/intellij/openapi/ui/ValidationInfo;"),
            ("isBackgroundJobRunning", "()Z"),
            ("dispose", "()V"),
        ],
    );
    add(
        "ShowcaseBuilder",
        "com/intellij/ide/util/projectWizard/ModuleBuilder",
        &[],
        false,
        &[
            ("getPresentableName", "()Ljava/lang/String;"),
            ("getBuilderId", "()Ljava/lang/String;"),
            ("getGroupName", "()Ljava/lang/String;"),
            ("getDescription", "()Ljava/lang/String;"),
            ("getNodeIcon", "()Ljavax/swing/Icon;"),
            ("getWeight", "()I"),
            ("isAvailable", "()Z"),
            (
                "getModuleType",
                "()Lcom/intellij/openapi/module/ModuleType;",
            ),
            (
                "modifyProjectTypeStep",
                "(Lcom/intellij/ide/util/projectWizard/SettingsStep;)Lcom/intellij/ide/util/projectWizard/ModuleWizardStep;",
            ),
            (
                "setupRootModel",
                "(Lcom/intellij/openapi/roots/ModifiableRootModel;)V",
            ),
        ],
    );
    add(
        "ShowcaseStep",
        "com/intellij/ide/util/projectWizard/ModuleWizardStep",
        &[],
        true,
        &[
            ("getComponent", "()Ljavax/swing/JComponent;"),
            ("updateDataModel", "()V"),
            ("validate", "()Z"),
            ("disposeUIResources", "()V"),
        ],
    );
    add(
        "PreviewGutter",
        "com/intellij/openapi/editor/markup/GutterIconRenderer",
        &["com/intellij/openapi/project/DumbAware"],
        true,
        &[
            ("getIcon", "()Ljavax/swing/Icon;"),
            ("getTooltipText", "()Ljava/lang/String;"),
            (
                "getClickAction",
                "()Lcom/intellij/openapi/actionSystem/AnAction;",
            ),
            ("isNavigateAction", "()Z"),
            ("equals", "(Ljava/lang/Object;)Z"),
            ("hashCode", "()I"),
        ],
    );
    add(
        "PreviewClick",
        "com/intellij/openapi/actionSystem/AnAction",
        &["com/intellij/openapi/project/DumbAware"],
        true,
        &[
            (
                "actionPerformed",
                "(Lcom/intellij/openapi/actionSystem/AnActionEvent;)V",
            ),
            (
                "getActionUpdateThread",
                "()Lcom/intellij/openapi/actionSystem/ActionUpdateThread;",
            ),
        ],
    );
    add(
        "ValueGlyph",
        "java/lang/Object",
        &["com/intellij/openapi/editor/EditorCustomElementRenderer"],
        true,
        &[
            (
                "calcWidthInPixels",
                "(Lcom/intellij/openapi/editor/Inlay;)I",
            ),
            (
                "paint",
                "(Lcom/intellij/openapi/editor/Inlay;Ljava/awt/Graphics;Ljava/awt/Rectangle;Lcom/intellij/openapi/editor/markup/TextAttributes;)V",
            ),
        ],
    );
    add(
        "Workspace",
        "javax/swing/JLayeredPane",
        &["com/intellij/openapi/Disposable"],
        true,
        &[("doLayout", "()V"), ("dispose", "()V")],
    );
    add(
        "ToolWindow",
        "java/lang/Object",
        &[
            "com/intellij/openapi/wm/ToolWindowFactory",
            "com/intellij/openapi/project/DumbAware",
        ],
        false,
        &[(
            "createToolWindowContent",
            "(Lcom/intellij/openapi/project/Project;Lcom/intellij/openapi/wm/ToolWindow;)V",
        )],
    );
    add(
        "Startup",
        "java/lang/Object",
        &["com/intellij/openapi/startup/ProjectActivity"],
        false,
        &[(
            "execute",
            "(Lcom/intellij/openapi/project/Project;Lkotlin/coroutines/Continuation;)Ljava/lang/Object;",
        )],
    );
    add(
        "PreviewProvider",
        "java/lang/Object",
        &[
            "com/intellij/openapi/fileEditor/FileEditorProvider",
            "com/intellij/openapi/project/DumbAware",
        ],
        false,
        &[
            (
                "accept",
                "(Lcom/intellij/openapi/project/Project;Lcom/intellij/openapi/vfs/VirtualFile;)Z",
            ),
            (
                "createEditor",
                "(Lcom/intellij/openapi/project/Project;Lcom/intellij/openapi/vfs/VirtualFile;)Lcom/intellij/openapi/fileEditor/FileEditor;",
            ),
            ("getEditorTypeId", "()Ljava/lang/String;"),
            (
                "getPolicy",
                "()Lcom/intellij/openapi/fileEditor/FileEditorPolicy;",
            ),
        ],
    );
    add(
        "PreviewEditor",
        "com/intellij/openapi/util/UserDataHolderBase",
        &["com/intellij/openapi/fileEditor/FileEditor"],
        true,
        &[
            ("getComponent", "()Ljavax/swing/JComponent;"),
            ("getPreferredFocusedComponent", "()Ljavax/swing/JComponent;"),
            ("getName", "()Ljava/lang/String;"),
            ("selectNotify", "()V"),
            ("getFile", "()Lcom/intellij/openapi/vfs/VirtualFile;"),
            (
                "setState",
                "(Lcom/intellij/openapi/fileEditor/FileEditorState;)V",
            ),
            ("isModified", "()Z"),
            ("isValid", "()Z"),
            ("dispose", "()V"),
            (
                "addPropertyChangeListener",
                "(Ljava/beans/PropertyChangeListener;)V",
            ),
            (
                "removePropertyChangeListener",
                "(Ljava/beans/PropertyChangeListener;)V",
            ),
        ],
    );
    add(
        "Badge",
        "java/lang/Object",
        &["com/intellij/openapi/editor/EditorCustomElementRenderer"],
        true,
        &[
            (
                "calcWidthInPixels",
                "(Lcom/intellij/openapi/editor/Inlay;)I",
            ),
            (
                "paint",
                "(Lcom/intellij/openapi/editor/Inlay;Ljava/awt/Graphics;Ljava/awt/Rectangle;Lcom/intellij/openapi/editor/markup/TextAttributes;)V",
            ),
        ],
    );
    for action in ["Refresh", "Check", "Run", "Preview", "Docs"] {
        add(
            &format!("{action}Action"),
            "com/intellij/openapi/project/DumbAwareAction",
            &[],
            false,
            &[
                (
                    "getActionUpdateThread",
                    "()Lcom/intellij/openapi/actionSystem/ActionUpdateThread;",
                ),
                (
                    "actionPerformed",
                    "(Lcom/intellij/openapi/actionSystem/AnActionEvent;)V",
                ),
            ],
        );
    }
    for name in ["ToggleBadges", "ToggleStable"] {
        add(
            name,
            "com/intellij/openapi/actionSystem/ToggleAction",
            &[],
            false,
            &[
                (
                    "getActionUpdateThread",
                    "()Lcom/intellij/openapi/actionSystem/ActionUpdateThread;",
                ),
                (
                    "isSelected",
                    "(Lcom/intellij/openapi/actionSystem/AnActionEvent;)Z",
                ),
                (
                    "setSelected",
                    "(Lcom/intellij/openapi/actionSystem/AnActionEvent;Z)V",
                ),
            ],
        );
    }
    add(
        "LineMarker",
        "java/lang/Object",
        &["com/intellij/codeInsight/daemon/LineMarkerProvider"],
        false,
        &[(
            "getLineMarkerInfo",
            "(Lcom/intellij/psi/PsiElement;)Lcom/intellij/codeInsight/daemon/LineMarkerInfo;",
        )],
    );
    add(
        "Completion",
        "com/intellij/codeInsight/completion/CompletionContributor",
        &[],
        false,
        &[(
            "fillCompletionVariants",
            "(Lcom/intellij/codeInsight/completion/CompletionParameters;Lcom/intellij/codeInsight/completion/CompletionResultSet;)V",
        )],
    );
    add(
        "RunSettings",
        "com/intellij/openapi/options/SettingsEditor",
        &[],
        true,
        &[
            ("createEditor", "()Ljavax/swing/JComponent;"),
            ("resetEditorFrom", "(Ljava/lang/Object;)V"),
            ("applyEditorTo", "(Ljava/lang/Object;)V"),
            ("disposeEditor", "()V"),
        ],
    );
    let name = "dev/cranpose/rust/ConfigurationType";
    let mut class = Class::new(
        name,
        "java/lang/Object",
        &[
            "com/intellij/execution/configurations/ConfigurationType",
            "com/intellij/openapi/project/DumbAware",
        ],
    );
    class.field(
        "factories",
        "[Lcom/intellij/execution/configurations/ConfigurationFactory;",
    );
    class.constructor("()V", "()V", &[], false);
    for (method, signature) in [
        ("getDisplayName", "()Ljava/lang/String;"),
        ("getConfigurationTypeDescription", "()Ljava/lang/String;"),
        ("getIcon", "()Ljavax/swing/Icon;"),
        ("getId", "()Ljava/lang/String;"),
        (
            "getConfigurationFactories",
            "()[Lcom/intellij/execution/configurations/ConfigurationFactory;",
        ),
    ] {
        class.forward(method, signature);
    }
    result.push((name.into(), class.finish()));
    let name = "dev/cranpose/rust/ConfigurationFactory";
    let mut class = Class::new(
        name,
        "com/intellij/execution/configurations/ConfigurationFactory",
        &[],
    );
    class.constructor(
        "(Lcom/intellij/execution/configurations/ConfigurationType;)V",
        "(Lcom/intellij/execution/configurations/ConfigurationType;)V",
        &[0],
        false,
    );
    class.forward("getId", "()Ljava/lang/String;");
    class.forward("createTemplateConfiguration","(Lcom/intellij/openapi/project/Project;)Lcom/intellij/execution/configurations/RunConfiguration;");
    result.push((name.into(), class.finish()));
    let name = "dev/cranpose/rust/RunConfiguration";
    let mut class = Class::new(
        name,
        "com/intellij/execution/configurations/RunConfigurationBase",
        &[],
    );
    class.field("configuration", "Ljava/lang/String;");
    let constructor = "(Lcom/intellij/openapi/project/Project;Lcom/intellij/execution/configurations/ConfigurationFactory;Ljava/lang/String;)V";
    class.constructor(constructor, constructor, &[0, 1, 2], true);
    class.forward("checkConfiguration", "()V");
    class.forward(
        "getConfigurationEditor",
        "()Lcom/intellij/openapi/options/SettingsEditor;",
    );
    class.forward("getState","(Lcom/intellij/execution/Executor;Lcom/intellij/execution/runners/ExecutionEnvironment;)Lcom/intellij/execution/configurations/RunProfileState;");
    for method in ["readExternal", "writeExternal"] {
        class.forward_after_super(method, "(Lorg/jdom/Element;)V");
    }
    result.push((name.into(), class.finish()));
    let name = "dev/cranpose/rust/RunState";
    let mut class = Class::new(
        name,
        "com/intellij/execution/configurations/CommandLineState",
        &[],
    );
    class.field("configuration", "Ljava/lang/Object;");
    class.constructor(
        "(Lcom/intellij/execution/runners/ExecutionEnvironment;Ljava/lang/Object;)V",
        "(Lcom/intellij/execution/runners/ExecutionEnvironment;)V",
        &[0],
        true,
    );
    class.forward(
        "startProcess",
        "()Lcom/intellij/execution/process/ProcessHandler;",
    );
    result.push((name.into(), class.finish()));
    result
}
