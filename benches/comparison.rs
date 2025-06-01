//! Benchmark comparisons between rust-debuginfo and LLDB
//!
//! Run with: cargo bench --bench comparison

use anyhow::Result;
use rust_debuginfo::{DebugDb, DebugInfo};
use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

/// Test addresses to resolve (these would be real addresses from your binary)
const TEST_ADDRESSES: &[u64] = &[
    0x100001000,
    0x100002000,
    0x100003000,
    0x100004000,
    0x100005000,
    0x100006000,
    0x100007000,
    0x100008000,
    0x100009000,
    0x10000a000,
];

#[derive(Debug)]
struct BenchmarkResult {
    name: String,
    cold_start: Duration,
    warm_queries: Duration,
    memory_mb: f64,
}

fn main() -> Result<()> {
    let binary_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./target/debug/rust_debuginfo".to_string());

    println!("Benchmarking with binary: {}", binary_path);
    println!("================================================\n");

    // Run benchmarks
    let rust_debuginfo_result = benchmark_rust_debuginfo(&binary_path)?;
    let lldb_python_result = benchmark_lldb_python(&binary_path)?;
    let lldb_cli_result = benchmark_lldb_cli(&binary_path)?;

    // Print results
    print_results(&[rust_debuginfo_result, lldb_python_result, lldb_cli_result]);

    Ok(())
}

fn benchmark_rust_debuginfo(binary_path: &str) -> Result<BenchmarkResult> {
    println!("Benchmarking rust-debuginfo...");

    // Cold start - time to first query
    let cold_start_begin = Instant::now();
    let db = DebugDb::new()?;
    let debug_info = DebugInfo::new(&db, binary_path)?;

    // First query
    let _first_result = debug_info.address_to_line(TEST_ADDRESSES[0]);
    let cold_start = cold_start_begin.elapsed();

    // Warm queries - benefit from caching
    let warm_start = Instant::now();
    for &addr in TEST_ADDRESSES {
        let _ = debug_info.address_to_line(addr);
    }
    let warm_queries = warm_start.elapsed();

    // Measure memory (approximate)
    let memory_mb = get_process_memory_mb();

    Ok(BenchmarkResult {
        name: "rust-debuginfo".to_string(),
        cold_start,
        warm_queries,
        memory_mb,
    })
}

fn benchmark_lldb_python(binary_path: &str) -> Result<BenchmarkResult> {
    println!("Benchmarking LLDB Python API...");

    // Create Python script
    let python_script = format!(
        r#"
import lldb
import time
import psutil
import os

binary_path = "{}"
addresses = {}

# Cold start
cold_start = time.time()
debugger = lldb.SBDebugger.Create()
target = debugger.CreateTarget(binary_path)

# First query
addr = target.ResolveLoadAddress(addresses[0])
sym = addr.GetSymbol()
cold_time = time.time() - cold_start

# Warm queries
warm_start = time.time()
for addr_val in addresses:
    addr = target.ResolveLoadAddress(addr_val)
    sym = addr.GetSymbol()
    # Get line info
    line_entry = addr.GetLineEntry()
warm_time = time.time() - warm_start

# Memory usage
process = psutil.Process(os.getpid())
memory_mb = process.memory_info().rss / 1024 / 1024

print(f"COLD_START:{{cold_time}}")
print(f"WARM_QUERIES:{{warm_time}}")
print(f"MEMORY_MB:{{memory_mb}}")

lldb.SBDebugger.Destroy(debugger)
"#,
        binary_path,
        format!("{:?}", TEST_ADDRESSES)
    );

    // get lldb path
    let lldb_path = Command::new("lldb").arg("-P").output()?.stdout;
    let lldb_path = String::from_utf8(lldb_path)?.trim().to_string();
    println!("Using LLDB Python API at: {lldb_path}",);

    // Run Python script
    let output = Command::new("xcrun")
        .env("PYTHONPATH", lldb_path)
        .arg("python3")
        .arg("-c")
        .arg(&python_script)
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to run Python script: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    if output.stdout.is_empty() {
        return Err(anyhow::anyhow!("Python script produced no output"));
    }

    // Parse results
    let stdout = String::from_utf8_lossy(&output.stdout);
    let cold_start = parse_duration(&stdout, "COLD_START:")?;
    let warm_queries = parse_duration(&stdout, "WARM_QUERIES:")?;
    let memory_mb = parse_float(&stdout, "MEMORY_MB:")?;

    Ok(BenchmarkResult {
        name: "LLDB Python API".to_string(),
        cold_start,
        warm_queries,
        memory_mb,
    })
}

