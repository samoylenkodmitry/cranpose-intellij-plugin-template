//! Composable navigation, snippets and transparent Cranpose editor overlays.
use crate::{
    jvm::{self, A, J, O},
    project::Project,
    source,
    surface::{Panel, Surface},
    workspace,
};
use anyhow::Result;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
#[derive(Default)]
pub struct State {
    callbacks: BTreeMap<String, i64>,
    overlays: Vec<Overlay>,
}
struct Overlay {
    surface: Weak<Surface>,
    panel: Weak<Panel>,
    editor: Option<O>,
    last_length: i32,
}
pub fn icon(j: &mut J<'_>) -> Result<O> {
    let class = j.class("dev/cranpose/rust/LineMarker")?;
    j.static_obj(
        "com/intellij/openapi/util/IconLoader",
        "getIcon",
        "(Ljava/lang/String;Ljava/lang/Class;)Ljavax/swing/Icon;",
        &[A::S("/icons/cranpose.svg"), A::O(&class)],
    )
}
pub fn line_marker(j: &mut J<'_>, element: &O) -> Result<O> {
    let child = j.obj(
        element,
        "getFirstChild",
        "()Lcom/intellij/psi/PsiElement;",
        &[],
    )?;
    if !child.is_null() {
        return j.null();
    }
    let file = j.obj(
        element,
        "getContainingFile",
        "()Lcom/intellij/psi/PsiFile;",
        &[],
    )?;
    if file.is_null() {
        return j.null();
    }
    let virtual_file = j.obj(
        &file,
        "getVirtualFile",
        "()Lcom/intellij/openapi/vfs/VirtualFile;",
        &[],
    )?;
    if virtual_file.is_null() {
        return j.null();
    }
    let path = j.text(&virtual_file, "getPath")?;
    if !path.ends_with(".rs") {
        return j.null();
    }
    let range = j.obj(
        element,
        "getTextRange",
        "()Lcom/intellij/openapi/util/TextRange;",
        &[],
    )?;
    let offset = j.int(&range, "getStartOffset")? as usize;
    let text = j.text(&file, "getText")?;
    let Some(symbol) = source::symbols(&text)
        .into_iter()
        .find(|s| s.offset == offset)
    else {
        return j.null();
    };
    let object = j.obj(
        element,
        "getProject",
        "()Lcom/intellij/openapi/project/Project;",
        &[],
    )?;
    let project = Project::get(j, &object)?;
    let key = format!("gutter:{path}:{}", symbol.name);
    let existing = project
        .editor
        .lock()
        .expect("editor")
        .callbacks
        .get(&key)
        .copied();
    let id = if let Some(id) = existing {
        id
    } else {
        let weak = Arc::downgrade(&project);
        let path = path.clone();
        let name = symbol.name.clone();
        let id = project.scope.register(move |j, op, _| match op {
            "Callback.fun" => j.string(&format!("Preview {name} beside source")),
            "Callback.get" => j.string("Cranpose composable"),
            "Callback.navigate" => {
                if let Some(project) = weak.upgrade() {
                    workspace::show(&project, j, &path, Some(&name), None)?;
                }
                j.null()
            }
            _ => j.null(),
        });
        project
            .editor
            .lock()
            .expect("editor")
            .callbacks
            .insert(key, id);
        id
    };
    let callback = jvm::callback(j, id)?;
    let icon = icon(j)?;
    let alignment = j.constant(
        "com/intellij/openapi/editor/markup/GutterIconRenderer$Alignment",
        "LEFT",
        "Lcom/intellij/openapi/editor/markup/GutterIconRenderer$Alignment;",
    )?;
    j.new("com/intellij/codeInsight/daemon/LineMarkerInfo","(Lcom/intellij/psi/PsiElement;Lcom/intellij/openapi/util/TextRange;Ljavax/swing/Icon;Lcom/intellij/util/Function;Lcom/intellij/codeInsight/daemon/GutterIconNavigationHandler;Lcom/intellij/openapi/editor/markup/GutterIconRenderer$Alignment;Ljava/util/function/Supplier;)V",&[A::O(element),A::O(&range),A::O(&icon),A::O(&callback),A::O(&callback),A::O(&alignment),A::O(&callback)])
}
pub fn complete(j: &mut J<'_>, parameters: &O, result: &O) -> Result<()> {
    let file = j.obj(
        parameters,
        "getOriginalFile",
        "()Lcom/intellij/psi/PsiFile;",
        &[],
    )?;
    let virtual_file = j.obj(
        &file,
        "getVirtualFile",
        "()Lcom/intellij/openapi/vfs/VirtualFile;",
        &[],
    )?;
    if virtual_file.is_null() || !j.text(&virtual_file, "getPath")?.ends_with(".rs") {
        return Ok(());
    }
    let text = j.text(&file, "getText")?;
    let offset = j.int(parameters, "getOffset")?.max(0) as usize;
    let byte = text
        .char_indices()
        .scan(0, |count, (byte, c)| {
            let before = *count;
            *count += c.len_utf16();
            Some((byte, before))
        })
        .find(|(_, n)| *n >= offset)
        .map(|(b, _)| b)
        .unwrap_or(text.len());
    let masked = source::mask(&text);
    if byte > 0 && masked.as_bytes()[byte - 1] == b' ' && text.as_bytes()[byte - 1] != b' ' {
        return Ok(());
    }
    let object = j.obj(
        &file,
        "getProject",
        "()Lcom/intellij/openapi/project/Project;",
        &[],
    )?;
    let project = Project::get(j, &object)?;
    let icon = icon(j)?;
    for &(key, title, code) in source::SNIPPETS {
        let existing = project
            .editor
            .lock()
            .expect("editor")
            .callbacks
            .get(key)
            .copied();
        let id = if let Some(id) = existing {
            id
        } else {
            let id = project.scope.register(move |j, op, args| {
                if op == "Callback.handleInsert" {
                    let context = &args[0];
                    let document = j.obj(
                        context,
                        "getDocument",
                        "()Lcom/intellij/openapi/editor/Document;",
                        &[],
                    )?;
                    let start = j.int(context, "getStartOffset")?;
                    let end = j.int(context, "getTailOffset")?;
                    j.void(
                        &document,
                        "replaceString",
                        "(IILjava/lang/CharSequence;)V",
                        &[A::I(start), A::I(end), A::S(code)],
                    )?;
                    let editor = j.obj(
                        context,
                        "getEditor",
                        "()Lcom/intellij/openapi/editor/Editor;",
                        &[],
                    )?;
                    let caret = j.obj(
                        &editor,
                        "getCaretModel",
                        "()Lcom/intellij/openapi/editor/CaretModel;",
                        &[],
                    )?;
                    j.void(
                        &caret,
                        "moveToOffset",
                        "(I)V",
                        &[A::I(start + code.encode_utf16().count() as i32)],
                    )?;
                }
                j.null()
            });
            project
                .editor
                .lock()
                .expect("editor")
                .callbacks
                .insert(key.into(), id);
            id
        };
        let callback = jvm::callback(j, id)?;
        let builder = j.static_obj(
            "com/intellij/codeInsight/lookup/LookupElementBuilder",
            "create",
            "(Ljava/lang/String;)Lcom/intellij/codeInsight/lookup/LookupElementBuilder;",
            &[A::S(key)],
        )?;
        let builder = j.obj(
            &builder,
            "withTypeText",
            "(Ljava/lang/String;)Lcom/intellij/codeInsight/lookup/LookupElementBuilder;",
            &[A::S("Cranpose")],
        )?;
        let builder = j.obj(
            &builder,
            "withTailText",
            "(Ljava/lang/String;)Lcom/intellij/codeInsight/lookup/LookupElementBuilder;",
            &[A::S(&format!("  {title}"))],
        )?;
        let builder = j.obj(
            &builder,
            "withIcon",
            "(Ljavax/swing/Icon;)Lcom/intellij/codeInsight/lookup/LookupElementBuilder;",
            &[A::O(&icon)],
        )?;
        let builder=j.obj(&builder,"withInsertHandler","(Lcom/intellij/codeInsight/completion/InsertHandler;)Lcom/intellij/codeInsight/lookup/LookupElementBuilder;",&[A::O(&callback)])?;
        j.void(
            result,
            "addElement",
            "(Lcom/intellij/codeInsight/lookup/LookupElement;)V",
            &[A::O(&builder)],
        )?;
    }
    Ok(())
}
pub fn attach_overlay(
    project: &Arc<Project>,
    j: &mut J<'_>,
    panel: &Arc<Panel>,
    payload: &str,
) -> Result<()> {
    let value: Value = serde_json::from_str(payload)?;
    if value["anchor"] != "editor" {
        return Ok(());
    }
    let Some(surface) = value["surface"]
        .as_u64()
        .and_then(|id| panel.overlay_surface(id as u32))
    else {
        return Ok(());
    };
    project
        .editor
        .lock()
        .expect("editor")
        .overlays
        .push(Overlay {
            surface: Arc::downgrade(&surface),
            panel: Arc::downgrade(panel),
            editor: None,
            last_length: 0,
        });
    fit_overlays(project, j)
}
pub fn fit_overlays(project: &Project, j: &mut J<'_>) -> Result<()> {
    let (_, selected) = project.selected(j)?;
    let overlays = std::mem::take(&mut project.editor.lock().expect("editor").overlays);
    let mut retained = vec![];
    for mut overlay in overlays {
        let Some(surface) = overlay.surface.upgrade() else {
            continue;
        };
        if overlay.panel.strong_count() == 0 {
            continue;
        }
        if let Some(editor) = &selected {
            let content = j.obj(
                editor,
                "getContentComponent",
                "()Ljavax/swing/JComponent;",
                &[],
            )?;
            let parent = j.obj(
                surface.component(),
                "getParent",
                "()Ljava/awt/Container;",
                &[],
            )?;
            if !j.same(&parent, &content)? {
                if !parent.is_null() {
                    j.void(
                        &parent,
                        "remove",
                        "(Ljava/awt/Component;)V",
                        &[A::O(surface.component())],
                    )?;
                }
                j.obj(
                    &content,
                    "add",
                    "(Ljava/awt/Component;)Ljava/awt/Component;",
                    &[A::O(surface.component())],
                )?;
                let document = j.obj(
                    editor,
                    "getDocument",
                    "()Lcom/intellij/openapi/editor/Document;",
                    &[],
                )?;
                overlay.last_length = j.int(&document, "getTextLength")?;
                overlay.editor = Some(editor.clone());
            }
            let scrolling = j.obj(
                editor,
                "getScrollingModel",
                "()Lcom/intellij/openapi/editor/ScrollingModel;",
                &[],
            )?;
            let area = j.obj(&scrolling, "getVisibleArea", "()Ljava/awt/Rectangle;", &[])?;
            j.void(
                surface.component(),
                "setBounds",
                "(Ljava/awt/Rectangle;)V",
                &[A::O(&area)],
            )?;
        } else {
            let parent = j.obj(
                surface.component(),
                "getParent",
                "()Ljava/awt/Container;",
                &[],
            )?;
            if !parent.is_null() {
                j.void(
                    &parent,
                    "remove",
                    "(Ljava/awt/Component;)V",
                    &[A::O(surface.component())],
                )?;
            }
            overlay.editor = None;
        }
        retained.push(overlay);
    }
    project.editor.lock().expect("editor").overlays = retained;
    Ok(())
}
pub fn caret(project: &Project, j: &mut J<'_>, event: &O) -> Result<()> {
    let editor = j.obj(
        event,
        "getEditor",
        "()Lcom/intellij/openapi/editor/Editor;",
        &[],
    )?;
    let overlays = std::mem::take(&mut project.editor.lock().expect("editor").overlays);
    let mut retained = vec![];
    for mut overlay in overlays {
        let Some(panel) = overlay.panel.upgrade() else {
            continue;
        };
        if overlay.surface.strong_count() == 0 {
            continue;
        }
        if let Some(target) = &overlay.editor
            && j.same(target, &editor)?
        {
            let document = j.obj(
                &editor,
                "getDocument",
                "()Lcom/intellij/openapi/editor/Document;",
                &[],
            )?;
            let length = j.int(&document, "getTextLength")?;
            let kind = if length > overlay.last_length {
                "type"
            } else if length < overlay.last_length {
                "delete"
            } else {
                "move"
            };
            overlay.last_length = length;
            let scrolling = j.obj(
                &editor,
                "getScrollingModel",
                "()Lcom/intellij/openapi/editor/ScrollingModel;",
                &[],
            )?;
            let area = j.obj(&scrolling, "getVisibleArea", "()Ljava/awt/Rectangle;", &[])?;
            let mut positions = vec![];
            for method in ["getNewPosition", "getOldPosition"] {
                let logical = j.obj(
                    event,
                    method,
                    "()Lcom/intellij/openapi/editor/LogicalPosition;",
                    &[],
                )?;
                let point = j.obj(
                    &editor,
                    "logicalPositionToXY",
                    "(Lcom/intellij/openapi/editor/LogicalPosition;)Ljava/awt/Point;",
                    &[A::O(&logical)],
                )?;
                positions.push((
                    j.field_int(&point, "x")? - j.field_int(&area, "x")?,
                    j.field_int(&point, "y")? - j.field_int(&area, "y")?,
                ));
            }
            panel.message("ide.caret",&serde_json::json!({"kind":kind,"x":positions[0].0,"y":positions[0].1,"fromX":positions[1].0,"fromY":positions[1].1,"lineHeight":j.int(&editor,"getLineHeight")?}).to_string());
        }
        retained.push(overlay);
    }
    project.editor.lock().expect("editor").overlays = retained;
    Ok(())
}
