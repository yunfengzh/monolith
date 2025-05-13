// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! Stump defines a global pointer to access monolith components.
//!
//! It also includes several auxiliary functions to exit tokio/bevy, wait bevy start.
//!
//! ## Safety
//! All components from [stump()] are independent each other, goto their document for safety.
//!
//! ## Howto exit tokio
//! Goto [Graceful Shutdown tokio](https://tokio.rs/tokio/topics/shutdown) for more. I implements
//! an inner struct tokio to cover the topic.
//! - [Stump::spawn] to replace tokio::spawn to let your coroutine be the charge of stump().tokio.
//! - [Stump::get_ct] to get a CancellationToken then `ct.cancelled()` when you're ready for exit
//! current coroutine, just like call `yield` in coroutine.
//! - [Stump::exit_tokio] to exit all tokio coroutines gracefully.
//!
//! ## Howto exit bevy
//! - In tokio: call Stump::exit_bevy to exit bevy.
//! - In bevy: send `AppExit::Success` to `EventWriter<AppExit>`.
//!
//! ## Howto get start notification from bevy.
//! await [Stump::bevy_started].

use std::{future::Future, sync::OnceLock};

use crate::from_bevy::*;
use crate::lock::Referee;
use crate::to_bevy::*;
use bevy::prelude::*;
use tokio::{
    sync::{
        Mutex, Notify,
        mpsc::{UnboundedSender, unbounded_channel},
    },
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::task_tracker::TaskTracker;
// }])>

// STUMP, global object <([{
static mut STUMP: OnceLock<Stump> = OnceLock::new();

/// Global function to access global object -- Stump.
pub fn stump() -> &'static mut Stump {
    // Safety: all fields of Stump are independent of each other and protected by their lock.
    unsafe { (*(&raw mut STUMP)).get_mut().unwrap() }
}

/// Initialize global pointer Stump and App initialization.
pub fn stump_new(mut app: App, config: Option<DeveloperBackendCallback>) -> App {
    let (s, r) = unbounded_channel::<TaskPayload>();
    app = to_bevy_init(app);
    app = from_bevy_init(app);

    // Safety: STUMP should be init once.
    unsafe {
        (*(&raw mut STUMP))
            .set(Stump {
                tokio: Tokio { ct: CancellationToken::new(), tt: TaskTracker::new() },
                std_backend: Mutex::new(Vec::new()),
                button_backend: Mutex::new(Vec::new()),
                developer_backend: DeveloperBackend::new(config),
                record_backend: RecordBackend::new(),
                replay_backend: ReplayBackend::new(),
                bevy_start_notification: Notify::new(),
                task_chan: TaskChannel::new(s, r),
                referee: Referee::new(),
            })
            .unwrap();
    }

    app
}

pub async fn stump_run() {
    let st = stump();
    if st.developer_backend.user_callback.is_some() {
        stump().spawn(async move {
            stump().developer_backend.fore_run().await;
        });
    }
    st.spawn(async move {
        stump().developer_backend.back_run().await;
    });
    st.spawn(async move {
        stump().replay_backend.run().await;
    });
}

/// Drop global pointer -- Stump.
pub fn stump_drop() {
    // Safety: STUMP should be drop once.
    unsafe {
        let _ = (*(&raw mut STUMP)).take().unwrap();
    }
}
// }])>
pub type TaskSender = UnboundedSender<TaskPayload>;

#[derive(Debug)]
struct Tokio {
    ct: CancellationToken,
    tt: TaskTracker,
}

/// Global object provides a lots of methods to access monolith objects.
#[derive(Debug)]
pub struct Stump {
    tokio: Tokio,

    pub(crate) std_backend: Mutex<Vec<Box<StdBackend>>>,
    pub(crate) button_backend: Mutex<Vec<Box<ButtonBackend>>>,
    pub(crate) developer_backend: DeveloperBackend,
    pub(crate) record_backend: RecordBackend,
    pub(crate) replay_backend: ReplayBackend,
    bevy_start_notification: Notify,

    pub(crate) task_chan: TaskChannel,

    pub(crate) referee: Referee,
}

impl Stump {
    // tokio <([{
    pub fn get_ct(&self) -> CancellationToken {
        self.tokio.ct.clone()
    }

    pub fn spawn<F>(&self, task: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.tokio.tt.spawn(task)
    }

    pub fn exit_tokio(&self) {
        self.tokio.ct.cancel();
    }

    pub async fn wait_tasktracker_exit(&self) {
        self.tokio.tt.close();
        self.tokio.tt.wait().await;
    }
    // }])>

    // from bevy <([{
    pub async fn new_foreend(&mut self, desc: String) -> Foreend {
        Foreend::new(desc).await
    }

    pub fn get_developer_backend(&self) -> &DeveloperBackend {
        &self.developer_backend
    }

    pub fn get_record_backend(&mut self) -> &mut RecordBackend {
        &mut self.record_backend
    }

    pub fn get_replay_backend(&mut self) -> &mut ReplayBackend {
        &mut self.replay_backend
    }

    pub(crate) fn bevy_start(&self) {
        self.bevy_start_notification.notify_one();
    }

    pub async fn bevy_started(&self) {
        self.bevy_start_notification.notified().await;
    }
    // }])>

    // to bevy <([{
    pub fn clone_tchan(&self) -> TaskSender {
        self.task_chan.get_sender()
    }

    pub fn exit_bevy(&self) {
        self.task_chan.send(TaskPayload::BevyExit).unwrap();
    }
    // }])>

    pub fn get_referee(&mut self) -> &mut Referee {
        &mut self.referee
    }
}
