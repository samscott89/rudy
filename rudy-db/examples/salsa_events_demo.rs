//! Demonstrates how salsa's incremental computation works in Rudy
//!
//! Run with: cargo run --example salsa_events_demo
//! For detailed salsa logs: RUST_LOG=salsa=info cargo run --example salsa_events_demo
//! For even more detail: RUST_LOG=salsa=debug cargo run --example salsa_events_demo

use rudy_db::{DebugDb, DebugInfo};
use salsa::AsDynDatabase;
use std::time::Instant;
use test_utils::artifacts_dir;

fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    println!("Salsa Incremental Computation Demo");
    println!("==================================\n");

    let binary_path = artifacts_dir(None).join("large");
    let db = DebugDb::new();

    // === Phase 1: Initial database creation ===
    println!("Phase 1: Creating database");
    println!("--------------------------");
    let start = Instant::now();
    let debug_info = DebugInfo::new(&db, &binary_path)?;
    println!(
        "Database created in {:.2}ms\n",
        start.elapsed().as_secs_f64() * 1000.0
    );

    // === Phase 2: First query (cold cache) ===
    println!("Phase 2: First function lookup (cold cache)");
    println!("--------------------------------------------");
    let start = Instant::now();
    let result = debug_info.find_function_by_name("main")?;
    let cold_time = start.elapsed();
    println!("Found: {:?}", result.map(|f| format!("{:#x}", f.address)));
    println!(
        "Cold lookup took: {:.2}ms\n",
        cold_time.as_secs_f64() * 1000.0
    );

    // === Phase 3: Repeated query (warm cache) ===
    println!("Phase 3: Same function lookup (warm cache)");
    println!("-------------------------------------------");
    let start = Instant::now();
    let result = debug_info.find_function_by_name("main")?;
    let warm_time = start.elapsed();
    println!("Found: {:?}", result.map(|f| format!("{:#x}", f.address)));
    println!(
        "Warm lookup took: {:.2}ms",
        warm_time.as_secs_f64() * 1000.0
    );

    let speedup = cold_time.as_secs_f64() / warm_time.as_secs_f64();
    println!("Speedup: {:.0}x faster\n", speedup);

    // === Phase 4: Different query ===
    println!("Phase 4: Different function lookup");
    println!("----------------------------------");
    let start = Instant::now();
    let result = debug_info.find_function_by_name("TestStruct0::method_0")?;
    let time = start.elapsed();
    println!("Found: {:?}", result.map(|f| format!("{:#x}", f.address)));
    println!("Lookup took: {:.2}ms\n", time.as_secs_f64() * 1000.0);

    // === Phase 5: Address resolution ===
    println!("Phase 5: Address to location resolution");
    println!("---------------------------------------");
    let start = Instant::now();
    let result = debug_info.address_to_location(0x100001000)?;
    let time = start.elapsed();
    println!(
        "Resolved: {:?}",
        result.map(|l| format!("{}:{}", l.file, l.line))
    );
    println!("Resolution took: {:.2}ms\n", time.as_secs_f64() * 1000.0);

    // === Phase 6: Bulk repeated queries ===
    println!("Phase 6: Bulk repeated queries (demonstrating cache hits)");
    println!("---------------------------------------------------------");
    let queries = [
        ("main", "find_function_by_name"),
        ("TestStruct0::method_0", "find_function_by_name"),
    ];

    let start = Instant::now();
    for _ in 0..10 {
        for (name, _desc) in &queries {
            let _ = debug_info.find_function_by_name(name)?;
        }
        let _ = debug_info.address_to_location(0x100001000)?;
    }
    let batch_time = start.elapsed();
    println!(
        "Performed 30 cached queries in {:.2}ms",
        batch_time.as_secs_f64() * 1000.0
    );
    println!(
        "Average per query: {:.3}ms\n",
        batch_time.as_secs_f64() * 1000.0 / 30.0
    );

    // === Analysis ===
    println!("Salsa Database Statistics");
    println!("========================");
    let stats = db.as_dyn_database().queries_info();
    for (query_name, _info) in stats {
        println!("{}", query_name);
    }

    println!("\nHow Salsa Incremental Computation Works:");
    println!("• First queries trigger expensive DWARF parsing and indexing");
    println!("• Results are memoized based on input arguments");
    println!("• Repeated queries with same arguments return cached results");
    println!("• When binaries change, only affected queries are invalidated");
    println!("• This enables fast incremental recompilation in debuggers");

    println!("\nTo see salsa's internal behavior:");
    println!("  RUST_LOG=salsa=info cargo run --example salsa_events_demo");

    Ok(())
}
