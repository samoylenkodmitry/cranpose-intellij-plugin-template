use super::*;
use std::{
    sync::{Arc, Barrier},
    time::{Duration, SystemTime},
};

const FILES: &[(&str, &[u8])] = &[
    ("runtime/src/lib.rs", b"runtime"),
    ("macro/Cargo.toml", b"manifest"),
];
#[test]
fn reuse_preserves_timestamps_and_new_content_preserves_existing_versions() {
    let root = tempfile::tempdir().expect("cache");
    let first = materialize(root.path(), FILES).expect("publish");
    let source = first.join("runtime/src/lib.rs");
    let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(123456);
    fs::File::options()
        .write(true)
        .open(&source)
        .expect("source")
        .set_modified(stamp)
        .expect("mtime");
    let actual = fs::metadata(&source)
        .expect("metadata")
        .modified()
        .expect("mtime");
    let second = materialize(root.path(), &[FILES[1], FILES[0]]).expect("reuse");
    assert_eq!(first, second, "input order must not invalidate the cache");
    assert_eq!(
        fs::metadata(&source)
            .expect("metadata")
            .modified()
            .expect("mtime"),
        actual
    );
    let updated = materialize(
        root.path(),
        &[("runtime/src/lib.rs", b"new runtime"), FILES[1]],
    )
    .expect("new version");
    assert_ne!(first, updated);
    assert_eq!(fs::read(source).expect("original version"), b"runtime");
}
#[test]
fn concurrent_publishers_share_one_complete_bundle() {
    let root = tempfile::tempdir().expect("cache");
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let root = root.path().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                materialize(&root, FILES).expect("concurrent publish")
            })
        })
        .collect();
    let paths: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().expect("publisher"))
        .collect();
    assert!(paths.iter().all(|path| path == &paths[0]));
    assert_eq!(fs::read_dir(root.path()).expect("cache entries").count(), 1);
    assert_eq!(
        fs::read(paths[0].join("runtime/src/lib.rs")).expect("complete asset"),
        b"runtime"
    );
}
#[test]
fn rejects_escaping_duplicate_and_overlapping_paths_before_writing() {
    let root = tempfile::tempdir().expect("cache");
    for path in [
        "",
        "../escape",
        "/absolute",
        "./local",
        "a//b",
        "a\\b",
        "C:/absolute",
    ] {
        assert!(
            materialize(root.path(), &[(path, b"data")]).is_err(),
            "{path}"
        );
    }
    assert!(materialize(root.path(), &[("a", b"1"), ("a", b"2")]).is_err());
    assert!(materialize(root.path(), &[("a", b"1"), ("a/b", b"2")]).is_err());
    assert_eq!(fs::read_dir(root.path()).expect("cache").count(), 0);
}
#[test]
fn corrupt_or_extra_assets_are_reported_without_overwriting_an_active_bundle() {
    for extra in [false, true] {
        let root = tempfile::tempdir().expect("cache");
        let bundle = materialize(root.path(), FILES).expect("publish");
        let path = bundle.join(if extra {
            "build.rs"
        } else {
            "runtime/src/lib.rs"
        });
        fs::write(&path, "changed").expect("simulate corruption");
        assert!(materialize(root.path(), FILES).is_err());
        assert_eq!(fs::read(path).expect("never overwritten"), b"changed");
    }
}
#[cfg(unix)]
#[test]
fn symlinks_cannot_redirect_verification_outside_the_bundle() {
    let root = tempfile::tempdir().expect("cache");
    let bundle = materialize(root.path(), FILES).expect("publish");
    let source = bundle.join("runtime/src/lib.rs");
    fs::remove_file(&source).expect("remove fixture asset");
    let external = root.path().join("external");
    fs::write(&external, b"runtime").expect("external");
    std::os::unix::fs::symlink(&external, &source).expect("symlink");
    assert!(materialize(root.path(), FILES).is_err());
    assert_eq!(fs::read(external).expect("preserved"), b"runtime");
}

#[test]
fn filesystem_aliases_never_publish_an_incomplete_bundle() {
    let root = tempfile::tempdir().expect("cache");
    match materialize(root.path(), &[("file.rs", b"lower"), ("FILE.rs", b"upper")]) {
        Ok(bundle) => {
            assert_eq!(fs::read(bundle.join("file.rs")).expect("lower"), b"lower");
            assert_eq!(fs::read(bundle.join("FILE.rs")).expect("upper"), b"upper");
        }
        Err(_) => assert_eq!(
            fs::read_dir(root.path())
                .expect("no published partial bundle")
                .count(),
            0
        ),
    }
}
