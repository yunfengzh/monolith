// vim: foldmarker=<([{,}])> foldmethod=marker

// <([{
use std::{
    collections::HashMap,
    error::Error,
    sync::{LazyLock, RwLock},
};

use monolith_macro_utils::{RhaiMap, scan_methods};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use rhai::*;
use yunfengzh_monolith::prelude::*;
// }])>

// SCRIPT <([{
const SCRIPT: &str = r#"
    let relic = #{
        count: 0,
    };

    print("script eval ${Status_Some}");
    fn init() {
        relic.count  = 1;
        print(`rhai init called, relic got a revive point! ${relic} ${Status_Some}`);
        ev.register("LifeEvent", "relic", "on_player_die");
        true
    }

    fn on_player_die(evt) {
        print(`rhai event handler: Player ${evt.state}, ${evt.critical_attack}`);
        if relic.count > 0 {
            player.set(3);
            return #{state: 1, msg: "revive done"};
        } else {
            return #{state: 0, msg: "No more reserve"};
        }
    }

    fn fight() {
        player.adjust(-15);
        ev.publish(#{critical_attack: 4, state: "from handgun"});
        1
    }

    let team = [];

    fn new_member(weapon) {
        let nm = 3; // CallFnOptions::rewind_scope(false) will make the variable global.
        if weapon == "bow" {
            team += #{ job: "archer", arrow: 3 };
        } else if weapon == "sword" {
            team += #{ job: "warrior", };
        }
    }
"#;
// }])>

#[derive(Clone, Debug, RhaiMap)]
struct LifeEvent {
    critical_attack: i64,
    state: String,
}

// Trait for MOD author, EventSystem, from rust to rhai, broadcast(); from rhai to rust: publish() <([{
// EventSystem::init() exposes two APIs by 'ev'.

static POOL: LazyLock<RwLock<HashMap<String, (String, String)>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

struct BroadcastTray {
    trait_name: String,
    method: String,
    params: String,
}

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
#[derive(Clone, Debug)]
struct Player {
    pub life: i32,
}

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
fn load<'a, 'b>(json: &'a String) -> Rhai<'b> {
    let mut rhai = Rhai::new(SCRIPT);
    let mut player = Player { life: 10 };
    let v: Vec<(String, bool, Dynamic)> = serde_json::from_str(json.as_str()).unwrap();
    for tuple in v {
        let _ = rhai.scope.remove::<Dynamic>(&tuple.0);
        if tuple.1 {
            rhai.scope.push_constant_dynamic(tuple.0, tuple.2);
        } else {
            rhai.scope.push_dynamic(tuple.0, tuple.2);
        }
    }
    api(&mut rhai, &mut player);
    rhai
}

fn save(rhai: &Rhai) -> Result<String, Box<dyn Error>> {
    let mut json = "[".to_string();
    for i in rhai.iter() {
        json += &serde_json::to_string(&i)?;
        json += ",";
    }
    json.pop();
    json += "]";
    Ok(json)
}

fn scope_to_json(scope: &Scope) -> String {
    let mut json = "".to_string();
    for i in scope.iter() {
        json += &serde_json::to_string(&i).unwrap();
    }
    json
}

fn compare<'a, 'b>(rhai: &Rhai<'a>) -> Rhai<'b> {
    let before = scope_to_json(&rhai.scope);
    let json = save(&rhai).unwrap();
    println!("before{before}");
    println!("save{json}");
    let ret = load(&json);
    let after = scope_to_json(&ret.scope);
    assert_eq!(before, after);
    ret
}

fn save_then_load(rhai: Rhai) -> Result<(), Box<dyn Error>> {
    println!("---------------");
    let mut rhai = compare(&rhai);
    let _: () = rhai.call("new_member", ("bow",))?;
    let mut rhai = compare(&rhai);
    let _: () = rhai.call("new_member", ("sword",))?;
    let _ = compare(&rhai);
    Ok(())
}
// }])>

// trait macro <([{
#[scan_methods]
trait LifeTrait {
    fn on_player_die(&self, i: u32) -> String;
}
// }])>

// enum <([{
#[derive(Clone, TryFromPrimitive, IntoPrimitive)]
#[repr(u32)]
enum Status {
    None = 0,
    Some = 100,
}

fn register_rust_enum(rhai: &mut Rhai) {
    rhai.scope.push_constant("Status_Some", <Status as Into<u32>>::into(Status::Some));
}
// }])>

fn api(rhai: &mut Rhai, player: &mut Player) {
    EventSystem::init(rhai);
    PlayerProxy::proxy(rhai, player);
    register_rust_enum(rhai);
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut rhai = Rhai::new(SCRIPT);
    let mut player = Player { life: 10 };
    api(&mut rhai, &mut player);
    let _: Dynamic = rhai.call("init", ())?;
    dbg!(&player);
    let _: i64 = rhai.call("fight", ())?;
    if player.life <= 0 {
        EventSystem::broadcast(&mut rhai, LifeEvent { critical_attack: -15, state: "need heal".to_string() })?;
    }
    dbg!(&player);
    save_then_load(rhai)?;
    println!("{:?}", get_method_names());
    Ok(())
}
