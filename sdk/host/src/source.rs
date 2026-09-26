//! Tolerant source navigation, including partially typed Rust files.
use serde::Serialize;
use std::sync::OnceLock;
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub offset: usize,
    pub line: usize,
}
pub fn symbols(source: &str) -> Vec<Symbol> {
    static DECLARATION: OnceLock<regex::Regex> = OnceLock::new();
    let regex=DECLARATION.get_or_init(||regex::Regex::new(r"#\s*\[\s*(?:[A-Za-z_][A-Za-z_0-9]*\s*::\s*)*composable\s*\](?:\s|#\s*\[[^\]]*\])*?(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+(r#)?([A-Za-z_][A-Za-z_0-9]*)").expect("composable pattern"));
    let masked = mask(source);
    regex
        .captures_iter(&masked)
        .filter_map(|capture| {
            let name = capture.get(2)?;
            let byte = capture.get(1).map(|s| s.start()).unwrap_or(name.start());
            Some(Symbol {
                name: name.as_str().into(),
                offset: source[..byte].encode_utf16().count(),
                line: source[..byte].bytes().filter(|b| *b == b'\n').count() + 1,
            })
        })
        .collect()
}
pub fn mask(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut result = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        let mut hidden = false;
        if bytes[index..].starts_with(b"//") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            hidden = true;
        } else if bytes[index..].starts_with(b"/*") {
            let mut depth = 1;
            index += 2;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            hidden = true;
        } else if bytes[index] == b'r' && (index == 0 || !bytes[index - 1].is_ascii_alphanumeric())
        {
            let mut quote = index + 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            if bytes.get(quote) == Some(&b'"') {
                let ending = format!("\"{}", "#".repeat(quote - index - 1));
                index = source[quote + 1..]
                    .find(&ending)
                    .map(|n| quote + 1 + n + ending.len())
                    .unwrap_or(bytes.len());
                hidden = true;
            } else {
                index += 1;
            }
        } else if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else {
                    let end = bytes[index] == b'"';
                    index += 1;
                    if end {
                        break;
                    }
                }
            }
            hidden = true;
        } else if bytes[index] == b'\''
            && index + 2 < bytes.len()
            && (bytes[index + 2] == b'\'' || bytes[index + 1] == b'\\')
        {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\'' {
                index += 1;
            }
            index = (index + 1).min(bytes.len());
            hidden = true;
        } else {
            index += 1;
        }
        if hidden {
            for byte in &mut result[start..index] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
        }
    }
    String::from_utf8(result).expect("mask preserves UTF-8 outside literals")
}
pub const STARTER: &str = r#"use cranpose::{
    AppLauncher, Button, ButtonSpec, Column, ColumnSpec, Modifier, Text, TextStyle,
    composable, rememberMutableStateOf,
};
#[cranpose::preview(name = "Default", width = 480, height = 640)]
#[composable]
fn App() {
    let count = rememberMutableStateOf(|| 0);
    Column(Modifier::empty().fill_max_size().padding(24.0), ColumnSpec::default(), move || {
        Text(format!("Count: {}", count.get()), Modifier::empty(), TextStyle::default());
        Button(Modifier::empty().padding(12.0), ButtonSpec::default(),
            move || count.set(count.get() + 1), IncrementLabel);
    });
}
#[composable]
fn IncrementLabel() {
    Text("Increment", Modifier::empty(), TextStyle::default());
}
fn main() {
    let app = AppLauncher::new().with_title("Cranpose app").with_size(480, 640);
    app.run(App);
}
"#;
pub const SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "cpcomposable",
        "Composable function",
        "#[cranpose::composable]\nfn MyComponent() {\n    \n}",
    ),
    (
        "cppreview",
        "Component preview",
        "#[cranpose::preview(name = \"Default\", width = 480, height = 640)]\n#[cranpose::composable]\nfn ComponentPreview() {\n    \n}",
    ),
    (
        "cpcolumn",
        "Column layout",
        "cranpose::Column(cranpose::Modifier::empty(), cranpose::ColumnSpec::default(), move || {\n    \n});",
    ),
    (
        "cprow",
        "Row layout",
        "cranpose::Row(cranpose::Modifier::empty(), cranpose::RowSpec::default(), move || {\n    \n});",
    ),
    (
        "cptext",
        "Text",
        "cranpose::Text(\"Hello\", cranpose::Modifier::empty(), cranpose::TextStyle::default());",
    ),
    (
        "cpstate",
        "Remembered mutable state",
        "let state = cranpose::rememberMutableStateOf(|| 0);",
    ),
    (
        "cpbutton",
        "Button with action",
        "cranpose::Button(cranpose::Modifier::empty(), cranpose::ButtonSpec::default(), move || {}, move || {\n    cranpose::Text(\"Button\", cranpose::Modifier::empty(), cranpose::TextStyle::default());\n});",
    ),
];
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_offsets_and_raw_names() {
        let s = "// é🙂\n#[cranpose::composable]\n#[track_caller]\npub(crate) fn Screen<T>() {}\n#[composable] fn r#type() {}";
        let found = symbols(s);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].offset,
            s[..s.find("Screen").expect("name")].encode_utf16().count()
        );
        assert_eq!(found[0].line, 4);
        assert_eq!(found[1].name, "type");
    }
    #[test]
    fn unfinished_source_still_has_navigation() {
        assert_eq!(
            symbols("#[composable] fn First() {}\n\"unfinished")[0].name,
            "First"
        );
        assert!(symbols("/* unfinished").is_empty());
    }
    #[test]
    fn ignores_nested_comments_and_raw_strings() {
        let s = "/* /* #[composable] fn Wrong() {} */ */\nconst S:&str=r##\" #[composable] fn Raw() {} \"##;\n#[composable] fn Real() {}";
        assert_eq!(
            symbols(s)
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Real"]
        );
    }
}
