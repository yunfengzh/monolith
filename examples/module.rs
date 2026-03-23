// vim: foldmarker=<([{,}])> foldmethod=marker

// <([{
use rhai::{
    AST, Dynamic, Engine, EvalAltResult, FnNamespace, FuncRegistration, Map, Module, Scope,
    module_resolvers::StaticModuleResolver,
};
// }])>

// SCRIPT <([{
const SCRIPT: &str = r#"
    import "player_handle" as p;

    let my_handler = #{
        count: 0,
    };

    fn init() {
        my_handler.count  = 23;
        true
    }

    fn test(off) { my_handler.count = my_handler.count + off; 23 }

    fn on_player_die(evt) {
        print(`Player ${evt.state}, ${evt.critical_attack}`);
        p::a.set(3);
        1
    }

    fn fight() {
        print(`${p::a}`);
        p::a.adjust(-15);
        1
    }
"#;
// }])>

struct Rhai {
    engine: Engine,
    ast: AST,
    scope: Scope<'static>,
}

impl Rhai {
    fn new() -> Self {
        let mut engine = Engine::new();
        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(64, 64);
        let scope = Scope::new();
        let ast = engine.compile(SCRIPT).unwrap();
        Self { engine, scope, ast }
    }
}

// rust injects event into rhai <([{
enum GameEvent {
    PlayerDie { critical_attack: i64, state: String },
}

fn broadcast_event(rhai: &mut Rhai, event: GameEvent) -> Result<bool, Box<EvalAltResult>> {
    let (event_map, func) = match event {
        GameEvent::PlayerDie { critical_attack, state } => {
            let mut m = Map::new();
            m.insert("critical_attack".into(), Dynamic::from(critical_attack));
            m.insert("state".into(), Dynamic::from(state));
            (m, "on_player_die")
        }
    };

    let _: i64 = rhai.engine.call_fn(&mut rhai.scope, &rhai.ast, func, (event_map,))?;

    Ok(true)
}
// }])>

// rhai changes rust objects by trait <([{
fn rhai_to_rust(rhai: &mut Rhai) -> Result<bool, Box<EvalAltResult>> {
    #[derive(Clone, Debug)]
    struct Player {
        pub life: i32,
    }

    let mut a = Player { life: 10 };
    // Player Handle <([{
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
        pub fn adjust(&mut self, value: i64) {
            let player: &mut Player = self.as_mut();
            player.life += value as i32;
        }

        pub fn set(&mut self, value: i64) {
            let player: &mut Player = self.as_mut();
            player.life = value as i32;
        }
    }

    let handle_a: PlayerHandle = (&mut a).into();
    // }])>
    // Rhai only supports method in global namespace!!!!
    let mut module = Module::new();
    module.set_var("a", handle_a);
    FuncRegistration::new("adjust")
        .with_namespace(FnNamespace::Global)
        .set_into_module(&mut module, PlayerHandle::adjust);
    FuncRegistration::new("set").with_namespace(FnNamespace::Global).set_into_module(&mut module, PlayerHandle::set);
    let mut resolver = StaticModuleResolver::new();
    resolver.insert("player_handle", module);
    rhai.engine.set_module_resolver(resolver);
    // rhai.engine.register_fn("adjust", PlayerHandle::adjust);
    // rhai.engine.register_fn("set", PlayerHandle::set);
    // rhai.scope.push("a", handle_a);
    println!("-{:?}", a);
    let _: i64 = rhai.engine.call_fn(&mut rhai.scope, &rhai.ast, "fight", ())?;
    if a.life <= 0 {
        broadcast_event(rhai, GameEvent::PlayerDie { critical_attack: -15, state: "need heal".to_string() })?;
    }
    println!("-{:?}", a);
    Ok(true)
}
// }])>

fn main() -> Result<(), Box<EvalAltResult>> {
    let mut rhai = Rhai::new();
    rhai_to_rust(&mut rhai)?;
    Ok(())
}
