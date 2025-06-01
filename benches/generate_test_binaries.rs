//! Generate test binaries of varying sizes for benchmarking
//!
//! Run with: cargo run --bin generate_test_binaries

use std::fs;
use std::io::Write;
use std::process::Command;

fn main() -> std::io::Result<()> {
    println!("🔨 Generating test binaries...\n");

    // Create test directory
    fs::create_dir_all("benches/test_binaries")?;

    // Generate small, medium, and large test programs
    generate_test_binary("small", 10, 5)?;
    generate_test_binary("medium", 100, 20)?;
    generate_test_binary("large", 500, 50)?;

    println!("\n✅ Test binaries generated in benches/test_binaries/");

    Ok(())
}

fn generate_test_binary(
    name: &str,
    num_structs: usize,
    functions_per_struct: usize,
) -> std::io::Result<()> {
    println!(
        "Generating {} binary ({} structs, {} functions each)...",
        name, num_structs, functions_per_struct
    );

    let mut code = String::new();

    // Generate headers
    code.push_str("#![allow(dead_code)]\n");
    code.push_str("use std::collections::HashMap;\n");
    code.push_str("use std::vec::Vec;\n\n");

    // Generate structs with various types
    for i in 0..num_structs {
        // Basic struct
        code.push_str(&format!(
            r#"
#[derive(Debug, Clone)]
pub struct TestStruct{i} {{
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
}}

impl TestStruct{i} {{
"#
        ));

        // Generate methods
        for j in 0..functions_per_struct {
            code.push_str(&format!(
                r#"
    pub fn method_{}(&self, param: i32) -> i32 {{
        let mut result = param;
        for i in 0..10 {{
            result = result.wrapping_add(i);
            if i % 2 == 0 {{
                result = result.wrapping_mul(2);
            }}
        }}
        result + self.id as i32
    }}
"#,
                j
            ));
        }

        code.push_str("}\n\n");

        // Generate some enums and nested types
        if i % 5 == 0 {
            code.push_str(&format!(
                r#"
#[derive(Debug)]
pub enum TestEnum{i} {{
    Variant1(i32),
    Variant2 {{ x: f64, y: f64 }},
    Variant3(TestStruct{}),
}}

pub type TestAlias{i} = HashMap<String, TestStruct{i}>;
"#,
                i / 5
            ));
        }
    }

    // Generate main function that uses the types
    code.push_str(
        r#"
fn main() {
    println!("Test binary for benchmarking");
    
    // Use some of the types to ensure they're not optimized out
    let mut total = 0u64;
"#,
    );

    for i in 0..num_structs.min(10) {
        let meth = i % functions_per_struct; // Cycle through methods
        code.push_str(&format!(
            r#"
    {{
        let s = TestStruct{i} {{
            id: {i},
            name: "test{i}".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::new(),
        }};
        total += s.id;
        
        // Call a method to ensure it's in the binary
        let _ = s.method_{meth}({i});
    }}
"#,
        ));
    }

    code.push_str(
        r#"
    // Some computation to prevent optimization
    println!("Total: {}", total);
    
    // Create some enum values
    let _e1 = TestEnum0::Variant1(42);
    
    // Use type aliases
    let _map: TestAlias0 = HashMap::new();
}
"#,
    );

    // Write source file
    let src_path = format!("benches/test_binaries/{}.rs", name);
    let mut file = fs::File::create(&src_path)?;
    file.write_all(code.as_bytes())?;

    // Compile with debug info
    let output = Command::new("rustc")
        .args(&[
            &src_path,
            "-g", // Generate debug info
            "-C",
            "opt-level=0", // No optimization
            "-C",
            "split-debuginfo=unpacked", // Unpacked debug info
            "-C",
            "debuginfo=2", // Full debug info
            "-o",
            &format!("benches/test_binaries/{}", name),
        ])
        .output()
        .expect("Failed to execute rustc");

    if !output.status.success() {
        eprintln!("Failed to compile {}:", name);
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }

    // Also create a release version with debug info for comparison
    let _output = Command::new("rustc")
        .args(&[
            &src_path,
            "-g", // Generate debug info
            "-C",
            "opt-level=3", // Full optimization
            "-C",
            "split-debuginfo=unpacked", // Unpacked debug info
            "-C",
            "debuginfo=2", // Full debug info
            "-o",
            &format!("benches/test_binaries/{}_release", name),
        ])
        .output()
        .expect("Failed to execute rustc");

    if !_output.status.success() {
        eprintln!("Failed to compile {} (release):", name);
        eprintln!("{}", String::from_utf8_lossy(&_output.stderr));
    }

    Ok(())
}
