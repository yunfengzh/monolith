// vim: foldmarker=<([{,}])> foldmethod=marker

// <([{
use std::error::Error;

use monolith_macro_utils::{RhaiMap, analyze_trait_methods};
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

#[derive(Clone, Debug, RhaiMap)]
struct Hurt {
    critical_attack: i64,
    state: String,
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
        let player: &mut Player = self.as_mut();
        player.life = value as i32;
    }
}
// }])>

// Player to rhai <([{
#[analyze_trait_methods]
trait Life {
    fn on_player_die(&self, evt: Hurt, cnt: i64) -> Dynamic;

    fn on_player_up(&self, cnt: i64);
    fn on_player_revive(&self) -> Hurt;
}

pub fn get_method_names() -> Vec<String> {
    vec!["on_player_die".to_string()]
}
// }])>

// The function shows how to load/save a rhai instance. During the process, MOD developer need not
// response load/save event at all. And only global and scope variables are saved.
// load/save <([{
fn load(json: &String) -> Rhai {
    let mut rhai = Rhai::new(TEAM);
    let mut player = Player::new();
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

fn scope_to_json(rhai: &Rhai) -> String {
    let mut json = "".to_string();
    for i in rhai.scope.iter() {
        json += &serde_json::to_string(&i).unwrap();
    }
    json
}

fn compare(rhai: &Rhai) -> Rhai {
    let before = scope_to_json(rhai);
    let json = save(&rhai).unwrap();
    println!("before{before}");
    let ret = load(&json);
    let after = scope_to_json(&ret);
    assert_eq!(before, after);
    ret
}

fn save_then_load() -> Result<(), Box<dyn Error>> {
    println!("---------------");
    let mut rhai = Rhai::new(TEAM);
    let mut player = Player::new();
    api(&mut rhai, &mut player);
    let mut rhai = compare(&rhai);
    let _: () = rhai.call("new_member", ("bow",))?;
    let mut rhai = compare(&rhai);
    let _: () = rhai.call("new_member", ("sword",))?;
    let _ = compare(&rhai);
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
    rhai.scope.push_constant("Status_Some", <Status as Into<u32>>::into(Status::Some));
}
// }])>

fn api(rhai: &mut Rhai, player: &mut Player) {
    PlayerProxy::proxy(rhai, player);
    register_rust_enum(rhai);
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("{:?}", get_method_names());
    let mut rhai = Rhai::new(RELIC);
    let mut player = Player::new();
    let x = rhai.search_trait("Life");
    if x.is_some() {
        player.consumers.push(LifeToRhai(&mut rhai as *mut _));
    }
    api(&mut rhai, &mut player);
    dbg!(&player);
    let _: i64 = rhai.call("fight", ())?;
    dbg!(&player);

    save_then_load()?;
    Ok(())
}
