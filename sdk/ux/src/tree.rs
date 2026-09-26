//! Searchable, collapsible trees with ancestor context preserved while filtering.
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

/// Entries are in display order, with parents preceding children.
pub struct TreeEntry<'a> {
    pub id: &'a str,
    pub parent: Option<&'a str>,
    pub search_text: Cow<'a, str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub index: usize,
    pub depth: usize,
    pub matches: bool,
    pub has_children: bool,
    pub expanded: bool,
}

/// A case-insensitive, whitespace-separated AND search. Matching descendants reveal
/// their ancestors, including collapsed ones, without changing saved expansion state.
/// Invalid identities or parent order are rejected rather than silently losing nodes.
pub fn filter_tree(
    entries: &[TreeEntry<'_>],
    query: &str,
    collapsed: &HashSet<String>,
) -> Result<Vec<TreeRow>, &'static str> {
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let searching = !terms.is_empty();
    let mut indices = HashMap::with_capacity(entries.len());
    let mut rows: Vec<TreeRow> = Vec::with_capacity(entries.len());
    let mut parents = Vec::with_capacity(entries.len());
    let mut hidden = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        if entry.id.is_empty() || indices.contains_key(entry.id) {
            return Err("Duplicate or empty tree identity");
        }
        let parent: Option<usize> = entry
            .parent
            .map(|id| {
                indices
                    .get(id)
                    .copied()
                    .ok_or("Parent must precede its child")
            })
            .transpose()?;
        let depth = parent.map_or(0, |parent| rows[parent].depth + 1);
        let text = if searching {
            entry.search_text.to_lowercase()
        } else {
            String::new()
        };
        let matches = searching && terms.iter().all(|term| text.contains(term));
        let expanded = searching || !collapsed.contains(entry.id);
        hidden.push(parent.is_some_and(|parent| hidden[parent] || !rows[parent].expanded));
        if let Some(parent) = parent {
            rows[parent].has_children = true;
        }
        rows.push(TreeRow {
            index,
            depth,
            matches,
            has_children: false,
            expanded,
        });
        parents.push(parent);
        indices.insert(entry.id, index);
    }
    if searching {
        let mut visible: Vec<bool> = rows.iter().map(|row| row.matches).collect();
        // Reverse propagation visits each edge once, even for very deep trees.
        for index in (0..rows.len()).rev() {
            if visible[index]
                && let Some(parent) = parents[index]
            {
                visible[parent] = true;
            }
        }
        rows.retain(|row| visible[row.index]);
    } else {
        rows.retain(|row| !hidden[row.index]);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entries() -> Vec<TreeEntry<'static>> {
        vec![
            TreeEntry {
                id: "r",
                parent: None,
                search_text: "Column".into(),
            },
            TreeEntry {
                id: "a",
                parent: Some("r"),
                search_text: "Row padding".into(),
            },
            TreeEntry {
                id: "b",
                parent: Some("a"),
                search_text: "Text Café src/card.rs".into(),
            },
            TreeEntry {
                id: "c",
                parent: Some("r"),
                search_text: "Button Save".into(),
            },
        ]
    }
    #[test]
    fn filtering_preserves_context_and_expansion_state() {
        let collapsed = HashSet::from(["r".into()]);
        let rows = filter_tree(&entries(), "CAFÉ card.rs", &collapsed).expect("tree");
        assert_eq!(
            rows.iter()
                .map(|r| (r.index, r.depth, r.matches))
                .collect::<Vec<_>>(),
            vec![(0, 0, false), (1, 1, false), (2, 2, true)]
        );
        assert!(rows[0].expanded);
        assert_eq!(
            filter_tree(&entries(), "", &collapsed).expect("tree").len(),
            1
        );
    }
    #[test]
    fn collapse_hides_descendants_but_keeps_siblings() {
        let rows = filter_tree(&entries(), "  ", &HashSet::from(["a".into()])).expect("tree");
        assert_eq!(
            rows.iter().map(|r| r.index).collect::<Vec<_>>(),
            vec![0, 1, 3]
        );
        assert!(!rows[1].expanded);
        assert!(rows[1].has_children);
    }
    #[test]
    fn no_match_is_empty_and_invalid_parent_cannot_loop() {
        assert!(
            filter_tree(&entries(), "not here", &HashSet::new())
                .expect("tree")
                .is_empty()
        );
        let mut bad = entries();
        bad[0].parent = Some("b");
        assert!(filter_tree(&bad, "", &HashSet::new()).is_err());
        bad[0].parent = None;
        bad[1].id = "r";
        assert!(filter_tree(&bad, "", &HashSet::new()).is_err());
    }
}
