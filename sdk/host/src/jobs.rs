//! One cancellable background operation per project. Workers never access the JVM.
use crate::{
    jvm::{J, O},
    project::Project,
};
use anyhow::{Result, ensure};
use cranpose_plugin_process::Cancellation;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Weak, atomic::Ordering},
};

pub struct Context {
    pub root: PathBuf,
    pub cancel: Cancellation,
    project: Weak<Project>,
    channel: String,
}
impl Context {
    /// Event delivery is discarded after cancellation or project disposal.
    pub fn send(&self, value: &Value) {
        if self.cancel.is_cancelled() {
            return;
        }
        self.deliver(value);
    }
    fn deliver(&self, value: &Value) {
        if let Some(project) = self.project.upgrade()
            && !project.closed.load(Ordering::Acquire)
        {
            let payload = value.to_string();
            for panel in project.live_panels() {
                panel.message(&self.channel, &payload);
            }
        }
    }
}

/// Call on the IDE thread. The worker runs only for a trusted project.
pub fn start(
    j: &mut J<'_>,
    object: &O,
    channel: &str,
    worker: impl FnOnce(Context) -> Result<()> + Send + 'static,
) -> Result<()> {
    let project = Project::get(j, object)?;
    ensure!(
        project.trusted(j)?,
        "Trust the project before starting build tools"
    );
    start_project(&project, channel, worker)
}
fn start_project(
    project: &Arc<Project>,
    channel: &str,
    worker: impl FnOnce(Context) -> Result<()> + Send + 'static,
) -> Result<()> {
    ensure!(!project.closed.load(Ordering::Acquire), "Project is closed");
    let cancel = Cancellation::default();
    {
        let mut active = project.job.lock().expect("project job");
        ensure!(
            active.is_none(),
            "A platform operation is already running; cancel or wait for it to finish"
        );
        *active = Some(cancel.clone());
    }
    let context = Context {
        root: project.root.clone(),
        cancel: cancel.clone(),
        project: Arc::downgrade(project),
        channel: channel.into(),
    };
    let finish = Context {
        root: project.root.clone(),
        cancel,
        project: Arc::downgrade(project),
        channel: channel.into(),
    };
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker(context)))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Background operation panicked")));
        // Keep the slot reserved through final event delivery so a new operation
        // cannot overtake the previous completion and receive stale status.
        finish.deliver(&json!({"type":"job_finished", "cancelled":finish.cancel.is_cancelled(), "error":result.err().map(|e|format!("{e:#}"))}));
        if let Some(project) = finish.project.upgrade() {
            project.job.lock().expect("project job").take();
        }
    });
    Ok(())
}

pub fn cancel(j: &mut J<'_>, object: &O) -> Result<()> {
    let project = Project::get(j, object)?;
    cancel_project(&project);
    Ok(())
}
pub(crate) fn cancel_project(project: &Project) {
    if let Some(cancel) = project.job.lock().expect("project job").as_ref() {
        cancel.cancel();
    }
}

#[cfg(feature = "ide-tests")]
pub(crate) fn integration_test(j: &mut J<'_>) -> Result<()> {
    use std::{sync::mpsc, time::Duration};
    let project = crate::ide_tests::project(j)?;
    let (started, receiver) = mpsc::sync_channel(1);
    let (finished, stopped) = mpsc::sync_channel(1);
    start_project(&project, "cranpose.job.test", move |context| {
        started.send(())?;
        while !context.cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        context.send(&json!({"ignored":"late result"}));
        finished.send(())?;
        Ok(())
    })?;
    receiver.recv_timeout(Duration::from_secs(2))?;
    ensure!(
        start_project(&project, "cranpose.job.test", |_| Ok(())).is_err(),
        "Concurrent operation was accepted"
    );
    cancel(j, &project.object)?;
    stopped.recv_timeout(Duration::from_secs(2))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while project.job.lock().expect("project job").is_some() {
        ensure!(
            std::time::Instant::now() < deadline,
            "Job slot was retained after cancellation"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let (started, receiver) = mpsc::sync_channel(1);
    let (finished, stopped) = mpsc::sync_channel(1);
    start_project(&project, "cranpose.job.test", move |context| {
        started.send(())?;
        while !context.cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        context.send(&json!({"ignored":"disposed project"}));
        finished.send(())?;
        Ok(())
    })?;
    receiver.recv_timeout(Duration::from_secs(2))?;
    project.dispose(j)?;
    stopped.recv_timeout(Duration::from_secs(2))?;
    ensure!(
        start_project(&project, "cranpose.job.test", |_| Ok(())).is_err(),
        "Disposed project accepted work"
    );
    Ok(())
}
