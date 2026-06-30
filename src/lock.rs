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
    PauseStage,
    SavegameStage,
}

/// Referee sends pause request to all Athletes then wait them ready then do further task (such as
/// savegame).
///
/// ## Safety
/// All functions of Referee are protected by `Referee::state`. So it's safe to access the struct
/// by [stump()](crate::stump::stump).
///
/// ## Pause stage and savegame stage.
/// Referee core:
/// 1. Pause stage: [Referee::pause_and_wait_confirmation].
/// 2. savegame stage: between [Referee::pause_and_wait_confirmation] and [Referee::resume]
///
/// ## [Referee::pause_and_wait_confirmation] Design
/// The flow of `pause_and_wait_confirmation` is summarized below (4 steps)
/// 1. Send pause signal to all registered athletes, store the number of these athletes to `i`.
/// 2. Wait confirm signals from these athletes until i = 0.
/// 3. relock Referee to check whether there're some new athletes, store it to `i`, go to step 2.
/// 4. Finally, returns to its caller to do savegame.
///
/// ## Chanllege about the flow:
/// 1. New athlete maybe tries to register into referee in stage 1 and stage 2.
/// 2. An active athlete may exit before send confirm to referee in step 2.
/// 3. `Referee::state` is designed as guard critical area, so it's inappropriate to hold it during
///    the whole `pause_and_wait_confirmation`.
///
/// ### Solution for the flow:
/// 1. New athletes are checked in the loop of pause_and_wait_confirmation(). See also later topic.
/// 2. Send confirm signal in Athlete::Drop.
/// 3. Lock is holded when necessary during steps.
///
/// ## A Usercase:
/// Consider later case, a zombie incubator which is also an athlete is creating zombies when a
/// pause request,
/// `
/// let zombie = create_zombie();
/// referee.register(zombie);
/// incubator.zombie_cnt++; // later the field should be recorded into savefile.
/// `
/// in the case, the zombie is called indirect new-athelte, and because incubator is busy for
/// creating zombie, it doesn't response to pause request! So to make sure logic correct, new
/// zombie must receive the pause request. [Athlete::forced_pause_request] is used to append a pause
/// request to the zombie.
///
/// An independent new-athlete is, on the contrary, can appear anytime during pause stage and
/// savegame stage.
///
/// But to [Referee::pause_and_wait_confirmation], it is impossible to distinguish two kinds of
/// new-athletes, so in pause stage, all new-athletes are forced a pause request; but to savegame
/// stage, according to above code, only independent new-athlete can appear, it will not accept
/// current pause request.
///
/// ### More limit on [PauseReason]
/// Athlete must exit when receive [PauseReason::Load].
/// When reason is not [PauseReason::Pause], a raii is returned which calls [Referee::resume] when
/// it's droped.
///
/// See `examples/hello.rs` for all cases.
pub struct Referee {
    // the mutex protects all fields of the struct. Snippets in the mutex MUST be short and quick.
    state: Mutex<RefereeState>,
    // Be used by pause_and_wait_confirmation to queue multiple requests. cmd_queue.1 stores current PauseReason.
    cmd_queue: (Condvar, PauseReason),
    // Be used to record new athletes during pause_and_wait_confirmation step 2.
    new_athlete_cnt: usize,

