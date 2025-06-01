// vim: foldmarker=<([{,}])> foldmethod=marker
// <([{
use std::any::type_name;
use std::cmp::max;
use std::path::PathBuf;
use std::{error::Error, f32::consts::PI, sync::OnceLock};

use bevy::color::palettes::css::*;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::ecs::system::RunSystemOnce;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::tasks::block_on;
use clap::{Parser, Subcommand, crate_name};
use monolith_macro_utils::*;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, to_vec};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::Notify;
use tokio::time::interval;
use tokio::{
    fs::File,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        oneshot::{self, Sender},
    },
    time::{Duration, sleep},
};
use tokio_util::sync::CancellationToken;
use yunfengzh_monolith::prelude::*;
// }])>

// main() show:
// - mix bevy and tokio in main().
// main() <([{
#[tokio::main]
async fn main() {
    let mut app = App::new();
    app = stump_new(app, Some(my_developer_callback));
    app.add_plugins((
        DefaultPlugins.set(LogPlugin {
            level: Level::ERROR,
            filter: crate_name!().to_owned() + "=debug",
            custom_layer: |_| None,
        }),
        MeshPickingPlugin,
    ));
    // app.add_plugins(bevy_inspector_egui::quick::WorldInspectorPlugin::default());

    // The coroutine ISN'T in the charge of stump().tokio!!!, it's an exception.
    let mygame_future = tokio::spawn(async move {
        let stump = stump();
        stump.bevy_started().await;
        new_mygame().await;
        // stump_run() also forks some coroutine by stump.spawn().
        stump_run().await;
        mygame_run().await.unwrap();
        // let's wait those coroutines spawned by stump().spawn() to exit.
        stump.wait_tasktracker_exit().await;
        drop_mygame();
    });
    app.run();

    // bevy hasn't End schedule, so there's no `stump().bevy_ended().await`. Put later lines after
    // `app.run()` has the similar result.
    stump().exit_tokio();
    block_on(mygame_future).unwrap();
    stump_drop();
}
// }])>

// Show:
// 1. how to expand DeveloperBackend with user-defined commands.
// 2. how to use RecordBackend/ReplayBackend.
// my_developer_callback(). Return true to stop propagate user input to monolith. <([{
fn my_developer_callback(l: &String) -> bool {
    // <([{
    #[derive(Parser, Debug)]
    #[command(disable_help_flag = true, disable_help_subcommand = true)]
    struct Cli {
        #[command(subcommand)]
        command: SubCommands,
    }
    // }])>

    #[derive(Subcommand, Debug)]
    enum SubCommands {
        Exit,
        Record { id: usize },
        Dump { id: usize, file: PathBuf },
        Replay { file: PathBuf, id: usize },
        Help,
    }

    // <([{
    let l = "foo ".to_owned() + l;
    let l: Vec<_> = l.split_whitespace().collect();
    let l = Cli::try_parse_from(l);
    if l.is_err() {
        return false;
    }
    let l = l.unwrap();
    // }])>

    match l.command {
        SubCommands::Exit => {
            mygame().board.send(GameEvent::Exit).unwrap();
            return true;
        }
        SubCommands::Record { id } => {
            block_on(async move {
                let ret = stump().get_record_backend().toggle(id).await;
                println!("id record state is {ret}");
            });
            return true;
        }
        SubCommands::Dump { id, file } => {
            block_on(async move {
                let v = stump().get_record_backend().dump(id).await;
                println!("Dump event for {id}: {:?}", v);
                let v = to_vec(&v).unwrap();
                fs::write(file, v).await.unwrap();
            });
            return true;
        }
        SubCommands::Replay { file, id } => {
            block_on(async move {
                let v = fs::read(file).await.unwrap();
                let v = from_slice::<Tape>(&v).unwrap();
                println!("Replay event for {id}: {:?}", v);
                stump().get_replay_backend().play(v).await
            });
            return true;
        }
        SubCommands::Help => {
            println!("Help from mygame():");
            println!("record id");
            println!("dump id file");
            println!("replay file id");
            println!("exit");
        }
    }
    false
}
// }])>

