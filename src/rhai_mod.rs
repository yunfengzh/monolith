// vim: foldmarker=<([{,}])> foldmethod=marker

use std::{collections::HashMap, sync::Mutex};

use rhai::*;

use crate::stump::stump;

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

pub fn declare_trait(obj: String, trait_name: String) {
    println!("{obj} -- {trait_name}");
    stump().rhai_manager.trait_list.insert(trait_name, obj);
}

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

    pub fn new_rhai(&mut self, title: &str, script: &str) {
        if !self.init_stage {
            panic!("init stage has passed!");
        }
        self.data.insert(title.to_string(), Rhai::new(script));
    }

    pub fn load_rhai(&mut self, title: &str, v: Vec<(String, bool, Dynamic)>) {
        self.data.get_mut(title).unwrap().load(v);
    }
}

#[derive(Debug)]
pub struct Rhai {
    lock: Mutex<()>,
    pub engine: Engine,
    ast: AST,
    pub scope: Option<Scope<'static>>,
    script_var_cnt: u32,
    trait_list: HashMap<String, String>,
}

impl Rhai {
    pub fn new(script: &str) -> Self {
        let mut engine = Engine::new();
        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(64, 64);
        let mut scope = Scope::new();
        let ast = engine.compile(script).unwrap();
        engine.register_fn("declare_trait", declare_trait);
        let _: Dynamic = engine.eval_ast_with_scope(&mut scope, &ast).unwrap();
        let mgr = &mut stump().rhai_manager;
        let trait_list = mgr.trait_list.clone();
        mgr.trait_list.clear();
        let script_var_cnt = scope.len() as u32;
        Self { lock: Mutex::new(()), engine, ast, scope: Some(scope), script_var_cnt, trait_list }
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

    pub fn iter(&self) -> impl Iterator<Item = (&str, bool, Dynamic)> {
        RhaiIter(self.script_var_cnt, self.scope.as_ref().unwrap().iter())
    }

    pub fn search_trait(&self, trait_name: &str) -> Option<&String> {
        self.trait_list.get(trait_name)
    }

    pub fn call<T: Clone + 'static + Send + Sync>(
        &mut self,
        fn_name: impl AsRef<str>,
        args: impl FuncArgs,
    ) -> Result<T, Box<EvalAltResult>> {
        // let _unused = self.lock.lock();
        let options = CallFnOptions::new().eval_ast(false).rewind_scope(true);
        self.engine.call_fn_with_options(options, self.scope.as_mut().unwrap(), &self.ast, fn_name, args)
    }
}
// }])>
