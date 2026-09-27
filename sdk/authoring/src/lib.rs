//! Shared source contracts for editor decorations and private preview builds.
//!
//! A literal is eligible only when replacing it with a runtime expression is
//! valid. Types, patterns, constants, macros, borrows, state initializers and
//! nested items are deliberately left to the compiler.
use anyhow::{Result, ensure};
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use source::SourceIndex;
use syn::{
    spanned::Spanned,
    visit_mut::{self, VisitMut},
};

pub mod runtime;
mod source;
pub mod transport;
pub const RUNTIME_SOURCE: &str = include_str!("runtime.rs");
pub const TRANSPORT_SOURCE: &str = include_str!("transport.rs");

pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_LITERALS: usize = 4096;
pub const MAX_STRING_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start: usize,
    pub end: usize,
    pub start_utf16: usize,
    pub end_utf16: usize,
    pub line: usize,
    pub column: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub range: Range,
    pub preview: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub name: String,
    pub range: Range,
    pub end: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Literal {
    pub id: usize,
    pub kind: String,
    pub value: String,
    pub suffix: String,
    pub range: Range,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: String,
    pub functions: Vec<Function>,
    pub calls: Vec<Call>,
    pub literals: Vec<Literal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<Format>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Format {
    pub literal: usize,
    pub end: usize,
    pub prefix: String,
    pub trailing_comma: bool,
}
impl Catalog {
    pub fn parse(source: &str) -> Result<Self> {
        ensure!(
            source.len() <= MAX_SOURCE_BYTES,
            "Source exceeds live editing budget"
        );
        let mut file = syn::parse_file(source)?;
        let mut walk = Walk {
            source: SourceIndex::new(source),
            active: false,
            runtime: false,
            functions: vec![],
            calls: vec![],
            literals: vec![],
            formats: vec![],
        };
        walk.visit_file_mut(&mut file);
        ensure!(
            walk.literals.len() <= MAX_LITERALS,
            "Too many live literals"
        );
        let mut digest = Sha256::new();
        digest.update(file.into_token_stream().to_string().as_bytes());
        // Call-site line numbers are embedded in the private binary. Moving
        // them requires compilation so source navigation cannot silently drift.
        for line in walk
            .calls
            .iter()
            .map(|c| c.range.line)
            .chain(walk.functions.iter().map(|f| f.range.line))
        {
            digest.update((line as u64).to_le_bytes());
        }
        let schema = format!("{:x}", digest.finalize());
        Ok(Self {
            schema,
            functions: walk.functions,
            calls: walk.calls,
            literals: walk.literals,
            formats: walk.formats,
        })
    }

    /// Apply a control edit to exactly the literal the user opened. A stale
    /// document never silently overwrites another expression.
    pub fn replacement(&self, source: &str, id: usize, value: &str) -> Result<String> {
        let current = Self::parse(source)?;
        ensure!(current == *self, "Source changed; reopen the value control");
        let literal = self
            .literals
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown live literal"))?;
        let token = match literal.kind.as_str() {
            "string" => {
                ensure!(
                    value.len() <= MAX_STRING_BYTES,
                    "String exceeds live editing budget"
                );
                syn::LitStr::new(value, proc_macro2::Span::call_site())
                    .to_token_stream()
                    .to_string()
            }
            "bool" => {
                ensure!(matches!(value, "true" | "false"), "Expected true or false");
                value.into()
            }
            "char" => {
                let mut chars = value.chars();
                let c = chars
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("Expected one character"))?;
                ensure!(chars.next().is_none(), "Expected one character");
                syn::LitChar::new(c, proc_macro2::Span::call_site())
                    .to_token_stream()
                    .to_string()
            }
            "int" | "float" => format!("{value}{}", literal.suffix),
            "color" => {
                let channels = runtime::color_channels(value).ok_or_else(|| {
                    anyhow::anyhow!("Expected four RGBA channels between 0 and 1")
                })?;
                let mut text = source[literal.range.start..literal.range.end].to_owned();
                let index = SourceIndex::new(&text);
                let original: syn::ExprCall = syn::parse_str(&text)?;
                for ((argument, value), suffix) in original
                    .args
                    .iter()
                    .zip(channels)
                    .zip(literal.suffix.split(',').collect::<Vec<_>>())
                    .rev()
                {
                    let range = index.range(argument.span());
                    text.replace_range(range.start..range.end, &format!("{value:?}{suffix}"));
                }
                text
            }
            _ => anyhow::bail!("Unsupported literal"),
        };
        let mut result = source.to_owned();
        result.replace_range(literal.range.start..literal.range.end, &token);
        let candidate = Self::parse(&result)?;
        ensure!(
            candidate.schema == self.schema,
            "Value would change the expression or its type"
        );
        Ok(result)
    }

    /// Inject only into the plugin's copied source; no application opt-in,
    /// manifest edit or release-build feature is required.
    pub fn instrument(&self, source: &str, file_key: &str) -> String {
        let mut insertions: Vec<(usize, u8, String)> = vec![];
        let mut replacements = vec![];
        for literal in &self.literals {
            if literal.kind == "color" {
                let text = &source[literal.range.start..literal.range.end];
                if let Some((constructor, arguments)) = text.split_once('(') {
                    let arguments = arguments.trim_end().trim_end_matches(')');
                    replacements.push((literal.range.start, literal.range.end, 2, format!("{{ let [__r, __g, __b, __a] = crate::__cranpose_dev::literal({file_key:?}, {:?}, {}, [{arguments}]); {constructor}(__r, __g, __b, __a) }}", self.schema, literal.id)));
                }
                continue;
            }
            if let Some(format) = self.formats.iter().find(|f| f.literal == literal.id) {
                let (parts, fields) =
                    runtime::format_parts(&literal.value).expect("catalog format");
                let mut template = String::new();
                let mut arguments = String::new();
                for (index, _) in parts.iter().enumerate() {
                    let name = format!("{}{index}", format.prefix);
                    template.push_str(&format!("{{{name}}}"));
                    if let Some(field) = fields.get(index) {
                        template.push_str(field);
                    }
                    arguments.push_str(&format!(", {name} = crate::__cranpose_dev::values::format_text(crate::__cranpose_dev::literal({file_key:?}, {:?}, {}, {:?}), {index})", self.schema, literal.id, literal.value));
                }
                // Preserve source line numbers even for multiline raw literals.
                replacements.push((
                    literal.range.start,
                    literal.range.end,
                    2,
                    format!(
                        "{template:?}{}",
                        "\n".repeat(
                            source[literal.range.start..literal.range.end]
                                .bytes()
                                .filter(|b| *b == b'\n')
                                .count()
                        )
                    ),
                ));
                if format.trailing_comma {
                    arguments.remove(0);
                }
                insertions.push((format.end, 0, arguments));
                continue;
            }
            insertions.push((
                literal.range.start,
                2,
                format!(
                    "crate::__cranpose_dev::literal({file_key:?}, {:?}, {}, ",
                    self.schema, literal.id
                ),
            ));
            insertions.push((literal.range.end, 0, ")".into()));
        }
        for call in &self.calls {
            insertions.push((call.range.start, 1, format!("{{ let _cranpose_origin = crate::__cranpose_api::__source_scope({:?}, file!(), {}u32, env!(\"CARGO_MANIFEST_DIR\")); ", format!("__cranpose_call:{}", call.name), call.range.line)));
            insertions.push((call.end, 0, " }".into()));
        }
        // Descending insertion preserves the original line numbers and source.
        replacements.extend(
            insertions
                .into_iter()
                .map(|(start, rank, text)| (start, start, rank, text)),
        );
        replacements.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.cmp(&a.2)));
        let mut output = source.to_owned();
        for (start, end, _, text) in replacements {
            output.replace_range(start..end, &text);
        }
        output
    }
}