fn benchmark_lldb_cli(binary_path: &str) -> Result<BenchmarkResult> {
    println!("Benchmarking LLDB CLI...");

    // Create batch commands
    let mut commands = vec![format!("target create {}", binary_path)];
    for addr in TEST_ADDRESSES {
        commands.push(format!("image lookup -a {:#x}", addr));
    }
    commands.push("quit".to_string());
    let batch_commands = commands.join("\n");

    // Cold start - includes LLDB startup
    let cold_start_begin = Instant::now();
    let _output = Command::new("lldb")
        .arg("-b")
        .arg("-o")
        .arg(&commands[0])
        .arg("-o")
        .arg(&commands[1])
        .arg("-o")
        .arg("quit")
        .output()?;
    let cold_start = cold_start_begin.elapsed();

    // Warm queries - run all queries in one session
    let warm_start = Instant::now();
    let _output = Command::new("lldb")
        .arg("-b")
        .arg("-s")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?
        .stdin
        .as_mut()
        .unwrap()
        .write_all(batch_commands.as_bytes())?;
    let warm_queries = warm_start.elapsed();

    // Memory is harder to measure for CLI
    let memory_mb = 0.0; // Would need to monitor externally

    Ok(BenchmarkResult {
        name: "LLDB CLI".to_string(),
        cold_start,
        warm_queries,
        memory_mb,
    })
}

fn parse_duration(output: &str, prefix: &str) -> Result<Duration> {
    let line = output
        .lines()
        .find(|l| l.starts_with(prefix))
        .ok_or_else(|| anyhow::anyhow!("Could not find {}", prefix))?;
    let value: f64 = line.trim_start_matches(prefix).parse()?;
    Ok(Duration::from_secs_f64(value))
}

fn parse_float(output: &str, prefix: &str) -> Result<f64> {
    let line = output
        .lines()
        .find(|l| l.starts_with(prefix))
        .ok_or_else(|| anyhow::anyhow!("Could not find {}", prefix))?;
    Ok(line.trim_start_matches(prefix).parse()?)
}

fn get_process_memory_mb() -> f64 {
    // Simple approximation - in real benchmark use proper memory measurement
    0.0
}

fn format_duration(dur: &Duration) -> String {
    if dur.as_millis() == 0 {
        format!("{:>8.2} ms", (dur.as_micros() as f64) / 1000f64)
    } else {
        format!("{:>8.2} ms", dur.as_millis())
    }
}

fn print_results(results: &[BenchmarkResult]) {
    println!("\n📊 Benchmark Results");
    println!("===================\n");

    // Find baseline (rust-debuginfo)
    let baseline = &results[0];

    for result in results {
        println!("📦 {}", result.name);
        println!("  Cold start:    {}", format_duration(&result.cold_start));
        println!(
            "  Warm queries:  {} ({:.1} ms/query)",
            format_duration(&result.warm_queries),
            result.warm_queries.as_millis() as f64 / TEST_ADDRESSES.len() as f64
        );
        if result.memory_mb > 0.0 {
            println!("  Memory usage:  {:>8.1} MB", result.memory_mb);
        }

        // Show speedup vs baseline
        if result.name != baseline.name {
            let cold_speedup = baseline.cold_start.as_secs_f64() / result.cold_start.as_secs_f64();
            let warm_speedup =
                baseline.warm_queries.as_secs_f64() / result.warm_queries.as_secs_f64();

            println!("\n  vs rust-debuginfo:");
            if cold_speedup > 1.0 {
                println!("    Cold: {:.1}x faster ✅", cold_speedup);
            } else {
                println!("    Cold: {:.1}x slower ❌", 1.0 / cold_speedup);
            }
            if warm_speedup > 1.0 {
                println!("    Warm: {:.1}x faster ✅", warm_speedup);
            } else {
                println!("    Warm: {:.1}x slower ❌", 1.0 / warm_speedup);
            }
        }
        println!();
    }
}