// MyGame Framework Overview:
// - A global object of all elements of the game.
// - Sprite, Round1 and Round2. They're all athletes of Referee lock.
// - AStarServer: a virtual algo placeholder.
// - Board: does system GameEvent and implements a user-defined Backend.
// - UI: a ui layout.
// My Game <([{
static mut MYGAME: OnceLock<MyGame> = OnceLock::new();

fn mygame() -> &'static mut MyGame {
    unsafe { (*(&raw mut MYGAME)).get_mut().unwrap() }
}

#[derive(Debug)]
struct MyGame {
    astar: AStarServer,
    ui: UI,
    board: Board,

    sprite: Option<Sprite>,
    migrate_state: Notify,
    round1: Option<Round1>,
    round2: Option<Round2>,
}

async fn new_mygame() {
    let cmd_chan = stump().clone_tchan();

    // We need setup camera before UI::new().
    bevy_new!(cmd_chan <- NewCamera {
        transform: Transform::from_xyz(0.0, 6., 12.0).looking_at(Vec3::new(0., 1., 0.), Vec3::Y),
    });

    bevy_new!(cmd_chan <- NewPointLight {
        point_light: PointLight { shadows_enabled: true, intensity: 10_000_000., range: 100.0, ..default() },
        transform: Transform::from_xyz(8.0, 16.0, 8.0),
    });

    unsafe {
        (*(&raw mut MYGAME))
            .set(MyGame {
                astar: AStarServer::new(),
                ui: UI::new().await,
                board: Board::new(),
                sprite: None,
                migrate_state: Notify::new(),
                round1: None,
                round2: None,
            })
            .unwrap();
    }
}

fn drop_mygame() {
    unsafe {
        let _ = (*(&raw mut MYGAME)).take().unwrap();
    }
}

async fn mygame_run() -> Result<(), Box<dyn Error>> {
    let stump = stump();
    let mg = mygame();

    stump.spawn(async move {
        mygame().astar.run().await;
    });

    stump.spawn(async move {
        mygame().board.run().await.unwrap();
    });

    mg.round1 = Some(Round1::new());
    stump.spawn(async move {
        mygame().round1.as_mut().unwrap().run().await.unwrap();
    });

    mg.sprite = Some(Sprite::new(Vec3::new(1.0, 0.0, 0.0)).await);
    stump.spawn(async move {
        mygame().sprite.as_mut().unwrap().framework_run().await.unwrap();
    });

    stump.get_ct().cancelled().await;

    println!("my game over!");

    Ok(())
}
// }])>

// UI, show use Task::RunUserSystem <([{
#[derive(Debug)]
struct UI {
    reshape_button: Entity,
    load_button: Entity,
    save_button: Entity,
    pause_button: (Entity, bool),
}

#[derive(Component)]
struct MyPauseButton;

impl UI {
    async fn new() -> Self {
        let ret = setup_ui().await;
        Self { reshape_button: ret[0], load_button: ret[1], save_button: ret[2], pause_button: (ret[3], true) }
    }
}

fn create_button(parent: &mut RelatedSpawnerCommands<ChildOf>, title: &str) -> Entity {
    parent
        .spawn((
            (
                Button,
                Node {
                    width: Val::Px(250.0),
                    height: Val::Px(65.0),
                    margin: UiRect::all(Val::Px(20.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(5.0)),
                    ..default()
                },
                BorderColor(Color::BLACK),
                BackgroundColor(Color::srgb(0.15, 0.15, 0.15)),
            ),
            children![(
                Text::new(title),
                TextFont { font_size: 35.0, ..default() },
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
                TextShadow::default(),
            )],
        ))
        .id()
}

// setup_ui <([{
fn bevy_setup_ui(In(ptr): In<usize>, mut commands: Commands) {
    // Safety: Task memory rule.
    let sender = unsafe { usize2box::<Sender<Vec<Entity>>>(ptr) };

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(20.0),
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            },
            Pickable { should_block_lower: false, is_hoverable: false },
        ))
        .with_children(|parent| {
            parent
                .spawn(Node {
                    width: Val::Percent(100.0),
                    position_type: PositionType::Absolute,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::FlexStart,
                    ..default()
                })
                .with_children(|parent| {
                    let mut ret = vec![];

                    parent.spawn((
                        Text::new("Label Example"),
                        TextFont { font_size: 60.0, ..default() },
                        BackgroundColor(Color::from(GREEN)),
                        Label,
                    ));

                    ret.push(create_button(parent, "Reshape"));
                    ret.push(create_button(parent, "Load"));
                    ret.push(create_button(parent, "Save"));
                    let pause_button = create_button(parent, "Pause");
                    ret.push(pause_button);
                    parent.commands().entity(pause_button).insert(MyPauseButton);

                    sender.send(ret).unwrap();
                });
        });
}

