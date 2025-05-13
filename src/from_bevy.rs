// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! from_bevy module defines [GameEvent] and uses classic [Foreend]-Backends (1 vs N) model to pass
//! GameEvent from bevy by tokio::unbounded_channel.
//!
//! ## Backend Overview
//! There're several Backends available to developer.
//! 1. [StdBackend] translates keys to GameEvent.
//! 2. [ButtonBackend] sends [GameEvent::Button] when a button is pressed.
//! 3. [DeveloperBackend] lets developer inject GameEvent directly into game, observe some
//!    variables etc.
//! 4. [RecordBackend]/[ReplayBackend] can record and replay GameEvent -- game recording feature.
//!    Note, due to schedule uncertainty, it's impossible to replay a game precisely!
//!
//! ### UserDefined Backend and SendFail-Drop Rule
//! Developer can define their own Backends by [Foreend::create_user_backend] and
//! [DeveloperBackend::create_user_backend].
//!
//! SendFail-Drop rule is when a backend fails to send data to the corresponding `Foreend`,
//! it's the time to recycle inner resource, goto `std_backend_broadcast` for more.

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    prelude::*,
};
use clap::{Parser, Subcommand};
use mockall::automock;
use serde::{Deserialize, Serialize};
use std::{
    cmp::min,
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{
        Mutex,
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        watch,
    },
    time::{Duration, Instant, Interval, interval},
};

use crate::stump::stump;
// }])>

// GameEvent <([{
/// GameEvent is a logic event which is sent from `Backend` to `Foreend`, it's recommended to
/// integrate them into your code because it makes user can remap keyboard shortcut conveniently.
///
/// ## Glimpse of GameEvent
/// GameEvent is consist of:
/// 1. basic-movement such as [GameEvent::MoveLeft].
/// 2. system events such as [GameEvent::Exit].
/// 3. button is pressed [GameEvent::Button].
/// 4. user-defined [GameEvent::UserDefined].
///
/// ## Extension of GameEvent, [GameEvent::UserDefined].
/// It's done by `num_enum` crate.
///
/// ```plaintext
/// #[derive(TryFromPrimitive, IntoPrimitive)]
/// #[repr(u32)]
/// enum UserEventID {
///     A = 1,
/// }
///
/// pub struct A {
///     ...
/// }
/// ```
///
/// Then developer can fill `GameEvent::UserDefined {id, payload: box2usize(A { ... })}`.
///
/// ## Safety of GameEvent
/// When GameEvent need to include a ptr, it should be stored as usize_t. It's recommended to
/// allocate ptr by Backend and free ptr by Foreend user. There're auxiliary functions
/// [box2usize](crate::utils::box2usize) and [usize2box](crate::utils::usize2box) for it.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
pub enum GameEvent {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Jump,

    // System event.
    Load { file_name: usize },
    Save { file_name: usize },
    Pause,
    Resume,
    Exit,

    Button { entity: u64 },

    UserDefined { id: u32, payload: usize },
}

/// Built-in game event group.
pub enum GameEventGroup {
    Move,
}
// }])>

// foreend <([{
/// Foreend is used by to accept GameEvent from Backends.
#[derive(Debug)]
pub struct Foreend {
    id: usize,
    sender: UnboundedSender<GameEvent>,
    receiver: UnboundedReceiver<GameEvent>,
}

impl Foreend {
    pub fn get_id(&self) -> usize {
        self.id
    }

    pub async fn new(desc: String) -> Self {
        let (sender, receiver) = unbounded_channel();
        let id = stump().developer_backend.register(desc, sender.clone()).await;
        Self { id, sender, receiver }
    }

    pub async fn create_std_backend(&self) -> &mut StdBackend {
        let mut lock = stump().std_backend.lock().await;
        lock.push(Box::new(StdBackend::new(self.create_user_backend(), self.id).await));
        let ret = lock.last_mut().unwrap();
        // Safety: we can safe return a StdBackend reference to our caller, here, ret is a Box<_>.
        // Caller is free to drop StdBackend, see create_user_backend().
        //
        // Here the lifetime of returned StdBackend is enlarged to self.
        unsafe { std::mem::transmute::<&'_ mut StdBackend, &'_ mut StdBackend>(ret) }
    }

