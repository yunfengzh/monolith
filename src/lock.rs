// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! [Referee]-[Athlete] is a brand-new lock model designed for savefile. Not like common
//! notification-observer model, [Referee] permits [Athlete] to prepare its data.
//!
//! Later is a sequence diagram
//!
//! ```plantuml
//! actor User
//! participant Referee
//! participant Athlete
//! Athlete -> Athlete: wait_pause()
//! User -> Referee: pause_and_wait_confirmation()
//! activate User
//! Referee --> Athlete: wakeup from wait_pause()
//! Athlete -> Athlete: prepare data
//! Athlete -> Athlete: ready_and_wait_resume()
//! activate Athlete
//! Athlete --> Referee: confirm_ready()
//! deactivate Athlete
//! deactivate User
//! User -> User: do sth, such as save game
//! User -> Referee: resume()
//! activate User
//! Referee --> Athlete: wakeup from wait_resume()
//! deactivate User
//! ```
//!
//! The key of the lock is **negotiation**. Such as generally sprite.pos is refreshed by physical
//! engine, so when player launches savegame request, sprite needs to get its pos back from
//! physical engine. In other words, athlete needs to take some time to prepare its data.
//!
//! The lock can be used in later contexts: Load, Save, Pause (launches by user normally),
//! AlgoPause (you algoserver maybe need to pause game for copying data for later usage) etc
//! [PauseReason].

use bevy::prelude::*;
use mockall::automock;
use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{
    Mutex,
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    watch::{Receiver, Sender},
};
use tokio_condvar::Condvar;

#[cfg(not(test))]
use crate::stump::stump;

#[cfg(test)]
use crate::lock::tests::referee_boundaryclass::stump;
// }])>

// save framework <([{
pub trait LoadSerialize {
    async fn ready_for_drop(&self);
}

/// SaveSerialize is a callback when Save request is launched.
#[automock]
pub trait SaveSerialize: Send + Sync + std::fmt::Debug {
    fn save(&self) -> Vec<u8>;
}
// }])>

// definitions shared by Referee and Athlete <([{
/// User needs to commit pause reason when apply pause request to Referee.
#[derive(Copy, Clone, Debug)]
pub enum PauseReason {
    None,
    Load,
    Pause,
    AlgoPause,
    Save,
}

/// Athlete reports his state to Referee when response to pause request.
pub enum ConfirmState {
    Cont,
    Drop,
}

// }])>

// Referee <([{
#[derive(Debug)]
enum RefereeState {
    None,
    WaitPauseResponse,
    ResetbyResumeFn,
}

/// Referee sends pause request to all Athletes then wait them to confirm ready then do further
/// task.
///
/// ## Safety
/// All functions of Referee are protected by `Referee::state`. So it's safe to access the struct
/// by [stump()](crate::stump::stump).
///
/// ## [Referee::pause_and_wait_confirmation] Design
/// The flow of `pause_and_wait_confirmation` is summarized below (3 stages)
///
/// 1. Send pause signal to all registered athletes, store the number of these athletes to `m`.
/// 2. Wait confirm signals from these athletes until m = 0.
/// 3. Tie up the loose ends, returns to its caller to do savegame.
///
/// ### Chanllege about the flow:
///
/// 1. New athlete maybe tries to register into referee after stage 1.
/// 2. An active athlete may exit before send confirm to referee in stage 2.
/// 3. `Referee::state` is designed as guard a quick region, so it's inappropriate to hold it
///    during the whole `pause_and_wait_confirmation`.
///
/// ### Solution:
///
/// 1. `Referee::new_athlete` is introduced to delay athlete registration until [Referee::resume]
///    is called.
/// 2. Send confirm signal in Athlete::Drop.
/// 3. Lock is released at the end of stage 1 and regain at the begin of stage 3. Stage 2 is
///    **NO-LOCK**.
///
/// ### Dead Lock:
/// Solution may cause a dead lock. Consider you have two sprites, one is King, another is Servant.
///
/// - King successly registers into referee, but it needs Servant to continue.
/// - Servant is on its way to referee registration.
/// - Now `pause_and_wait_confirmation` is called, it finds there's an athlete -- King so it sends
///   pause signal to King and wait King to response (stage 2).
/// - However, King is waiting for Servant.
/// - Servant is put into `Referee::new_athlete` and only is wakeup in `Referee::resume`.
/// 3-level dead lock!!!
///
/// So in the case, you need to rewrite King code to
///
/// ```plaintext
/// loop {
/// 	tokio::select! {
/// 		wait_servant_signal() => { break; }
/// 		athlete.wait_pause() => { athlete.confirm_ready(); athlete.wait_resume().await; }
/// 	}
/// }
/// ```
///
/// ### parameter `reason` of [Referee::pause_and_wait_confirmation]
/// When reason is [PauseReason::Load] or [PauseReason::Save], it's developer's responsibility to
/// prevent new athlete registers into Referee.
///
/// When reason is not [PauseReason::Pause], a raii is returned which calls [Referee::resume] when
/// it's droped.
///
/// See `examples/hello.rs` for all cases.
pub struct Referee {
    // the mutex protects all fields of the struct. Snippets in the mutex MUST be short and quick.
    state: Mutex<RefereeState>,
    // Be used by pause_and_wait_confirmation to queue multiple requests. cmd_queue.1 stores current PauseReason.
    cmd_queue: (Condvar, PauseReason),
    // The field is used to cache new athlete registration when Referee is busy for pause process.
    new_athlete: Condvar,

