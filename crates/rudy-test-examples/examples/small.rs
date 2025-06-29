#![allow(dead_code)]
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::iter::FromIterator;
use std::vec::Vec;

#[derive(Debug, Clone)]
pub struct TestStruct0 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct0 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug)]
pub enum TestEnum0 {
    Variant1(i32),
    Variant2 { x: f64, y: f64 },
    Variant3(TestStruct0),
}

pub type TestAlias0 = HashMap<String, TestStruct0>;

#[derive(Debug, Clone)]
pub struct TestStruct1 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct1 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug, Clone)]
pub struct TestStruct2 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct2 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug, Clone)]
pub struct TestStruct3 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct3 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug, Clone)]
pub struct TestStruct4 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct4 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug, Clone)]
pub struct TestStruct5 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct5 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug)]
pub enum TestEnum5 {
    Variant1(i32),
    Variant2 { x: f64, y: f64 },
    Variant3(TestStruct1),
}

pub type TestAlias5 = HashMap<String, TestStruct5>;

#[derive(Debug, Clone)]
pub struct TestStruct6 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct6 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug, Clone)]
pub struct TestStruct7 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct7 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug, Clone)]
pub struct TestStruct8 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct8 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

#[derive(Debug, Clone)]
pub struct TestStruct9 {
    pub id: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub flags: [bool; 8],
    pub map: HashMap<String, i32>,
    pub btree_map: BTreeMap<String, i32>,
}

impl TestStruct9 {
    pub fn method_0(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_1(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_2(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_3(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }

    pub fn method_4(&self, param: i32) -> i32 {
        let mut result = param;
        for i in 0..10 {
            result = result.wrapping_add(i);
            if i % 2 == 0 {
                result = result.wrapping_mul(2);
            }
        }
        result + self.id as i32
    }
}

fn main() {
    println!("Test binary for benchmarking");

    // Use some of the types to ensure they're not optimized out
    let mut total = 0u64;

    {
        let s = TestStruct0 {
            id: 0,
            name: "test0".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_0(0);
    }

    {
        let s = TestStruct1 {
            id: 1,
            name: "test1".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_1(1);
    }

    {
        let s = TestStruct2 {
            id: 2,
            name: "test2".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_2(2);
    }

    {
        let s = TestStruct3 {
            id: 3,
            name: "test3".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_3(3);
    }

    {
        let s = TestStruct4 {
            id: 4,
            name: "test4".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_4(4);
    }

    {
        let s = TestStruct5 {
            id: 5,
            name: "test5".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_0(5);
    }

    {
        let s = TestStruct6 {
            id: 6,
            name: "test6".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_1(6);
    }

    {
        let s = TestStruct7 {
            id: 7,
            name: "test7".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_2(7);
    }

    {
        let s = TestStruct8 {
            id: 8,
            name: "test8".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_3(8);
    }

    {
        let s = TestStruct9 {
            id: 9,
            name: "test9".to_string(),
            data: vec![1, 2, 3],
            flags: [false; 8],
            map: HashMap::from_iter([(String::from("key"), 0x42)]),
            btree_map: BTreeMap::from_iter([(String::from("key"), 42)]),
        };
        total += s.id;

        // Call a method to ensure it's in the binary
        let _ = s.method_4(9);
    }

    // Some computation to prevent optimization
    println!("Total: {}", total);

    // Create some enum values
    let _e1 = TestEnum0::Variant1(42);

    // Use type aliases
    let _map: TestAlias0 = HashMap::new();
}
