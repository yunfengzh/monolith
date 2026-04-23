// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! Memory model: Application should calls later at the begin of it
//!   1. [RhaiMgr::new_rhai] multiple times
//!   2. [RhaiMgr::init_done]
//! which makes all [Rhai] instances has a static lifetime even player uses LOAD to switch a game
//! context to another. And trait_to_rhai proc-macro can safely save '*mut Rhai' to its generated
//! objects. The only defect is player need restart his game when a MOD is added/removed.
//!
//! Rust to rhai: by trait_to_rhai macro.
//! Rhai to rust: by API, grouped by objs (system vars).
//!
//! Rhai script has two kinds of vars: one is called script var, another is system-level var.

use std::collections::HashMap;

use rhai::*;
use tokio::sync::{Mutex, MutexGuard};

use crate::stump::stump;
// }])>

// Rhai <([{
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
pub struct RhaiMgr {
    data: HashMap<String, Rhai>,
    trait_list: Option<HashMap<String, String>>, // TODO: the field is Rhai::trait_list, class var
    init_stage: bool,
}

impl RhaiMgr {
    pub(crate) fn new() -> Self {
        Self { data: HashMap::new(), trait_list: Some(HashMap::new()), init_stage: true }
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

    pub fn load_rhai(&mut self, title: &str, v: Vec<(String, bool, Dynamic)>) {
        self.data.get_mut(title).unwrap().load(v);
    }
}

#[derive(Debug)]
pub struct Rhai {
    lock: Mutex<()>, // TODO: name
    pub engine: Engine,
    ast: AST,
    pub scope: Option<Scope<'static>>,
    script_var_cnt: u32,
    trait_list: HashMap<String, String>,
}

impl Rhai {
    fn new(script: &str) -> Self {
        let mut engine = Engine::new();
        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(64, 64);
        let mut scope = Scope::new();
        let ast = engine.compile(script).unwrap();
        engine.register_fn("declare_trait", Rhai::declare_trait);
        let _: Dynamic = engine.eval_ast_with_scope(&mut scope, &ast).unwrap();
        let mgr = &mut stump().rhai_manager;
        let trait_list = mgr.trait_list.take().unwrap();
        mgr.trait_list = Some(HashMap::new());
        let script_var_cnt = scope.len() as u32;
        Self { lock: Mutex::new(()), engine, ast, scope: Some(scope), script_var_cnt, trait_list }
    }

    /// TODO: be used to sync all requests from game. Rhai::call can be called from script
    /// internally.
    pub async fn lock(&mut self) -> MutexGuard<'_, ()> {
        self.lock.lock().await
    }

    fn declare_trait(obj: String, trait_name: String) {
        stump().rhai_manager.trait_list.as_mut().unwrap().insert(trait_name, obj);
    }

    pub fn load(&mut self, v: Vec<(String, bool, Dynamic)>) {
        self.scope.take();
        self.scope = Some(Scope::new());
        let scope = self.scope.as_mut().unwrap();
        for tuple in v {
            if tuple.1 {
                scope.push_constant_dynamic(tuple.0, tuple.2);
            } else {
                scope.push_dynamic(tuple.0, tuple.2);
            }
        }
    }

    pub fn iter_script_vars(&self) -> impl Iterator<Item = (&str, bool, Dynamic)> {
        RhaiIter(self.script_var_cnt, self.scope.as_ref().unwrap().iter())
    }

    pub fn iter_all_vars(&self) -> impl Iterator<Item = (&str, bool, Dynamic)> {
        self.scope.as_ref().unwrap().iter()
    }

    pub fn search_trait(&self, trait_name: &str) -> Option<&String> {
        self.trait_list.get(trait_name)
    }

    pub fn call<T: Clone + 'static + Send + Sync>(
        &mut self,
        fn_name: impl AsRef<str>,
        args: impl FuncArgs,
    ) -> Result<T, Box<EvalAltResult>> {
        let options = CallFnOptions::new().eval_ast(false).rewind_scope(true);
        self.engine.call_fn_with_options(options, self.scope.as_mut().unwrap(), &self.ast, fn_name, args)
    }
}
// }])>
