#[path = "../../process/tests/support/mod.rs"]
mod support;
use cranpose_plugin_host::session::{Options, Session, SessionEvent};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

fn start(directory: &std::path::Path, mode: &str) -> Session {
    let fixture = support::command(directory, "root", mode);
    Session::start(Options {
        command: std::iter::once(fixture.get_program())
            .chain(fixture.get_args())
            .map(|s| s.to_string_lossy().into_owned())
            .collect(),
        directory: None,
        environment: fixture
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|v| {
                    (
                        key.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>(),
        timeout: Duration::from_millis(1000),
    })
    .expect("session")
}
#[test]
fn timeout_auth_failure_and_eof_stop_children_without_ui_disposal() {
    for mode in ["stubborn", "bad-auth", "eof"] {
        let directory = tempfile::tempdir().expect("fixture");
        let session = start(directory.path(), mode);
        support::ready(directory.path());
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            assert!(
                Instant::now() < deadline,
                "missing stopped event for {mode}"
            );
            if matches!(
                session.events.recv_timeout(Duration::from_millis(100)),
                Ok(SessionEvent::Stopped(_))
            ) {
                break;
            }
        }
        support::exited(directory.path());
        // The Session is deliberately still alive, as it can be in an idle IDE tab.
        drop(session);
    }
}
#[test]
fn closing_during_handshake_is_nonblocking_and_stops_the_tree() {
    let directory = tempfile::tempdir().expect("fixture");
    let session = start(directory.path(), "handshake");
    support::ready(directory.path());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !directory.path().join("connected").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    session.close();
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "IDE thread blocked"
    );
    support::exited(directory.path());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "handshake timeout delayed cancellation"
    );
}
