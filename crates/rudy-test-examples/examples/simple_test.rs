fn function_call(x: i32) -> i32 {
    // Line 2
    // Line 3
    let y = x + 1; // Line 4 - test expects this line
    y
}

fn main() {
    let x = 42;
    let result = function_call(x);
    println!("Result: {result}");

    // Some additional lines to match the test expectations
    // Line 14
    // Line 15
    const Z: u64 = 0xdeadbeef; // Line 16 - test expects this line
    println!("Z = 0x{Z:x}"); // Line 17 - resolves to this line

    // Line 19
}
