use std::error::Error;

use yunfengzh_monolith::rover::{Rover, run_code};

fn zyf(a: u32, b: u32) -> u32 {
    println!("zyf example is called{a}, {b}");
    a + b
}

fn fyz(a: u32) -> u32 {
    println!("fyz example is called{a}");
    a + 3
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut jit = Rover::new();
    jit.import_func("zyf".to_string(), vec!["a".to_string(), "b".to_string()], "c".to_string(), zyf as *const u8)?;
    jit.import_func("fyz".to_string(), vec!["a".to_string()], "c".to_string(), fyz as *const u8)?;
    println!("the answer oof is: {}", run_oof(&mut jit)?);
    println!("the answer foo is: {}", run_foo(&mut jit)?);
    Ok(())
}

fn run_foo(jit: &mut Rover) -> Result<isize, Box<dyn Error>> {
    unsafe { run_code(jit, FOO_CODE, (1, 0)) }
}

fn run_oof(jit: &mut Rover) -> Result<isize, Box<dyn Error>> {
    unsafe { run_code(jit, OOF_CODE, (1, 0)) }
}

const FOO_CODE: &str = r#"
    fn foo(a, b) -> (c) {
        let c = 27;
        c = c + zyf(1, 1);
        c = c + oof(6, 7);
        c
    }
"#;

const OOF_CODE: &str = r#"
    fn oof(a, b) -> (c) {
        let c = 72;
        c = c + fyz(3);
        c
    }
"#;
