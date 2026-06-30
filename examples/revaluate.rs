// vim: foldmarker=<([{,}])> foldmethod=marker

// <([{
use std::{collections::HashMap, error::Error, sync::OnceLock};

use ::serde::{Deserialize, Serialize};
use rhai::*;
// }])>

// sample scripts <([{
const RELIC: &str = r#"
    let team = [];
    team += #{ job: "archer", arrow: 3 };

    let relic = #{
        count: 1,

        on_player_die: |evt, cnt| {
            print(`${team[0]}`);
            let cnt = 100;
            while cnt > 0 {
                cnt -= 1;
                team += cnt;
            }
            return #{new_life: 3, state: 1, msg: "revive done"};
        }
    };

    declare_trait("relic", "Life");

    fn fight() {
        print(`${team[0]}`);
        player.adjust(-15);
        print(`${team[0]}`);
        print(`${team[3]}`);
        0
    }
"#;
// }])>

// RhaiMgr <([{
#[derive(Debug)]
struct RhaiMgr {
    data: Vec<(String, Rhai)>,
    trait_list: HashMap<String, String>,
}

impl RhaiMgr {
    fn new() -> Self {
        Self { data: Vec::new(), trait_list: HashMap::new() }
    }

    fn new_rhai(&mut self, title: &str, script: &str) -> &mut Rhai {
        self.data.push((title.to_string(), Rhai::new(script)));
        &mut self.data.last_mut().unwrap().1
    }

    fn iter_rhai(&self) -> impl Iterator<Item = &(String, Rhai)> {
        self.data.iter()
    }
}

static mut RHAI_MANAGER: OnceLock<RhaiMgr> = OnceLock::new();
fn rhai_mgr() -> &'static mut RhaiMgr {
    unsafe { (*(&raw mut RHAI_MANAGER)).get_mut().unwrap() }
}
fn rhai_mgr_new() {
    unsafe {
        (*(&raw mut RHAI_MANAGER)).set(RhaiMgr::new()).unwrap();
    }
}

impl Default for RhaiMgr {
    fn default() -> Self {
        Self::new()
    }
}
// }])>

// Rhai <([{

#[derive(Debug)]
struct Rhai {
    engine: Engine,
    ast: AST,
    scope: Scope<'static>,
    system_vars_range: (u32, u32),
    script_vars_range: (u32, u32),
    trait_list: HashMap<String, String>,
}

impl Rhai {
    fn new(script: &str) -> Self {
        let mut engine = Engine::new();
        engine.set_max_call_levels(16);
        engine.set_max_expr_depths(64, 64);
        let ast = engine.compile(script).unwrap();
        Self {
            engine,
            ast,
            scope: Scope::new(),
            system_vars_range: (0, 0),
            script_vars_range: (0, 0),
            trait_list: HashMap::new(),
        }
    }

    fn eval_script(&mut self) {
        let scope = &mut self.scope;
        let system_vars_end = scope.len() as u32;
        self.system_vars_range = (0, system_vars_end);
        self.engine.register_fn("declare_trait", Rhai::declare_trait);
        let _: Dynamic = self.engine.eval_ast_with_scope(scope, &self.ast).unwrap();
        let mgr = &mut rhai_mgr();
        self.trait_list = mgr.trait_list.clone();
        mgr.trait_list = HashMap::new();
        let script_vars_end = scope.len() as u32;
        self.script_vars_range = (system_vars_end, script_vars_end);
    }

    fn declare_trait(obj: String, trait_name: String) {
        let rhai_manager = rhai_mgr();
        rhai_manager.trait_list.insert(trait_name, obj);
    }

    fn search_impl_er(&self, trait_name: &str) -> Option<&String> {
        self.trait_list.get(trait_name)
    }

