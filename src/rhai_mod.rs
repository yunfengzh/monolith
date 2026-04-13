// vim: foldmarker=<([{,}])> foldmethod=marker

use std::{
    collections::HashMap,
    sync::{LazyLock, RwLock},
};

use rhai::*;

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
    RHAIMGR.write().unwrap().trait_list.insert(trait_name, obj);
}

static RHAIMGR: LazyLock<RwLock<RhaiMgr>> = LazyLock::new(|| RwLock::new(RhaiMgr::new()));

struct RhaiMgr {
    trait_list: HashMap<String, String>,
    register_trait: bool, // TODO: design disable the field after all scripts are loaded.
}

unsafe impl Send for RhaiMgr {}
unsafe impl Sync for RhaiMgr {}

impl RhaiMgr {
    fn new() -> Self {
        Self { trait_list: HashMap::new(), register_trait: false }
    }
}

#[derive(Debug)]
pub struct Rhai {
    pub engine: Engine,
    ast: AST,
    pub scope: Scope<'static>,
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
        let trait_list = RHAIMGR.read().unwrap().trait_list.clone();
        let script_var_cnt = scope.len() as u32;
        Self { engine, ast, scope, script_var_cnt, trait_list }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, bool, Dynamic)> {
        RhaiIter(self.script_var_cnt, self.scope.iter())
    }

    pub fn search_trait(&self, trait_name: &str) -> Option<&String> {
        self.trait_list.get(trait_name)
    }

    pub fn call<T: Clone + 'static>(
        &mut self,
        fn_name: impl AsRef<str>,
        args: impl FuncArgs,
    ) -> Result<T, Box<EvalAltResult>> {
        let options = CallFnOptions::new().eval_ast(false).rewind_scope(true);
        self.engine.call_fn_with_options(options, &mut self.scope, &self.ast, fn_name, args)
    }
}
// }])>
