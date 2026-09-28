//! Repeatable editor-only placement measurements, run in the actual test IDE.
use super::*;
use std::time::Instant;

pub fn integration_test(project: &Arc<Project>, j: &mut J<'_>) -> Result<()> {
    let factory = j.static_obj(
        "com/intellij/openapi/editor/EditorFactory",
        "getInstance",
        "()Lcom/intellij/openapi/editor/EditorFactory;",
        &[],
    )?;
    let mut samples = vec![];
    let fixtures = [
        (32, 0),
        (128, 0),
        (256, 0),
        (1024, 0),
        (256, 360),
        (1024, 1440),
        (32, 30_000),
        (128, 30_000),
        (256, 30_000),
        (1024, 30_000),
    ];
    // Fresh IDEs continue compiling their editor code after a short warm-up.
    // Optimized profiling repeats in reverse order, then the original order,
    // so sparse/dense conclusions do not depend on being first in the suite.
    let rounds = if cfg!(debug_assertions) { 1 } else { 3 };
    for round in 0..rounds {
        let mut fixtures = fixtures.to_vec();
        if round % 2 == 1 {
            fixtures.reverse();
        }
        for (count, padding) in fixtures {
            let mut source = String::from("// 🦀 placement fixture\n#[composable]\nfn Card(){\n");
            for index in 0..count {
                source.push_str(&format!("    Text(\"label {index}\");\n"));
            }
            source.push_str("}\n");
            source.push_str(&"// unrelated source padding\n".repeat(padding));
            let mut catalog = Catalog::parse(&source)?;
            ensure!(catalog.literals.len() == count, "Fixture catalog size");
            let document = j.obj(
                &factory,
                "createDocument",
                "(Ljava/lang/CharSequence;)Lcom/intellij/openapi/editor/Document;",
                &[A::S(&source)],
            )?;
            let editor = j.obj(&factory, "createEditor", "(Lcom/intellij/openapi/editor/Document;Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/editor/Editor;", &[A::O(&document), A::O(&project.object)])?;
            let result = (|| -> Result<()> {
                let model = j.obj(
                    &editor,
                    "getInlayModel",
                    "()Lcom/intellij/openapi/editor/InlayModel;",
                    &[],
                )?;
                crate::inlays::integration_test(j, &model)?;
                if padding == 30_000 {
                    ensure!(
                        !batch_placement(j, &editor, &catalog)?,
                        "Sparse file entered a batch"
                    );
                }
                watch_editor(project, j, &editor)?;
                place(project, j, &editor, "placement.rs", &catalog)?;
                remember(project, j, &document, &source, &catalog)?;
                let mut edit = 0;
                for iteration in 0..9 {
                    let modes = if iteration % 2 == 0 {
                        ["unbatched", "batched", "automatic", "reuse"]
                    } else {
                        ["reuse", "automatic", "batched", "unbatched"]
                    };
                    for mode in modes {
                        let identities = identities(project);
                        let token = if edit % 2 == 0 {
                            "\"label 🦀\""
                        } else {
                            "\"label x\""
                        };
                        edit += 1;
                        let range = &catalog.literals[0].range;
                        write(
                            project,
                            j,
                            &document,
                            range.start_utf16,
                            range.end_utf16,
                            token,
                        )?;
                        source.replace_range(range.start..range.end, token);
                        catalog = Catalog::parse(&source)?;
                        let started = Instant::now();
                        let mut clear_ms = None;
                        if mode != "reuse" {
                            clear_placed(project, j)?;
                            clear_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                            if mode == "automatic" {
                                place(project, j, &editor, "placement.rs", &catalog)?;
                            } else {
                                place_with_batch(
                                    project,
                                    j,
                                    &editor,
                                    "placement.rs",
                                    &catalog,
                                    mode == "batched",
                                )?;
                            }
                        } else {
                            refresh_placed(project, j, &editor, "placement.rs", &catalog)?;
                        }
                        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                        if mode == "reuse" {
                            ensure!(
                                identities == self::identities(project),
                                "Literal edit recreated anchors"
                            );
                        }
                        let items = project
                            .authoring
                            .lock()
                            .expect("authoring")
                            .placed
                            .iter()
                            .filter_map(|p| p.literal.map(|id| (p.object.clone(), id)))
                            .collect::<Vec<_>>();
                        ensure!(items.len() == count, "Missing placement glyphs");
                        for (object, id) in &items {
                            ensure!(j.bool(object, "isValid")?, "Invalid placement glyph");
                            ensure!(
                                j.int(object, "getOffset")? as usize
                                    == catalog.literals[*id].range.end_utf16,
                                "Incorrect Unicode glyph offset"
                            );
                            ensure!(
                                j.int(object, "getWidthInPixels")? == 14,
                                "Missing reserved glyph space"
                            );
                        }
                        if iteration >= 2 {
                            samples.push(json!({
                                "glyphs": count,
                                "round": round,
                                "padding_lines": padding,
                                "source_bytes": source.len(),
                                "iteration": iteration - 2,
                                "mode": mode,
                                "replace_ms": elapsed_ms,
                                "clear_ms": clear_ms,
                            }));
                        }
                        remember(project, j, &document, &source, &catalog)?;
                    }
                }
                if count == 1024 && padding == 0 && round == 0 {
                    caret_behavior(project, j, &editor, &catalog)?;
                }
                Ok(())
            })();
            if result.is_err() && j.env.exception_check()? {
                j.env.exception_describe()?;
                j.env.exception_clear()?;
            }
            let cleanup = detach(project, j);
            let released = j.void(
                &factory,
                "releaseEditor",
                "(Lcom/intellij/openapi/editor/Editor;)V",
                &[A::O(&editor)],
            );
            result?;
            cleanup?;
            released?;
        }
    }
    let output = j.static_obj(
        "java/lang/System",
        "getProperty",
        "(Ljava/lang/String;)Ljava/lang/String;",
        &[A::S("cranpose.test.output")],
    )?;
    std::fs::write(
        Path::new(&j.read_string(&output)?).join("authoring-placement.json"),
        serde_json::to_vec_pretty(
            &json!({"samples":samples,"optimized_host":!cfg!(debug_assertions),"scope":"headless IDE glyph replacement after Unicode edits only; excludes parsing, document writes, transport and display"}),
        )?,
    )?;
    lifecycle(project, j, &factory)?;
    Ok(())
}

