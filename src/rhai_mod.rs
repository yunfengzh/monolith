// vim: foldmarker=<([{,}])> foldmethod=marker

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

#[derive(Debug)]
pub struct Rhai<'a> {
    pub engine: Engine,
    ast: AST,
    pub scope: Scope<'a>,
    script_var_cnt: u32,
}

impl<'a> Rhai<'a> {
    pub fn new(script: &str) -> Self {
        let mut engine = Engine::new();
        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(64, 64);
        let mut scope = Scope::new();
        let ast = engine.compile(script).unwrap();
        let _: Dynamic = engine.eval_ast_with_scope(&mut scope, &ast).unwrap();
        let script_var_cnt = scope.len() as u32;
        Self { engine, ast, scope, script_var_cnt }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, bool, Dynamic)> {
        RhaiIter(self.script_var_cnt, self.scope.iter())
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
