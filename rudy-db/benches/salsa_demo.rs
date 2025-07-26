//! Benchmarks demonstrating salsa incremental computation benefits
//!
//! Run with: cargo bench --bench salsa_demo

use divan::Bencher;
use itertools::Itertools as _;
use rudy_db::{DebugDb, DebugInfo};
use test_utils::artifacts_dir;

fn main() {
    divan::main();
}

/// Benchmark cold database initialization and first query
#[divan::bench]
fn find_function_cold(bencher: Bencher) {
    let binary_path = artifacts_dir(None).join("large");

    bencher.bench(|| {
        // Cold: Create new database each time
        let db = DebugDb::new();
        let debug_info = DebugInfo::new(&db, &binary_path).unwrap();

        // Perform a simple query to trigger indexing
        let result = debug_info.find_function_by_name("main").unwrap();
        divan::black_box(result);
    });
}

/// Demonstration: Cold vs Warm query performance
/// This shows the dramatic difference between first query and cached queries
#[divan::bench]
fn find_function_warm(bencher: Bencher) {
    let binary_path = artifacts_dir(None).join("large");

    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, &binary_path).unwrap();

    // Measure cold query
    bencher.bench_local(move || {
        // Measure warm query (should be much faster)
        let result = debug_info.find_function_by_name("main").unwrap();
        divan::black_box(result);
    });
}

/// Benchmark cold address resolution
#[divan::bench(args = [10, 50, 100])]
fn address_resolution_cold(bencher: Bencher, num_addresses: usize) {
    let binary_path = artifacts_dir(None).join("large");

    // Generate some test addresses to look up
    let addresses: Vec<u64> = (0..num_addresses)
        .map(|i| 0x100001000 + (i as u64 * 0x1000))
        .collect();

    bencher.bench(|| {
        // Cold cache - measure first-time lookups
        let db = DebugDb::new();
        let debug_info = DebugInfo::new(&db, &binary_path).unwrap();

        for &addr in &addresses {
            let result = debug_info.address_to_location(addr).unwrap();
            divan::black_box(result);
        }
    });
}

/// Measure address resolution speedup from caching
#[divan::bench(args = [10, 50, 100])]
fn address_resolution_warm(bencher: Bencher, num_addresses: usize) {
    let binary_path = artifacts_dir(None).join("large");

    // Generate some test addresses to look up
    let addresses: Vec<u64> = (0..num_addresses)
        .map(|i| 0x100001000 + (i as u64 * 0x1000))
        .collect();
    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, &binary_path).unwrap();

    bencher.bench_local(move || {
        for &addr in &addresses {
            let result = debug_info.address_to_location(addr).unwrap();
            divan::black_box(result);
        }
    });
}

fn bench_args() -> impl Iterator<Item = (&'static str, &'static str)> {
    ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
        .into_iter()
        .cartesian_product(["examples/small", "examples/medium", "examples/large"])
}

/// Benchmark showing different sized binaries
#[divan::bench(args = bench_args())]
fn indexing_by_binary_size(bencher: Bencher, (arch, name): (&'static str, &'static str)) {
    let binary_path = artifacts_dir(Some(arch)).join(name);

    bencher.bench(|| {
        let db = DebugDb::new();
        let debug_info = DebugInfo::new(&db, &binary_path).unwrap();

        // Trigger initial indexing with a query
        let result = debug_info.find_function_by_name("main").unwrap();
        divan::black_box(result);
    });
}
