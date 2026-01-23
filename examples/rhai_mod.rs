use std::error::Error;

// vim: foldmarker=<([{,}])> foldmethod=marker
// <([{
use rhai::*;
// }])>

// Context: user mod updates host objects by certified API.
// See more from rhai.src/examples/arrays_and_structs.rs
// Player and PlayerHandle <([{
fn update_host() -> Result<(), Box<EvalAltResult>> {
    // Host type and objects <([{
    #[derive(Clone, Debug)]
    struct Player {
        pub life: i32,
    }

    let mut a = Player { life: 20 };
    let mut b = Player { life: 60 };
    // }])>
    // rhai proxy to host objects <([{
    #[derive(Clone)]
    struct PlayerHandle(usize);

    impl From<&mut Player> for PlayerHandle {
        fn from(p: &mut Player) -> Self {
            Self((p as *mut Player).expose_provenance())
        }
    }

    impl AsMut<Player> for PlayerHandle {
        fn as_mut(&mut self) -> &mut Player {
            unsafe { &mut *std::ptr::with_exposed_provenance_mut(self.0) }
        }
    }

    impl PlayerHandle {
        pub fn update(&mut self, value: i64) {
            let player: &mut Player = self.as_mut();
            player.life += value as i32;
        }
    }

    let handle_a: PlayerHandle = (&mut a).into();
    let handle_b: PlayerHandle = (&mut b).into();
    // }])>
    let mut engine = Engine::new();
    // validate rhai proxy by Scope <([{
    engine.register_fn("update", PlayerHandle::update);
    let mut scope = Scope::new();
    scope.push("a", handle_a);
    scope.push("b", handle_b);
    // }])>
    let ast = engine.compile(
        r#"
            fn fight() {
                a.update(-15);
                b.update(-3);
                1
            }
        "#,
    )?;
    println!("before fight: a {:?} - b {:?}", a, b);
    let result = engine.call_fn::<i64>(&mut scope, &ast, "fight", ())?;
    println!("fight {result}: a {:?} - b {:?}", a, b);
    Ok(())
}
// }])>

fn main() -> Result<(), Box<dyn Error>> {
    update_host()?;

    Ok(())
}
