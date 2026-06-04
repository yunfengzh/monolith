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

// TODO: new sample, a script send message to b script, by rust message-system (athlete).
// Two scripts are provided, RELIC demostrates the basic usage of a script. TEAM shows how to
// load/save a script.
// sample scripts <([{
const RELIC: &str = r#"
    let relic = #{
        count: 1,

        on_player_die: |evt, cnt| {
            print(`rhai method event handler: Player ${evt}, ${cnt}`);
            if this.count > 0 {
                this.count -= 1;
                // TODO: init all rust vars before evaluate the script!
                player.set(3);
                return #{state: 1, msg: "revive done"};
            } else {
                return #{state: 0, msg: "No more reserve"};
            }
        }
   };

    print("script eval ${Status_Some}");
    declare_trait("relic", "Life");

    // TODO: remove later func
    fn on_player_die(evt, cnt) {
        print(`rhai func event handler: Player ${evt}, ${cnt}`);
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

// structs shared between rust and rhai, doc them to rhai developer <([{
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
// }])>

// import enum to rhai <([{
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

// Rhai to rust <([{
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

// Here we can make sure player raw pointer available. Alternative is 'PlayerProxy(Arc<..>);'
// Don't  worry, untrusted script can't access PlayerProxy inner field because we don't expose
// PlayerProxy::set/get methods.
#[derive(Clone)]
struct PlayerProxy(*mut Player);

unsafe impl Send for PlayerProxy {}
unsafe impl Sync for PlayerProxy {}

impl PlayerProxy {
    fn proxy(rhai: &mut Rhai, player: &mut Player) {
        let proxy: PlayerProxy = PlayerProxy(player as *mut _);
        rhai.engine.register_fn("adjust", PlayerProxy::adjust);
        rhai.engine.register_fn("set", PlayerProxy::set);
        rhai.scope.push("player", proxy);
    }

    pub fn adjust(&mut self, mut value: i64) {
        let player: &mut Player = unsafe { &mut *self.0 };
        // Always double-check input from an untrusted script.
        value = value.clamp(-20, -1);
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

fn api_or_proxy(rhai: &mut Rhai, player: &mut Player) {
    // Make rust object accessed by untrusted-script -- by proxy.
    PlayerProxy::proxy(rhai, player);
    // TODO: More such as web.channel -- an rust object open a connection for game server.
    register_rust_enum(rhai);
}
// }])>

// Rust to rhai <([{
#[trait_to_rhai]
trait Life {
    fn on_player_die(&self, evt: Hurt, cnt: i64) -> Revive;

    fn on_player_up(&self, cnt: i64);
    fn on_player_revive(&self) -> Hurt;
}
// }])>

// load/save <([{
async fn load(json: &String) {
    let rhai_raw = stump().rhai_manager.get_rhai("team");
    let rhai_lock = unsafe { &mut *rhai_raw };
    let mut rhai = unsafe { &mut *rhai_raw };
    rhai.load_init();
    let mut player = Player::new();
    api_or_proxy(&mut rhai, &mut player);
    let v: Vec<(String, bool, Dynamic)> = serde_json::from_str(json.as_str()).unwrap();
    let _unused = rhai_lock.toplevel_lock().await;
    rhai.load_script_vars(v);
}

async fn save() -> Result<String, Box<dyn Error>> {
    let rhai_raw = stump().rhai_manager.get_rhai("team");
    let rhai_lock = unsafe { &mut *rhai_raw };
    let rhai = unsafe { &mut *rhai_raw };
    let mut json = "[".to_string();
    let _unused = rhai_lock.toplevel_lock().await;
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

async fn compare() {
    let before = scope_to_json();
    let json = save().await.unwrap();
    println!("compare: {before}");
    load(&json).await;
    let after = scope_to_json();
    assert_eq!(before, after);
}

async fn save_then_load() -> Result<(), Box<dyn Error>> {
    println!("---------------");
    let rhai_raw = stump().rhai_manager.get_rhai("team");
    let rhai = unsafe { &mut *rhai_raw };
    compare().await;
    let _: () = rhai.call("new_member", ("bow",))?;
    compare().await;
    let _: () = rhai.call("new_member", ("sword",))?;
    compare().await;
    Ok(())
}

fn team_init() {
    println!("----team init-----------");
    let rhai_raw = stump().rhai_manager.new_rhai("team", TEAM);
    let mut rhai = unsafe { &mut *rhai_raw };
    let mut player = Player::new();
    api_or_proxy(&mut rhai, &mut player);
    rhai.eval_script();
}
// }])>

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let app = App::new();
    _ = stump_new(app, None);
    let rhai_raw = stump().rhai_manager.new_rhai("relic", RELIC);
    let mut rhai = unsafe { &mut *rhai_raw };
    let mut player = Player::new();
    api_or_proxy(&mut rhai, &mut player);
    rhai.eval_script();
    team_init();
    stump().rhai_manager.init_done();

    let rhai_lock = unsafe { &mut *rhai_raw };
    let mut rhai = unsafe { &mut *rhai_raw };
    let mut player = Player::new();
    let x = rhai.search_impl_er("Life");
    if x.is_some() {
        player.consumers.push(LifeToRhai(rhai_raw));
    }
    api_or_proxy(&mut rhai, &mut player);
    dbg!(&player);
    let _unused = rhai_lock.toplevel_lock().await;
    let _: i64 = rhai.call("fight", ())?;
    dbg!(&player);

    save_then_load().await?;
    stump_drop();
    Ok(())
}