async fn setup_ui() -> Vec<Entity> {
    let (resp_tx, resp_rx) = oneshot::channel::<Vec<Entity>>();
    let ptr = box2usize::<Sender<Vec<Entity>>>(resp_tx);
    stump()
        .clone_tchan()
        .send(TaskPayload::RunUserSystem {
            callback: |world: &mut World, ptr: usize| {
                world.run_system_once_with(bevy_setup_ui, ptr).unwrap();
            },
            ptr,
        })
        .unwrap();
    resp_rx.await.unwrap()
}
// }])>

// ui_change_pause_button <([{
fn bevy_change_pause_button(
    In(ptr): In<usize>,
    mut text_query: Query<&mut Text>,
    mypb: Query<&Children, With<MyPauseButton>>,
) -> Result<(), Box<dyn Error>> {
    let mut text = text_query.get_mut(mypb.single()?[0]).unwrap();
    // Safety: Task memory rule.
    let flag = unsafe { usize2box::<bool>(ptr) };
    if *flag {
        text.0 = "Pause".to_string();
    } else {
        text.0 = "Resume".to_string();
    }
    Ok(())
}

fn ui_change_pause_button(flag: bool) {
    let ptr = box2usize::<bool>(flag);
    stump()
        .clone_tchan()
        .send(TaskPayload::RunUserSystem {
            callback: |world: &mut World, ptr: usize| {
                world.run_system_once_with(bevy_change_pause_button, ptr).unwrap().unwrap();
            },
            ptr,
        })
        .unwrap();
}
// }])>

// ui_set_label <([{
fn bevy_set_label(In(ptr): In<usize>, mut query: Query<&mut Text, With<Label>>) {
    let Ok(mut text) = query.single_mut() else {
        return;
    };
    // Safety: Task memory rule.
    let str = unsafe { usize2box::<String>(ptr) };
    text.0 = *str;
}

fn ui_set_label(str: String) {
    let ptr = box2usize::<String>(str);
    stump()
        .clone_tchan()
        .send(TaskPayload::RunUserSystem {
            callback: |world: &mut World, ptr: usize| {
                world.run_system_once_with(bevy_set_label, ptr).unwrap();
            },
            ptr,
        })
        .unwrap();
}
// }])>
// }])>

// Round1 and Round2: round of fighting game, or game scenario/level <([{
// Round1 <([{
#[derive(Serialize, Deserialize, Debug)]
struct Round1 {
    life: u32,
}

impl Round1 {
    fn new() -> Self {
        Self { life: 10 }
    }

    async fn run(&mut self) -> Result<bool, Box<dyn Error>> {
        let mg = mygame();
        let stump = stump();
        let cancellation_token = stump.get_ct();

        println!("round1 start");
        ui_set_label("Round1".to_string());
        let mut athlete =
            stump.get_referee().register(Box::new(unsafe { &*(&raw const *self) }), "round1".to_string()).await;
        let mut interval = interval(Duration::from_secs(15));
        interval.tick().await;

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    println!("round1 be cancelled!");
                    athlete.async_drop().await;
                    return Ok(false);
                }
                _ = mg.migrate_state.notified() => { break; }
                _ = interval.tick() => { round1_to_round2().await?; }
                reason = athlete.wait_pause() => {
                    match reason {
                        PauseReason::Load => {
                            break;
                        }
                        _ => {
                            athlete.ready_and_wait_resume(ConfirmState::Cont).await;
                        }
                    }
                }
            }
        }

        athlete.async_drop().await;
        println!("exit from loop, bye round1");

        Ok(true)
    }
}

impl SaveSerialize for &Round1 {
    fn save(&self) -> Vec<u8> {
        (**self).save()
    }
}

impl SaveSerialize for Round1 {
    fn save(&self) -> Vec<u8> {
        SaveUtils::typename_data(to_vec(type_name::<Self>()).unwrap(), to_vec(&self).unwrap())
    }
}