    counter: AtomicUsize,
    roster: HashMap<usize, (Box<dyn SaveSerialize>, String)>,

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
            new_athlete_cnt: 0,
            counter: AtomicUsize::new(1),
            roster: HashMap::new(),
            pause: Sender::new(PauseReason::None),
            resume: Sender::new(()),
            confirm: (s, r),
        }
    }

    // Safety, methods listed here is protected by lock. <([{
    /// Safety: You can pass a raw pointer to su parameter, if you can make sure its lifetime is
    /// larger than our return value, Athlete.async_drop will remove the pointer. See
    /// examples/hello.rs for more.
    pub async fn register(&mut self, su: Box<dyn SaveSerialize>, desc: String) -> Athlete {
        let lock = self.state.lock().await;
        let forced_pause_request = match *lock {
            RefereeState::None => None,
            RefereeState::PauseStage => {
                self.new_athlete_cnt += 1;
                Some(self.cmd_queue.1)
            }
            RefereeState::SavegameStage => None,
        };

        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        let athlete = Athlete {
            id,
            forced_pause_request,
            pause: self.pause.subscribe(),
            confirm: self.confirm.0.clone(),
            resume: self.resume.subscribe(),
        };
        self.roster.insert(id, (su, desc));
        return athlete;
    }

    async fn unregister(&mut self, athlete: &mut Athlete) {
        let lock = self.state.lock().await;
        self.roster.remove(&athlete.id);
        match *lock {
            RefereeState::None | RefereeState::SavegameStage => {}
            RefereeState::PauseStage => {
                athlete.confirm_ready(ConfirmState::Drop);
            }
        }
    }

    pub async fn pause_and_wait_confirmation(&mut self, reason: PauseReason) -> Option<RefereeGuard<'_>> {
        let mut lock = self.state.lock().await;
        loop {
            if matches!(self.cmd_queue.1, PauseReason::None) {
                self.cmd_queue.1 = reason;
                break;
            } else {
                lock = self.cmd_queue.0.wait(lock).await;
            }
        }

        // step 1: wakeup all athletes by pause_chan <([{
        let _ = self.pause.send(reason);
        *lock = RefereeState::PauseStage;
        // }])>

        let cur_athletes = self.roster.len();
        let mut droped_athletes = 0;
        let mut new_athletes = 0;
        let mut i = cur_athletes;
        loop {
            drop(lock);

            // step 2: collect confirmation from athletes, NOLOCK! RefereeState::WaitPauseResponse <([{
            while i != 0 {
                let state = self.confirm.1.recv().await.unwrap();
                match reason {
                    // Force all athlets exit, ConfirmState::Drop only is sent by Athlete::async_drop().
                    PauseReason::Load => match state {
                        ConfirmState::Drop => {}
                        _ => {
                            error!("PauseReason::Load receives ConfirmState::Drop only.");
                        }
                    },
                    _ => {}
                }
                match state {
                    ConfirmState::Drop => {
                        droped_athletes += 1;
                    }
                    _ => {}
                }
                i = i - 1;
            }
            // }])>

            // step 3: relock to see whether there're new athletes during we release lock. <([{
            lock = self.state.lock().await;
            i = self.new_athlete_cnt;
            self.new_athlete_cnt = 0;
            if i == 0 {
                // exit with lock
                break;
            }
            new_athletes += i;
            // }])>
        }

        // step 4: locked, RefereeState::DoingJob, resume() will reset it to RefereeState::None <([{
        *lock = RefereeState::SavegameStage;
        info!(
            "PauseReason {:?}, new: {}, drop: {}, current {}",
            self.cmd_queue.1, new_athletes, droped_athletes, cur_athletes
        );
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
    }
    // }])>

    // Async version of std::fmt::Debug of Referee, is used in async fn.
    pub async fn async_debug_fmt(&self) {
        let lock = self.state.lock().await;
        println!("state: {:?}", (*lock));
        println!("athletes:");
        self.roster.iter().for_each(|(k, v)| println!("  {:?}-{:?}", k, v.1));
    }

    // Safety: get_athlets is designed for PauseReason::Save context, so no lock at all.
    pub fn get_athlets(&self) -> Box<dyn Iterator<Item = &Box<dyn SaveSerialize>> + '_ + Send> {
        Box::new(self.roster.iter().map(|i| &i.1.0))
    }
}

