use crate::{
    glyphs,
    jvm::{self, A, J, O},
};
use anyhow::Result;
use cranpose_plugin_authoring::{Catalog, Function};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Definition {
    file: String,
    name: String,
    line: usize,
}

#[derive(Deserialize)]
pub(crate) struct Row {
    #[serde(flatten)]
    definition: Definition,
    recompositions: u64,
    instances: u64,
}

impl Row {
    fn label(&self) -> String {
        let unit = if self.recompositions == 1 {
            "recomposition"
        } else {
            "recompositions"
        };
        let mut label = format!("Preview · {} {unit}", self.recompositions);
        if self.instances > 1 {
            label.push_str(&format!(" · {} instances", self.instances));
        }
        label
    }
}

struct Placed {
    id: u64,
    gpu: Arc<AtomicBool>,
    inlay: O,
    label: Arc<Mutex<String>>,
    _scope: jvm::Scope,
}

impl Placed {
    fn dispose(&self, j: &mut J<'_>) -> Result<()> {
        j.void(&self.inlay, "dispose", "()V", &[])
    }
}

struct Editor {
    object: O,
    stamp: Option<i64>,
    functions: Vec<Function>,
    placed: BTreeMap<Definition, Placed>,
}

impl Editor {
    fn clear(&mut self, j: &mut J<'_>) -> Result<()> {
        while let Some((_, placed)) = self.placed.first_key_value() {
            placed.dispose(j)?;
            self.placed.pop_first();
        }
        Ok(())
    }

    fn update(&mut self, j: &mut J<'_>, document: &O, path: &str, rows: &[Row]) -> Result<()> {
        let stamp = j.long(document, "getModificationStamp")?;
        if self.stamp != Some(stamp) {
            self.functions = if j.int(document, "getTextLength")? as usize
                <= cranpose_plugin_authoring::MAX_SOURCE_BYTES
            {
                Catalog::parse(&j.text(document, "getText")?)
                    .map(|catalog| catalog.functions)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            self.stamp = Some(stamp);
        }
        let model = j.obj(
            &self.object,
            "getInlayModel",
            "()Lcom/intellij/openapi/editor/InlayModel;",
            &[],
        )?;
        let mut current = std::collections::BTreeSet::new();
        for row in rows
            .iter()
            .filter(|row| row.definition.file == path && row.instances > 0)
        {
            let key = &row.definition;
            let previous = self.placed.get(key);
            let anchor = match previous {
                Some(placed) if j.bool(&placed.inlay, "isValid")? => {
                    Some(j.int(&placed.inlay, "getOffset")? as usize)
                }
                _ => None,
            };
            let function = definition(&self.functions, key, anchor);
            let Some(function) = function else {
                if let Some(placed) = previous {
                    placed.dispose(j)?;
                    self.placed.remove(key);
                }
                continue;
            };
            let offset = function.range.start_utf16 as i32;
            let label = row.label();
            current.insert(key);
            if let Some(placed) = previous {
                if anchor == Some(offset as usize) {
                    let changed = {
                        let mut current = placed.label.lock().expect("counter label");
                        if *current == label {
                            false
                        } else {
                            *current = label;
                            true
                        }
                    };
                    if changed {
                        j.void(&placed.inlay, "update", "()V", &[])?;
                    }
                    continue;
                }
                placed.dispose(j)?;
                self.placed.remove(key);
            }
            if let Some(placed) = place(j, &model, offset, label)? {
                self.placed.insert(key.clone(), placed);
            }
        }
        let obsolete: Vec<_> = self
            .placed
            .keys()
            .filter(|key| !current.contains(key))
            .cloned()
            .collect();
        for key in obsolete {
            self.placed[&key].dispose(j)?;
            self.placed.remove(&key);
        }
        Ok(())
    }
}

fn definition<'a>(
    functions: &'a [Function],
    key: &Definition,
    anchor: Option<usize>,
) -> Option<&'a Function> {
    let mut named = functions
        .iter()
        .filter(|function| function.name == key.name);
    let first = named.next()?;
    if named.next().is_none() {
        return Some(first);
    }
    functions.iter().find(|function| {
        function.name == key.name
            && match anchor {
                Some(offset) => function.range.start_utf16 == offset,
                None => function.range.line == key.line,
            }
    })
}

