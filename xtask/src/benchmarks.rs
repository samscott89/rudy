use std::{fs, io::Write, path::Path, process::Command};

pub fn generate(
    examples_path: &Path,
    name: &str,
    num_structs: usize,
    functions_per_struct: usize,
) -> std::io::Result<()> {
    println!(
        "Generating {name} binary ({num_structs} structs, {functions_per_struct} functions each)..."
    );

    let mut code = String::new();

    // Generate headers
    code.push_str("#![allow(dead_code)]\n");
    code.push_str("use std::collections::HashMap;\n");
    code.push_str("use std::collections::BTreeMap;\n");
    code.push_str("use std::iter::FromIterator;\n");
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
    pub btree_map: BTreeMap<String, i32>,
}}

impl TestStruct{i} {{
"#
        ));

        // Generate methods
        for j in 0..functions_per_struct {
            code.push_str(&format!(
                r#"
    pub fn method_{j}(&self, param: i32) -> i32 {{
        let mut result = param;
        for i in 0..10 {{
            result = result.wrapping_add(i);
            if i % 2 == 0 {{
                result = result.wrapping_mul(2);
            }}
        }}
        result + self.id as i32
    }}
"#
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
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
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
    let src_path = examples_path.join(format!("{name}.rs"));
    let mut file = fs::File::create(&src_path)?;
    file.write_all(code.as_bytes())?;

    // Format the source file using rustfmt
    let status = Command::new("rustfmt")
        .arg(&src_path)
        .status()
        .expect("Failed to run rustfmt");

    if !status.success() {
        eprintln!("Warning: rustfmt failed to format the generated source file.");
    }

    println!("Generated source file at: {}", src_path.display());
    Ok(())
}
