use std::collections::HashMap;

fn test_fn(s: String, v: Vec<i32>, map: HashMap<String, i32>) {
    println!("String: {s}, Vec: {v:?}, Map: {map:?}");
}

fn main() {
    let s = String::from("hello");
    let v: Vec<i32> = vec![1, 2, 3];
    let mut map: HashMap<String, i32> = HashMap::new();
    map.insert("key".to_string(), 42);

    test_fn(s, v, map);
}