impl std::fmt::Debug for Referee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lock = self.state.blocking_lock();
        write!(f, "state: {:?}", (*lock))?;
        self.roster.iter().for_each(|(k, v)| {
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

    forced_pause_request: Option<PauseReason>,

    pause: Receiver<PauseReason>,
    confirm: UnboundedSender<ConfirmState>,
    resume: Receiver<()>,
}

impl Athlete {
    pub async fn wait_pause(&mut self) -> PauseReason {
        if self.forced_pause_request.is_some() {
            return self.forced_pause_request.take().unwrap();
        }
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
            // a trick to renew our test object -- Referee.
            forget(replace(&mut stump.referee, referee));
            guard
        }

        pub fn mystump_drop(guard: MutexGuard<'static, ()>) {
            unsafe {
                // a trick to renew our test object -- Referee.
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
            assert!(matches!((*referee_mut).state.get_mut(), RefereeState::SavegameStage));
            assert_eq!((*referee_mut).roster.len(), 0);
            for _ in (*referee_mut).get_athlets() {
                assert!(false);
            }
        }
        raii.async_drop().await;

        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.roster.len(), 0);

        mystump_drop(guard);
    }

    // The test forks four athletes:
    // - a: follows normal flow, wait_pause(), confirm_ready(), wait_resume().
    // - b: exits.
    // - c: try to register when PauseStage.
    // - d: try to register when SavegameStage.
    // RefereeState::WaitPauseResponse.
    #[tokio::test]
    async fn referee_pausereason_save() {
        let guard = mystump_new();
        let referee = &mut stump().referee;
        let referee_mut = referee as *mut Referee;
        let (t3, r3) = oneshot::channel();
        let (t4, r4) = oneshot::channel();

        // Athlete a, follows normal flow.
        let mut mock_a = MockSaveSerialize::new();
        mock_a.expect_save().once().returning(|| vec![1]);
        let mut a = referee.register(Box::new(mock_a), "a".to_string()).await;
        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.roster.len(), 1);
        let task_a = tokio::spawn(async move {
            let pr = a.wait_pause().await;
            assert!(matches!(pr, PauseReason::Save));
            let referee = &mut stump().referee;
            assert!(matches!(referee.state.get_mut(), RefereeState::PauseStage));
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
        assert_eq!(referee.roster.len(), 2);
        let task_b = tokio::spawn(async move {
            sleep(Duration::from_millis(100)).await;
            b.async_drop().await;
        });

        // Athlete c, try to register then exit (PauseStage).
        let mut mock_c = MockSaveSerialize::new();
        mock_c.expect_save().never();
        let task_c = tokio::spawn(async move {
            r3.await.unwrap();
            let referee = &mut stump().referee;
            let mut c = referee.register(Box::new(mock_c), "c".to_string()).await;
            assert!(matches!(c.forced_pause_request.unwrap(), PauseReason::Save));
            assert!(matches!(referee.state.get_mut(), RefereeState::PauseStage));
            assert!(matches!(referee.new_athlete_cnt, 1));
            assert_eq!(referee.roster.len(), 3);
            c.async_drop().await;
        });

        // Athlete d, try to register then exit (SavegameStage).
        let mut mock_d = MockSaveSerialize::new();
        mock_d.expect_save().never();
        let task_d = tokio::spawn(async move {
            r4.await.unwrap();
            let referee = &mut stump().referee;
            let mut d = referee.register(Box::new(mock_d), "d".to_string()).await;
            assert!(d.forced_pause_request.is_none());
            assert!(matches!(referee.state.get_mut(), RefereeState::SavegameStage));
            assert!(matches!(referee.new_athlete_cnt, 0));
            assert_eq!(referee.roster.len(), 2);
            d.async_drop().await;
        });

        // Referee.
        let mut raii = referee.pause_and_wait_confirmation(PauseReason::Save).await.unwrap();
        unsafe {
            assert!(matches!((*referee_mut).state.get_mut(), RefereeState::SavegameStage));
            assert_eq!((*referee_mut).roster.len(), 1);
            for i in (*referee_mut).get_athlets() {
                assert_eq!(i.save(), vec![1]);
            }
            t4.send(()).unwrap();
            sleep(Duration::from_millis(100)).await;
        }
        raii.async_drop().await;

        let _ = join!(task_a, task_b, task_c, task_d);
        assert!(matches!(*referee.state.get_mut(), RefereeState::None));
        assert_eq!(referee.roster.len(), 0);

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
        assert_eq!(referee.roster.len(), 1);
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
        assert_eq!(referee.roster.len(), 0);

        mystump_drop(guard);
    }
}
// }])>