    counter: AtomicUsize,
    pool: HashMap<usize, (Box<dyn SaveSerialize>, String)>,

    pause: Sender<PauseReason>,
    confirm: (UnboundedSender<ConfirmState>, UnboundedReceiver<ConfirmState>),
    resume: Sender<()>,
}

impl Referee {
    pub fn new() -> Self {
        let (s, r) = unbounded_channel::<ConfirmState>();
        Self {
            state: Mutex::new(RefereeState::None),
            cmd_queue: (Condvar::new(), PauseReason::None),
            new_athlete: Condvar::new(),
            counter: AtomicUsize::new(1),
            pool: HashMap::new(),
            pause: Sender::new(PauseReason::None),
            resume: Sender::new(()),
            confirm: (s, r),
        }
    }

    // Safety, methods listed here is protected by lock. <([{
    pub async fn register(&mut self, su: Box<dyn SaveSerialize>, desc: String) -> Athlete {
        let mut lock = self.state.lock().await;
        loop {
            match *lock {
                RefereeState::None => {
                    let id = self.counter.fetch_add(1, Ordering::SeqCst);
                    let athlete = Athlete {
                        id,
                        pause: self.pause.subscribe(),
                        confirm: self.confirm.0.clone(),
                        resume: self.resume.subscribe(),
                    };
                    self.pool.insert(id, (su, desc));
                    return athlete;
                }
                _ => {
                    let reason = self.cmd_queue.1;
                    if matches!(reason, PauseReason::Load) || matches!(reason, PauseReason::Save) {
                        error!("New athlete is appended into referee when {:?}", reason);
                    }
                    lock = self.new_athlete.wait(lock).await;
                }
            }
        }
    }

    async fn unregister(&mut self, athlete: &mut Athlete) {
        let lock = self.state.lock().await;
        self.pool.remove(&athlete.id);
        match *lock {
            RefereeState::None => {}
            RefereeState::WaitPauseResponse => {
                athlete.confirm_ready(ConfirmState::Drop);
            }
            RefereeState::ResetbyResumeFn => {
                unreachable!();
            }
        }
    }

    pub async fn pause_and_wait_confirmation(&mut self, reason: PauseReason) -> Option<RefereeGuard> {
        let mut lock = self.state.lock().await;
        loop {
            if matches!(self.cmd_queue.1, PauseReason::None) {
                self.cmd_queue.1 = reason;
                break;
            } else {
                lock = self.cmd_queue.0.wait(lock).await;
            }
        }

        // stage 1: wakeup all athletes by pause_chan <([{
        let _ = self.pause.send(reason);
        *lock = RefereeState::WaitPauseResponse;
        let m = self.pool.len();
        // }])>
        drop(lock);

        // stage 2: collect confirmation from athletes, NOLOCK! RefereeState::WaitPauseResponse <([{
        let mut i = m;
        let mut droped_cnt = 0;
        while i != 0 {
            let state = self.confirm.1.recv().await.unwrap();
            match reason {
                // Force all athlets exit, ConfirmState::Drop only is sent by Athlete::async_drop().
                PauseReason::Load => match state {
                    ConfirmState::Drop => {}
                    _ => {
                        panic!("PauseReason::Load receives ConfirmState::Drop only.");
                    }
                },
                _ => {}
            }
            match state {
                ConfirmState::Drop => {
                    droped_cnt += 1;
                }
                _ => {}
            }
            i = i - 1;
        }
        // }])>

        let mut lock = self.state.lock().await;
        // stage 3: regain lock, RefereeState::DoingJob, resume() will reset it to RefereeState::None <([{
        *lock = RefereeState::ResetbyResumeFn;
        let n = self.pool.len();
        // if m != n, some athletes has been dropped during stage 2.
        assert_eq!(m, n + droped_cnt);
        // }])>

        if matches!(reason, PauseReason::Pause) { None } else { Some(RefereeGuard { phantom: PhantomData }) }
    }