impl LoadSerialize for Round1 {
    async fn ready_for_drop(&self) {
        todo!()
    }
}
// }])>

// Round2 <([{
#[derive(Serialize, Deserialize, Debug)]
struct Round2 {
    inherit_life: u32,
    life: u32,
}

impl SaveSerialize for &Round2 {
    fn save(&self) -> Vec<u8> {
        (**self).save()
    }
}

impl SaveSerialize for Round2 {
    fn save(&self) -> Vec<u8> {
        SaveUtils::typename_data(to_vec(type_name::<Self>()).unwrap(), to_vec(&self).unwrap())
    }
}

impl LoadSerialize for Round2 {
    async fn ready_for_drop(&self) {
        todo!()
    }
}

impl From<Round1> for Round2 {
    fn from(value: Round1) -> Self {
        Self { inherit_life: value.life, life: max(value.life + 10, 10) }
    }
}

impl Round2 {
    async fn run(&mut self) -> Result<bool, Box<dyn Error>> {
        let mg = mygame();
        let stump = stump();
        let cancellation_token = stump.get_ct();

        let mut athlete =
            stump.get_referee().register(Box::new(unsafe { &*(&raw const *self) }), "round2".to_string()).await;
        println!("round2 start");
        ui_set_label("Round2".to_string());

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    println!("round2 be cancelled!");
                    athlete.async_drop().await;
                    return Ok(false);
                }
                _ = mg.migrate_state.notified() => { break; }
                reason = athlete.wait_pause() => {
                    match reason {
                        PauseReason::Load => {
                            break;
                        }
                        _ => {
                            athlete.ready_and_wait_resume(ConfirmState::Cont).await;
                        }
                    }
                }
            }
        }

        athlete.async_drop().await;
        println!("exit from loop, bye round2");

        Ok(true)
    }
}
// }])>

async fn round1_to_round2() -> Result<bool, Box<dyn Error>> {
    let mg = mygame();
    println!("round1 >> round2");
    let r1 = mg.round1.take().unwrap();
    mg.round2 = Some(Round2::from(r1));
    mg.migrate_state.notify_one();
    let _ = stump().spawn(async move {
        mygame().round2.as_mut().unwrap().run().await.unwrap();
    });

    Ok(true)
}
// }])>

// Sprite shows how to:
// - hardcode GameEvent in Sprite::run().
// - setup foreend, initialize StdBackend.
// - response to Referee pause request.
// - response cancel-token to exit current coroute gracefully.
// - user-defined GameEvent and links it to StdBackend.
// Sprite <([{
// Sprite Concurrency Overview.
// 1. Sprite::run() coroutine, rw to struct Sprite.
// 2. Coroutines which send PauseReason::Save/AlgoPause request to Sprite, ro.
// Sprite::run() uses `select!(wait_pause() -> {})` branch to solve the conflict.
// 3. bevy_sprite_picked(): run from bevy system thread, ro to Sprite.entity/foreend.
// The system is associated with Sprite::entity, which is binding to Sprite itself, so it can
// always access Sprite data correctly.
// 4. mygame coroutine: new and drop Sprite.
// 5. Board coroutine: drop old Sprite then create new Sprite in GameEvent::Load event.
// Case 4 and 5 essentially makes sprite fields are initialized and destructed in different
// coroutine. So I introduce Sprite::framework_run() >> Sprite::run(). So the fields can be
// initialized and destructed in the same coroutine.
#[derive(Serialize, Deserialize, Debug)]
struct Sprite {
    pos: Transform,

    alternative: bool,

    #[serde(skip)]
    run_reach_end: Notify,

    #[serde(skip)]
    cancellation_token: Option<CancellationToken>,

    #[serde(skip)]
    cmd_chan: Option<TaskSender>,

    #[serde(skip)]
    foreend: Option<Foreend>,

    #[serde(skip)]
    athlete: Option<Athlete>,

    #[serde(skip)]
    entity: Option<Entity>,

    // Sprite has two shapes:
    // 1. cuboid, created every time.
    // 2. sphere, a handle of pre-registered into bevy.
    // When send NewEntity/SyncEntity to bevy, you can choose any of them.
    #[serde(skip)]
    sphere: Option<Handle<Mesh>>,
}