fn place(j: &mut J<'_>, model: &O, offset: i32, text: String) -> Result<Option<Placed>> {
    let label = Arc::new(Mutex::new(text));
    let captured = label.clone();
    let gpu = Arc::new(AtomicBool::new(false));
    let gpu_paint = gpu.clone();
    let scope = jvm::Scope::default();
    let id = scope.register(move |j, op, args| match op {
        "CounterBadge.calcWidthInPixels" => {
            let width = glyphs::badge_width(j, &args[0], &captured.lock().expect("counter label"))?;
            j.boxed_int(width)
        }
        "CounterBadge.calcHeightInPixels" => {
            let editor = j.obj(
                &args[0],
                "getEditor",
                "()Lcom/intellij/openapi/editor/Editor;",
                &[],
            )?;
            let height = j.int(&editor, "getLineHeight")?;
            j.boxed_int(height)
        }
        "CounterBadge.paint" => {
            if !gpu_paint.load(Ordering::Acquire) {
                glyphs::badge(j, args, &captured.lock().expect("counter label"), "muted")?;
            }
            j.null()
        }
        _ => j.null(),
    });
    let renderer = j.new("dev/cranpose/rust/CounterBadge", "(J)V", &[A::J(id)])?;
    let inlay = j.obj(model, "addBlockElement", "(IZZILcom/intellij/openapi/editor/EditorCustomElementRenderer;)Lcom/intellij/openapi/editor/Inlay;",
        &[A::I(offset), A::Z(true), A::Z(true), A::I(0), A::O(&renderer)])?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Ok((!inlay.is_null()).then_some(Placed {
        id: NEXT.fetch_add(1, Ordering::Relaxed),
        gpu,
        inlay,
        label,
        _scope: scope,
    }))
}

#[derive(Default)]
pub(crate) struct Inlays {
    editors: Vec<Editor>,
}

impl Inlays {
    pub(crate) fn anchors(&self) -> Vec<crate::decorations::Counter> {
        self.editors
            .iter()
            .flat_map(|editor| {
                editor
                    .placed
                    .values()
                    .map(|placed| crate::decorations::Counter {
                        id: placed.id,
                        editor: editor.object.clone(),
                        inlay: placed.inlay.clone(),
                        label: placed.label.lock().expect("counter label").clone(),
                        gpu: placed.gpu.clone(),
                    })
            })
            .collect()
    }
    pub fn clear(&mut self, j: &mut J<'_>) -> Result<()> {
        for editor in &mut self.editors {
            editor.clear(j)?;
        }
        self.editors.clear();
        Ok(())
    }

    pub fn update(&mut self, j: &mut J<'_>, project: &O, rows: &[Row]) -> Result<()> {
        if rows.is_empty() || j.bool(project, "isDisposed")? {
            return self.clear(j);
        }
        let factory = j.static_obj(
            "com/intellij/openapi/editor/EditorFactory",
            "getInstance",
            "()Lcom/intellij/openapi/editor/EditorFactory;",
            &[],
        )?;
        let all = j.obj(
            &factory,
            "getAllEditors",
            "()[Lcom/intellij/openapi/editor/Editor;",
            &[],
        )?;
        let manager = j.static_obj(
            "com/intellij/openapi/fileEditor/FileDocumentManager",
            "getInstance",
            "()Lcom/intellij/openapi/fileEditor/FileDocumentManager;",
            &[],
        )?;
        let mut current = std::collections::BTreeSet::new();
        for object in j.elements(&all)? {
            if j.bool(&object, "isDisposed")? {
                continue;
            }
            let owner = j.obj(
                &object,
                "getProject",
                "()Lcom/intellij/openapi/project/Project;",
                &[],
            )?;
            if !j.same(&owner, project)? {
                continue;
            }
            let document = j.obj(
                &object,
                "getDocument",
                "()Lcom/intellij/openapi/editor/Document;",
                &[],
            )?;
            let file = j.obj(
                &manager,
                "getFile",
                "(Lcom/intellij/openapi/editor/Document;)Lcom/intellij/openapi/vfs/VirtualFile;",
                &[A::O(&document)],
            )?;
            if file.is_null() {
                continue;
            }
            let path = j.text(&file, "getPath")?;
            if !rows.iter().any(|row| row.definition.file == path) {
                continue;
            }
            let mut found = None;
            for (index, editor) in self.editors.iter().enumerate() {
                if j.same(&object, &editor.object)? {
                    found = Some(index);
                    break;
                }
            }
            let index = found.unwrap_or_else(|| {
                let index = self.editors.len();
                self.editors.push(Editor {
                    object,
                    stamp: None,
                    functions: Vec::new(),
                    placed: BTreeMap::new(),
                });
                index
            });
            current.insert(index);
            self.editors[index].update(j, &document, &path, rows)?;
        }
        for index in (0..self.editors.len()).rev() {
            if !current.contains(&index) {
                self.editors[index].clear(j)?;
                self.editors.remove(index);
            }
        }
        Ok(())
    }
}

#[cfg(feature = "ide-tests")]
#[path = "tests/recompositions.rs"]
mod tests;

#[cfg(feature = "ide-tests")]
pub(crate) use tests::integration_test;
