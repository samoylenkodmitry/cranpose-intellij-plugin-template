use super::*;
use anyhow::{Context, ensure};
use serde_json::json;

pub(crate) fn integration_test(j: &mut J<'_>) -> Result<()> {
    let project = crate::ide_tests::project(j)?;
    let factory = j.static_obj(
        "com/intellij/openapi/editor/EditorFactory",
        "getInstance",
        "()Lcom/intellij/openapi/editor/EditorFactory;",
        &[],
    )?;
    let file = j.new(
        "com/intellij/testFramework/LightVirtualFile",
        "(Ljava/lang/String;Ljava/lang/CharSequence;)V",
        &[
            A::S("counters.rs"),
            A::S("// 🦀\n#[composable]\nfn Counter() {}\n\n#[composable]\nfn Label() {}\n"),
        ],
    )?;
    let manager = j.static_obj(
        "com/intellij/openapi/fileEditor/FileDocumentManager",
        "getInstance",
        "()Lcom/intellij/openapi/fileEditor/FileDocumentManager;",
        &[],
    )?;
    let document = j.obj(
        &manager,
        "getDocument",
        "(Lcom/intellij/openapi/vfs/VirtualFile;)Lcom/intellij/openapi/editor/Document;",
        &[A::O(&file)],
    )?;
    let path = j.text(&file, "getPath")?;
    let first = j.obj(&factory, "createEditor", "(Lcom/intellij/openapi/editor/Document;Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/editor/Editor;", &[A::O(&document), A::O(&project.object)])?;
    let mut state = Inlays::default();
    let mut split = None;
    let result = (|| -> Result<()> {
        let rows = |count| -> Result<Vec<Row>> {
            Ok(serde_json::from_value(json!([
                {"file":path,"name":"Counter","line":3,"recompositions":count,"instances":2},
                {"file":path,"name":"Label","line":6,"recompositions":0,"instances":1}
            ]))?)
        };
        state.update(j, &project.object, &rows(0)?)?;
        ensure!(
            state.editors.len() == 1 && state.editors[0].placed.len() == 2,
            "Missing code counters"
        );
        let key = Definition {
            file: path.clone(),
            name: "Counter".into(),
            line: 3,
        };
        let inlay = state.editors[0].placed[&key].inlay.clone();
        let placement = j.obj(
            &inlay,
            "getPlacement",
            "()Lcom/intellij/openapi/editor/Inlay$Placement;",
            &[],
        )?;
        ensure!(
            j.text(&placement, "toString")? == "ABOVE_LINE",
            "Counter must sit above its definition"
        );
        ensure!(
            j.int(&inlay, "getHeightInPixels")? > 0 && j.int(&inlay, "getWidthInPixels")? > 0,
            "Invisible block inlay"
        );
        state.update(j, &project.object, &rows(12)?)?;
        ensure!(
            j.same(&inlay, &state.editors[0].placed[&key].inlay)?,
            "Counter update recreated the inlay"
        );
        ensure!(
            *state.editors[0].placed[&key].label.lock().expect("label")
                == "Preview · 12 recompositions · 2 instances",
            "Aggregate label"
        );
        let pixels = glyphs::rendered(j, &inlay)?;
        ensure!(pixels.len() > 40, "Counter did not paint");
        let component = j.obj(&first, "getComponent", "()Ljavax/swing/JComponent;", &[])?;
        j.void(&component, "setSize", "(II)V", &[A::I(720), A::I(230)])?;
        j.void(&component, "doLayout", "()V", &[])?;
        let content = j.obj(
            &first,
            "getContentComponent",
            "()Ljavax/swing/JComponent;",
            &[],
        )?;
        j.void(&content, "setSize", "(II)V", &[A::I(720), A::I(230)])?;
        let image = j.new(
            "java/awt/image/BufferedImage",
            "(III)V",
            &[A::I(720), A::I(230), A::I(2)],
        )?;
        let graphics = j.obj(&image, "createGraphics", "()Ljava/awt/Graphics2D;", &[])?;
        j.void(
            &content,
            "paint",
            "(Ljava/awt/Graphics;)V",
            &[A::O(&graphics)],
        )?;
        j.void(&graphics, "dispose", "()V", &[])?;
        let output = j.static_obj(
            "java/lang/System",
            "getProperty",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[A::S("cranpose.test.output")],
        )?;
        let output = j.read_string(&output)?;
        let file = j.new(
            "java/io/File",
            "(Ljava/lang/String;)V",
            &[A::S(&format!("{output}/recomposition-counter.png"))],
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
        let offset = j.int(&inlay, "getOffset")?;
        let doc = document.clone();
        let scope = jvm::Scope::default();
        let id = scope.register(move |j, _, _| {
            j.void(
                &doc,
                "insertString",
                "(ILjava/lang/CharSequence;)V",
                &[A::I(0), A::S("\n\n")],
            )?;
            j.null()
        });
        let callback = jvm::callback(j, id)?;
        j.static_void(
            "com/intellij/openapi/command/WriteCommandAction",
            "runWriteCommandAction",
            "(Lcom/intellij/openapi/project/Project;Ljava/lang/Runnable;)V",
            &[A::O(&project.object), A::O(&callback)],
        )?;
        state.update(j, &project.object, &rows(13)?)?;
        ensure!(
            j.same(&inlay, &state.editors[0].placed[&key].inlay)?
                && j.int(&inlay, "getOffset")? == offset + 2,
            "Counter lost its definition after an edit"
        );
        split = Some(j.obj(&factory, "createEditor", "(Lcom/intellij/openapi/editor/Document;Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/editor/Editor;", &[A::O(&document), A::O(&project.object)])?);
        state.update(j, &project.object, &rows(14)?)?;
        ensure!(
            state.editors.len() == 2 && state.editors.iter().all(|editor| editor.placed.len() == 2),
            "New split editor missed counters"
        );
        let other = split.take().context("split")?;
        j.void(
            &factory,
            "releaseEditor",
            "(Lcom/intellij/openapi/editor/Editor;)V",
            &[A::O(&other)],
        )?;
        state.update(j, &project.object, &rows(15)?)?;
        ensure!(state.editors.len() == 1, "Closed editor retained");
        state.update(j, &project.object, &[])?;
        ensure!(
            !j.bool(&inlay, "isValid")?,
            "Stopping preview left a counter behind"
        );
        state.update(j, &project.object, &rows(0)?)?;
        ensure!(
            state.editors[0].placed.len() == 2,
            "Restart did not restore counters"
        );
        state.clear(j)?;
        Ok(())
    })();
    if result.is_err() && j.env.exception_check()? {
        j.env.exception_clear()?;
    }
    state.clear(j)?;
    for editor in std::iter::once(first).chain(split) {
        j.void(
            &factory,
            "releaseEditor",
            "(Lcom/intellij/openapi/editor/Editor;)V",
            &[A::O(&editor)],
        )?;
    }
    result
}