impl Sprite {
    async fn new(pos: Vec3) -> Self {
        let pos = Transform::from_translation(pos).with_rotation(Quat::from_rotation_x(-PI / 5.));
        Self {
            pos,
            alternative: false,
            run_reach_end: Notify::new(),
            cancellation_token: None,
            cmd_chan: None,
            foreend: None,
            athlete: None,
            entity: None,
            sphere: None,
        }
    }

    async fn framework_run(&mut self) -> Result<bool, Box<dyn Error + Send>> {
        let stump = stump();

        self.cancellation_token = Some(stump.get_ct());
        self.cmd_chan = Some(stump.clone_tchan());

        // Safety: self is registered as SaveSerialize object into stump.referee, is removed in
        // athlete.async_drop() in later line, so self is always available during stump.referee.
        self.athlete =
            Some(stump.get_referee().register(Box::new(unsafe { &*(&raw const *self) }), "sprite".to_string()).await);

        self.foreend = Some(stump.new_foreend("Sprite".to_string()).await);

        let cmd_chan = self.cmd_chan.as_ref().unwrap();

        // Here, we only set material of self.entity, its mesh is set in self.reshape().
        let material: StandardMaterial = Color::from(RED).into();
        self.entity = Some(bevy_new!(cmd_chan <- NewEntity {
            appearance: Appearance { mesh: M0::Bool { b: false }, material: M1::Material { material } },
            transform: self.pos,
        }));

        let mesh = Sphere::default();
        self.sphere = Some(bevy_new!(cmd_chan <- RegisterMesh { mesh: mesh.into() }));

        self.reshape();

        // Now, bevy_sprite_picked() is registered into bevy, it can access self readonly.
        self.sprite_more_customizations().await;

        let ret = self.run().await;

        let cmd_chan = self.cmd_chan.as_ref().unwrap();

        bevy_delete!(cmd_chan <- UnregisterMesh {
            handle: self.sphere.take().unwrap(),
        });

        // Now, bevy_sprite_picked() is removed from bevy by delete self.entity.
        bevy_delete!(cmd_chan <- DelEntity {
            entity: self.entity.take().unwrap(),
        });

        self.foreend.take().unwrap().async_drop().await;

        self.athlete.take().unwrap().async_drop().await;

        // We've reached end, let's notify GameEvent::Load branch of Board to switch old sprite to
        // new sprite. Rust will free old sprite memory. Note for developer:
        // 1. don't run any methods of Sprite after the method ends.
        // 2. put the initilization/destruction of complex fields in the method, so they can run
        //    in the same coroutine -- simplify design.
        self.run_reach_end.notify_one();

        ret
    }