    pub async fn create_button_backend(&self) -> &mut ButtonBackend {
        let mut lock = stump().button_backend.lock().await;
        lock.push(Box::new(ButtonBackend::new(self.create_user_backend())));
        let ret = lock.last_mut().unwrap();
        // Safety: see create_std_backend.
        unsafe { std::mem::transmute::<&'_ mut ButtonBackend, &'_ mut ButtonBackend>(ret) }
    }

    // TODO: automatically free.
    pub fn create_user_backend(&self) -> UnboundedSender<GameEvent> {
        self.sender.clone()
    }

    pub async fn wait_event(&mut self) -> GameEvent {
        self.receiver.recv().await.unwrap()
    }

    pub fn empty_events(&mut self) {
        while self.receiver.try_recv().is_ok() {}
    }

    pub async fn async_drop(&mut self) {
        stump().developer_backend.unregister(self.id).await;
    }
}
// }])>

// std backend <([{
/// ## Syntax of key-combination
/// StdBackend supports a simple syntax to describe key-combination when [StdBackend::register].
/// 1. `ii` double-click i keys.
/// 2. `<ctrl>i`: ctrl + i.
///
/// ## Built-in [GameEventGroup]
/// Currently, some basic movement keys are supported -- [StdBackend::register_group].
#[derive(Debug)]
pub struct StdBackend {
    key_to_game_event: Mutex<HashMap<String, GameEvent>>,
    sender: UnboundedSender<GameEvent>,
    record_proxy: RecordProxy,
}

impl StdBackend {
    async fn new(sender: UnboundedSender<GameEvent>, id: usize) -> Self {
        Self {
            key_to_game_event: Mutex::new(HashMap::new()),
            sender,
            record_proxy: stump().record_backend.register(id).await,
        }
    }

    pub async fn register_group(&mut self, geg: GameEventGroup) {
        let mut lock = self.key_to_game_event.lock().await;
        match geg {
            GameEventGroup::Move => {
                (*lock).insert("s".to_string(), GameEvent::MoveDown);
                (*lock).insert("w".to_string(), GameEvent::MoveUp);
                (*lock).insert("a".to_string(), GameEvent::MoveLeft);
                (*lock).insert("d".to_string(), GameEvent::MoveRight);
                (*lock).insert("<space>".to_string(), GameEvent::Jump);
            }
        }
    }

    pub async fn register(&mut self, key: String, ge: GameEvent) {
        let mut lock = self.key_to_game_event.lock().await;
        (*lock).insert(key, ge);
    }

    fn broadcast(&mut self, key: &str) -> bool {
        let lock = self.key_to_game_event.blocking_lock();
        if let Some(ge) = (*lock).get(key) {
            self.record_proxy.send((*ge).clone());
            return self.sender.send((*ge).clone()).is_ok();
        }
        return true;
    }
}

fn std_backend_broadcast(key_combination: &String) {
    let mut lock = stump().std_backend.blocking_lock();
    lock.retain_mut(|item| item.broadcast(&key_combination));
}

fn std_backend_callback(mut double_click: Local<(Option<Instant>, String)>, mut er: EventReader<KeyboardInput>) {
    for i in er.read() {
        if i.state == ButtonState::Released {
            info!("{:?}", i);
            let cur = match &i.logical_key {
                Key::Character(c) => c.as_str(),
                Key::Space => "<space>",
                Key::Control => "<ctrl>",
                _ => "",
            }
            .to_owned();
            let now = Instant::now();
            let diff = now.duration_since(double_click.0.unwrap_or(now));
            let mut key_combination = cur.clone();
            if diff <= Duration::from_millis(200) {
                let prev = &double_click.1;
                info!("double click!!!!{:?}-{:?}", prev, cur);
                if cur == "<ctrl>" {
                    key_combination = cur.clone() + prev;
                } else {
                    key_combination = prev.to_owned() + &cur;
                }
            }

            if key_combination != "" {
                std_backend_broadcast(&key_combination);
            }

            double_click.0 = Some(now);
            double_click.1 = cur;
        }
    }
}
// }])>

