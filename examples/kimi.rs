use rhai::{CustomType, Dynamic, Engine, Scope, TypeBuilder};

#[derive(Debug, Clone, CustomType)]
struct GameState {
    pub score: i64,
    pub level: i64,
    pub player_name: String,
}

impl GameState {
    fn new(name: String) -> Self {
        Self { score: 0, level: 1, player_name: name }
    }
}

struct GameEngine {
    rhai_engine: Engine,
    ast: rhai::AST,
    scope: Scope<'static>,
}

impl GameEngine {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut engine = Engine::new();

        engine
            .build_type::<GameState>()
            .register_fn("my_new", GameState::new)
            .register_fn("add_score", |state: &mut GameState, points: i64| {
                state.score += points;
                state.score
            })
            .register_fn("level_up", |state: &mut GameState| {
                state.level += 1;
                state.level
            });

        let script = r#"
            fn on_enemy_killed(enemy_type) {
                let points = if enemy_type == "boss" { 1000 } else { 100 };
                game.add_score(points);
                if game.score > 1000 * game.level {
                    game.level_up();
                }
                game.score
            }
            
            fn on_collect_item(item) {
                if item == "coin" {
                    game.add_score(20);
                } else if item == "gem" {
                    game.add_score(50);
                }
            }
            
            fn get_status() {
                `${game.player_name}: Level ${game.level}, Score ${game.score}`
            }
        "#;

        let ast = engine.compile(script)?;
        let mut scope = Scope::new();

        engine.run_with_scope(&mut scope, r#"let game = my_new("player1");"#)?;

        Ok(Self { rhai_engine: engine, ast, scope })
    }

    fn trigger_event(&mut self, event: &str, args: impl rhai::FuncArgs) -> Result<Dynamic, Box<dyn std::error::Error>> {
        let result = self.rhai_engine.call_fn(&mut self.scope, &self.ast, event, args)?;
        Ok(result)
    }

    fn get_state(&mut self) -> Result<GameState, Box<dyn std::error::Error>> {
        let state: GameState = self.scope.get_value("game").unwrap();
        Ok(state)
    }

    fn modify_state<F>(&mut self, f: F) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnOnce(&mut GameState),
    {
        let mut state: GameState = self.scope.get_value("game").unwrap();
        f(&mut state);
        self.scope.set_value("game", state);
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut ge = GameEngine::new()?;

    let _ = ge.trigger_event("on_enemy_killed", ("normal",))?;
    let _ = ge.trigger_event("on_enemy_killed", ("boss",))?;
    let _ = ge.trigger_event("on_collect_item", ("gem",))?;

    let status: String = ge.trigger_event("get_status", ())?.cast();
    println!("{}", status); // Player1: Level 2, Score 1150

    ge.modify_state(|state| {
        state.player_name = "Hero".to_string();
    })?;

    let status: String = ge.trigger_event("get_status", ())?.cast();
    println!("{}", status); // Hero: Level 2, Score 1150

    Ok(())
}
