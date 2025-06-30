//! Benchmark comparisons between Rudy and LLDB
//!
//! Run with: cargo bench --bench comparison

use anyhow::Result;
use rudy_db::{DebugDb, DebugInfo};
use std::process::Command;
use std::time::{Duration, Instant};

/// Function names to look up - these match what we generate in test binaries
const TEST_FUNCTIONS: &[&str] = &[
    "main",
    "TestStruct0::method_0",
    "TestStruct1::method_1",
    "TestStruct2::method_2",
    "TestStruct4::method_4",
];

#[derive(Debug)]
struct BenchmarkResult {
    name: String,
    init_only: Duration,  // Time to just load/attach, no queries
    cold_start: Duration, // Time to first query result
    warm_queries: Duration,
    memory_mb: f64,
}

fn main() -> Result<()> {
    let binary_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./test-artifacts/aarch64-apple-darwin/medium".to_string());

    println!("Benchmarking with binary: {binary_path}");
    println!("================================================\n");

    // First, find addresses for our test functions
    let test_addresses = find_test_addresses(&binary_path)?;
    if test_addresses.is_empty() {
        return Err(anyhow::anyhow!("No test functions found in binary"));
    }

    println!(
        "Found {} test functions to benchmark\n",
        test_addresses.len()
    );

    // Run benchmarks
    let rudy_db_result = benchmark_rudy_db(&binary_path, &test_addresses)?;
    let lldb_python_result = benchmark_lldb_python(&binary_path, &test_addresses)?;
    let lldb_cli_result = benchmark_lldb_cli(&binary_path, &test_addresses)?;

    // Print results
    print_results(
        &[rudy_db_result, lldb_python_result, lldb_cli_result],
        test_addresses.len(),
    );

    Ok(())
}

fn find_test_addresses(binary_path: &str) -> Result<Vec<(u64, String)>> {
    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, binary_path)?;

    let mut addresses = Vec::new();

    for func_name in TEST_FUNCTIONS {
        match debug_info.resolve_function(func_name)? {
            Some(func) => {
                println!("  Found {} at {:#x}", func_name, func.address);
                addresses.push((func.address, func_name.to_string()));
            }
            None => {
                println!("  Warning: {func_name} not found");
            }
        }
    }

    Ok(addresses)
}

fn benchmark_rudy_db(
    binary_path: &str,
    test_addresses: &[(u64, String)],
) -> Result<BenchmarkResult> {
    println!("Benchmarking Rudy...");

    // Init only - just load the binary, no queries
    let init_start = Instant::now();
    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, binary_path)?;
    let init_only = init_start.elapsed();

    // Cold start - time to first query
    let cold_start_begin = Instant::now();
    let first_result = debug_info.address_to_line(test_addresses[0].0);
    let cold_start = cold_start_begin.elapsed();

    // Validate first result
    if let Some(loc) = first_result {
        println!(
            "  First query validated: {} -> {}:{}",
            test_addresses[0].1, loc.file, loc.line
        );
    }

    // Warm queries - benefit from caching
    let mut found = 0;
    for (addr, _name) in test_addresses {
        let _ = debug_info.address_to_line(*addr);
    }
    let warm_start = Instant::now();
    for (addr, _name) in test_addresses {
        if debug_info.address_to_line(*addr).is_some() {
            found += 1;
        }
    }
    let warm_queries = warm_start.elapsed();
    println!("  Resolved {}/{} addresses", found, test_addresses.len());

    // Measure memory after all operations
    let memory_mb = get_process_memory_mb();

    Ok(BenchmarkResult {
        name: "rudy".to_string(),
        init_only,
        cold_start,
        warm_queries,
        memory_mb,
    })
}