// button backend <([{
/// Send [GameEvent::Button] when a button is pressed.
#[derive(Debug)]
pub struct ButtonBackend {
    entity_to_game_event: Mutex<HashMap<Entity, GameEvent>>,
    sender: UnboundedSender<GameEvent>,
}

impl ButtonBackend {
    fn new(sender: UnboundedSender<GameEvent>) -> Self {
        Self { entity_to_game_event: Mutex::new(HashMap::new()), sender }
    }

    pub async fn register(&mut self, entity: Entity, ge: GameEvent) {
        let mut lock = self.entity_to_game_event.lock().await;
        (*lock).insert(entity, ge);
    }

    fn broadcast(&self, entity: &Entity) -> bool {
        let lock = self.entity_to_game_event.blocking_lock();
        if let Some(ge) = (*lock).get(entity) {
            return self.sender.send((*ge).clone()).is_ok();
        }
        return true;
    }
}

fn button_callback(mut interaction_query: Query<(&Interaction, Entity), (Changed<Interaction>, With<Button>)>) {
    for (interaction, entity) in &mut interaction_query {
        match *interaction {
            Interaction::Pressed => {
                info!("{:?} button is pressed", entity);
                let mut lock = stump().button_backend.blocking_lock();
                lock.retain(|item| item.broadcast(&entity));
            }
            _ => {}
        }
    }
}
// }])>

// deleveloper backend <([{
pub type DeveloperBackendSender = UnboundedSender<(usize, GameEvent)>;
pub(crate) type DeveloperBackendCallback = fn(&String) -> bool;

/// DeveloperBackend reuses current console as a debug console to observe inner variables, inject
/// GameEvent etc, type `help` in the console.
///
/// ## Extension of developer console
/// Pass user-defined callback to [stump_new](crate::stump::stump_new) to append more commands from
/// developer. Pass `None` to disable developer console.
///
/// ## DeveloperBackend is the core components
/// All `Foreened` are registered to DeveloperBackend to get a unique id.
///
/// ## Another way to send GameEvent
/// [DeveloperBackend::create_user_backend] provides another ways to inject GameEvent.
#[derive(Debug)]
pub struct DeveloperBackend {
    counter: AtomicUsize,
    pool: Mutex<HashMap<usize, (String, UnboundedSender<GameEvent>)>>,
    sender: DeveloperBackendSender,
    receiver: UnboundedReceiver<(usize, GameEvent)>,
    pub(crate) user_callback: Option<DeveloperBackendCallback>,
}

#[automock]
impl DeveloperBackend {
    pub fn create_user_backend(&self) -> DeveloperBackendSender {
        self.sender.clone()
    }

    pub(crate) fn new(user_callback: Option<DeveloperBackendCallback>) -> Self {
        let (sender, receiver) = unbounded_channel();
        Self { counter: AtomicUsize::new(1), pool: Mutex::new(HashMap::new()), sender, receiver, user_callback }
    }

