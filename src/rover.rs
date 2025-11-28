// vim: foldmarker=<([{,}])> foldmethod=marker

// Module level Doc <([{
//! [Rover] is a programable embeded rust-like JIT engine. It can be used for rogue-like game for
//! base-stone, card game to define a new card, 4x game to adjust country income based on random
//! event or leader ability or nationality, develop MOD etc.
//!
//! Safety: don't use std::Vec as the notify queue of a special event since add/del the queue may
//! make the reference passed to my engine dangle.
//!
//! https://gitee.com/mirrors_bytecodealliance/cranelift-jit-demo.git

use std::{
    collections::HashMap,
    error::Error,
    hash::{BuildHasherDefault, DefaultHasher},
    mem,
};

use cranelift_codegen::{
    ir::{AbiParam, Block, BlockArg, InstBuilder, Value, condcodes::IntCC, types},
    settings::{self, Configurable},
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
// }])>

pub enum ASTNode {
    Literal(String),
    Identifier(String),
    Field(String),
    Assign(String, Box<ASTNode>),
    Or(Box<ASTNode>, Box<ASTNode>),
    And(Box<ASTNode>, Box<ASTNode>),
    Eq(Box<ASTNode>, Box<ASTNode>),
    Ne(Box<ASTNode>, Box<ASTNode>),
    Lt(Box<ASTNode>, Box<ASTNode>),
    Le(Box<ASTNode>, Box<ASTNode>),
    Gt(Box<ASTNode>, Box<ASTNode>),
    Ge(Box<ASTNode>, Box<ASTNode>),
    Add(Box<ASTNode>, Box<ASTNode>),
    Sub(Box<ASTNode>, Box<ASTNode>),
    Mul(Box<ASTNode>, Box<ASTNode>),
    Div(Box<ASTNode>, Box<ASTNode>),
    IfElse(Box<ASTNode>, Vec<ASTNode>, Vec<ASTNode>),
    WhileLoop(Box<ASTNode>, Vec<ASTNode>),
    Call(String, Vec<ASTNode>),
    GlobalDataAddr(String),
}

static mut UFUN: HashMap<String, (FuncId, *const u8), BuildHasherDefault<DefaultHasher>> =
    HashMap::with_hasher(BuildHasherDefault::new());

fn symbol_lookup(name: &str) -> Option<*const u8> {
    unsafe { Some((*(&raw mut UFUN)).get(name).unwrap().1) }
}

pub unsafe fn run_code<I, O>(jit: &mut Rover, code: &str, input: I) -> Result<O, Box<dyn Error>> {
    unsafe {
        let code_ptr = jit.compile(code)?;
        let code_fn = mem::transmute::<_, fn(I) -> O>(code_ptr);
        Ok(code_fn(input))
    }
}

// Syntax <([{
peg::parser!(pub grammar parser() for str {
    pub rule function() -> (String, Vec<String>, String, Vec<ASTNode>)
        = [' ' | '\t' | '\n']* "fn" _ name:identifier() _
        "(" params:((_ i:identifier() _ {i}) ** ",") ")" _
        "->" _
        "(" returns:(_ i:identifier() _ {i}) ")" _
        "{" _ "\n"
        stmts:statements()
        _ "}" _ "\n" _
        { (name, params, returns, stmts) }

    rule statements() -> Vec<ASTNode>
        = s:(statement()*) { s }

    rule statement() -> ASTNode
        = _ e:expression() _ ";" "\n" { e }
        / _ e:expression() _ "\n" { e }

    rule expression() -> ASTNode
        = if_else()
        / while_loop()
        / assignment()
        / binary_op()

    rule if_else() -> ASTNode
        = "if" _ e:expression() _ "{" _ "\n"
        then_body:statements() _ "}" _ "else" _ "{" _ "\n"
        else_body:statements() _ "}"
        { ASTNode::IfElse(Box::new(e), then_body, else_body) }

    rule while_loop() -> ASTNode
        = "while" _ e:expression() _ "{" _ "\n"
        loop_body:statements() _ "}"
        { ASTNode::WhileLoop(Box::new(e), loop_body) }

    rule assignment() -> ASTNode
        = "let"? _ i:identifier() _ "=" _ e:expression() {ASTNode::Assign(i, Box::new(e))}

    rule binary_op() -> ASTNode = precedence!{
        a:@ _ "||" _ b:(@) { ASTNode::Or(Box::new(a), Box::new(b)) }
        --
        a:@ _ "&&" _ b:(@) { ASTNode::And(Box::new(a), Box::new(b)) }
        --
        a:@ _ "==" _ b:(@) { ASTNode::Eq(Box::new(a), Box::new(b)) }
        a:@ _ "!=" _ b:(@) { ASTNode::Ne(Box::new(a), Box::new(b)) }
        a:@ _ "<"  _ b:(@) { ASTNode::Lt(Box::new(a), Box::new(b)) }
        a:@ _ "<=" _ b:(@) { ASTNode::Le(Box::new(a), Box::new(b)) }
        a:@ _ ">"  _ b:(@) { ASTNode::Gt(Box::new(a), Box::new(b)) }
        a:@ _ ">=" _ b:(@) { ASTNode::Ge(Box::new(a), Box::new(b)) }
        --
        a:@ _ "+" _ b:(@) { ASTNode::Add(Box::new(a), Box::new(b)) }
        a:@ _ "-" _ b:(@) { ASTNode::Sub(Box::new(a), Box::new(b)) }
        --
        a:@ _ "*" _ b:(@) { ASTNode::Mul(Box::new(a), Box::new(b)) }
        a:@ _ "/" _ b:(@) { ASTNode::Div(Box::new(a), Box::new(b)) }
        --
        i:identifier() _ "(" args:((_ e:expression() _ {e}) ** ",") ")" { ASTNode::Call(i, args) }
        i:identifier() { ASTNode::Identifier(i) }
        // pf:postfix() { pf }
        l:literal() { l }
    }

    rule postfix() -> ASTNode
        = i:identifier() tail:(_ "." _ id:identifier() {id})* {
            ASTNode::Field(tail.into_iter().fold(i, |acc, field| acc + "." + &field))
    }

    rule identifier() -> String
        = quiet!{ n:$(['a'..='z' | 'A'..='Z' | '_']['a'..='z' | 'A'..='Z' | '0'..='9' | '_']*) { n.to_owned() } }
        / expected!("identifier")

    rule literal() -> ASTNode
        = n:$(['0'..='9']+) { ASTNode::Literal(n.to_owned()) }
        / "&" i:identifier() { ASTNode::GlobalDataAddr(i) }

    rule _() =  quiet!{[' ' | '\t']*}
});
// }])>

// Rover <([{
pub struct Rover {
    ctx: cranelift_codegen::Context,

    module: JITModule,
}

impl Rover {
    pub fn new() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        let isa_builder = cranelift_native::builder().unwrap_or_else(|msg| {
            panic!("host machine is not supported: {}", msg);
        });
        let isa = isa_builder.finish(settings::Flags::new(flag_builder)).unwrap();
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol_lookup_fn(Box::new(symbol_lookup));
        // builder.symbol("zyf", zyf as *const u8);

        let module = JITModule::new(builder);
        Self { ctx: module.make_context(), module }
    }

    pub fn import_func(
        &mut self,
        func: String,
        params: Vec<String>,
        _result: String,
        addr: *const u8,
    ) -> Result<(), Box<dyn Error>> {
        let int = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();

        for _arg in params {
            sig.params.push(AbiParam::new(int));
        }

        sig.returns.push(AbiParam::new(int));

        let decl_func = self.module.declare_function(&func, Linkage::Import, &sig).expect("problem declaring function");

        unsafe {
            (*(&raw mut UFUN)).insert(func.to_string(), (decl_func, addr));
        }

        Ok(())
    }

    pub fn compile(&mut self, input: &str) -> Result<*const u8, String> {
        let (name, params, the_return, stmts) = parser::function(input).map_err(|e| e.to_string())?;

        self.translate(params, the_return, stmts)?;
        println!("{:?}", self.ctx.func);

        let id = self
            .module
            .declare_function(&name, Linkage::Export, &self.ctx.func.signature)
            .map_err(|e| e.to_string())?;
        self.module.define_function(id, &mut self.ctx).map_err(|e| e.to_string())?;
        self.module.clear_context(&mut self.ctx);
        self.module.finalize_definitions().unwrap();

        Ok(self.module.get_finalized_function(id))
    }

    fn translate(&mut self, params: Vec<String>, the_return: String, nodes: Vec<ASTNode>) -> Result<(), String> {
        let int = self.module.target_config().pointer_type();

        for _p in &params {
            self.ctx.func.signature.params.push(AbiParam::new(int));
        }

        self.ctx.func.signature.returns.push(AbiParam::new(int));

        let mut builder = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut builder);

        let entry_block = builder.create_block();

        builder.append_block_params_for_function_params(entry_block);

        builder.switch_to_block(entry_block);

        builder.seal_block(entry_block);

        let variables = declare_variables(int, &mut builder, &params, &the_return, &nodes, entry_block);

        let mut trans = ASTToIR { int, builder, variables, module: &mut self.module };
        for i in nodes {
            trans.translate_astnode(i);
        }

        let return_variable = trans.variables.get(&the_return).unwrap();
        let return_value = trans.builder.use_var(*return_variable);

        trans.builder.ins().return_(&[return_value]);

        trans.builder.finalize();
        Ok(())
    }
}
// }])>

