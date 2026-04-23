// vim: foldmarker=<([{,}])> foldmethod=marker

// <([{
use std::error::Error;

use ::serde::{Deserialize, Serialize};
use bevy::app::App;
use monolith_macro_utils::trait_to_rhai;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use rhai::*;
use yunfengzh_monolith::prelude::*;
// }])>

// SCRIPT <([{
const RELIC: &str = r#"
    let relic = #{
        count: 1,
    };

    print("script eval ${Status_Some}");
    declare_trait("relic", "Life");

    fn on_player_die(evt, cnt) {
        print(`rhai event handler: Player ${evt}, ${cnt}`);
        if relic.count > 0 {
            relic.count -= 1;
            player.set(3);
            return #{state: 1, msg: "revive done"};
        } else {
            return #{state: 0, msg: "No more reserve"};
        }
    }

    fn fight() {
        player.adjust(-15);
        1
    }
"#;

const TEAM: &str = r#"
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Hurt {
    critical_attack: i64,
    state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Revive {
    state: i64,
    msg: String,
}

// Rhai to Player <([{
#[derive(Clone, Debug)]
struct Player {
    pub life: i32,
    consumers: Vec<LifeToRhai>,
}

impl Player {
    fn new() -> Self {
        Self { life: 10, consumers: Vec::new() }
    }
}

// It is developer's responsibility to make player-pointer available. Feel free to PlayerProxy(Arc<..>);
#[derive(Clone)]
struct PlayerProxy(*mut Player);

unsafe impl Send for PlayerProxy {}
unsafe impl Sync for PlayerProxy {}

impl PlayerProxy {
    fn proxy(rhai: &mut Rhai, player: &mut Player) {
        let proxy: PlayerProxy = PlayerProxy(player as *mut _);
        rhai.engine.register_fn("adjust", PlayerProxy::adjust);
        rhai.engine.register_fn("set", PlayerProxy::set);
        rhai.scope.as_mut().unwrap().push("player", proxy);
    }

    pub fn adjust(&mut self, value: i64) {
        let player: &mut Player = unsafe { &mut *self.0 };
        player.life += value as i32;
        if player.life <= 0 {
            for i in player.consumers.iter() {
                let ret = i.on_player_die(Hurt { critical_attack: value, state: "need heal".to_string() }, 17);
                println!("result from rhai: {:?}", ret);
            }
        }
    }

    pub fn set(&mut self, mut value: i64) {
        // Always double-check input from an untrusted script.
        value = value.clamp(1, 20);
        let player: &mut Player = unsafe { &mut *self.0 };
        player.life = value as i32;
    }
}
// }])>

// Player to rhai <([{
#[trait_to_rhai]
trait Life {
    fn on_player_die(&self, evt: Hurt, cnt: i64) -> Revive;

    fn on_player_up(&self, cnt: i64);
    fn on_player_revive(&self) -> Hurt;
}
// }])>

// The function shows how to load/save a rhai instance. During the process, MOD developer need not
// response load/save event at all. And only global and scope variables are saved.
// load/save <([{
fn load(json: &String) {
    let rhai_raw = stump().rhai_manager.get_rhai("team");
    let mut rhai = unsafe { &mut *rhai_raw };
    let mut player = Player::new();
    let v: Vec<(String, bool, Dynamic)> = serde_json::from_str(json.as_str()).unwrap();
    rhai.load(v);
    api(&mut rhai, &mut player);
}

fn save() -> Result<String, Box<dyn Error>> {
    let rhai_raw = stump().rhai_manager.get_rhai("team");
    let rhai = unsafe { &mut *rhai_raw };
    let mut json = "[".to_string();
    for i in rhai.iter_script_vars() {
        json += &serde_json::to_string(&i)?;
        json += ",";
    }
    json.pop();
    json += "]";
    Ok(json)
}

fn scope_to_json() -> String {
    let rhai_raw = stump().rhai_manager.get_rhai("team");
    let rhai = unsafe { &mut *rhai_raw };
    let mut json = "".to_string();
    for i in rhai.iter_all_vars() {
        json += &serde_json::to_string(&i).unwrap();
    }
    json
}

fn compare() {
    let before = scope_to_json();
    let json = save().unwrap();
    println!("compare: {before}");
    load(&json);
    let after = scope_to_json();
    assert_eq!(before, after);
}

fn save_then_load() -> Result<(), Box<dyn Error>> {
    println!("---------------");
    let rhai_raw = stump().rhai_manager.get_rhai("team");
    let mut rhai = unsafe { &mut *rhai_raw };
    let mut player = Player::new();
    api(&mut rhai, &mut player);
    compare();
    let _: () = rhai.call("new_member", ("bow",))?;
    compare();
    let _: () = rhai.call("new_member", ("sword",))?;
    compare();
    Ok(())
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
    rhai.scope.as_mut().unwrap().push_constant("Status_Some", <Status as Into<u32>>::into(Status::Some));
}
// }])>

fn api(rhai: &mut Rhai, player: &mut Player) {
    PlayerProxy::proxy(rhai, player);
    register_rust_enum(rhai);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let app = App::new();
    _ = stump_new(app, None);
    let rhai_raw = stump().rhai_manager.new_rhai("relic", RELIC);
    stump().rhai_manager.new_rhai("team", TEAM);
    stump().rhai_manager.init_done();

    let mut rhai = unsafe { &mut *rhai_raw };
    let rhai_call = unsafe { &mut *rhai_raw };
    let mut player = Player::new();
    let x = rhai.search_trait("Life");
    if x.is_some() {
        player.consumers.push(LifeToRhai(rhai_raw));
    }
    api(&mut rhai, &mut player);
    dbg!(&player);
    let _unused = rhai.lock().await;
    let _: i64 = rhai_call.call("fight", ())?;
    dbg!(&player);

    save_then_load()?;
    stump_drop();
    Ok(())
}
