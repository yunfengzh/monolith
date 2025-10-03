// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! to_bevy defines Task and pass them from tokio by tokio::unbounded_channel. It also includes
//! necessary bevy system to deal with them.
//!
//! ## Bevy System
//! `exclusive_system` is registered into bevy to execute Task from developer in `Update` schedule.
//! According to `bevy.src/examples/ecs/ecs_guid.rs`, the concurrency of the system is lower, but
//! it's the only way to include `World` as input parametere, so to call any system (including
//! user-defined system) by `world.run_system_once_with` I still choose it as to_bevy entry system.
//!
//! ## Task and Task Memory Rule
//! Task is consist of [TaskPayload] and [TaskAttachment]. TaskPayload is passed by
//! tokio::unbounded_channel from tokio to bevy, it has a light-weight size.
//! 1. caller allocates TaskAttachment and stores it to TaskPayload::ptr
//!    ([box2usize](crate::utils::box2usize)).
//! 2. pass TaskPayload to bevy system.
//! 3. frees attachment object in the corresponding system ([usize2box](crate::utils::usize2box))
//!
//! [TaskPayload] includes some basic tasks, to extend task use [TaskPayload::RunUserSystem].
//!
//! [TaskPayload::BevyExit] is used to exit bevy.

use bevy::{app::AppExit, ecs::system::RunSystemOnce, prelude::*};
use monolith_macro_utils::payload_to_attachment;
use tokio::sync::{
    mpsc::{
        UnboundedReceiver, UnboundedSender,
        error::{SendError, TryRecvError},
    },
    oneshot::Sender,
};

use crate::{from_bevy::GameEvent, stump::stump};
// }])>

// Task <([{
#[derive(Debug)]
pub struct TaskChannel(UnboundedSender<TaskPayload>, UnboundedReceiver<TaskPayload>);

impl TaskChannel {
    pub(crate) fn new(s: UnboundedSender<TaskPayload>, r: UnboundedReceiver<TaskPayload>) -> Self {
        Self(s, r)
    }
    pub fn get_sender(&self) -> UnboundedSender<TaskPayload> {
        self.0.clone()
    }

    pub fn send(&self, task: TaskPayload) -> Result<(), SendError<TaskPayload>> {
        self.0.send(task)
    }

    pub fn try_recv(&mut self) -> Result<TaskPayload, TryRecvError> {
        self.1.try_recv()
    }
}

pub enum M0 {
    Mesh { mesh: Mesh },
    Handle { handle: Handle<Mesh> },
    Bool { b: bool },
}

pub enum M1 {
    Material { material: StandardMaterial },
    Handle { handle: Handle<StandardMaterial> },
    Bool { b: bool },
}

pub struct Appearance {
    pub mesh: M0,
    pub material: M1,
}

pub enum TaskPayload {
    NewPointLight { ptr: usize },
    NewCamera { ptr: usize },

    NewEntity { ptr: usize },
    SyncEntity { ptr: usize },
    DelEntity { ptr: usize },

    GetTransform { ptr: usize },
    UpdateTransform { ptr: usize },

    RegisterMesh { ptr: usize },
    UnregisterMesh { ptr: usize },

    RunUserSystem { callback: fn(&mut World, usize), ptr: usize },

    BevyExit,
}

pub enum TaskAttachment {
    NewPointLight { point_light: PointLight, transform: Transform, resp: Sender<Entity> },
    NewCamera { transform: Transform, resp: Sender<Entity> },

    NewEntity { transform: Transform, appearance: Appearance, resp: Sender<Entity> },
    SyncEntity { entity: Entity, transform: Option<Transform>, appearance: Option<Appearance> },
    DelEntity { entity: Entity },

    GetTransform { entity: Entity, resp: Sender<Transform> },
    UpdateTransform { entity: Entity, event: GameEvent, callback: fn(GameEvent, &mut Transform) },

    RegisterMesh { mesh: Mesh, resp: Sender<Handle<Mesh>> },
    UnregisterMesh { handle: Handle<Mesh> },

    RunUserSystem { callback: fn(&mut World, usize), ptr: usize },

    BevyExit,
}
// }])>

// bevy task callback, do task <([{
fn exclusive_system(world: &mut World) {
    loop {
        let Ok(task) = stump().task_chan.1.try_recv() else {
            break;
        };
        match task {
            TaskPayload::RunUserSystem { callback, ptr } => {
                callback(world, ptr);
            }
            _ => {
                world.run_system_once_with(do_task, task).unwrap();
            }
        }
    }
}

