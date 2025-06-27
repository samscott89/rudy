# Benchmarks

This directory contains performance benchmarks comparing Rudy with LLDB and measuring specific operations.

## Setup

1. Install dependencies:
   ```bash
   # For LLDB Python benchmarks
   pip3 install psutil
   ```

2. Generate test binaries:
   ```bash
   cargo run --bin generate_test_binaries
   ```
   This creates test binaries of varying sizes in `bin/test_binaries/`.

## Running Benchmarks

### Operation Benchmarks
Measure Rudy performance for specific operations:
```bash
cargo run --release --bin bench_operations [binary_path]
```

### Comparison Benchmarks
Compare Rudy with LLDB:
```bash
cargo run --release --bin comparison [binary_path]
```

### Full Benchmark Suite
Run all benchmarks on test binaries:
```bash
./bin/run_all.sh
```

## Benchmark Types

### 1. Cold Start
- Time to create database and load binary
- Measures initialization overhead

### 2. Address Resolution  
- Time to resolve memory addresses to source locations
- Shows benefit of caching (cold vs warm)

### 3. Function Lookup
- Time to resolve function names to debug info
- Tests string matching and index performance

### 4. Variable Resolution
- Time to get all variables at an address
- Most complex operation, shows full stack performance

### 5. Incremental Benefits
- Performance across multiple "debugging sessions"
- Demonstrates Salsa's caching advantages

## Interpreting Results

### vs LLDB Comparison
- **Cold start**: We may be slower (LLDB is highly optimized)
- **Warm queries**: We should be much faster due to caching
- **Memory usage**: We should use less memory due to lazy loading

### Key Metrics
- **ms/query**: Lower is better
- **Speedup**: >1.0x means we're faster
- **Memory MB**: Lower is better

## Adding New Benchmarks

1. Create a new file in `bin/`
2. Add specific operations to measure
3. Use consistent timing methodology:
   ```rust
   let start = Instant::now();
   // operation to measure
   let elapsed = start.elapsed();
   ```

## Known Limitations

- LLDB CLI measurements include process spawn overhead
- Memory measurements are approximate
- Test addresses may not resolve to real locations