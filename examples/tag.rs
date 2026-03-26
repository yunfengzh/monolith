// vim: foldmarker=<([{,}])> foldmethod=marker

// <([{
use std::{
    collections::HashMap,
    error::Error,
    sync::{LazyLock, RwLock},
};

use monolith_macro_utils::RhaiMap;
use rhai::*;
use yunfengzh_monolith::prelude::*;
// }])>

// SCRIPT <([{
const SCRIPT: &str = r#"
    let my_handler = #{
        count: 0,
    };

    print("script eval");
    fn init() {
        my_handler.count  = 23;
        print(`rhai init called ${my_handler}`);
        ev.register("LifeEvent", "my_handler", "on_player_die");
        true
    }

    fn on_player_die(evt) {
        print(`rhai event handler: Player ${evt.state}, ${evt.critical_attack}`);
        player.set(3);
        #{state: 1, msg: "revive done"}
    }

    fn fight() {
        player.adjust(-15);
        ev.publish(#{critical_attack: 4, state: "from handgun"});
        1
    }
    let x = 0;
    x.tag = 2;

    let team = [];

    fn new_member(weapon) {
        if weapon == "bow" {
            team += #{ ability: "archer", arrow: 3 };
        } else if weapon == "sword" {
            team += #{ ability: "warrior", };
        }
    }
"#;
// }])>

#[derive(Clone, Debug, RhaiMap)]
struct LifeEvent {
    critical_attack: i64,
    state: String,
}

#[derive(Clone, Debug)]
struct Player {
    pub life: i32,
}

// Basic API for MOD author <([{
// EventSystem::init() exposes two APIs by 'ev'.

static POOL: LazyLock<RwLock<HashMap<String, (String, String)>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

#[derive(Clone)]
struct EventSystem();

impl EventSystem {
    fn init(rhai: &mut Rhai) {
        // Generally, we need prepare API for MOD author, it is done by obj.method, such as, 'ev'
        // implements two methods register/publish.
        let es = EventSystem();
        rhai.engine.register_fn("register", EventSystem::register);
        rhai.engine.register_fn("publish", EventSystem::publish);
        rhai.scope.push("ev", es);
    }

    fn register(&mut self, event_trait: String, obj: String, method: String) {
        POOL.write().unwrap().insert(event_trait, (obj, method));
    }

    fn broadcast(rhai: &mut Rhai, event: LifeEvent) -> Result<bool, Box<dyn Error>> {
        match event {
            LifeEvent { .. } => {
                let val = POOL.read().unwrap();
                let val = val.get("LifeEvent");
                if val.is_some() {
                    let (_, func) = val.unwrap();
                    let m: Map = event.into();
                    let ret: Dynamic = rhai.call(func, (m,))?;
                    println!("rust result from rhai event: {:?}", ret);
                }
            }
        };

        Ok(true)
    }

    fn publish(&mut self, evt: rhai::Map) {
        let evt = Map::from(evt);
        dbg!(evt);
    }
}
// }])>

// Rust struct is exported to rhai script by proxy <([{
#[derive(Clone)]
struct PlayerProxy(usize);

impl From<&mut Player> for PlayerProxy {
    fn from(p: &mut Player) -> Self {
        Self((p as *mut Player).expose_provenance())
    }
}

impl AsMut<Player> for PlayerProxy {
    fn as_mut(&mut self) -> &mut Player {
        unsafe { &mut *std::ptr::with_exposed_provenance_mut(self.0) }
    }
}

impl PlayerProxy {
    fn proxy(rhai: &mut Rhai, player: &mut Player) {
        let proxy: PlayerProxy = player.into();
        rhai.engine.register_fn("adjust", PlayerProxy::adjust);
        rhai.engine.register_fn("set", PlayerProxy::set);
        rhai.scope.push("player", proxy);
    }

    pub fn adjust(&mut self, value: i64) {
        let player: &mut Player = self.as_mut();
        player.life += value as i32;
    }

    pub fn set(&mut self, mut value: i64) {
        // Always double-check input from an untrusted script.
        value = value.clamp(1, 20);
        let player: &mut Player = self.as_mut();
        player.life = value as i32;
    }
}
// }])>

// The function shows how to load/save a rhai instance. During the process, MOD developer need not
// response load/save event at all. And only global and scope variables are saved.
// load/save <([{
fn load(json: &String) -> Rhai<'_> {
    let mut rhai = Rhai::new(SCRIPT);
    let tuple: (String, bool, Dynamic) = serde_json::from_str(json.as_str()).unwrap();
    if tuple.1 {
        rhai.scope.push_constant_dynamic(tuple.0, tuple.2);
    } else {
        rhai.scope.push_dynamic(tuple.0, tuple.2);
    }
    rhai
}

fn save(rhai: &Rhai) -> Result<String, Box<dyn Error>> {
    let mut json = "".to_string();
    for i in rhai.scope.iter() {
        json += &serde_json::to_string(&i)?;
    }
    Ok(json)
}

fn save_then_load(mut rhai: Rhai) -> Result<(), Box<dyn Error>> {
    println!("---------------");
    let ret = save(&rhai)?;
    let mut json = "".to_string();
    for i in rhai.scope.iter() {
        json += &serde_json::to_string(&i)?;
    }
    println!("aa{json}");
    let _: () = rhai.call("new_member", ("bow",))?;
    json.clear();
    for i in rhai.scope.iter() {
        json += &serde_json::to_string(&i)?;
    }
    println!("bb{json}");
    let _: () = rhai.call("new_member", ("sword",))?;
    json.clear();
    for i in rhai.scope.iter() {
        json += &serde_json::to_string(&i)?;
    }
    println!("cc{json}");
    Ok(())
}
// }])>

fn main() -> Result<(), Box<dyn Error>> {
    let mut rhai = Rhai::new(SCRIPT);
    EventSystem::init(&mut rhai);
    let mut player = Player { life: 10 };
    PlayerProxy::proxy(&mut rhai, &mut player);
    let _: Dynamic = rhai.call("init", ())?;
    dbg!(&player);
    let _: i64 = rhai.call("fight", ())?;
    if player.life <= 0 {
        EventSystem::broadcast(&mut rhai, LifeEvent { critical_attack: -15, state: "need heal".to_string() })?;
    }
    dbg!(&player);
    save_then_load(rhai)?;
    Ok(())
}
