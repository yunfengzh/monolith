struct B {
    f: f32,
}

impl Drop for B {
    fn drop(&mut self) {
        println!("B drop");
    }
}
struct A {
    i: u32,
    s: String,
    b: B,
}

impl Drop for A {
    fn drop(&mut self) {
        println!("A drop");
    }
}

fn main() {
    let mut a = A { i: 0, s: "".to_string(), b: B { f: 0.0 } };
    println!("start");
    a.b = B { f: 0.1 };
    println!("end");
}