fn benchmark_lldb_python(
    binary_path: &str,
    test_addresses: &[(u64, String)],
) -> Result<BenchmarkResult> {
    println!("Benchmarking LLDB Python API...");

    // Extract just the addresses
    let addrs: Vec<u64> = test_addresses.iter().map(|(a, _)| *a).collect();

    // Create Python script
    let python_script = format!(
        r#"
import lldb
import time
import psutil
import os

binary_path = "{binary_path}"
addresses = {addrs:?}
names = {:?}

# Init only - just create target
init_start = time.time()
debugger = lldb.SBDebugger.Create()
target = debugger.CreateTarget(binary_path)
init_time = time.time() - init_start

# Cold start - time to first query
cold_start = time.time()
addr = target.ResolveLoadAddress(addresses[0])
sym = addr.GetSymbol()
line_entry = addr.GetLineEntry()
if line_entry.IsValid():
    print(f"VALIDATION: {{names[0]}} -> {{line_entry.GetFileSpec().GetFilename()}}:{{line_entry.GetLine()}}")
cold_time = time.time() - cold_start

# Warm queries
warm_start = time.time()
found = 0
for i, addr_val in enumerate(addresses):
    addr = target.ResolveLoadAddress(addr_val)
    sym = addr.GetSymbol()
    line_entry = addr.GetLineEntry()
    if line_entry.IsValid():
        found += 1
warm_time = time.time() - warm_start
print(f"FOUND:{{found}}/{{len(addresses)}}")

# Memory usage
process = psutil.Process(os.getpid())
memory_mb = process.memory_info().rss / 1024 / 1024

print(f"INIT_ONLY:{{init_time}}")
print(f"COLD_START:{{cold_time}}")
print(f"WARM_QUERIES:{{warm_time}}")
print(f"MEMORY_MB:{{memory_mb}}")

lldb.SBDebugger.Destroy(debugger)
"#,
        test_addresses.iter().map(|(_, n)| n).collect::<Vec<_>>()
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

    // Print validation info if present
    if let Some(validation) = stdout.lines().find(|l| l.starts_with("VALIDATION:")) {
        println!(
            "  First query validated: {}",
            validation.trim_start_matches("VALIDATION:")
        );
    }
    if let Some(found) = stdout.lines().find(|l| l.starts_with("FOUND:")) {
        println!("  Resolved {}", found.trim_start_matches("FOUND:"));
    }

    let init_only = parse_duration(&stdout, "INIT_ONLY:")?;
    let cold_start = parse_duration(&stdout, "COLD_START:")?;
    let warm_queries = parse_duration(&stdout, "WARM_QUERIES:")?;
    let memory_mb = parse_float(&stdout, "MEMORY_MB:")?;

    Ok(BenchmarkResult {
        name: "LLDB Python API".to_string(),
        init_only,
        cold_start,
        warm_queries,
        memory_mb,
    })
}

fn benchmark_lldb_cli(
    binary_path: &str,
    test_addresses: &[(u64, String)],
) -> Result<BenchmarkResult> {
    println!("Benchmarking LLDB CLI...");

    // Create batch commands
    let mut commands = vec![format!("target create {}", binary_path)];
    for (addr, _name) in test_addresses {
        commands.push(format!("image lookup -a {addr:#x}"));
    }
    commands.push("quit".to_string());

    // Init only - just create target
    let init_start = Instant::now();
    let _output = Command::new("lldb")
        .arg("-b")
        .arg("-o")
        .arg("quit")
        .output()?;
    let init_only = init_start.elapsed();

    // Cold start - includes first query
    let cold_start_begin = Instant::now();
    let output = Command::new("lldb")
        .arg("-b")
        .arg("-o")
        .arg(&commands[0])
        .arg("-o")
        .arg(&commands[1])
        .arg("-o")
        .arg("quit")
        .output()?;
    let cold_start = cold_start_begin.elapsed().saturating_sub(init_only); // Subtract init time

    // Quick validation check
    let output_str = String::from_utf8_lossy(&output.stdout);
    if output_str.contains("Summary:") {
        println!("  First query validated (found symbol info)");
    }

    // Warm queries - run all queries in one session
    let warm_start = Instant::now();
    let _output = Command::new("lldb")
        .arg("-b")
        .args(commands.iter().flat_map(|cmd| vec!["-o", cmd.as_str()]))
        .arg("-o")
        .arg("quit")
        .output()?;
    let warm_queries = warm_start.elapsed().saturating_sub(init_only);

    // Memory is harder to measure for CLI
    let memory_mb = 0.0; // Would need to monitor externally

    Ok(BenchmarkResult {
        name: "LLDB CLI".to_string(),
        init_only,
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
    // Get current process memory usage
    #[cfg(target_os = "macos")]
    {
        use std::mem;
        use std::os::raw::c_int;

        #[repr(C)]
        struct RUsage {
            ru_utime: [i64; 2],
            ru_stime: [i64; 2],
            ru_maxrss: i64, // Maximum resident set size in bytes on macOS
            _rest: [i64; 14],
        }

        unsafe extern "C" {
            fn getrusage(who: c_int, usage: *mut RUsage) -> c_int;
        }

        const RUSAGE_SELF: c_int = 0;
        let mut usage: RUsage = unsafe { mem::zeroed() };

        if unsafe { getrusage(RUSAGE_SELF, &mut usage) } == 0 {
            // On macOS, ru_maxrss is in bytes
            return usage.ru_maxrss as f64 / 1024.0 / 1024.0;
        }
    }

    #[cfg(target_os = "linux")]
    {
        // On Linux, read from /proc/self/status
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("VmRSS:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<f64>() {
                            return kb / 1024.0; // Convert KB to MB
                        }
                    }
                }
            }
        }
    }

    // Fallback
    0.0
}

fn format_duration(dur: &Duration) -> String {
    if dur.as_millis() == 0 {
        format!("{:>8.3} ms", (dur.as_micros() as f64) / 1000f64)
    } else {
        format!("{:>8.3} ms", dur.as_millis())
    }
}

fn print_results(results: &[BenchmarkResult], num_test_addresses: usize) {
    println!("\n📊 Benchmark Results");
    println!("===================\n");

    // Find baseline (rudy)
    let baseline = &results[0];

    for result in results {
        println!("📦 {}", result.name);
        println!("  Init only:     {}", format_duration(&result.init_only));
        println!("  First query:   {}", format_duration(&result.cold_start));
        println!(
            "  Warm queries:  {} ({:.1} ms/query)",
            format_duration(&result.warm_queries),
            result.warm_queries.as_millis() as f64 / num_test_addresses as f64
        );
        if result.memory_mb > 0.0 {
            println!("  Memory usage:  {:>8.1} MB", result.memory_mb);
        }

        // Show speedup vs baseline
        if result.name != baseline.name {
            let init_speedup = baseline.init_only.as_secs_f64() / result.init_only.as_secs_f64();
            let cold_speedup = baseline.cold_start.as_secs_f64() / result.cold_start.as_secs_f64();
            let warm_speedup =
                baseline.warm_queries.as_secs_f64() / result.warm_queries.as_secs_f64();

            println!("\n  vs rudy:");
            if init_speedup > 1.0 {
                println!("    Init: {init_speedup:.1}x faster ✅");
            } else {
                println!("    Init: {:.1}x slower ❌", 1.0 / init_speedup);
            }
            if cold_speedup > 1.0 {
                println!("    Cold: {cold_speedup:.1}x faster ✅");
            } else {
                println!("    Cold: {:.1}x slower ❌", 1.0 / cold_speedup);
            }
            if warm_speedup > 1.0 {
                println!("    Warm: {warm_speedup:.1}x faster ✅");
            } else {
                println!("    Warm: {:.1}x slower ❌", 1.0 / warm_speedup);
            }
        }
        println!();
    }
}