    async fn run(&mut self) -> Result<bool, Box<dyn Error + Send>> {
        struct SpriteWrapper(*mut Sprite);
        unsafe impl Send for SpriteWrapper {}

        let self_raw = SpriteWrapper(&raw mut *self);
        println!("sprite start");

        let cancellation_token = self.cancellation_token.as_ref().unwrap();
        let cmd_chan = self.cmd_chan.as_ref().unwrap();
        let athlete = self.athlete.as_mut().unwrap();
        let foreend = self.foreend.as_mut().unwrap();

        // Sprite in village.
        let std_backend = foreend.create_std_backend().await;
        std_backend.register_group(GameEventGroup::Move).await;
        // Define our GameEvent and link them to new key-combination.
        let inventory = box2usize::<Inventory>(Inventory { sword: 1, meat: "beef".to_string() });
        std_backend
            .register("ii".to_owned(), GameEvent::UserDefined { id: UserEventID::Inventory.into(), payload: inventory })
            .await;
        std_backend
            .register(
                "<ctrl>v".to_owned(),
                GameEvent::UserDefined { id: UserEventID::Inventory.into(), payload: inventory },
            )
            .await;

        // Sprite in dungeon.
        let button_backend = foreend.create_button_backend().await;
        let reshape_button = &mygame().ui.reshape_button;
        button_backend.register(*reshape_button, GameEvent::Button { entity: reshape_button.to_bits() }).await;

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    println!("sprite be cancelled!");
                    return Ok(false);
                }
                reason = athlete.wait_pause() => {
                    match reason {
                        // PauseReason::Load forces athlete exit.
                        PauseReason::Load => {
                            break;
                        }
                        _ => {
                            // Get back self.pos from bevy or physical engine.
                            self.pos = bevy_exec_ret!(cmd_chan <- GetTransform {
                                entity: self.entity.unwrap(),
                            });
                            // Now, coroutine 1 is paused, coroutine 2 can access self exclusively.
                            athlete.ready_and_wait_resume(ConfirmState::Cont).await;
                            foreend.empty_events();
                        }
                    }
                }
                ge = foreend.wait_event() => {
                    // Safety: no memory corruption.
                    unsafe { if (*self_raw.0).do_event(cmd_chan, ge).await? == false { break; } }
                }
            }
        }

        println!("exit from loop, bye sprite");

        Ok(true)
    }

    async fn do_event(&mut self, cmd_chan: &TaskSender, ge: GameEvent) -> Result<bool, Box<dyn Error + Send>> {
        match ge {
            GameEvent::MoveUp | GameEvent::MoveDown | GameEvent::MoveLeft | GameEvent::MoveRight => {
                bevy_exec!(cmd_chan <- UpdateTransform {
                    entity: self.entity.unwrap(), event: ge, callback: |ge, transform| {
                        match ge {
                            GameEvent::MoveUp => {
                                transform.translation.y += 1.0;
                            }
                            GameEvent::MoveDown => {
                                transform.translation.y -= 1.0;
                            }
                            GameEvent::MoveLeft => {
                                transform.translation.x -= 1.0;
                            }
                            GameEvent::MoveRight => {
                                transform.translation.x += 1.0;
                            }
                            _ => { unreachable!();}
                        }
                    }
                });
            }
            GameEvent::Jump => {
                println!("<Jump> do nothing.");
            }
            GameEvent::Button { entity } => {
                let button = Entity::from_bits(entity);
                assert_eq!(button, mygame().ui.reshape_button);
                self.alternative = !self.alternative;
                self.reshape();
            }
            GameEvent::UserDefined { id, payload, .. } => match UserEventID::try_from(id).unwrap() {
                UserEventID::Inventory => {
                    // Safety: Task memory rule.
                    let inventory = unsafe { usize2box::<Inventory>(payload) };
                    println!("Great!, let us open our inventory");
                    println!("sword: {:?}, meat: {:?}", inventory.sword, inventory.meat);
                    // Here, memory is leak again to support re-open the inventory.
                    Box::leak(inventory);
                }
            },
            _ => {}
        }
        Ok(true)
    }

    fn reshape(&self) {
        let mesh = if self.alternative {
            M0::Handle { handle: self.sphere.clone().unwrap() }
        } else {
            M0::Mesh { mesh: Cuboid::default().into() }
        };

        let cmd_chan = self.cmd_chan.as_ref().unwrap();
        bevy_exec!(cmd_chan <- SyncEntity {
            entity: self.entity.unwrap(),
            transform: None,
            appearance: Some(Appearance {
                mesh,
                material: M1::Bool { b: false },
            }),
        });
    }

    async fn sprite_more_customizations(&self) {
        let ptr = box2usize::<Entity>(self.entity.unwrap());
        self.cmd_chan
            .as_ref()
            .unwrap()
            .send(TaskPayload::RunUserSystem {
                callback: |world: &mut World, ptr: usize| {
                    world.run_system_once_with(bevy_more_customizations, ptr).unwrap();
                },
                ptr,
            })
            .unwrap();
    }
}

// LoadSerialize and SaveSerialize <([{
impl SaveSerialize for Sprite {
    fn save(&self) -> Vec<u8> {
        // Storing TypeId instead of type_name is cheaper, but TypeId doesn't implement Serialize.
        SaveUtils::typename_data(to_vec(type_name::<Self>()).unwrap(), to_vec(&self).unwrap())
    }
}

impl SaveSerialize for &Sprite {
    fn save(&self) -> Vec<u8> {
        (**self).save()
    }
}

impl LoadSerialize for Sprite {
    async fn ready_for_drop(&self) {
        self.run_reach_end.notified().await;
    }
}
// }])>