fn caret_behavior(
    project: &Arc<Project>,
    j: &mut J<'_>,
    editor: &O,
    catalog: &Catalog,
) -> Result<()> {
    clear_placed(project, j)?;
    let model = j.obj(
        editor,
        "getCaretModel",
        "()Lcom/intellij/openapi/editor/CaretModel;",
        &[],
    )?;
    let caret = j.obj(
        &model,
        "getPrimaryCaret",
        "()Lcom/intellij/openapi/editor/Caret;",
        &[],
    )?;
    let end = catalog.literals[0].range.end_utf16 as i32;
    for offset in [0, end - 1, end] {
        let mut positions = vec![];
        for automatic in [false, true] {
            clear_placed(project, j)?;
            j.void(&caret, "moveToOffset", "(I)V", &[A::I(offset)])?;
            j.void(&caret, "setSelection", "(II)V", &[A::I(0), A::I(offset)])?;
            if automatic {
                ensure!(
                    batch_placement(j, editor, catalog)? == (offset != end),
                    "Caret batching policy"
                );
                place(project, j, editor, "placement.rs", catalog)?;
            } else {
                place_with_batch(project, j, editor, "placement.rs", catalog, false)?;
            }
            let visual = j.obj(
                &caret,
                "getVisualPosition",
                "()Lcom/intellij/openapi/editor/VisualPosition;",
                &[],
            )?;
            positions.push((
                j.int(&caret, "getOffset")?,
                j.int(&caret, "getSelectionStart")?,
                j.int(&caret, "getSelectionEnd")?,
                j.field_int(&visual, "line")?,
                j.field_int(&visual, "column")?,
            ));
        }
        ensure!(
            positions[0] == positions[1],
            "Batch changed caret or selection: {positions:?}"
        );
    }
    clear_placed(project, j)?;
    j.void(&caret, "removeSelection", "()V", &[])?;
    j.void(&caret, "moveToOffset", "(I)V", &[A::I(0)])?;
    let visual = j.obj(
        editor,
        "offsetToVisualPosition",
        "(I)Lcom/intellij/openapi/editor/VisualPosition;",
        &[A::I(end)],
    )?;
    let secondary = j.obj(
        &model,
        "addCaret",
        "(Lcom/intellij/openapi/editor/VisualPosition;)Lcom/intellij/openapi/editor/Caret;",
        &[A::O(&visual)],
    )?;
    ensure!(!secondary.is_null(), "Secondary caret fixture");
    ensure!(
        !batch_placement(j, editor, catalog)?,
        "Secondary caret was ignored"
    );
    j.void(&model, "removeSecondaryCarets", "()V", &[])?;
    Ok(())
}