    pub(crate) async fn fore_run(&self) {
        let cancellation_token = stump().get_ct();

        let mut lines = BufReader::new(tokio::io::stdin()).lines();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => { break; }
                l = lines.next_line() => { let l = l.unwrap().unwrap(); self.exec(l).await; }
            }
        }
    }

    async fn exec(&self, l: String) {
        if l.trim().is_empty() || self.user_callback.is_none() || (self.user_callback.unwrap())(&l) {
            return;
        }

        // <([{
        let stump = stump();

        #[derive(Parser, Debug)]
        #[command(disable_help_flag = true, disable_help_subcommand = true)]
        struct Cli {
            #[command(subcommand)]
            command: SubCommands,
        }
        // }])>

        #[derive(Subcommand, Debug)]
        enum SubCommands {
            Let { id: usize, action: String },
            List { action: String },
            Help,
        }

        // <([{
        let l = "foo ".to_owned() + &l;
        let l: Vec<_> = l.split_whitespace().collect();
        let l = Cli::try_parse_from(l);
        if l.is_err() {
            println!("illegal syntax");
            return;
        }
        let l = l.unwrap();
        // }])>
        match l.command {
            SubCommands::Let { id, action } => {
                if action == "moveleft" || action == "left" {
                    self.sender.send((id, GameEvent::MoveLeft)).unwrap();
                    return;
                } else if action == "moveright" || action == "right" {
                    self.sender.send((id, GameEvent::MoveRight)).unwrap();
                    return;
                }
            }
            SubCommands::List { action } => {
                if action == "foreend" {
                    let lock = self.pool.lock().await;
                    (*lock).iter().for_each(|(k, v)| println!("{:?}-{:?}", k, v.0));
                    return;
                } else if action == "referee" {
                    stump.referee.async_debug_fmt().await;
                    return;
                }
            }
            SubCommands::Help => {
                println!("let id moveleft/left");
                println!("let id moveright/right");
                println!("list foreend/referee");
                return;
            }
        }
    }

    pub(crate) async fn back_run(&mut self) {
        let cancellation_token = stump().get_ct();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => { break; }
                Some((id, ge)) = self.receiver.recv() => { self.redirect(id, ge).await; }
            }
        }
    }

    async fn redirect(&mut self, id: usize, ge: GameEvent) {
        let mut lock = self.pool.lock().await;
        let Some(fe) = (*lock).get(&id) else {
            return;
        };
        if fe.1.send(ge).is_err() {
            (*lock).remove(&id);
        }
    }

    async fn register(&mut self, desc: String, fe: UnboundedSender<GameEvent>) -> usize {
        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        let mut lock = self.pool.lock().await;
        (*lock).insert(id, (desc, fe));
        id
    }

    async fn unregister(&mut self, id: usize) {
        let mut lock = self.pool.lock().await;
        (*lock).remove(&id);
    }
}

impl Drop for DeveloperBackend {
    fn drop(&mut self) {
        if self.user_callback.is_some() {
            println!("!!!!!press enter to exit to your console!!!!!");
            println!("!!!!!see https://docs.rs/tokio/latest/tokio/io/struct.Stdin.html for more!!!!!");
        }
    }
}

// }])>

// record/replay backend <([{

#[derive(Debug, Serialize, Deserialize)]
struct RecordUnit {
    ge: GameEvent,
    interval: u64,
}

impl RecordUnit {
    fn new(ge: GameEvent, interval: Duration) -> Self {
        Self { ge, interval: interval.as_millis() as u64 }
    }
}

/// Record format, support serialization.
#[derive(Debug, Serialize, Deserialize)]
pub struct Tape {
    owner: usize,
    data: VecDeque<RecordUnit>,
}

type RecordClientChan = (watch::Receiver<Option<Instant>>, UnboundedSender<RecordUnit>);
type RecordServerChan = (watch::Sender<Option<Instant>>, UnboundedReceiver<RecordUnit>);

/// Record proxy, accept GameEvent, you can also enable recording any time.
#[derive(Debug)]
pub struct RecordProxy {
    chan: RecordClientChan,
    enabled: Option<Instant>,
}

impl RecordProxy {
    pub fn send(&mut self, ge: GameEvent) {
        let ctrl = &mut self.chan.0;
        if ctrl.has_changed().unwrap() {
            ctrl.mark_unchanged();
            self.enabled = *ctrl.borrow();
        }
        if self.enabled.is_some() {
            let now = Instant::now();
            self.chan.1.send(RecordUnit::new(ge, now - self.enabled.unwrap())).unwrap();
            self.enabled = Some(now);
        }
    }
}