// AST to IR <([{
struct ASTToIR<'a> {
    int: types::Type,
    builder: FunctionBuilder<'a>,
    variables: HashMap<String, Variable>,
    module: &'a mut JITModule,
}

impl<'a> ASTToIR<'a> {
    fn translate_astnode(&mut self, node: ASTNode) -> Value {
        match node {
            ASTNode::Literal(literal) => {
                let imm: i32 = literal.parse().unwrap();
                self.builder.ins().iconst(self.int, i64::from(imm))
            }

            ASTNode::Add(lhs, rhs) => {
                let lhs = self.translate_astnode(*lhs);
                let rhs = self.translate_astnode(*rhs);
                self.builder.ins().iadd(lhs, rhs)
            }

            ASTNode::Sub(lhs, rhs) => {
                let lhs = self.translate_astnode(*lhs);
                let rhs = self.translate_astnode(*rhs);
                self.builder.ins().isub(lhs, rhs)
            }

            ASTNode::Mul(lhs, rhs) => {
                let lhs = self.translate_astnode(*lhs);
                let rhs = self.translate_astnode(*rhs);
                self.builder.ins().imul(lhs, rhs)
            }

            ASTNode::Div(lhs, rhs) => {
                let lhs = self.translate_astnode(*lhs);
                let rhs = self.translate_astnode(*rhs);
                self.builder.ins().udiv(lhs, rhs)
            }

            ASTNode::Eq(lhs, rhs) => self.translate_icmp(IntCC::Equal, *lhs, *rhs),
            ASTNode::Ne(lhs, rhs) => self.translate_icmp(IntCC::NotEqual, *lhs, *rhs),
            ASTNode::Lt(lhs, rhs) => self.translate_icmp(IntCC::SignedLessThan, *lhs, *rhs),
            ASTNode::Le(lhs, rhs) => self.translate_icmp(IntCC::SignedLessThanOrEqual, *lhs, *rhs),
            ASTNode::Gt(lhs, rhs) => self.translate_icmp(IntCC::SignedGreaterThan, *lhs, *rhs),
            ASTNode::Ge(lhs, rhs) => self.translate_icmp(IntCC::SignedGreaterThanOrEqual, *lhs, *rhs),
            ASTNode::And(lhs, rhs) => {
                let lhs = self.translate_astnode(*lhs);
                let rhs = self.translate_astnode(*rhs);
                self.builder.ins().band(lhs, rhs)
            }
            ASTNode::Or(lhs, rhs) => {
                let lhs = self.translate_astnode(*lhs);
                let rhs = self.translate_astnode(*rhs);
                self.builder.ins().bor(lhs, rhs)
            }
            ASTNode::Field(field) => {
                println!("field--{:?}", field);
                self.builder.ins().iconst(self.int, 0)
            }
            ASTNode::Call(name, args) => self.translate_call(name, args),
            ASTNode::GlobalDataAddr(name) => self.translate_global_data_addr(name),
            ASTNode::Identifier(name) => {
                let variable = self.variables.get(&name).expect("variable not defined");
                self.builder.use_var(*variable)
            }
            ASTNode::Assign(name, expr) => self.translate_assign(name, *expr),
            ASTNode::IfElse(condition, then_body, else_body) => {
                self.translate_if_else(*condition, then_body, else_body)
            }
            ASTNode::WhileLoop(condition, loop_body) => self.translate_while_loop(*condition, loop_body),
        }
    }

