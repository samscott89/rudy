//! Demo showing salsa events during typical debugging operations
//!
//! Run with: cargo run --bin salsa_events_demo

use rudy_db::{DebugDb, DebugInfo};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone)]
struct EventInfo {
    timestamp: Instant,
    event_type: String,
    query_name: String,
    duration_ms: Option<f64>,
    was_cached: bool,
}

fn main() -> anyhow::Result<()> {
    println!("🔬 Salsa Events Demo: Debugging Operations with Incremental Computation");
    println!("======================================================================\n");

    let start_time = Instant::now();
    let events = Arc::new(Mutex::new(Vec::<EventInfo>::new()));
    let events_clone = events.clone();

    // Create database with event logging
    let db = DebugDb::new_with_events(Some(Box::new(move |event| {
        let timestamp = Instant::now();
        let mut events = events_clone.lock().unwrap();

        match &event.kind {
            salsa::EventKind::WillExecute { database_key } => {
                events.push(EventInfo {
                    timestamp,
                    event_type: "WillExecute".to_string(),
                    query_name: format!("{database_key:?}"),
                    duration_ms: None,
                    was_cached: false,
                });
            }
            salsa::EventKind::DidValidateMemoizedValue { database_key, .. } => {
                events.push(EventInfo {
                    timestamp,
                    event_type: "CacheHit".to_string(),
                    query_name: format!("{database_key:?}"),
                    duration_ms: None,
                    was_cached: true,
                });
            }
            salsa::EventKind::WillCheckCancellation => {
                // Skip these - too noisy for demo
            }
            _ => {
                // Skip other event types for simplicity
            }
        }
    })));

    // Use a test binary
    let binary_path =
        if std::path::Path::new("./test-artifacts/aarch64-apple-darwin/large").exists() {
            "./test-artifacts/aarch64-apple-darwin/large"
        } else {
            println!("⚠️  Test artifacts not found. Run 'cargo xtask download-artifacts' first.");
            return Ok(());
        };

    println!("📂 Loading binary: {binary_path}");
    let load_start = Instant::now();
    let debug_info = DebugInfo::new(&db, binary_path)?;
    let load_time = load_start.elapsed();
    println!("   Loaded in {:.2}ms\n", load_time.as_secs_f64() * 1000.0);

    // === Demonstration 1: Function Discovery ===
    println!("🔍 Demo 1: Function Discovery (Cold Cache)");
    println!("------------------------------------------");
    let demo1_start = Instant::now();

    let functions = ["main", "TestStruct0::method_0", "TestStruct1::method_1"];
    for func_name in &functions {
        let start = Instant::now();
        match debug_info.find_function_by_name(func_name) {
            Ok(Some(func)) => {
                let elapsed = start.elapsed();
                println!(
                    "   ✅ Found {} at {:#x} ({:.2}ms)",
                    func_name,
                    func.address,
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            Ok(None) => {
                let elapsed = start.elapsed();
                println!(
                    "   ❌ {} not found ({:.2}ms)",
                    func_name,
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            Err(e) => {
                println!("   ⚠️  Error looking up {func_name}: {e}");
            }
        }
    }

    let demo1_time = demo1_start.elapsed();
    println!("   Total: {:.2}ms\n", demo1_time.as_secs_f64() * 1000.0);

    // === Demonstration 2: Address Resolution ===
    println!("🗺️  Demo 2: Address to Location Resolution");
    println!("------------------------------------------");
    let demo2_start = Instant::now();

    let addresses = [0x100001000, 0x100002000, 0x100003000];
    for &addr in &addresses {
        let start = Instant::now();
        match debug_info.address_to_location(addr) {
            Ok(Some(location)) => {
                let elapsed = start.elapsed();
                println!(
                    "   📍 {:#x} -> {}:{} ({:.2}ms)",
                    addr,
                    location.file,
                    location.line,
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            Ok(None) => {
                let elapsed = start.elapsed();
                println!(
                    "   ❓ {:#x} -> No location found ({:.2}ms)",
                    addr,
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            Err(e) => {
                println!("   ⚠️  Error resolving {addr:#x}: {e}");
            }
        }
    }

    let demo2_time = demo2_start.elapsed();
    println!("   Total: {:.2}ms\n", demo2_time.as_secs_f64() * 1000.0);

    // === Demonstration 3: Repeat Operations (Hot Cache) ===
    println!("🔥 Demo 3: Repeat Operations (Hot Cache)");
    println!("----------------------------------------");
    let demo3_start = Instant::now();

    println!("   Repeating function lookups...");
    for func_name in &functions {
        let start = Instant::now();
        let _ = debug_info.find_function_by_name(func_name);
        let elapsed = start.elapsed();
        println!(
            "   ⚡ {} ({:.3}ms)",
            func_name,
            elapsed.as_secs_f64() * 1000.0
        );
    }

    println!("   Repeating address lookups...");
    for &addr in &addresses {
        let start = Instant::now();
        let _ = debug_info.address_to_location(addr);
        let elapsed = start.elapsed();
        println!(
            "   ⚡ {:#x} ({:.3}ms)",
            addr,
            elapsed.as_secs_f64() * 1000.0
        );
    }

    let demo3_time = demo3_start.elapsed();
    println!("   Total: {:.2}ms\n", demo3_time.as_secs_f64() * 1000.0);

    // === Summary ===
    let total_time = start_time.elapsed();
    println!("📊 Summary");
    println!("----------");
    println!(
        "   Initial loading:     {:.2}ms",
        load_time.as_secs_f64() * 1000.0
    );
    println!(
        "   Cold operations:     {:.2}ms",
        demo1_time.as_secs_f64() * 1000.0 + demo2_time.as_secs_f64() * 1000.0
    );
    println!(
        "   Hot operations:      {:.2}ms",
        demo3_time.as_secs_f64() * 1000.0
    );
    println!(
        "   Total runtime:       {:.2}ms",
        total_time.as_secs_f64() * 1000.0
    );

    let speedup = (demo1_time.as_secs_f64() + demo2_time.as_secs_f64()) / demo3_time.as_secs_f64();
    println!("   🚀 Cache speedup:     {speedup:.1}x faster");

    // === Salsa Events Analysis ===
    println!("\n🔬 Salsa Events Analysis");
    println!("------------------------");
    let events = events.lock().unwrap();

    let total_executions = events
        .iter()
        .filter(|e| e.event_type == "WillExecute")
        .count();
    let cache_hits = events.iter().filter(|e| e.event_type == "CacheHit").count();
    let cache_hit_rate = if total_executions + cache_hits > 0 {
        cache_hits as f64 / (total_executions + cache_hits) as f64 * 100.0
    } else {
        0.0
    };

    println!("   Query executions:    {total_executions}");
    println!("   Cache hits:          {cache_hits}");
    println!("   Cache hit rate:      {cache_hit_rate:.1}%");

    println!("\n   Recent query events:");
    for (i, event) in events.iter().take(10).enumerate() {
        let relative_time = event.timestamp.duration_since(start_time);
        println!(
            "   {:2}. [{:6.1}ms] {} - {}",
            i + 1,
            relative_time.as_secs_f64() * 1000.0,
            event.event_type,
            event.query_name.chars().take(60).collect::<String>()
        );
    }

    if events.len() > 10 {
        println!("   ... and {} more events", events.len() - 10);
    }

    println!("\n✅ Demo complete! This shows how salsa provides:");
    println!("   • Incremental computation with automatic caching");
    println!("   • Dramatic speedups on repeated queries");
    println!("   • Transparent performance optimizations for debugging tools");

    Ok(())
}