    fn call_method<T: Clone + 'static + Send + Sync>(
        &mut self,
        obj: impl AsRef<str>,
        method: impl AsRef<str>,
        args: impl FuncArgs,
    ) -> Result<T, Box<EvalAltResult>> {
        let scope = &mut self.scope;
        let scope2: &mut Scope<'static> = unsafe { &mut *(scope as *mut _) };
        let value = scope.get_value_mut::<Map>(obj.as_ref()).unwrap();
        let obj = scope2.get_mut(obj.as_ref()).unwrap();
        let om: FnPtr = value.get(method.as_ref()).unwrap().clone_cast();
        om.call_as_method(&self.engine, &self.ast, obj, args)
    }

    fn call<T: Clone + 'static + Send + Sync>(
        &mut self,
        fn_name: impl AsRef<str>,
        args: impl FuncArgs,
    ) -> Result<T, Box<EvalAltResult>> {
        let options = CallFnOptions::new().eval_ast(false).rewind_scope(true);
        self.engine.call_fn_with_options(options, &mut self.scope, &self.ast, fn_name, args)
    }
}
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

// Rhai to rust <([{
#[derive(Clone, Debug)]
struct Player {
    life: i32,
    consumers: Vec<LifeToRhai>,
}

impl Player {
    fn new() -> Self {
        Self { life: 10, consumers: Vec::new() }
    }

    fn adjust(&mut self, value: i64) {
        self.life += value as i32;
        if self.life <= 0 {
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

#[derive(Clone)]
struct PlayerProxy {
    p: *mut Player,
}

unsafe impl Send for PlayerProxy {}
unsafe impl Sync for PlayerProxy {}

impl PlayerProxy {
    fn proxy(rhai: &mut Rhai, player: &mut Player) {
        let proxy = PlayerProxy { p: player as *mut _ };
        rhai.engine.register_fn("show", PlayerProxy::show);
        rhai.engine.register_fn("adjust", PlayerProxy::adjust);
        rhai.engine.register_fn("get_helmet", PlayerProxy::get_helmet);
        rhai.engine.register_fn("set", PlayerProxy::set);
        rhai.scope.push("player", proxy);
    }

    fn get_helmet(self) -> bool {
        return false;
    }

    fn set(self, val: i64) {
        let player = unsafe { &mut *self.p };
        player.life = val as i32;
    }

    fn adjust(self, val: i64) {
        let player = unsafe { &mut *self.p };
        player.adjust(val);
    }

    fn show(&mut self, msg: &str) {
        println!("PlayerProxy{0}-{msg}", self.p as usize);
    }
}
// }])>

// LifeToRhai <([{
trait Life {
    fn on_player_die(&self, evt: Hurt, unused: i64) -> Revive;
}

#[derive(Clone, Debug)]
struct LifeToRhai(*mut Rhai);

impl Life for LifeToRhai {
    fn on_player_die(&self, evt: Hurt, unused: i64) -> Revive {
        let rhai = unsafe { &mut *self.0 };
        let rhai_call = unsafe { &mut *self.0 };
        let evt: Dynamic = rhai::serde::to_dynamic(evt).unwrap();
        let obj = rhai.search_impl_er("Life").unwrap();
        let ret: Dynamic = rhai_call.call_method(obj, "on_player_die", (evt, unused)).unwrap();
        rhai::serde::from_dynamic(&ret).unwrap()
    }
}
// }])>

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    rhai_mgr_new();
    let rhai_manager = rhai_mgr();
    let rhai_manager2 = rhai_mgr();
    let rhai = rhai_manager2.new_rhai("relic", RELIC);
    let mut player = Player::new();
    PlayerProxy::proxy(rhai, &mut player);
    rhai.eval_script();

    for (_, i) in rhai_manager.iter_rhai() {
        let x = i.search_impl_er("Life");
        if x.is_some() {
            player.consumers.push(LifeToRhai(i as *const _ as *mut _));
        }
    }
    dbg!(&player);
    let _: i64 = rhai.call("fight", ())?;
    dbg!(&player);

    Ok(())
}