    fn translate_assign(&mut self, name: String, expr: ASTNode) -> Value {
        let new_value = self.translate_astnode(expr);
        let variable = self.variables.get(&name).unwrap();
        self.builder.def_var(*variable, new_value);
        new_value
    }

    fn translate_icmp(&mut self, cmp: IntCC, lhs: ASTNode, rhs: ASTNode) -> Value {
        let lhs = self.translate_astnode(lhs);
        let rhs = self.translate_astnode(rhs);
        self.builder.ins().icmp(cmp, lhs, rhs)
    }

    fn translate_if_else(&mut self, condition: ASTNode, then_body: Vec<ASTNode>, else_body: Vec<ASTNode>) -> Value {
        let condition_value = self.translate_astnode(condition);

        let then_block = self.builder.create_block();
        let else_block = self.builder.create_block();
        let merge_block = self.builder.create_block();

        self.builder.append_block_param(merge_block, self.int);

        self.builder.ins().brif(condition_value, then_block, &[], else_block, &[]);

        self.builder.switch_to_block(then_block);
        self.builder.seal_block(then_block);
        let mut then_return = self.builder.ins().iconst(self.int, 0);
        for stmt in then_body {
            then_return = self.translate_astnode(stmt);
        }

        self.builder.ins().jump(merge_block, &[BlockArg::Value(then_return)]);

        self.builder.switch_to_block(else_block);
        self.builder.seal_block(else_block);
        let mut else_return = self.builder.ins().iconst(self.int, 0);
        for stmt in else_body {
            else_return = self.translate_astnode(stmt);
        }

        self.builder.ins().jump(merge_block, &[BlockArg::Value(else_return)]);

        self.builder.switch_to_block(merge_block);

        self.builder.seal_block(merge_block);

        let phi = self.builder.block_params(merge_block)[0];

        phi
    }