// Bevy: More customizations on Sprite::entity <([{
fn bevy_sprite_picked(click: Trigger<Pointer<Click>>) {
    let mg = mygame();
    let sprite = mg.sprite.as_ref().unwrap();
    assert_eq!(click.target(), sprite.entity.unwrap());
    mg.astar.tx.send((sprite.foreend.as_ref().unwrap().get_id(), (Vec3::ZERO, Vec3::new(1.2, 2.3, 3.4)))).unwrap();
}

fn bevy_more_customizations(In(ptr): In<usize>, mut commands: Commands) {
    let entity = unsafe { usize2box::<Entity>(ptr) };
    let mut ec = commands.entity(*entity);
    ec.observe(bevy_sprite_picked);
}
// }])>

// UserEvent, GameEvent::UserDefined <([{
pub struct Inventory {
    pub sword: u32,
    pub meat: String,
}

#[derive(TryFromPrimitive, IntoPrimitive)]
#[repr(u32)]
enum UserEventID {
    Inventory = 1,
}
// }])>
// }])>

// AStarServer shows:
// - send GameEvent to other objects.
// - apply AlgoPause to referee.
// A star server <([{
#[derive(Debug)]
struct AStarServer {
    tx: UnboundedSender<(usize, (Vec3, Vec3))>,
    rx: UnboundedReceiver<(usize, (Vec3, Vec3))>,

    // Here, we use developer backend to send our GameEvents.
    redirect: DeveloperBackendSender,
}

impl AStarServer {
    fn new() -> Self {
        let (tx, rx) = unbounded_channel();
        Self { tx, rx, redirect: stump().get_developer_backend().create_user_backend() }
    }

    async fn run(&mut self) {
        let cancellation_token = stump().get_ct();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => { break; }
                ge = self.rx.recv() => self.do_event(ge.unwrap()).await,
            }
        }
    }

    async fn do_event(&self, ge: (usize, (Vec3, Vec3))) {
        println!("astar{:?}", ge);
        for i in AStarServer::a_star_algo(ge.1.0, ge.1.1).await {
            sleep(Duration::from_millis(200)).await;
            self.redirect.send((ge.0, i)).unwrap();
        }
    }

    async fn a_star_algo(_source: Vec3, _dest: Vec3) -> Vec<GameEvent> {
        let referee = stump().get_referee();
        let mut raii = referee.pause_and_wait_confirmation(PauseReason::AlgoPause).await.take().unwrap();
        // Copy data from game for later algo such as get sprite.pos by Task::GetTransform.
        raii.async_drop().await;
        // Now, run our algo and return the result.
        vec![GameEvent::MoveUp, GameEvent::MoveDown]
    }
}
// }])>

// Some GameEvent are system events, here shows how to response them.
// exit bevy from mygame internally.
// implement a User-defined Backend.
// send PauseReason::Load/Save to referee.
// switch from old sprite to new in PauseReason::Load arm
// Board: does system GameEvent and implements user-defined Backend <([{
// Board acts as a Backend which redirects GameEvent::Button to other GameEvent. If you hope the
// backend GameEvent can be recorded, see StdBackend::record_proxy.
#[derive(Debug)]
struct Board {
    redirect: Option<UnboundedSender<GameEvent>>,
}

impl Board {
    fn new() -> Self {
        Self { redirect: None }
    }

    fn send(&self, ge: GameEvent) -> Result<(), Box<dyn Error>> {
        self.redirect.as_ref().unwrap().send(ge)?;
        Ok(())
    }

