use super::*;

#[test]
fn search_keeps_opaque_identities_for_duplicate_labels_and_unicode() {
    let items = vec![
        ChoiceItem {
            id: "a::demo".into(),
            label: "Demo".into(),
            detail: "Étoile · bin".into(),
        },
        ChoiceItem {
            id: "b::demo".into(),
            label: "Demo".into(),
            detail: "Other · example".into(),
        },
    ];
    assert_eq!(matching(&items, "DEMO"), vec![0, 1]);
    assert_eq!(matching(&items, "éTOILE demo"), vec![0]);
    assert_eq!(matching(&items, "example demo"), vec![1]);
    assert!(matching(&items, "demo missing").is_empty());
    assert_eq!(items[matching(&items, "other")[0]].id, "b::demo");
}

#[test]
fn pagination_is_bounded_and_clamps_after_catalog_shrinks() {
    let items: Vec<_> = (0..10_001)
        .map(|id| ChoiceItem {
            id: id.to_string(),
            label: format!("Target {id}"),
            detail: "bin".into(),
        })
        .collect();
    let all = matching(&items, "  ");
    assert_eq!(all.len(), 10_001);
    assert_eq!(page_start(2_000, all.len()), 10_000);
    assert_eq!(page_start(2_000, 7), 5);
    assert_eq!(page_start(usize::MAX, 0), 0);
    assert_eq!(page_start(usize::MAX, 1), 0);
    for size in [0, 1, 5, 6, 7, 10_001] {
        let start = page_start(usize::MAX, size);
        assert!(size.saturating_sub(start) <= PAGE_SIZE);
    }
}