    pub async fn resume(&mut self) {
        let mut lock = self.state.lock().await;
        match self.cmd_queue.1 {
            PauseReason::Load => {
                // Later make sure all athletes exit.
                loop {
                    if self.resume.is_closed() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }
            PauseReason::Save | PauseReason::Pause | PauseReason::AlgoPause => {
                let _ = self.resume.send(());
            }
            _ => {
                panic!("invalid parameter");
            }
        }
        self.cmd_queue.1 = PauseReason::None;
        self.cmd_queue.0.notify_all();
        *lock = RefereeState::None;
        self.new_athlete.notify_all();
    }
    // }])>

    // Async version of std::fmt::Debug of Referee, is used in async fn.
    pub async fn async_debug_fmt(&self) {
        let lock = self.state.lock().await;
        println!("state: {:?}", (*lock));
        println!("athletes:");
        self.pool.iter().for_each(|(k, v)| println!("  {:?}-{:?}", k, v.1));
    }

    // Safety: get_athlets is designed for PauseReason::Save context, so no lock at all.
    pub fn get_athlets(&self) -> Box<dyn Iterator<Item = &Box<dyn SaveSerialize>> + '_ + Send> {
        Box::new(self.pool.iter().map(|i| &i.1.0))
    }
}

impl std::fmt::Debug for Referee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lock = self.state.blocking_lock();
        write!(f, "state: {:?}", (*lock))?;
        self.pool.iter().for_each(|(k, v)| {
            write!(f, "{:?}-{:?}", k, v.1).unwrap();
        });
        Ok(())
    }
}

/// RAII object returns by [Referee::pause_and_wait_confirmation]
pub struct RefereeGuard<'a> {
    phantom: PhantomData<&'a Referee>,
}

impl<'a> RefereeGuard<'a> {
    pub async fn async_drop(&mut self) {
        stump().referee.resume().await;
    }
}
// }])>

// Athlete <([{
/// Athlete cooperate with Referee to response to pause request, prepare data for savefile.
#[derive(Debug)]
pub struct Athlete {
    id: usize,

    pause: Receiver<PauseReason>,
    confirm: UnboundedSender<ConfirmState>,
    resume: Receiver<()>,
}

impl Athlete {
    pub async fn wait_pause(&mut self) -> PauseReason {
        self.pause.changed().await.unwrap();
        *self.pause.borrow_and_update()
    }

    pub fn confirm_ready(&mut self, state: ConfirmState) {
        self.confirm.send(state).unwrap();
    }

    pub async fn ready_and_wait_resume(&mut self, state: ConfirmState) {
        self.confirm_ready(state);
        self.resume.changed().await.unwrap();
        self.resume.borrow_and_update();
    }

    // TODO: async drop isn't supported by rust, so caller need to call later function manually.
    pub async fn async_drop(&mut self) {
        stump().referee.unregister(self).await;
    }
}
// }])>

// mod tests <([{
#[cfg(test)]
mod tests {
    pub(crate) mod referee_boundaryclass {
        use std::{
            mem::{forget, replace},
            ptr::drop_in_place,
            sync::{MutexGuard, OnceLock},
        };

        use super::*;

        static mut P: OnceLock<MockStump> = OnceLock::new();

        pub(crate) struct MockStump {
            pub(crate) referee: Referee,

            // `cargo test' will run tests in the same process, share the same static variables.
            // Since my test uses stump() global function, so all tests must be run sequentially to
            // avoid race.
            sequential_mutex: std::sync::Mutex<()>,
        }

        pub fn mystump_new() -> MutexGuard<'static, ()> {
            unsafe {
                (*(&raw mut P))
                    .get_or_init(|| MockStump { referee: Referee::new(), sequential_mutex: std::sync::Mutex::new(()) });
            }
            let stump = stump();
            let guard = stump.sequential_mutex.lock().unwrap();
            let referee = Referee::new();
            forget(replace(&mut stump.referee, referee));
            guard
        }

        pub fn mystump_drop(guard: MutexGuard<'static, ()>) {
            unsafe {
                drop_in_place(&mut stump().referee);
            }
            drop(guard);
        }

