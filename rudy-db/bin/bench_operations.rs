//! Detailed operation benchmarks for Rudy
//!
//! This benchmark measures specific operations to show our strengths

use std::time::{Duration, Instant};

use anyhow::Result;
use rudy_db::{DebugDb, DebugInfo};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();
    let binary = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./target/debug/rudy_db".to_string());

    println!("🔬 Rudy Operation Benchmarks");
    println!("=====================================\n");

    // Create database once
    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, &binary)?;

    // Benchmark different operations
    benchmark_cold_start(&binary)?;
    benchmark_address_resolution(&debug_info)?;
    benchmark_function_lookup(&debug_info)?;
    // benchmark_variable_resolution(&debug_info)?;
    benchmark_incremental_benefit(&binary)?;

    Ok(())
}

fn benchmark_cold_start(binary: &str) -> Result<()> {
    println!("📊 Cold Start Benchmark");
    println!("----------------------");

    let mut times = Vec::new();

    for i in 0..5 {
        let start = Instant::now();
        let db = DebugDb::new();
        let _debug_info = DebugInfo::new(&db, binary)?;
        let elapsed = start.elapsed();
        times.push(elapsed);
        println!("  Run {}: {:>6.2} ms", i + 1, elapsed.as_millis());
    }

    let avg = times.iter().sum::<Duration>() / times.len() as u32;
    println!("  Average: {:>6.2} ms\n", avg.as_millis());

    Ok(())
}

fn benchmark_address_resolution(debug_info: &DebugInfo) -> Result<()> {
    println!("📊 Address → Location Resolution");
    println!("-------------------------------");

    // Test addresses (would be real in production)
    let addresses = vec![
        0x100001000,
        0x100002000,
        0x100003000,
        0x100004000,
        0x100005000,
    ];

    // First pass - cold cache
    let cold_start = Instant::now();
    for &addr in &addresses {
        let _ = debug_info.address_to_location(addr);
    }
    let cold_time = cold_start.elapsed();

    // Second pass - warm cache
    let warm_start = Instant::now();
    for &addr in &addresses {
        let _ = debug_info.address_to_location(addr);
    }
    let warm_time = warm_start.elapsed();

    println!(
        "  Cold cache: {:>6.2} ms ({:.2} ms/addr)",
        cold_time.as_millis(),
        cold_time.as_millis() as f64 / addresses.len() as f64
    );
    println!(
        "  Warm cache: {:>6.2} ms ({:.2} ms/addr)",
        warm_time.as_millis(),
        warm_time.as_millis() as f64 / addresses.len() as f64
    );
    println!(
        "  Speedup: {:.1}x\n",
        cold_time.as_secs_f64() / warm_time.as_secs_f64()
    );

    Ok(())
}

fn benchmark_function_lookup(debug_info: &DebugInfo) -> Result<()> {
    println!("📊 Function Name → Info Lookup");
    println!("-----------------------------");

    let functions = vec!["main", "method_0", "method_1", "method_10", "method_100"];

    let start = Instant::now();
    let mut found = 0;
    for func in &functions {
        if debug_info.find_function_by_name(func)?.is_some() {
            found += 1;
        }
    }
    let elapsed = start.elapsed();

    println!(
        "  Resolved {}/{} functions in {:.2} ms",
        found,
        functions.len(),
        elapsed.as_millis()
    );
    println!(
        "  Average: {:.2} ms/lookup\n",
        elapsed.as_millis() as f64 / functions.len() as f64
    );

    Ok(())
}

fn benchmark_incremental_benefit(binary: &str) -> Result<()> {
    println!("📊 Incremental Computation Benefits");
    println!("----------------------------------");

    // Simulate multiple debugging sessions
    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, binary)?;

    // Common addresses accessed across "sessions"
    let common_addresses = vec![0x100001000, 0x100002000];
    let mut session_times = Vec::new();

    for session in 0..3 {
        let start = Instant::now();

        // Each session queries some common addresses
        for &addr in &common_addresses {
            let _ = debug_info.address_to_location(addr).unwrap();
        }

        // And some unique ones
        for i in 0..5 {
            let addr = 0x100010000 + (session * 0x1000) + (i * 0x100);
            let _ = debug_info.address_to_location(addr).unwrap();
        }

        let elapsed = start.elapsed();
        session_times.push(elapsed);
        println!("  Session {}: {:.2} ms", session + 1, elapsed.as_millis());
    }

    // Show how later sessions are faster due to caching
    if session_times.len() > 1 {
        let speedup = session_times[0].as_secs_f64() / session_times.last().unwrap().as_secs_f64();
        println!("  Speedup from caching: {speedup:.1}x\n");
    }

    Ok(())
}
