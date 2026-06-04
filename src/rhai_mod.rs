// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! A MOD architecture based on https://rhai.rs/. The module encapsulates rhai into [RhaiMgr] and
//! [Rhai]. Later use rust to represent application side, rhai to represent rhai script. Rhai
//! script is treated as untrusted script.
//!
//! ## Memory Model
//!
//! Application should follow later steps at init stage
//!
//! 1. [RhaiMgr::new_rhai] to get Rhai pointer.
//! 2. in script, calls [Rhai::declare_trait] to register supported traits.
//! 3. repeat steps 1 and 2 until all scripts are loaded.
//! 4. [RhaiMgr::init_done], the method also closes step 1 and step 2 forever.
//!
//! the model makes all [Rhai] instances has a static lifetime even player uses LOAD to switch a
//! game context to another. And trait_to_rhai proc-macro can safely save '*mut Rhai' to its
//! generated objects. The only defect is player need restart his game when a MOD is added/removed.
//!
//! ## Flow between Rust and Rhai
//!
//! To achieve control from rust to rhai
//!
//! 1. rust need define some traits for rhai to implement, by these traits, rust can inject events
//!    etc to rhai.
//! 2. Script need call [Rhai::declare_trait] to declare which traits are supported in its global
//!    statements. It is a system function only available when a Rhai instance is setup. It's
//!    advised that global statements only include `declare_trait(...)` and script vars.
//! 3. proc-macro [trait_to_rhai] for automatically generate code from rust to rhai.
//!
//! To achieve control from rhai to rust
//!
//! 1. Typically, it's called API by [Engine::register_fn], but I recommend group them by
//! obj by [Scope::push].
//! 2. You can also define a proxy var to make rhai access rust inner var. Don't worry, if there
//!    isn't Proxy::set/get method, the inner field of proxy var can't be accessed by rhai script.
//!
//! These vars are called system vars.
//!
//! ## Share data between rust and rhai
//!
//! In fact, shared data is also the part of a protocol or API. That is, you need doc the struct of
//! the data then the struct with `#derive[Serialize, Deserialize]`, rhai will do the remain
//! translation.
//!
//! ## Load/Save Script
//!
//! 1. To save script vars, [Rhai::iter_script_vars].
//! 2. To load script vars, [Rhai::load].
//! 3. It's up to you to decide how to save system vars.
//!
//! ## [Rhai::toplevel_lock]
//! When application calls rhai function/method initiatively or load/save, it must calls
//! [Rhai::toplevel_lock] to prevent potential race on Rhai. [Rhai::lock] can't be placed into
//! [Rhai::call] due to rhai maybe tries to call [Rhai::call] in API then lead to deadlock.
//!
//! `example/rhai.rs` is the best way to start.

use std::collections::HashMap;

use rhai::*;
use tokio::sync::{Mutex, MutexGuard};

use crate::stump::stump;
// }])>

// RhaiMgr <([{
#[derive(Debug)]
pub struct RhaiMgr {
    data: HashMap<String, Rhai>,
    trait_list: HashMap<String, String>,
    init_stage: bool,
}

impl RhaiMgr {
    pub(crate) fn new() -> Self {
        Self { data: HashMap::new(), trait_list: HashMap::new(), init_stage: true }
    }

    pub fn init_done(&mut self) {
        self.init_stage = false;
    }

    pub fn new_rhai(&mut self, title: &str, script: &str) -> *mut Rhai {
        if !self.init_stage {
            panic!("init stage has passed!");
        }
        self.data.insert(title.to_string(), Rhai::new(script));
        self.data.get_mut(title).unwrap() as *mut _
    }

    pub fn get_rhai(&mut self, title: &str) -> *mut Rhai {
        self.data.get_mut(title).unwrap() as *mut _
    }
}
// }])>

// Rhai and RhaiIter <([{
struct RhaiIter<A>(u32, A);

impl<'a, A: Iterator<Item = (&'a str, bool, Dynamic)>> Iterator for RhaiIter<A> {
    type Item = (&'a str, bool, Dynamic);

    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            None
        } else {
            self.0 -= 1;
            self.1.next()
        }
    }
}

#[derive(Debug)]
pub struct Rhai {
    lock: Mutex<()>,
    pub engine: Engine,
    ast: AST,
    pub scope: Scope<'static>,
    system_vars_range: (u32, u32),
    script_vars_range: (u32, u32),
    trait_list: HashMap<String, String>,
}

impl Rhai {
    fn new(script: &str) -> Self {
        let mut engine = Engine::new();
        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(64, 64);
        let ast = engine.compile(script).unwrap();
        Self {
            lock: Mutex::new(()),
            engine,
            ast,
            scope: Scope::new(),
            system_vars_range: (0, 0),
            script_vars_range: (0, 0),
            trait_list: HashMap::new(),
        }
    }

    pub fn eval_script(&mut self) {
        let scope = &mut self.scope;
        let system_vars_end = scope.len() as u32;
        self.system_vars_range = (0, system_vars_end);
        self.engine.register_fn("declare_trait", Rhai::declare_trait);
        let _: Dynamic = self.engine.eval_ast_with_scope(scope, &self.ast).unwrap();
        let mgr = &mut stump().rhai_manager;
        self.trait_list = mgr.trait_list.clone();
        mgr.trait_list = HashMap::new();
        let script_vars_end = scope.len() as u32;
        self.script_vars_range = (system_vars_end, script_vars_end);
    }

    /// toplevel_lock() is used by rust to launch a request to a script initiatively, or load/save
    /// context.
    pub async fn toplevel_lock(&mut self) -> MutexGuard<'_, ()> {
        self.lock.lock().await
    }

    fn declare_trait(obj: String, trait_name: String) {
        stump().rhai_manager.trait_list.insert(trait_name, obj);
    }

    pub fn load_init(&mut self) {
        self.scope = Scope::new();
    }
    pub fn load_script_vars(&mut self, v: Vec<(String, bool, Dynamic)>) {
        let scope = &mut self.scope;
        for tuple in v {
            if tuple.1 {
                scope.push_constant_dynamic(tuple.0, tuple.2);
            } else {
                scope.push_dynamic(tuple.0, tuple.2);
            }
        }
    }

    pub fn iter_script_vars(&self) -> impl Iterator<Item = (&str, bool, Dynamic)> {
        RhaiIter(
            self.script_vars_range.1 - self.script_vars_range.0,
            self.scope.iter().skip(self.script_vars_range.0 as usize),
        )
    }

    pub fn iter_all_vars(&self) -> impl Iterator<Item = (&str, bool, Dynamic)> {
        self.scope.iter()
    }

    pub fn search_impl_er(&self, trait_name: &str) -> Option<&String> {
        self.trait_list.get(trait_name)
    }

    pub fn call_method<T: Clone + 'static + Send + Sync>(
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

    pub fn call<T: Clone + 'static + Send + Sync>(
        &mut self,
        fn_name: impl AsRef<str>,
        args: impl FuncArgs,
    ) -> Result<T, Box<EvalAltResult>> {
        let options = CallFnOptions::new().eval_ast(false).rewind_scope(true);
        self.engine.call_fn_with_options(options, &mut self.scope, &self.ast, fn_name, args)
    }
}
// }])>
