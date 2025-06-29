fn function_call(x: i32) -> i32 {
    let y = x + 1;
    y + 2
}

fn main() {
    let x = 42;
    let result = function_call(x);
    println!("Result: {result}");

    const Z: u64 = 0xdeadbeef;
    println!("Z = 0x{Z:x}");
}
