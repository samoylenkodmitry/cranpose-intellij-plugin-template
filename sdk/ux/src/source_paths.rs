//! Map compiler source locations back to an editable source tree.
use std::path::{Path, PathBuf};

/// Resolve a Rust source location, then apply the most specific directory map.
///
/// Cargo can report `file!()` relative to the workspace while
/// `CARGO_MANIFEST_DIR` names a member package. When the file already starts
/// with that member's workspace-relative path, joining it to the manifest
/// directory would duplicate the member path. Package-relative and absolute
/// locations are also accepted. No filesystem probing or filename guessing is
/// needed, so deleted or not-yet-written editor files resolve the same way.
pub fn resolve(
    file: &Path,
    manifest_dir: &Path,
    compiled_root: &Path,
    original_root: &Path,
    mappings: &[(PathBuf, PathBuf)],
) -> PathBuf {
    let workspace_relative = !compiled_root.as_os_str().is_empty()
        && manifest_dir
            .strip_prefix(compiled_root)
            .ok()
            .filter(|member| !member.as_os_str().is_empty())
            .is_some_and(|member| file.starts_with(member));
    let path = if workspace_relative {
        compiled_root.join(file)
    } else {
        manifest_dir.join(file)
    };
    // Nested generated crates must win over their containing workspace, even
    // when log delivery or checkpoint restoration changes mapping order.
    if let Some((private, original)) = mappings
        .iter()
        .filter(|(private, _)| !private.as_os_str().is_empty() && path.starts_with(private))
        .max_by_key(|(private, _)| private.components().count())
    {
        return original.join(path.strip_prefix(private).expect("matching directory"));
    }
    if !compiled_root.as_os_str().is_empty()
        && let Ok(relative) = path.strip_prefix(compiled_root)
    {
        return original_root.join(relative);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_members_accept_both_compiler_path_forms_in_either_map_order() {
        let root = Path::new("/cache/workspace");
        let manifest = root.join(".generated/launcher");
        let project = Path::new("/project");
        let mut mappings = vec![
            (root.to_owned(), project.to_owned()),
            (manifest.clone(), project.join("app")),
        ];
        for _ in 0..2 {
            for file in [
                PathBuf::from("src/screens/detail.rs"),
                PathBuf::from(".generated/launcher/src/screens/detail.rs"),
                manifest.join("src/screens/detail.rs"),
            ] {
                assert_eq!(
                    resolve(&file, &manifest, root, project, &mappings),
                    project.join("app/src/screens/detail.rs")
                );
            }
            mappings.reverse();
        }
    }

    #[test]
    fn workspace_members_do_not_duplicate_their_package_path() {
        for file in ["packages/app/src/main.rs", "src/main.rs"] {
            assert_eq!(
                resolve(
                    Path::new(file),
                    Path::new("/cache/packages/app"),
                    Path::new("/cache"),
                    Path::new("/project"),
                    &[],
                ),
                PathBuf::from("/project/packages/app/src/main.rs")
            );
        }
    }

    #[test]
    fn unrelated_and_external_sources_are_not_rebased() {
        for (file, manifest, expected) in [
            (
                "src/lib.rs",
                "/registry/widget",
                "/registry/widget/src/lib.rs",
            ),
            ("/other/lib.rs", "/cache/app", "/other/lib.rs"),
            ("src/lib.rs", "/cache-other", "/cache-other/src/lib.rs"),
        ] {
            assert_eq!(
                resolve(
                    Path::new(file),
                    Path::new(manifest),
                    Path::new("/cache"),
                    Path::new("/project"),
                    &[]
                ),
                PathBuf::from(expected)
            );
        }
        assert_eq!(
            resolve(
                Path::new("src/main.rs"),
                Path::new("/project"),
                Path::new(""),
                Path::new("/other"),
                &[]
            ),
            PathBuf::from("/project/src/main.rs")
        );
    }
}