    fn translate_while_loop(&mut self, condition: ASTNode, loop_body: Vec<ASTNode>) -> Value {
        let header_block = self.builder.create_block();
        let body_block = self.builder.create_block();
        let exit_block = self.builder.create_block();

        self.builder.ins().jump(header_block, &[]);
        self.builder.switch_to_block(header_block);

        let condition_value = self.translate_astnode(condition);
        self.builder.ins().brif(condition_value, body_block, &[], exit_block, &[]);

        self.builder.switch_to_block(body_block);
        self.builder.seal_block(body_block);

        for stmt in loop_body {
            self.translate_astnode(stmt);
        }
        self.builder.ins().jump(header_block, &[]);

        self.builder.switch_to_block(exit_block);

        self.builder.seal_block(header_block);
        self.builder.seal_block(exit_block);

        self.builder.ins().iconst(self.int, 0)
    }

    fn translate_call(&mut self, name: String, args: Vec<ASTNode>) -> Value {
        let user_func = unsafe { (*(&raw mut UFUN)).get(&name) };
        let func_id = if user_func.is_some() {
            user_func.unwrap().0
        } else {
            let mut sig = self.module.make_signature();

            for _arg in &args {
                sig.params.push(AbiParam::new(self.int));
            }

            sig.returns.push(AbiParam::new(self.int));

            self.module.declare_function(&name, Linkage::Import, &sig).unwrap()
        };
        let local_callee = self.module.declare_func_in_func(func_id, self.builder.func);

        let mut arg_values = Vec::new();
        for arg in args {
            arg_values.push(self.translate_astnode(arg))
        }
        let call = self.builder.ins().call(local_callee, &arg_values);

        self.builder.inst_results(call)[0]
    }

