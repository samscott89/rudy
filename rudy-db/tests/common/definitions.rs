use std::collections::BTreeMap;

// Test structs that will be included in our test binary
#[derive(Debug)]
pub struct TestPerson {
    pub name: String,
    pub age: u32,
    pub email: Option<String>,
}

#[derive(Debug)]
pub struct TestPoint {
    pub x: f64,
    pub y: f64,
}

// Struct with only basic types that should work
#[derive(Debug)]
pub struct TestBasicStruct {
    pub id: u32,
    pub count: u64,
    pub enabled: bool,
    pub bytes: [u8; 4],
}

impl TestBasicStruct {
    #[inline(never)]
    pub fn num_bytes(&self) -> usize {
        self.bytes.len() + 2
    }
}

#[derive(Debug)]
pub struct TestComplexData {
    pub id: u64,
    pub values: Vec<i32>,
    pub metadata: BTreeMap<String, String>,
    pub location: TestPoint,
}

#[derive(Debug)]
pub enum TestEnum {
    Unit,
    Tuple(u32, String),
    Struct { x: f64, y: f64 },
}

#[derive(Debug)]
#[repr(C)]
pub enum ReprCEnum {
    Unit,
    Tuple(u32, String),
    Struct { x: f64, y: f64 },
}

#[derive(Debug)]
#[repr(u8)]
pub enum U8Enum {
    First,
    Second,
    Third,
    // skip fourth to see what happens
    Fifth = 5,
}