// record backend <([{
/// To record a GameEvent to RecordBackend:
///
/// 1. [RecordBackend::register] to get a [RecordProxy]
/// 2. [RecordProxy::send] to send your GameEvent.
/// 3. [RecordBackend::toggle] to enable recording or not.
/// 4. [RecordBackend::dump] to get and empty GameEvent for a specific Foreend id.
#[derive(Debug)]
pub struct RecordBackend {
    lock: Mutex<()>,
    clients: HashMap<usize, (bool, RecordServerChan)>,
}

impl RecordBackend {
    pub(crate) fn new() -> Self {
        Self { lock: Mutex::new(()), clients: HashMap::new() }
    }

    pub async fn register(&mut self, id: usize) -> RecordProxy {
        let _ = self.lock.lock().await;
        let (s, r) = unbounded_channel();
        let ctrl = watch::Sender::new(None);
        let mut ctrl_r = ctrl.subscribe();
        ctrl_r.mark_unchanged();
        self.clients.insert(id, (false, (ctrl, r)));
        RecordProxy { chan: (ctrl_r, s), enabled: None }
    }

    pub async fn dump(&mut self, id: usize) -> Tape {
        let _ = self.lock.lock().await;
        let mut ret = Tape { owner: id, data: VecDeque::new() };
        let Some(client) = self.clients.get_mut(&id) else {
            return ret;
        };
        while let Ok(unit) = client.1.1.try_recv() {
            ret.data.push_back(unit);
        }
        ret
    }

    pub async fn toggle(&mut self, id: usize) -> bool {
        let _ = self.lock.lock().await;
        let Some(client) = self.clients.get_mut(&id) else {
            return false;
        };
        client.0 = !client.0;
        let mut ret = client.0;
        let tmp = { if client.0 { Some(Instant::now()) } else { None } };
        if client.1.0.send(tmp).is_err() {
            self.clients.remove(&id);
            ret = false;
        }
        ret
    }
}
// }])>

// replay backend <([{
#[derive(Debug)]
struct VTape(Vec<Tape>);

impl VTape {
    fn new() -> Self {
        Self { 0: Vec::new() }
    }

    fn calculate_min(&mut self) -> ((usize, u64), Instant, Interval) {
        let head_vec: Vec<_> = self.0.iter().enumerate().map(|i| (i.0, i.1.data.front().unwrap().interval)).collect();

        let min = head_vec.into_iter().min_by(|x, y| x.1.cmp(&y.1));
        let min = min.unwrap_or((usize::MAX, u64::MAX));

        (min, Instant::now(), interval(Duration::from_millis(min.1)))
    }

    fn update_elapsed_time(&mut self, elapsed_time: u64) {
        self.0.iter_mut().for_each(|i| i.data.front_mut().unwrap().interval -= elapsed_time);
    }

    fn remove_min_elem(&mut self, idx: usize) -> (usize, GameEvent) {
        let tape = self.0.get_mut(idx).unwrap();
        let id = tape.owner;
        let ge = tape.data.pop_front().unwrap().ge;
        if tape.data.is_empty() {
            self.0.swap_remove(idx);
        }
        (id, ge)
    }
}

/// Replay user events.
#[derive(Debug)]
pub struct ReplayBackend {
    lock: Mutex<()>,
    new_tape: (UnboundedSender<Tape>, UnboundedReceiver<Tape>),
}

impl ReplayBackend {
    pub(crate) fn new() -> Self {
        let (s, r) = unbounded_channel();
        Self { lock: Mutex::new(()), new_tape: (s, r) }
    }

    pub async fn play(&mut self, tape: Tape) {
        let _ = self.lock.lock().await;
        if tape.data.is_empty() {
            return;
        }
        self.new_tape.0.send(tape).unwrap();
    }