        pub fn stump() -> &'static mut MockStump {
            unsafe { (*(&raw mut P)).get_mut().unwrap() }
        }
    }

    use std::time::Duration;

    use tokio::{join, sync::oneshot, time::sleep};

    use super::*;

    use referee_boundaryclass::*;

    // Test when there's no athlete at all.
    #[tokio::test]
    async fn referee_pausereason_save_empty() {
        let guard = mystump_new();
        let mut referee = &mut stump().referee;
        let referee_mut = &raw mut referee;

        let mut raii = referee.pause_and_wait_confirmation(PauseReason::Save).await.unwrap();
        unsafe {
            assert!(matches!((*referee_mut).state.get_mut(), RefereeState::ResetbyResumeFn));
            assert_eq!((*referee_mut).pool.len(), 0);
            for _ in (*referee_mut).get_athlets() {
                assert!(false);
            }
        }
        raii.async_drop().await;

        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.pool.len(), 0);

        mystump_drop(guard);
    }

    // The test forks three athletes:
    // - a: follows normal flow, wait_pause(), confirm_ready(), wait_resume().
    // - b: exits.
    // - c: try to register when referee is in pause_and_wait_confirmation(),
    // RefereeState::WaitPauseResponse.
    #[tokio::test]
    async fn referee_pausereason_save() {
        let guard = mystump_new();
        let referee = &mut stump().referee;
        let referee_mut = referee as *mut Referee;
        let (t3, r3) = oneshot::channel();

        // Athlete a, follows normal flow.
        let mut mock_a = MockSaveSerialize::new();
        mock_a.expect_save().once().returning(|| vec![2]);
        let mut a = referee.register(Box::new(mock_a), "a".to_string()).await;
        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.pool.len(), 1);
        let task_a = tokio::spawn(async move {
            let pr = a.wait_pause().await;
            assert!(matches!(pr, PauseReason::Save));
            let referee = &mut stump().referee;
            assert!(matches!(referee.state.get_mut(), RefereeState::WaitPauseResponse));
            // Notify athlete3 to register during referee is in pause process.
            t3.send(()).unwrap();
            sleep(Duration::from_millis(100)).await;
            a.ready_and_wait_resume(ConfirmState::Cont).await;
            a.async_drop().await;
        });

        // Athlete b, exits.
        let mut mock_b = MockSaveSerialize::new();
        mock_b.expect_save().never();
        let mut b = referee.register(Box::new(mock_b), "a".to_string()).await;
        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.pool.len(), 2);
        let task_b = tokio::spawn(async move {
            sleep(Duration::from_millis(100)).await;
            b.async_drop().await;
        });

        // Athlete c, try to register then exit.
        let mut mock_c = MockSaveSerialize::new();
        mock_c.expect_save().never();
        let task_c = tokio::spawn(async move {
            r3.await.unwrap();
            let referee = &mut stump().referee;
            let mut c = referee.register(Box::new(mock_c), "c".to_string()).await;
            assert!(matches!(referee.state.get_mut(), RefereeState::None));
            assert_eq!(referee.pool.len(), 1);
            c.async_drop().await;
        });

        // Referee.
        let mut raii = referee.pause_and_wait_confirmation(PauseReason::Save).await.unwrap();
        unsafe {
            assert!(matches!((*referee_mut).state.get_mut(), RefereeState::ResetbyResumeFn));
            assert_eq!((*referee_mut).pool.len(), 1);
            for i in (*referee_mut).get_athlets() {
                assert_eq!(i.save(), vec![2]);
            }
        }
        raii.async_drop().await;

        let _ = join!(task_a, task_b, task_c);
        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.pool.len(), 0);

        mystump_drop(guard);
    }

    // The test forks one athlete:
    // - a: follows normal flow, wait_pause(), exit if receiving PauseReason::Load.
    #[tokio::test]
    async fn referee_pausereason_load() {
        let guard = mystump_new();
        let referee = &mut stump().referee;

        // Athlete a, follows normal flow.
        let mock_a = MockSaveSerialize::new();
        let mut a = referee.register(Box::new(mock_a), "a".to_string()).await;
        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.pool.len(), 1);
        let task_a = tokio::spawn(async move {
            let pr = a.wait_pause().await;
            assert!(matches!(pr, PauseReason::Load));
            a.async_drop().await;
        });

        // Referee.
        let mut raii = referee.pause_and_wait_confirmation(PauseReason::Load).await.unwrap();
        raii.async_drop().await;

        let _ = join!(task_a);
        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.pool.len(), 0);

        mystump_drop(guard);
    }
}
// }])>
