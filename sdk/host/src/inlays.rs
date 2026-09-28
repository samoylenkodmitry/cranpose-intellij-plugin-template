//! Synchronous Rust operations inside IntelliJ's inlay batch boundary.
use crate::jvm::{self, A, J, O};
use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};

/// Run inlay additions, removals or updates as one editor layout operation.
///
/// Use batching only for large sets: the IDE's setup cost depends on document
/// size. The closure must only operate on inlays. In particular, do not edit the
/// document, access caret/folding/soft-wrap state, or transform coordinates.
/// The IDE can place the caret differently when inlays change at its offset.
/// Callers must validate their editing behavior and avoid batching small sets.
///
/// The callback and captured values are released before this function returns,
/// including on a Rust error or Java exception. Rust errors stay in Rust rather
/// than becoming exceptions in the IDE's batch-completion listeners.
pub fn execute<T: Send + 'static>(
    j: &mut J<'_>,
    model: &O,
    batch: bool,
    operation: impl FnOnce(&mut J<'_>) -> Result<T> + Send + 'static,
) -> Result<T> {
    if !batch {
        return operation(j);
    }
    let operation = Mutex::new(Some(operation));
    let outcome = Arc::new(Mutex::new(None));
    let result = outcome.clone();
    let scope = jvm::Scope::default();
    let id = scope.register(move |j, _, _| {
        let operation = operation.lock().expect("inlay operation").take();
        if let Some(operation) = operation {
            *result.lock().expect("inlay result") = Some(operation(j));
        }
        j.null()
    });
    let callback = jvm::callback(j, id)?;
    j.void(
        model,
        "execute",
        "(ZLjava/lang/Runnable;)V",
        &[A::Z(true), A::O(&callback)],
    )?;
    outcome
        .lock()
        .expect("inlay result")
        .take()
        .context("IDE did not execute the inlay operation")?
}

#[cfg(feature = "ide-tests")]
pub(crate) fn integration_test(j: &mut J<'_>, model: &O) -> Result<()> {
    use anyhow::ensure;
    use std::sync::atomic::{AtomicUsize, Ordering};

    for batch in [false, true] {
        for fail in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let count = calls.clone();
            let capture = Arc::new(());
            let weak = Arc::downgrade(&capture);
            let inside = model.clone();
            let result = execute(j, model, batch, move |j| {
                let _capture = capture;
                count.fetch_add(1, Ordering::Relaxed);
                ensure!(j.bool(&inside, "isInBatchMode")? == batch, "Batch boundary");
                ensure!(!fail, "deliberate inlay operation failure");
                Ok(42)
            });
            ensure!(calls.load(Ordering::Relaxed) == 1, "Operation ran twice");
            ensure!(weak.upgrade().is_none(), "Operation capture leaked");
            ensure!(!j.bool(model, "isInBatchMode")?, "Batch remained open");
            if fail {
                ensure!(
                    result
                        .err()
                        .is_some_and(|e| e.to_string() == "deliberate inlay operation failure"),
                    "Rust operation error was lost"
                );
            } else {
                ensure!(result? == 42, "Operation result was lost");
            }
        }
    }
    // Method lookup fails before the runnable executes. Its captures still
    // belong to the temporary scope, even while a Java exception is pending.
    let capture = Arc::new(());
    let weak = Arc::downgrade(&capture);
    let invalid = j.new("java/lang/Object", "()V", &[])?;
    let result = execute(j, &invalid, true, move |_| {
        let _capture = capture;
        anyhow::bail!("Unreachable operation")
    }) as Result<()>;
    ensure!(result.is_err(), "Invalid receiver unexpectedly succeeded");
    ensure!(
        weak.upgrade().is_none(),
        "Failed JNI call leaked its capture"
    );
    if j.env.exception_check()? {
        j.env.exception_clear()?;
    }
    ensure!(
        !j.bool(model, "isInBatchMode")?,
        "Failed call left a batch open"
    );
    Ok(())
}