    fn translate_global_data_addr(&mut self, name: String) -> Value {
        let sym = self.module.declare_data(&name, Linkage::Export, true, false).expect("problem declaring data object");
        let local_id = self.module.declare_data_in_func(sym, self.builder.func);

        let pointer = self.module.target_config().pointer_type();
        self.builder.ins().symbol_value(pointer, local_id)
    }
}

fn declare_variables(
    int: types::Type,
    builder: &mut FunctionBuilder,
    params: &[String],
    the_return: &str,
    stmts: &[ASTNode],
    entry_block: Block,
) -> HashMap<String, Variable> {
    let mut variables = HashMap::new();
    for (i, name) in params.iter().enumerate() {
        let val = builder.block_params(entry_block)[i];
        let var = declare_variable(int, builder, &mut variables, name);
        builder.def_var(var, val);
    }
    let zero = builder.ins().iconst(int, 0);
    let return_variable = declare_variable(int, builder, &mut variables, the_return);
    builder.def_var(return_variable, zero);
    for expr in stmts {
        declare_variables_in_stmt(int, builder, &mut variables, expr);
    }

    variables
}

fn declare_variables_in_stmt(
    int: types::Type,
    builder: &mut FunctionBuilder,
    variables: &mut HashMap<String, Variable>,
    expr: &ASTNode,
) {
    match *expr {
        ASTNode::Assign(ref name, _) => {
            declare_variable(int, builder, variables, name);
        }
        ASTNode::IfElse(ref _condition, ref then_body, ref else_body) => {
            for stmt in then_body {
                declare_variables_in_stmt(int, builder, variables, stmt);
            }
            for stmt in else_body {
                declare_variables_in_stmt(int, builder, variables, stmt);
            }
        }
        ASTNode::WhileLoop(ref _condition, ref loop_body) => {
            for stmt in loop_body {
                declare_variables_in_stmt(int, builder, variables, stmt);
            }
        }
        _ => (),
    }
}

fn declare_variable(
    int: types::Type,
    builder: &mut FunctionBuilder,
    variables: &mut HashMap<String, Variable>,
    name: &str,
) -> Variable {
    *variables.entry(name.into()).or_insert_with(|| builder.declare_var(int))
}
// }])>

// mod tests <([{
#[cfg(test)]
mod tests {
    pub(crate) mod rover_boundaryclass {
        pub fn callab(a: u32, b: u32) -> u32 {
            a + b
        }

        pub fn calla3(a: u32) -> u32 {
            a + 3
        }
    }

    use crate::rover::{Rover, run_code};
    use rover_boundaryclass::*;

    // Test call relationship.
    #[tokio::test]
    async fn rover_call() {
        let mut rover = Rover::new();
        rover
            .import_func(
                "callab".to_string(),
                vec!["a".to_string(), "b".to_string()],
                "c".to_string(),
                callab as *const u8,
            )
            .unwrap();
        rover.import_func("calla3".to_string(), vec!["a".to_string()], "c".to_string(), calla3 as *const u8).unwrap();

        const OOF_CODE: &str = r#"
            fn oof(a, b) -> (c) {
                let c = 72 + a;
                c = calla3(b + c);
            }
        "#;

        const FOO_CODE: &str = r#"
            fn foo(a, b) -> (c) {
                let c = 27;
                c = c + callab(1, 1);
                c = c + oof(6, 7);
                c
            }
        "#;

        unsafe {
            let x = run_code::<(isize, isize), isize>(&mut rover, OOF_CODE, (3, 7)).unwrap();
            assert_eq!(x, 85);
            let y = run_code::<(isize, isize), isize>(&mut rover, FOO_CODE, (1, 0)).unwrap();
            assert_eq!(y, 117);
        }
    }

    // Test field syntax.
    #[tokio::test]
    async fn rover_field() {}

    // Test loop syntax (continue/break).
    #[tokio::test]
    async fn rover_loop() {}
}
// }])>
