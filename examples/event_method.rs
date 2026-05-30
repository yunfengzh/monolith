// vim: foldmarker=<([{,}])> foldmethod=marker

// <([{
use rhai::{Dynamic, Engine, EvalAltResult, FnPtr, Map, Scope};
use std::thread;
use std::time::Duration;
// }])>

// SCRIPT <([{
const SCRIPT: &str = r#"
    let my_handler = #{
        count: 0,
        on_player_joined: |event| {
            if event.id == 1 {
                print(`🎉 VIP Player ${event.name} joined! Special welcome!`);
                true
            } else {
                print(`Player ${event.name} joined normally`);
                false
            }
        },

        on_player_moved: |event| {
            print(`🎮 Tracking player ${event.id} move to (${event.x}, ${event.y}, ${this.count})`);

            if event.x > 500.0 || event.y > 500.0 {
                print("⚠️ Out of bounds!");
            }
            true
        },

        on_player_left: |event| {
            print(`👋 Goodbye player ${event.id}`);
            true
        }
    };

    fn init() {
        my_handler.count  = 23;
        true
    }

    fn test(off) { my_handler.count = my_handler.count + off; 23 }
"#;
// }])>

#[derive(Debug, Clone)]
pub enum GameEvent {
    PlayerJoined { id: i64, name: String },
    PlayerMoved { id: i64, x: f64, y: f64 },
    PlayerLeft { id: i64 },
}

pub struct ScriptEventSystem {
    engine: Engine,
    ast: rhai::AST,
    scope: Scope<'static>,
}

// ScriptEventSystem impl <([{
impl ScriptEventSystem {
    pub fn new() -> Result<Self, Box<EvalAltResult>> {
        let mut engine = Engine::new();
        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(64, 64);

        let ast = engine.compile(SCRIPT)?;

        let mut scope = Scope::new();

        let _: Dynamic = engine.eval_ast_with_scope(&mut scope, &ast).unwrap();
        println!("scope(eval):{:?}", scope);

        // use rhai::CallFnOptions
        // let options = CallFnOptions::new().eval_ast(false).rewind_scope(false);
        // let _: bool = engine.call_fn_with_options(options, &mut scope, &ast, "init", ())?;
        let _: bool = engine.call_fn(&mut scope, &ast, "init", ())?;
        println!("scope(init):{:?}", scope);

        Ok(Self { engine, ast, scope })
    }

    pub fn broadcast_event(&mut self, event: GameEvent) -> Result<bool, Box<EvalAltResult>> {
        let (event_map, method_name) = match &event {
            GameEvent::PlayerJoined { id, name } => {
                let mut m = Map::new();
                m.insert("type".into(), Dynamic::from("joined"));
                m.insert("id".into(), Dynamic::from(*id));
                m.insert("name".into(), Dynamic::from(name.clone()));
                (m, "on_player_joined")
            }
            GameEvent::PlayerMoved { id, x, y } => {
                let mut m = Map::new();
                m.insert("type".into(), Dynamic::from("moved"));
                m.insert("id".into(), Dynamic::from(*id));
                m.insert("x".into(), Dynamic::from(*x));
                m.insert("y".into(), Dynamic::from(*y));
                (m, "on_player_moved")
            }
            GameEvent::PlayerLeft { id } => {
                let mut m = Map::new();
                m.insert("type".into(), Dynamic::from("left"));
                m.insert("id".into(), Dynamic::from(*id));
                (m, "on_player_left")
            }
        };

        let my_handler = self.scope.get_value_ref::<Map>("my_handler");
        let binding = my_handler.unwrap();
        let opj: FnPtr = binding.get(method_name).unwrap().clone_cast();
        let result: bool = opj.call(&self.engine, &self.ast, (event_map,))?;
        // TODO: FnPtr::call_as_method
        // use rhai::NativeCallContext;
        // let result: bool =
        //     opj.call_raw(&NativeCallContext::new(&self.engine, method_name), my_handler, (event_map,))?;

        Ok(result)
    }

    pub fn run_event_loop(mut self) {
        let events = vec![
            GameEvent::PlayerJoined { id: 1, name: "Alice".into() },
            GameEvent::PlayerMoved { id: 1, x: 100.0, y: 200.0 },
            GameEvent::PlayerJoined { id: 2, name: "Bob".into() },
            GameEvent::PlayerMoved { id: 2, x: 50.0, y: 50.0 },
            GameEvent::PlayerLeft { id: 1 },
        ];

        for (i, event) in events.iter().enumerate() {
            thread::sleep(Duration::from_secs(1));
            println!("\n📢 Broadcasting event {}: {:?}", i + 1, event);

            match self.broadcast_event(event.clone()) {
                Ok(result) => println!("✅ Ok {}", result),
                Err(e) => println!("❌ Error: {}", e),
            }
        }

        let _: i64 = self.engine.call_fn(&mut self.scope, &self.ast, "test", (2_i64,)).unwrap();
        println!("scope(test a):{:?}", self.scope);
        let _: i64 = self.engine.call_fn(&mut self.scope, &self.ast, "test", (2_i64,)).unwrap();
        println!("scope(test b):{:?}", self.scope);
    }
}
// }])>

fn main() -> Result<(), Box<EvalAltResult>> {
    let system = ScriptEventSystem::new()?;

    system.run_event_loop();

    Ok(())
}
