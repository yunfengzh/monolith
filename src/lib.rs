// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! Monolith is a framework based on tokio and bevy.
//!
//! Later pseudo-code gives an overview:
//!
//! ```plaintext
//! async sprite::run() {
//!     sprite_in_village();
//!     ...
//!     sprite_enter_dungeon();
//!     loop {
//!         match evt = from_bevy().await {
//!             GameEvent::MoveLeft => {
//!                 self.pos.translation.x -= 1.0;
//!                 to_bevy().send(Task::SyncEntity, self.entity, self.pos);
//!             }
//!             ...
//!         }
//!     }
//!     ...
//! }
//! ```
//!
//! 1. Sprite::run should be based on sprite state machine rather than being split to fit with ECS.
//! 2. Sprite::run hardcodes GameEvent (logic event).
//! 3. Monolith treats bevy as a display and peripheral wrapper library -- from_bevy and to_bevy.
//! 4. Monolith supports tokio, that is, you can run time-cost algorithm in coroutine context.
//!
//! And `examples/hello.rs` shows more about how to use monolith components.
//!
//! ## Why ecs
//! ECS which bring an innovation idea that split the fields of a struct into components. Put all
//! components close to each other to improve cache hit rate. But it can be done manually in tokio.
//!
//! 1. `Sprite { ... } -> SpritePos -> Vec<SpritePos>`.
//! 2. run algorithm in coroutine.
//! 3. Optionally, update original objects.
//!
//! In conclusion, bevy needs ecs to accelerate graphic performance, but it shouldn't impose its
//! architecture to developer. Bevy system should run noblock, graphic algorithm instead of game
//! logic.
// }])>

pub mod from_bevy;
pub mod lock;
pub mod stump;
pub mod to_bevy;
pub mod utils;

pub mod prelude {
    pub use crate::from_bevy::*;
    pub use crate::lock::*;
    pub use crate::stump::*;
    pub use crate::to_bevy::*;
    pub use crate::utils::*;
}