fn lifecycle(project: &Arc<Project>, j: &mut J<'_>, factory: &O) -> Result<()> {
    let mut source = "// 🦀\n#[composable]\nfn Card(){ let tint = Color(0.1,0.2,0.3,1.0); let gap = 24.0; Text(tint); Space(gap); }".to_owned();
    let document = j.obj(
        factory,
        "createDocument",
        "(Ljava/lang/CharSequence;)Lcom/intellij/openapi/editor/Document;",
        &[A::S(&source)],
    )?;
    let editor = j.obj(factory, "createEditor", "(Lcom/intellij/openapi/editor/Document;Lcom/intellij/openapi/project/Project;)Lcom/intellij/openapi/editor/Editor;", &[A::O(&document), A::O(&project.object)])?;
    let result = (|| -> Result<()> {
        watch_editor(project, j, &editor)?;
        let mut catalog = Catalog::parse(&source)?;
        ensure!(!catalog.references.is_empty(), "Reference fixture");
        place(project, j, &editor, "placement.rs", &catalog)?;
        remember(project, j, &document, &source, &catalog)?;
        let before = identities(project);
        let color = catalog
            .literals
            .iter()
            .find(|l| l.kind == "color")
            .expect("fixture color");
        let value = "Color(0.8, 0.15, 0.2, 0.25)";
        write(
            project,
            j,
            &document,
            color.range.start_utf16,
            color.range.end_utf16,
            value,
        )?;
        source.replace_range(color.range.start..color.range.end, value);
        catalog = Catalog::parse(&source)?;
        refresh_placed(project, j, &editor, "placement.rs", &catalog)?;
        ensure!(
            before == identities(project),
            "Color edit recreated its aliases"
        );
        ensure!(
            reusable_placed(project, j, "placement.rs", &catalog)?,
            "Retained alias positions disagree"
        );
        remember(project, j, &document, &source, &catalog)?;

        // Even with identical schema and offsets, an invalid native anchor must
        // force replacement. Retired callbacks must no longer return metadata.
        let glyph = project
            .authoring
            .lock()
            .expect("authoring")
            .placed
            .iter()
            .find(|p| p.literal.is_some())
            .map(|p| p.object.clone())
            .expect("fixture glyph");
        j.void(&glyph, "dispose", "()V", &[])?;
        refresh_placed(project, j, &editor, "placement.rs", &catalog)?;
        ensure!(before != identities(project), "Invalid anchor was retained");
        for id in before {
            ensure!(
                jvm::invoke(j, id, "ValueGlyph.calcWidthInPixels", &[])?.is_null(),
                "Retired callback remained active"
            );
        }
        let before = identities(project);
        let name = source.find("Card").expect("fixture name");
        let offset = source[..name].encode_utf16().count();
        write(project, j, &document, offset, offset + 4, "Tile")?;
        source.replace_range(name..name + 4, "Tile");
        catalog = Catalog::parse(&source)?;
        refresh_placed(project, j, &editor, "placement.rs", &catalog)?;
        ensure!(
            before != identities(project),
            "Structural edit retained old preview action"
        );
        let gutter = project
            .authoring
            .lock()
            .expect("authoring")
            .placed
            .iter()
            .find(|p| p.literal.is_none())
            .map(|p| p.object.clone())
            .expect("fixture gutter");
        let renderer = j.obj(
            &gutter,
            "getGutterIconRenderer",
            "()Lcom/intellij/openapi/editor/markup/GutterIconRenderer;",
            &[],
        )?;
        ensure!(
            j.text(&renderer, "getTooltipText")?.contains("Tile"),
            "Preview action kept old function"
        );
        remember(project, j, &document, &source, &catalog)?;
        let retained = identities(project);
        refresh_placed(project, j, &editor, "placement.rs", &catalog)?;
        ensure!(
            retained == identities(project),
            "Settled source recreated anchors"
        );
        clear_placed(project, j)?;
        for id in retained {
            ensure!(
                jvm::invoke(j, id, "ValueGlyph.calcWidthInPixels", &[])?.is_null(),
                "Retained callback survived cleanup"
            );
        }
        Ok(())
    })();
    if result.is_err() && j.env.exception_check()? {
        j.env.exception_describe()?;
        j.env.exception_clear()?;
    }
    let cleanup = detach(project, j);
    let released = j.void(
        factory,
        "releaseEditor",
        "(Lcom/intellij/openapi/editor/Editor;)V",
        &[A::O(&editor)],
    );
    result?;
    cleanup?;
    released
}

fn identities(project: &Project) -> Vec<i64> {
    project
        .authoring
        .lock()
        .expect("authoring")
        .placed
        .iter()
        .map(|p| p.callback)
        .collect()
}

fn remember(
    project: &Project,
    j: &mut J<'_>,
    document: &O,
    source: &str,
    catalog: &Catalog,
) -> Result<()> {
    project.authoring.lock().expect("authoring").current = Some(Parsed {
        path: "placement.rs".into(),
        stamp: j.long(document, "getModificationStamp")?,
        source: source.into(),
        catalog: Some(catalog.clone()),
    });
    Ok(())
}

fn write(
    project: &Project,
    j: &mut J<'_>,
    document: &O,
    start: usize,
    end: usize,
    text: &str,
) -> Result<()> {
    let document = document.clone();
    let text = text.to_owned();
    let id = jvm::register(move |j, _, _| {
        j.void(
            &document,
            "replaceString",
            "(IILjava/lang/CharSequence;)V",
            &[A::I(start as i32), A::I(end as i32), A::S(&text)],
        )?;
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    let result = j.static_void(
        "com/intellij/openapi/command/WriteCommandAction",
        "runWriteCommandAction",
        "(Lcom/intellij/openapi/project/Project;Ljava/lang/Runnable;)V",
        &[A::O(&project.object), A::O(&callback)],
    );
    jvm::unregister(id);
    result
}