    pub(crate) async fn run(&mut self) {
        #[cfg(test)]
        use crate::from_bevy::tests::replaybackend_boundaryclass::stump;

        let stump = stump();
        let cancellation_token = stump.get_ct();
        let mut vecs = VTape::new();

        loop {
            // dbg!(&vecs);
            let (current, prev, mut interval) = vecs.calculate_min();
            interval.tick().await; // skip immediate tick.

            tokio::select! {
                _ = cancellation_token.cancelled() => { break; }
                Some(tape) = self.new_tape.1.recv() => {
                    let first = tape.data.front().unwrap().interval;
                    if first < current.1 {
                        vecs.update_elapsed_time(min((Instant::now() - prev).as_millis() as u64, current.1));
                    }
                    vecs.0.push(tape);
                },
                _ = interval.tick() => {
                    vecs.update_elapsed_time(current.1);
                    let (id, ge) = vecs.remove_min_elem(current.0);
                    stump.developer_backend.redirect(id, ge).await;
                },
            }
        }
    }
}
// }])>
// }])>

fn startup() {
    stump().bevy_start();
}

pub(crate) fn from_bevy_init(mut app: App) -> App {
    app.add_systems(Update, std_backend_callback).add_systems(Update, button_callback);
    app.add_systems(Startup, startup);
    app
}

// mod tests <([{
#[cfg(test)]
mod tests {
    pub(crate) mod replaybackend_boundaryclass {
        use std::{
            mem::{forget, replace},
            ptr::drop_in_place,
            sync::{MutexGuard, OnceLock},
        };

        use tokio_util::sync::CancellationToken;

        use super::*;

        static mut P: OnceLock<MockStump> = OnceLock::new();

        pub(crate) struct MockStump {
            pub(crate) developer_backend: MockDeveloperBackend,
            pub(crate) replay_backend: ReplayBackend,

            // `cargo test' will run tests in the same process, share the same static variables.
            // Since my test uses stump() global function, so all tests must be run sequentially to
            // avoid race.
            sequential_mutex: std::sync::Mutex<()>,
        }

        impl MockStump {
            pub fn get_ct(&self) -> CancellationToken {
                CancellationToken::new()
            }
        }

        pub fn stump() -> &'static mut MockStump {
            unsafe { (*(&raw mut P)).get_mut().unwrap() }
        }