fn do_task(
    In(task): In<TaskPayload>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<&mut Transform>,
    mut app_exit_events: MessageWriter<AppExit>,
) {
    match task {
        TaskPayload::NewPointLight { ptr } => {
            payload_to_attachment!(TaskAttachment::NewPointLight{ point_light, transform, resp} <- ptr);
            let entity = commands.spawn((point_light, transform)).id();
            resp.send(entity).unwrap();
        }
        TaskPayload::NewCamera { ptr } => {
            payload_to_attachment!(TaskAttachment::NewCamera { transform, resp } <- ptr);
            let entity = commands.spawn((Camera3d::default(), transform)).id();
            resp.send(entity).unwrap();
        }
        TaskPayload::NewEntity { ptr } => {
            payload_to_attachment!(TaskAttachment::NewEntity { appearance, transform, resp } <- ptr);
            let mesh = match appearance.mesh {
                M0::Mesh { mesh } => Mesh3d(meshes.add(mesh)),
                M0::Handle { handle } => Mesh3d(handle),
                M0::Bool { .. } => Mesh3d::default(),
            };
            let material = match appearance.material {
                M1::Material { material } => MeshMaterial3d(materials.add(material)),
                M1::Handle { handle } => MeshMaterial3d(handle),
                M1::Bool { .. } => MeshMaterial3d::default(),
            };
            let entity = commands.spawn((mesh, material, transform)).id();
            resp.send(entity).unwrap();
        }
        TaskPayload::SyncEntity { ptr } => {
            payload_to_attachment!(TaskAttachment::SyncEntity { entity, transform, appearance } <- ptr);
            handle_sync_pbr(&mut commands, entity, transform, appearance);
        }
        TaskPayload::DelEntity { ptr } => {
            payload_to_attachment!(TaskAttachment::DelEntity { entity } <- ptr);
            commands.entity(entity).despawn();
        }
        TaskPayload::RegisterMesh { ptr } => {
            payload_to_attachment!(TaskAttachment::RegisterMesh { mesh, resp } <- ptr);
            let handle = meshes.add(mesh);
            resp.send(handle).unwrap();
        }
        TaskPayload::UnregisterMesh { ptr } => {
            payload_to_attachment!(TaskAttachment::UnregisterMesh { handle } <- ptr);
            meshes.remove(&handle);
        }
        TaskPayload::BevyExit => {
            app_exit_events.write(AppExit::Success);
        }
        TaskPayload::GetTransform { ptr } => {
            payload_to_attachment!(TaskAttachment::GetTransform { entity, resp } <- ptr);
            if let Ok(transform) = query.get(entity) {
                resp.send(*transform).unwrap();
            }
        }
        TaskPayload::UpdateTransform { ptr } => {
            payload_to_attachment!(TaskAttachment::UpdateTransform { entity, event, callback } <- ptr);
            if let Ok(mut transform) = query.get_mut(entity) {
                callback(event, &mut transform);
            }
        }
        _ => {
            unreachable!();
        }
    }
}

fn handle_sync_pbr(
    commands: &mut Commands,
    entity: Entity,
    transform: Option<Transform>,
    appearance: Option<Appearance>,
) {
    struct WorldSyncProxy {
        entity: Entity,
        transform: Option<Transform>,
        appearance: Option<Appearance>,
    }
    impl Command for WorldSyncProxy {
        fn apply(self, world: &mut World) {
            let world_mut = &raw mut *world;
            if let Some(transform) = self.transform {
                let mut p = world.get_mut::<Transform>(self.entity).unwrap();
                *p = transform;
            }
            if let Some(appearance) = self.appearance {
                match appearance.mesh {
                    M0::Mesh { mesh } => {
                        let mut p = world.get_mut::<Mesh3d>(self.entity).unwrap();
                        // Safety: maybe it's safe, I'm not sure.
                        let mut meshes = unsafe { (*world_mut).get_resource_mut::<Assets<Mesh>>().unwrap() };
                        meshes.remove(&*p);
                        *p = Mesh3d(meshes.add(mesh));
                    }
                    M0::Handle { handle } => {
                        let mut p = world.get_mut::<Mesh3d>(self.entity).unwrap();
                        *p = Mesh3d(handle);
                    }
                    _ => {}
                }
                match appearance.material {
                    M1::Material { material } => {
                        let mut p = world.get_mut::<MeshMaterial3d<StandardMaterial>>(self.entity).unwrap();
                        // Safety: maybe it's safe, I'm not sure.
                        let mut materials =
                            unsafe { (*world_mut).get_resource_mut::<Assets<StandardMaterial>>().unwrap() };
                        materials.remove(&*p);
                        *p = MeshMaterial3d(materials.add(material));
                    }
                    M1::Handle { handle } => {
                        let mut p = world.get_mut::<MeshMaterial3d<StandardMaterial>>(self.entity).unwrap();
                        *p = MeshMaterial3d(handle);
                    }
                    _ => {}
                }
            }
        }
    }
    commands.queue(WorldSyncProxy { entity, transform, appearance });
}
// }])>

pub(crate) fn to_bevy_init(mut app: App) -> App {
    app.add_systems(Update, exclusive_system);
    app
}
