//! Match an edited text slot to its rendered view, without guessing by label alone.
use crate::{Catalog, runtime::format_parts};
use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct TextTarget {
    pub lines: Vec<usize>,
    parts: Vec<String>,
    formatted: bool,
}
impl TextTarget {
    /// Only text whose initializer is under the edit is eligible. Aliases may
    /// lead to more than one Text call; an ambiguous rendered match is skipped.
    pub fn at(catalog: &Catalog, offset_utf16: usize) -> Option<Self> {
        let literal = catalog
            .literals
            .iter()
            .find(|l| l.range.start_utf16 <= offset_utf16 && offset_utf16 < l.range.end_utf16)?;
        if literal.kind != "string" {
            return None;
        }
        let formatted = catalog.formats.iter().any(|f| f.literal == literal.id);
        let parts = if formatted {
            format_parts(&literal.value)?.0
        } else {
            vec![literal.value.clone()]
        };
        if parts.iter().all(String::is_empty) {
            return None;
        }
        let mut lines = Vec::new();
        for range in std::iter::once(&literal.range).chain(
            catalog
                .references
                .iter()
                .filter(|r| r.literal == literal.id)
                .map(|r| &r.range),
        ) {
            if let Some(call) = catalog
                .calls
                .iter()
                .filter(|c| c.name == "Text" && c.range.start <= range.start && c.end >= range.end)
                .min_by_key(|c| c.end - c.range.start)
            {
                lines.push(call.range.line);
            }
        }
        lines.sort_unstable();
        lines.dedup();
        (!lines.is_empty()).then_some(Self {
            lines,
            parts,
            formatted,
        })
    }
    fn matches(&self, text: &str) -> bool {
        if !self.formatted || self.parts.len() == 1 {
            return text == self.parts[0];
        }
        let Some(mut rest) = text.strip_prefix(&self.parts[0]) else {
            return false;
        };
        for part in &self.parts[1..self.parts.len() - 1] {
            let Some((_, after)) = rest.split_once(part.as_str()) else {
                return false;
            };
            rest = after;
        }
        rest.ends_with(self.parts.last().expect("format parts"))
    }
    /// Source call identity and the new rendered text must both agree. Never
    /// point at an old label, duplicate view, truncated tree or invalid bounds.
    pub fn bounds(&self, file: &str, payload: &str, request: u64) -> Option<[f64; 4]> {
        if payload.len() > 8 * 1024 * 1024 {
            return None;
        }
        let snapshot: Snapshot = serde_json::from_str(payload).ok()?;
        if snapshot.schema != 2 || snapshot.request_id != request || snapshot.truncated {
            return None;
        }
        let suffix = format!("/{}", file.replace('\\', "/"));
        let mut matches = snapshot.nodes.into_iter().filter(|node| {
            node.kind == "Text"
                && self.matches(&node.text)
                && node.sources.iter().any(|s| {
                    let source_file = s.file.replace('\\', "/");
                    let path = if source_file.starts_with('/') || s.manifest_dir.is_empty() {
                        source_file
                    } else {
                        format!("{}/{}", s.manifest_dir.replace('\\', "/"), source_file)
                    };
                    s.name == "__cranpose_call:Text"
                        && self.lines.contains(&s.line)
                        && (path == file.replace('\\', "/") || path.ends_with(&suffix))
                })
                && [node.x, node.y, node.width, node.height]
                    .iter()
                    .all(|v| v.is_finite())
                && node.width > 0.0
                && node.height > 0.0
        });
        let node = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some([node.x, node.y, node.width, node.height])
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    schema: u32,
    request_id: u64,
    truncated: bool,
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    kind: String,
    text: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    sources: Vec<Source>,
}
#[derive(Deserialize)]
struct Source {
    name: String,
    file: String,
    line: usize,
    #[serde(default, rename = "manifestDir")]
    manifest_dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn target(source: &str, token: &str) -> TextTarget {
        TextTarget::at(
            &Catalog::parse(source).expect("catalog"),
            source[..source.find(token).expect("token")]
                .encode_utf16()
                .count(),
        )
        .expect("target")
    }
    fn snapshot(text: &str, line: usize) -> serde_json::Value {
        json!({"schema":2,"requestId":9,"truncated":false,"nodes":[{"kind":"Text","text":text,"x":20,"y":30,"width":80,"height":24,"sources":[{"name":"__cranpose_call:Text","file":"/private/preview/src/main.rs","line":line}]}]})
    }
    #[test]
    fn waits_for_new_text_at_the_exact_call_and_rejects_ambiguous_views() {
        let t = target("#[composable]\nfn Card(){ Text(\"New\"); }", "New");
        let mut s = snapshot("Old", 2);
        assert!(t.bounds("src/main.rs", &s.to_string(), 9).is_none());
        s["nodes"][0]["text"] = json!("New");
        assert_eq!(
            t.bounds("src/main.rs", &s.to_string(), 9),
            Some([20.0, 30.0, 80.0, 24.0])
        );
        assert!(t.bounds("other/main.rs", &s.to_string(), 9).is_none());
        assert!(t.bounds("src/main.rs", &s.to_string(), 10).is_none());
        s["nodes"][0]["sources"][0]["line"] = json!(3);
        assert!(t.bounds("src/main.rs", &s.to_string(), 9).is_none());
        s["nodes"][0]["sources"][0]["line"] = json!(2);
        let duplicate = s["nodes"][0].clone();
        s["nodes"].as_array_mut().expect("nodes").push(duplicate);
        assert!(t.bounds("src/main.rs", &s.to_string(), 9).is_none());
    }
    #[test]
    fn resolves_unicode_aliases_and_compiler_owned_format_fields() {
        let t = target(
            "// 🦀\n#[composable]\nfn Card(){ let title = \"Café\"; Text(title); }",
            "Café",
        );
        assert!(
            t.bounds("src/main.rs", &snapshot("Café", 3).to_string(), 9)
                .is_some()
        );
        let t = target(
            "#[composable]\nfn Card(){ Text(format!(\"Got {count:02} notes!\")); }",
            "Got",
        );
        assert!(t.matches("Got 04 notes!"));
        assert!(!t.matches("Got 04 notes"));
        assert!(!t.matches("Other 04 notes!"));
        let plain = target(
            "#[composable]\nfn Card(){ Text(format!(\"Ready\")); }",
            "Ready",
        );
        assert!(plain.matches("Ready"));
    }
    #[test]
    fn skips_non_text_changes_and_invalid_snapshots() {
        let c = Catalog::parse("#[composable]\nfn Card(){ Space(12); }").expect("catalog");
        assert!(TextTarget::at(&c, c.literals[0].range.start_utf16).is_none());
        let t = target("#[composable]\nfn Card(){ Text(\"New\"); }", "New");
        let mut s = snapshot("New", 2);
        s["truncated"] = json!(true);
        assert!(t.bounds("src/main.rs", &s.to_string(), 9).is_none());
        s["truncated"] = json!(false);
        s["nodes"][0]["width"] = json!(-1);
        assert!(t.bounds("src/main.rs", &s.to_string(), 9).is_none());
    }
}
