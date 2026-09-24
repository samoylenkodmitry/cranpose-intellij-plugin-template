use super::*;

fn file(name: &str) -> EditorFile {
    EditorFile {
        path: format!("/src/{name}"),
        name: name.to_string(),
    }
}

#[test]
fn a_new_file_goes_to_the_front() {
    let recent = remember_file(&[file("a.rs"), file("b.rs")], file("c.rs"));
    assert_eq!(recent, vec![file("c.rs"), file("a.rs"), file("b.rs")]);
}

#[test]
fn a_reopened_file_moves_to_the_front_once() {
    let recent = remember_file(&[file("a.rs"), file("b.rs")], file("b.rs"));
    assert_eq!(recent, vec![file("b.rs"), file("a.rs")]);
}

#[test]
fn the_history_is_bounded() {
    let many: Vec<EditorFile> = (0..RECENT_FILES)
        .map(|index| file(&format!("{index}.rs")))
        .collect();
    let recent = remember_file(&many, file("new.rs"));
    assert_eq!(recent.len(), RECENT_FILES);
    assert_eq!(recent[0], file("new.rs"));
}