        pub fn mystump_new(db: MockDeveloperBackend) -> MutexGuard<'static, ()> {
            unsafe {
                (*(&raw mut P)).get_or_init(|| MockStump {
                    developer_backend: MockDeveloperBackend::default(),
                    replay_backend: ReplayBackend::new(),
                    sequential_mutex: std::sync::Mutex::new(()),
                });
                let stump = stump();
                let guard = stump.sequential_mutex.lock().unwrap();
                forget(replace(&mut stump.developer_backend, db));
                // Later lines to fulfill later statement:
                // `stump.replay_backend = ReplayBackend::new();`
                // Note original statement will implicitly calls stump.replay_backend.drop() then crashed
                let rb = ReplayBackend::new();
                forget(replace(&mut stump.replay_backend, rb));
                guard
            }
        }

        // When running tests, P's drop doesn't be called!
        pub fn mystump_drop(guard: MutexGuard<'static, ()>) {
            unsafe {
                // drop P.developer_backend so MockDeveloperBackend can run its check.
                drop_in_place(&mut stump().developer_backend);
            }
            drop(guard);
        }
    }

    use replaybackend_boundaryclass::{mystump_drop, mystump_new};
    use tokio::{task::JoinHandle, time::sleep};
    use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};

    use super::*;
    use replaybackend_boundaryclass::stump;

    fn replaybackend_run() -> (&'static mut ReplayBackend, JoinHandle<()>) {
        let rb = &mut stump().replay_backend;
        let handle = tokio::spawn(async move {
            stump().replay_backend.run().await;
        });
        (rb, handle)
    }

    // Test there's no tape at all.
    #[tokio::test]
    async fn replaybackend_play_empty() {
        let mut db = MockDeveloperBackend::default();
        db.expect_redirect().never();

        let guard = mystump_new(db);
        let (rb, run) = replaybackend_run();

        let tape = Tape { owner: 0, data: vec![].into() };
        rb.play(tape).await;

        let _ = tokio::time::timeout(Duration::from_secs(3), run).await;
        mystump_drop(guard);
    }

    // two tapes
    #[tokio::test]
    async fn replaybackend_play() {
        let mut prev = Instant::now();
        let (s, r) = unbounded_channel();
        let mut db = MockDeveloperBackend::default();
        db.expect_redirect().returning(move |id, evt| {
            let tmp = Instant::now();
            s.send((id, evt, tmp - prev)).unwrap();
            prev = tmp;
        });

        let guard = mystump_new(db);
        let (rb, run) = replaybackend_run();

        let data = vec![
            RecordUnit::new(GameEvent::MoveLeft, Duration::from_millis(100)),
            RecordUnit::new(GameEvent::MoveLeft, Duration::from_millis(200)),
        ]
        .into();
        let tape = Tape { owner: 0, data };
        rb.play(tape).await;

        let data = vec![
            RecordUnit::new(GameEvent::MoveRight, Duration::from_millis(110)),
            RecordUnit::new(GameEvent::MoveRight, Duration::from_millis(180)),
        ]
        .into();
        let tape = Tape { owner: 1, data };
        rb.play(tape).await;

        let _ = tokio::time::timeout(Duration::from_secs(3), run).await;
        mystump_drop(guard);

        // timeout_run() also makes s be closed, so r is closed, result is got.
        let result = UnboundedReceiverStream::new(r).collect::<Vec<_>>().await;
        let target = vec![
            (0, GameEvent::MoveLeft, 100),
            (1, GameEvent::MoveRight, 10),
            (1, GameEvent::MoveRight, 180),
            (0, GameEvent::MoveLeft, 10),
        ];
        assert_eq!(target.len(), result.len());
        for (idx, (id, evt, dur)) in result.iter().enumerate() {
            let j = target.get(idx).unwrap();
            assert!(*id == j.0 && *evt == j.1);
            let dur = dur.as_millis();
            assert!(dur > j.2);
        }
    }

    #[tokio::test]
    async fn replaybackend_play_intrude() {
        let mut prev = Instant::now();
        let (s, r) = unbounded_channel();
        let mut db = MockDeveloperBackend::default();
        db.expect_redirect().returning(move |id, evt| {
            let tmp = Instant::now();
            s.send((id, evt, tmp - prev)).unwrap();
            prev = tmp;
        });

        let guard = mystump_new(db);
        let (rb, run) = replaybackend_run();

        let data = vec![
            RecordUnit::new(GameEvent::MoveLeft, Duration::from_millis(200)),
            RecordUnit::new(GameEvent::MoveLeft, Duration::from_millis(200)),
        ]
        .into();
        let tape = Tape { owner: 0, data };
        rb.play(tape).await;

        sleep(Duration::from_millis(50)).await;
        let data = vec![
            RecordUnit::new(GameEvent::MoveRight, Duration::from_millis(100)),
            RecordUnit::new(GameEvent::MoveRight, Duration::from_millis(180)),
        ]
        .into();
        let tape = Tape { owner: 1, data };
        rb.play(tape).await;

        let _ = tokio::time::timeout(Duration::from_secs(3), run).await;
        mystump_drop(guard);

        // timeout_run() also makes s be closed, so r is closed, result is got.
        let result = UnboundedReceiverStream::new(r).collect::<Vec<_>>().await;
        let target = vec![
            (1, GameEvent::MoveRight, 100),
            (0, GameEvent::MoveLeft, 0),
            (1, GameEvent::MoveRight, 80),
            (0, GameEvent::MoveLeft, 20),
        ];
        assert_eq!(target.len(), result.len());
        for (idx, (id, evt, dur)) in result.iter().enumerate() {
            let j = target.get(idx).unwrap();
            assert!(*id == j.0 && *evt == j.1);
            let dur = dur.as_millis();
            assert!(dur > j.2);
        }
    }
}
// }])>
