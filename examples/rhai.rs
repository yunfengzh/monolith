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
            // later line will be failed, "No writable property 'p' ...", so PlayerProxy inner field
            // is safe!
            // player.p = 0;
            print(`>> relic event(${evt}, ${cnt})`);
            if this.count > 0 {
                this.count -= 1;
                return #{new_life: 3, state: 1, msg: "revive done"};
            } else {
                return #{new_life: 0, state: 0, msg: "No more reserve"};
            }
        }
    };

    print("script eval ${Status_Some}");
    declare_trait("relic", "Life");

    fn fight() {
        player.adjust(-15);
        return 0;
    }
"#;

const TEAM: &str = r#"
    let team = [];
    let team_handler = #{
        count: 1,

        on_player_die: |evt, cnt| {
            print(`>> team event(${evt}, ${cnt})`);
            return #{new_life: 0, state: 1, msg: "do nothing"};
        }
    };

    print("team_handler");
    player.show("player is team leader");
    declare_trait("team_handler", "Life");

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
    new_life: i64,
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

    // TODO: document about recursive call.
    fn adjust(&mut self, value: i64) {
        self.life += value as i32;
        if self.life <= 0 {
            // Rule about Life trait:
            // 1. Event observers are called one-by-one in the order of registration.
            // 2. If an observer can rescue player, finished loop immediately.
            for i in self.consumers.iter() {
                let ret = i.on_player_die(Hurt { critical_attack: value, state: "need heal".to_string() }, 17);
                println!("<< result from rhai: {:?}", ret);
                if ret.new_life > 0 {
                    self.life = ret.new_life as i32;
                    println!("player is rescued");
                    break;
                }
            }
        }
        if self.life <= 0 {
            println!("player is died");
        }
    }
}

// Here we can make sure player raw pointer available. Alternative is 'PlayerProxy(Arc<..>);'
// Don't  worry, untrusted script can't access PlayerProxy inner field because we don't expose
// PlayerProxy::set/get methods.
#[derive(Clone)]
struct PlayerProxy {
    p: *mut Player,
    // more fields can be defined here.
}

unsafe impl Send for PlayerProxy {}
unsafe impl Sync for PlayerProxy {}

impl PlayerProxy {
    fn proxy(rhai: &mut Rhai, player: &mut Player) {
        let proxy = PlayerProxy { p: player as *mut _ };
        rhai.engine.register_fn("show", PlayerProxy::show);
        rhai.engine.register_fn("adjust", PlayerProxy::adjust);
        rhai.scope.push("player", proxy);
    }

    // TODO: remove later functions.
    fn show(&mut self, msg: &str) {
        println!("PlayerProxy{0}-{msg}", self.p as usize);
    }

    fn adjust(self, val: i64) {
        let player = unsafe { &mut *self.p };
        player.adjust(val);
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
    fn on_player_die(&self, evt: Hurt, unused: i64) -> Revive;
}
// }])>

// load/save <([{
async fn load(json: &String) {
    let rhai = unsafe { &mut *stump().rhai_manager.get_rhai("team") };
    let rhai_lock: &mut Rhai = unsafe { &mut *(rhai as *mut _) };
    rhai.load_init();
    let mut player = Player::new();
    api_or_proxy(rhai, &mut player);
    let v: Vec<(String, bool, Dynamic)> = serde_json::from_str(json.as_str()).unwrap();
    let _unused = rhai_lock.toplevel_lock().await;
    rhai.load_script_vars(v);
}

async fn save() -> Result<String, Box<dyn Error>> {
    let rhai = unsafe { &mut *stump().rhai_manager.get_rhai("team") };
    let rhai_lock: &mut Rhai = unsafe { &mut *(rhai as *mut _) };
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
    let mut rhai = stump().rhai_manager.new_rhai("team", TEAM);
    let mut player = Player::new();
    api_or_proxy(&mut rhai, &mut player);
    rhai.eval_script();
}
// }])>

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let app = App::new();
    _ = stump_new(app, None);
    let mut rhai = stump().rhai_manager.new_rhai("relic", RELIC);
    let mut player = Player::new();
    api_or_proxy(&mut rhai, &mut player);
    rhai.eval_script();
    team_init();
    stump().rhai_manager.init_done();

    // let mut player = Player::new();
    // api_or_proxy(rhai, &mut player);
    for (_, i) in stump().rhai_manager.iter_rhai() {
        let x = i.search_impl_er("Life");
        if x.is_some() {
            player.consumers.push(LifeToRhai(i as *const _ as *mut _));
        }
    }
    dbg!(&player);
    // player.adjust(-15);
    // player.adjust(-15);
    let rhai_raw = stump().rhai_manager.get_rhai("relic");
    let rhai_lock = unsafe { &mut *rhai_raw };
    let _unused = rhai_lock.toplevel_lock().await;
    let _: i64 = rhai.call("fight", ())?;
    dbg!(&player);

    save_then_load().await?;
    stump_drop();
    Ok(())
}