struct Walk {
    source: SourceIndex,
    active: bool,
    runtime: bool,
    functions: Vec<Function>,
    calls: Vec<Call>,
    literals: Vec<Literal>,
    formats: Vec<Format>,
}
fn attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs
        .iter()
        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == name))
}
impl Walk {
    fn literal(&mut self, expr: &syn::Expr) -> Option<Literal> {
        let (literal, negative) = match expr {
            syn::Expr::Lit(l) => (&l.lit, false),
            syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => match &*u.expr {
                syn::Expr::Lit(l) => (&l.lit, true),
                _ => return None,
            },
            _ => return None,
        };
        let (kind, mut value, suffix) = match literal {
            syn::Lit::Str(l) if !negative && l.value().len() <= MAX_STRING_BYTES => {
                ("string", l.value(), "".into())
            }
            syn::Lit::Bool(l) if !negative => ("bool", l.value.to_string(), "".into()),
            syn::Lit::Char(l) if !negative => ("char", l.value().to_string(), "".into()),
            syn::Lit::Int(l) => ("int", l.base10_digits().into(), l.suffix().into()),
            syn::Lit::Float(l) => ("float", l.base10_digits().into(), l.suffix().into()),
            _ => return None,
        };
        if negative {
            value.insert(0, '-');
        }
        Some(Literal {
            id: self.literals.len(),
            kind: kind.into(),
            value,
            suffix,
            range: self.source.range(expr.span()),
        })
    }
}
impl VisitMut for Walk {
    fn visit_item_fn_mut(&mut self, function: &mut syn::ItemFn) {
        let previous = self.active;
        let previous_runtime = self.runtime;
        self.runtime = function.sig.constness.is_none();
        self.active = attr(&function.attrs, "composable") && function.sig.constness.is_none();
        if self.active {
            self.functions.push(Function {
                name: function.sig.ident.to_string(),
                range: self.source.range(function.sig.ident.span()),
                preview: function.sig.inputs.is_empty() && function.sig.generics.params.is_empty(),
            });
        }
        if self.runtime {
            self.visit_block_mut(&mut function.block);
        }
        self.active = previous;
        self.runtime = previous_runtime;
    }
    fn visit_item_const_mut(&mut self, _: &mut syn::ItemConst) {}
    fn visit_item_static_mut(&mut self, _: &mut syn::ItemStatic) {}
    fn visit_type_mut(&mut self, _: &mut syn::Type) {}
    fn visit_pat_mut(&mut self, _: &mut syn::Pat) {}
    fn visit_attribute_mut(&mut self, _: &mut syn::Attribute) {}
    fn visit_generic_argument_mut(&mut self, _: &mut syn::GenericArgument) {}
    fn visit_expr_const_mut(&mut self, _: &mut syn::ExprConst) {}
    fn visit_expr_reference_mut(&mut self, _: &mut syn::ExprReference) {}
    fn visit_expr_repeat_mut(&mut self, repeat: &mut syn::ExprRepeat) {
        self.visit_expr_mut(&mut repeat.expr);
    }
    fn visit_expr_call_mut(&mut self, call: &mut syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func
            && let Some(segment) = path.path.segments.last()
        {
            let name = segment.ident.to_string();
            // Values evaluated only during initialisation cannot be tuned live.
            if name.starts_with("remember") {
                return;
            }
            if name == "key" {
                // Identity keys must remain stable, but their content is normal
                // composition and can contain tunable values and source calls.
                if let Some(content) = call.args.last_mut() {
                    self.visit_expr_mut(content);
                }
                return;
            }
            if self.active
                && name.chars().next().is_some_and(char::is_uppercase)
                && !["Some", "Ok", "Err", "Color", "Size", "Offset", "Rect"]
                    .contains(&name.as_str())
            {
                self.calls.push(Call {
                    name,
                    range: self.source.range(call.func.span()),
                    end: self.source.offsets(call.span().end()).0,
                });
            }
        }
        visit_mut::visit_expr_call_mut(self, call);
    }
    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        if self.runtime
            && let syn::Expr::Call(call) = expr
            && let syn::Expr::Path(path) = &*call.func
            && (path
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "Color")
                || (path.path.segments.last().is_some_and(|s| s.ident == "rgba")
                    && path
                        .path
                        .segments
                        .iter()
                        .rev()
                        .nth(1)
                        .is_some_and(|s| s.ident == "Color")))
            && call.args.len() == 4
        {
            let values = call
                .args
                .iter()
                .map(|e| self.literal(e))
                .collect::<Option<Vec<_>>>();
            if let Some(values) = values
                && values.iter().all(|v| v.kind == "float")
            {
                let value = values
                    .iter()
                    .map(|v| v.value.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                if runtime::color_channels(&value).is_some() {
                    let id = self.literals.len();
                    let suffix = values
                        .iter()
                        .map(|v| v.suffix.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    let constructor = path.to_token_stream().to_string();
                    self.literals.push(Literal {
                        id,
                        kind: "color".into(),
                        value,
                        suffix: suffix.clone(),
                        range: self.source.range(expr.span()),
                    });
                    *expr = syn::parse_quote!(__cranpose_color_slot!(#id, #constructor, #suffix));
                    return;
                }
            }
        }
        if self.active
            && let Some(literal) = self.literal(expr)
        {
            let id = literal.id;
            let kind = &literal.kind;
            let suffix = &literal.suffix;
            // Retain kind and suffix in the schema: a changed type never updates
            // the running process through a value-only message.
            *expr = syn::parse_quote!(__cranpose_live_slot!(#id, #kind, #suffix));
            self.literals.push(literal);
        } else {
            visit_mut::visit_expr_mut(self, expr);
        }
    }
    fn visit_expr_macro_mut(&mut self, expr: &mut syn::ExprMacro) {
        use syn::{Token, parse::Parser, punctuated::Punctuated};
        let path = expr.mac.path.to_token_stream().to_string().replace(' ', "");
        if !self.active || !matches!(path.as_str(), "format" | "std::format" | "alloc::format") {
            return;
        }
        let parser = Punctuated::<syn::Expr, Token![,]>::parse_terminated;
        let Ok(mut args) = parser.parse2(expr.mac.tokens.clone()) else {
            return;
        };
        let trailing_comma = args.trailing_punct();
        let Some(syn::Expr::Lit(first)) = args.first_mut() else {
            return;
        };
        let syn::Lit::Str(text) = &first.lit else {
            return;
        };
        let Some((_, fields)) = runtime::format_parts(&text.value()) else {
            return;
        };
        if text.value().len() > MAX_STRING_BYTES || fields.len() > 64 {
            return;
        }
        let id = self.literals.len();
        self.literals.push(Literal {
            id,
            kind: "string".into(),
            value: text.value(),
            suffix: String::new(),
            range: self.source.range(text.span()),
        });
        let mut prefix = format!("__cranpose_text_{id}_");
        let tokens = expr.mac.tokens.to_string();
        while tokens.contains(&prefix) {
            prefix.push('_');
        }
        self.formats.push(Format {
            literal: id,
            end: self
                .source
                .offsets(expr.mac.delimiter.span().close().start())
                .0,
            prefix,
            trailing_comma,
        });
        first.lit = syn::Lit::Str(syn::LitStr::new(&format!("{id}:{fields:?}"), text.span()));
        for argument in args.iter_mut().skip(1) {
            self.visit_expr_mut(argument);
        }
        expr.mac.tokens = args.to_token_stream();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_lines_and_identity_changes_rebuild_but_key_content_is_live() {
        let source = "#[composable] fn App() { key(42, || Text(\"a\")); }";
        let catalog = Catalog::parse(source).expect("catalog");
        assert!(catalog.functions[0].preview);
        assert_eq!(catalog.literals.len(), 1);
        assert_eq!(catalog.literals[0].value, "a");
        assert_eq!(
            catalog.schema,
            Catalog::parse(&source.replace("\"a\"", "\"longer\""))
                .expect("changed literal")
                .schema
        );
        assert_ne!(
            catalog.schema,
            Catalog::parse(&source.replace("42", "43"))
                .expect("changed key")
                .schema
        );
        assert_ne!(
            catalog.schema,
            Catalog::parse(&source.replace("Text(", "\nText("))
                .expect("moved call")
                .schema
        );
        assert!(
            !Catalog::parse("#[composable] fn Item(value: i32) { Text(value); }")
                .expect("parameterized composable")
                .functions[0]
                .preview
        );
    }
    #[test]
    fn values_share_schema_but_structure_and_types_do_not() {
        let old = "#[composable] fn App() { Text(\"hello\", 12.0f32, -4i32, true); }";
        let catalog = Catalog::parse(old).expect("catalog");
        assert_eq!(catalog.literals.len(), 4);
        let edited = catalog.replacement(old, 0, "🙂 new\ntext").expect("edit");
        assert_eq!(
            catalog.schema,
            Catalog::parse(&edited).expect("edited").schema
        );
        assert_ne!(
            catalog.schema,
            Catalog::parse(&old.replace("f32", "f64"))
                .expect("type")
                .schema
        );
        assert_ne!(
            catalog.schema,
            Catalog::parse(&old.replace("Text(", "Button("))
                .expect("call")
                .schema
        );
        assert!(catalog.replacement(&edited, 0, "stale").is_err());
        assert!(
            catalog
                .replacement(old, 1, "1.0); panic!(\"oops\"); Text(0.0")
                .is_err()
        );
    }
    #[test]
    fn excludes_constants_patterns_macros_borrows_and_initializers() {
        let source = r#"#[composable] fn App() {
            const C: usize = 4; let a: [u8; 2] = [1; 2]; let s = rememberMutableStateOf(|| 0);
            let promoted: &'static [u8] = &[7,8]; let x = format!("{}", 1);
            match 3 { 1 => Text("yes"), _ => Text("no") }
        } fn helper() { Text("outside"); }"#;
        let catalog = Catalog::parse(source).expect("catalog");
        assert_eq!(
            catalog
                .literals
                .iter()
                .map(|v| v.value.as_str())
                .collect::<Vec<_>>(),
            ["1", "{}", "1", "3", "yes", "no"]
        );
        let instrumented = catalog.instrument(source, "src/main.rs");
        syn::parse_file(&instrumented).expect("instrumented Rust");
        assert_eq!(source.lines().count(), instrumented.lines().count());
    }
    #[test]
    fn unicode_ranges_and_nested_call_wrappers_preserve_syntax() {
        let source = "// 🙂\n#[composable] fn App() { Column(|| Text(\"é🦀\", Modifier::empty().padding(-2.5))); }";
        let catalog = Catalog::parse(source).expect("catalog");
        let first = &catalog.literals[0];
        assert_eq!(&source[first.range.start..first.range.end], "\"é🦀\"");
        assert_eq!(
            first.range.start_utf16,
            source[..first.range.start].encode_utf16().count()
        );
        assert_eq!(catalog.calls.len(), 2);
        syn::parse_file(&catalog.instrument(source, "src/é.rs")).expect("wrappers");
        assert!(Catalog::parse("#[composable] fn App() { Text(\"unfinished").is_err());
    }
}
