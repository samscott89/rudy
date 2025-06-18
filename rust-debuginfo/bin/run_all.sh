#!/bin/bash
# Run all benchmarks on test binaries

set -e

echo "🔨 Building rust-debuginfo in release mode..."
cargo build --release

echo -e "\n📦 Generating test binaries..."
cargo run --release --bin generate_test_binaries

echo -e "\n🏃 Running benchmarks...\n"

for binary in bin/test_binaries/{small,medium,large}; do
    if [ -f "$binary" ]; then
        echo "=========================================="
        echo "Benchmarking: $(basename $binary)"
        echo "=========================================="
        
        echo -e "\n--- Operation Benchmarks ---"
        cargo run --release --bin operations "$binary"
        
        echo -e "\n--- Comparison Benchmarks ---"
        cargo run --release --bin comparison "$binary"
        
        echo -e "\n"
    fi
done

echo "✅ All benchmarks complete!"