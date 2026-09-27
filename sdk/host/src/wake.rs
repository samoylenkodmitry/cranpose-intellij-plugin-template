//! Coalesced, disposable delivery from Rust workers to the IDE event queue.
use crate::jvm::{self, J, O};
use anyhow::Result;
use jni::{JNIEnv, JavaVM, objects::JValue};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const IDLE: u8 = 0;
const QUEUED: u8 = 1;
const CLOSED: u8 = 2;

#[derive(Default)]
struct Gate(AtomicU8);
impl Gate {
    fn request(&self) -> bool {
        self.0
            .compare_exchange(IDLE, QUEUED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn take(&self) -> bool {
        self.0
            .compare_exchange(QUEUED, IDLE, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn close(&self) {
        self.0.store(CLOSED, Ordering::Release);
    }
}

pub struct Wake {
    vm: JavaVM,
    application: O,
    callback: O,
    gate: Arc<Gate>,
    id: i64,
}
impl Wake {
    /// The task must capture its owner weakly. Closing makes already queued
    /// callbacks inert; workers may keep a Wake without retaining the owner.
    pub fn new(
        j: &mut J<'_>,
        task: impl for<'a> Fn(&mut J<'a>) -> Result<()> + Send + Sync + 'static,
    ) -> Result<Arc<Self>> {
        let vm = j.env.get_java_vm()?;
        let application = j.application()?;
        let gate = Arc::new(Gate::default());
        let pending = gate.clone();
        let id = jvm::register(move |j, _, _| {
            if pending.take() {
                task(j)?;
            }
            j.null()
        });
        let callback = match jvm::callback(j, id) {
            Ok(callback) => callback,
            Err(error) => {
                jvm::unregister(id);
                return Err(error);
            }
        };
        Ok(Arc::new(Self {
            vm,
            application,
            callback,
            gate,
            id,
        }))
    }
    pub fn request(&self, j: &mut J<'_>) -> Result<()> {
        self.enqueue(&mut j.env)
    }
    pub fn request_from_worker(&self) -> Result<()> {
        // The guard detaches a newly attached worker on return. Only bootstrap
        // types and existing global references are used on this native thread.
        let mut env = self.vm.attach_current_thread()?;
        self.enqueue(&mut env)
    }
    fn enqueue(&self, env: &mut JNIEnv<'_>) -> Result<()> {
        if !self.gate.request() {
            return Ok(());
        }
        let result = env.with_local_frame(16, |env| -> jni::errors::Result<()> {
            env.call_method(
                &self.application,
                "invokeLater",
                "(Ljava/lang/Runnable;)V",
                &[JValue::Object(self.callback.as_obj())],
            )?;
            Ok(())
        });
        if result.is_err() {
            self.gate.take();
        }
        result.map_err(Into::into)
    }
    pub fn close(&self) {
        self.gate.close();
        jvm::unregister(self.id);
    }
}
impl Drop for Wake {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(feature = "ide-tests")]
pub fn integration_test(j: &mut J<'_>) -> Result<()> {
    use crate::jvm::A;
    use anyhow::ensure;
    use std::sync::atomic::AtomicUsize;
    let toolkit = j.static_obj(
        "java/awt/Toolkit",
        "getDefaultToolkit",
        "()Ljava/awt/Toolkit;",
        &[],
    )?;
    let queue = j.obj(
        &toolkit,
        "getSystemEventQueue",
        "()Ljava/awt/EventQueue;",
        &[],
    )?;
    let event_loop = j.obj(
        &queue,
        "createSecondaryLoop",
        "()Ljava/awt/SecondaryLoop;",
        &[],
    )?;
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let completion = event_loop.clone();
    let wake = Wake::new(j, move |j| {
        ensure!(
            j.static_call("java/awt/EventQueue", "isDispatchThread", "()Z", &[])?
                .z()?,
            "Worker callback must run on the EDT"
        );
        observed.fetch_add(1, Ordering::AcqRel);
        j.bool(&completion, "exit")?;
        Ok(())
    })?;
    // Stop the nested loop on failure as well, so CI does not wait forever.
    let scope = jvm::Scope::default();
    let timeout_loop = event_loop.clone();
    let timeout = scope.register(move |j, _, _| {
        j.bool(&timeout_loop, "exit")?;
        j.null()
    });
    let timer_callback = jvm::callback(j, timeout)?;
    let timer = j.new(
        "javax/swing/Timer",
        "(ILjava/awt/event/ActionListener;)V",
        &[A::I(5000), A::O(&timer_callback)],
    )?;
    j.void(&timer, "setRepeats", "(Z)V", &[A::Z(false)])?;
    let result = (|| -> Result<()> {
        let queued = wake.clone();
        std::thread::spawn(move || -> Result<()> {
            for _ in 0..1000 {
                queued.request_from_worker()?;
            }
            Ok(())
        })
        .join()
        .expect("wake worker")?;
        ensure!(
            count.load(Ordering::Acquire) == 0,
            "Delivery must be deferred"
        );
        j.void(&timer, "start", "()V", &[])?;
        ensure!(j.bool(&event_loop, "enter")?, "Event loop did not start");
        ensure!(count.load(Ordering::Acquire) == 1, "Burst did not coalesce");
        wake.request(j)?;
        wake.close();
        let retired = wake.callback.clone();
        let late = wake.clone();
        std::thread::spawn(move || late.request_from_worker())
            .join()
            .expect("late worker")?;
        j.void(&retired, "run", "()V", &[])?;
        ensure!(
            count.load(Ordering::Acquire) == 1,
            "Retired callback ran after close"
        );
        ensure!(
            Arc::strong_count(&count) == 1,
            "Disposal retained the callback captures"
        );
        Ok(())
    })();
    wake.close();
    j.void(&timer, "stop", "()V", &[])?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bursts_coalesce_and_close_cannot_be_reopened() {
        let gate = Arc::new(Gate::default());
        let queued = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..32)
                .map(|_| {
                    let gate = gate.clone();
                    scope.spawn(move || usize::from(gate.request()))
                })
                .collect();
            workers
                .into_iter()
                .map(|w| w.join().expect("worker"))
                .sum::<usize>()
        });
        assert_eq!(queued, 1);
        assert!(gate.take());
        assert!(
            gate.request(),
            "An edit arriving during delivery needs a new wake"
        );
        gate.close();
        assert!(!gate.take(), "A queued callback cannot run after close");
        assert!(
            !gate.request(),
            "A late worker cannot reopen a closed owner"
        );
    }
}