    async fn run(&mut self) -> Result<bool, Box<dyn Error>> {
        let mg = mygame();
        let cancellation_token = stump().get_ct();

        let stump = stump();
        let mut foreend = stump.new_foreend("Board".to_string()).await;
        let std_backend = foreend.create_std_backend().await;
        std_backend.register("q".to_owned(), GameEvent::Exit).await;

        let button_backend = foreend.create_button_backend().await;
        let ui = &mg.ui;
        let load_button = &ui.load_button;
        button_backend.register(*load_button, GameEvent::Button { entity: load_button.to_bits() }).await;
        let save_button = &ui.save_button;
        button_backend.register(*save_button, GameEvent::Button { entity: save_button.to_bits() }).await;
        let pause_button = &ui.pause_button.0;
        button_backend.register(*pause_button, GameEvent::Button { entity: pause_button.to_bits() }).await;

        self.redirect = Some(foreend.create_user_backend());

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    println!("system event coroute be cancelled!");
                    return Ok(false);
                }
                ge = { foreend.wait_event() } => { if self.do_event(ge).await? == false { break; } }
            }
        }

        Ok(true)
    }

    async fn do_event(&self, ge: GameEvent) -> Result<bool, Box<dyn Error>> {
        let stump = stump();
        let mg = mygame();
        println!("board event {:?}", ge);
        let referee = stump.get_referee();
        match ge {
            GameEvent::Pause => {
                referee.pause_and_wait_confirmation(PauseReason::Pause).await;
                // do sth. here
            }
            GameEvent::Resume => {
                referee.resume().await;
            }
            GameEvent::Load { file_name } => {
                // First of all, caller need make sure no new athlete registered to referee.
                let file_name = unsafe { usize2box::<String>(file_name) };
                let mut raii = referee.pause_and_wait_confirmation(PauseReason::Load).await.take().unwrap();
                raii.async_drop().await;
                let mut buf = Vec::new();
                BufReader::new(File::open(*file_name).await?).read_to_end(&mut buf).await?;
                let total_len = buf.len();
                let mut pos = 0;
                while pos < total_len {
                    let ((name_start, name_end), (data_start, data_end)) = SaveUtils::get_typename_data(&buf, pos);
                    pos = data_end;
                    let name = from_slice::<&str>(&buf[name_start..name_end]).unwrap();
                    if name == type_name::<Round1>() {
                        mg.round1.take();
                        mg.round1 = Some(from_slice::<Round1>(&buf[data_start..data_end]).unwrap());
                        stump.spawn(async move {
                            mygame().round1.as_mut().unwrap().run().await.unwrap();
                        });
                    } else if name == type_name::<Round2>() {
                        mg.round2.take();
                        mg.round2 = Some(from_slice::<Round2>(&buf[data_start..data_end]).unwrap());
                        stump.spawn(async move {
                            mygame().round2.as_mut().unwrap().run().await.unwrap();
                        });
                    } else if name == type_name::<Sprite>() {
                        let old = mg.sprite.take().unwrap();
                        old.ready_for_drop().await;
                        drop(old);
                        mg.sprite = Some(from_slice::<Sprite>(&buf[data_start..data_end]).unwrap());
                        stump.spawn(async move {
                            mygame().sprite.as_mut().unwrap().framework_run().await.unwrap();
                        });
                    }
                }
            }
            GameEvent::Save { file_name } => {
                // First of all, caller need make sure no new athlete registered to referee.
                let file_name = unsafe { usize2box::<String>(file_name) };
                let mut buf = BufWriter::new(File::create(*file_name).await?);

                struct RefereeWrapper(*mut Referee);
                unsafe impl Send for RefereeWrapper {}
                let referee_const = RefereeWrapper(referee);
                let mut raii = referee.pause_and_wait_confirmation(PauseReason::Save).await.take().unwrap();
                // Safety: raii only holds a mutable pointer of referee, not use it at all. So
                // referee can be borrowed again.
                unsafe {
                    for i in (*referee_const.0).get_athlets() {
                        buf.write(i.save().as_slice()).await?;
                    }
                }
                raii.async_drop().await;

                buf.flush().await?;
            }
            GameEvent::Exit => {
                stump.exit_bevy();
                return Ok(false);
            }
            GameEvent::Button { entity, .. } => {
                let entity = Entity::from_bits(entity);
                let ui = &mut mg.ui;
                let mybackend = self.redirect.as_ref().unwrap();
                if entity == ui.save_button {
                    mybackend.send(GameEvent::Save { file_name: box2usize::<String>("savefile".to_string()) })?;
                } else if entity == ui.load_button {
                    mybackend.send(GameEvent::Load { file_name: box2usize::<String>("savefile".to_string()) })?;
                } else if entity == ui.pause_button.0 {
                    let flag = ui.pause_button.1;
                    mybackend.send(if flag { GameEvent::Pause } else { GameEvent::Resume })?;
                    ui_change_pause_button(!flag);
                    ui.pause_button.1 = !flag;
                }
            }
            _ => {}
        }
        Ok(true)
    }
}
// }])>
