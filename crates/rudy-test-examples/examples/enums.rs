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

fn main() {
    let unit = TestEnum::Unit;
    let tuple = TestEnum::Tuple(42, "Hello".to_string());
    let struct_variant = TestEnum::Struct {
        x: 3.14321,
        y: 2.71,
    };

    let repr_c_unit = ReprCEnum::Unit;
    let repr_c_tuple = ReprCEnum::Tuple(42, "World".to_string());
    let repr_c_struct = ReprCEnum::Struct { x: 1.23, y: 4.56 };

    let u8_first = U8Enum::First;
    let u8_second = U8Enum::Second;
    let u8_third = U8Enum::Third;
    let u8_fifth = U8Enum::Fifth;
    println!("TestEnum variants:");
    println!("{unit:?}");
    println!("{tuple:?}");
    println!("{struct_variant:?}");
    println!("ReprCEnum variants:");
    println!("{repr_c_unit:?}");
    println!("{repr_c_tuple:?}");
    println!("{repr_c_struct:?}");
    println!("U8Enum variants:");
    println!("{u8_first:?}");
    println!("{u8_second:?}");
    println!("{u8_third:?}");
    println!("{u8_fifth:?}");

    // result and option
    let option_value: Option<i32> = Some(42);
    let result_value: Result<i32, String> = Ok(42);
    println!("Option value: {option_value:?}");
    println!("Result value: {result_value:?}");
}
