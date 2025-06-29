# Rudy Test Suite

This directory contains the integration tests for rudy-db. The tests are organized to support cross-platform testing and CI/CD.

## Test Organization

Tests are divided into two main categories:

### 1. Static Tests (`static_tests.rs`)
These tests analyze pre-built binaries and can run on any platform. They test:
- Function resolution
- Position/line number resolution
- Type resolution
- Method discovery
- Debug info loading

Static tests use pre-built artifacts stored in `test-artifacts/` for different target platforms.

### 2. Dynamic Tests (`dynamic_tests.rs`)
These tests introspect the current running process and must run on each target platform. They test:
- Live type introspection
- Enum variant resolution
- Platform-specific behavior

### 3. Live Introspection Tests (`live_introspection.rs`)
Comprehensive tests that read debug info from the running test process itself, including:
- String, struct, vector introspection
- Smart pointer handling (Box, Arc, Rc, Mutex, RefCell)
- HashMap and BTreeMap introspection
- Synthetic method execution

## Test Artifacts

Test artifacts are pre-built binaries checked into version control to ensure consistent testing across platforms.

### Building Test Artifacts

Use the xtask pattern to build artifacts:

```bash
# Build artifacts for all platforms (requires cross-compilation setup)
cargo xtask build-test-artifacts

# Build artifacts for current platform only
cargo xtask build-test-artifacts --current-platform

# Generate test binaries (small, medium, large)
cargo xtask generate-test-binaries
```

### Artifact Structure

```
test-artifacts/
├── aarch64-apple-darwin/
│   ├── simple_test
│   └── lldb_demo
├── x86_64-apple-darwin/
│   ├── simple_test
│   └── lldb_demo
├── aarch64-unknown-linux-gnu/
│   ├── simple_test
│   └── lldb_demo
├── x86_64-unknown-linux-gnu/
│   ├── simple_test
│   └── lldb_demo
└── generated/
    ├── small
    ├── medium
    └── large
```

## Running Tests

### Run all tests
```bash
cargo test -p rudy-db
```

### Run specific test categories
```bash
# Static tests only
cargo test -p rudy-db --test static_tests

# Dynamic tests only
cargo test -p rudy-db --test dynamic_tests

# Live introspection tests
cargo test -p rudy-db --test live_introspection
```

### Run tests with xtask
```bash
# Run tests for current platform
cargo xtask test

# Run tests for specific target
cargo xtask test --target x86_64-unknown-linux-gnu
```

## Adding New Tests

### Adding a Static Test
1. Create your test function in `static_tests.rs`
2. Use `binary_path(target, example)` to get the path to test artifacts
3. Use the `#[apply(binary_target)]` attribute to run the test for all platforms

Example:
```rust
#[apply(binary_target)]
fn test_my_feature(#[case] target: &str) {
    setup!(target);
    let path = binary_path(target, "simple_test");
    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, &path).unwrap();
    
    // Your test logic here
}
```

### Adding a Dynamic Test
1. Create your test function in `dynamic_tests.rs`
2. Use `std::env::current_exe()` to get the current process path
3. Test will automatically run on each platform in CI

Example:
```rust
#[test]
fn test_my_live_feature() {
    setup!();
    let db = DebugDb::new();
    let exe_path = std::env::current_exe().unwrap();
    let debug_info = DebugInfo::new(&db, exe_path.to_str().unwrap()).unwrap();
    
    // Your test logic here
}
```

### Adding Test Examples
1. Create a new example in `rudy-db/examples/`
2. Update `xtask/src/main.rs` to include the new example in the build list
3. Run `cargo xtask build-test-artifacts` to generate artifacts
4. Commit the generated artifacts to version control

## CI/CD

The GitHub Actions workflow (`.github/workflows/test.yml`) runs:
1. **Static tests**: Runs on Ubuntu using checked-in test artifacts from all platforms
2. **Dynamic tests**: Runs on each platform (Ubuntu x86_64, macOS x86_64/aarch64)
3. **Unit tests**: Runs on Ubuntu and macOS

### Test Artifacts in CI

Test artifacts are **not** built in CI. Instead, they should be built locally and committed to the repository:

```bash
# Build artifacts locally (requires Docker for Linux cross-compilation on macOS)
cargo xtask build-test-artifacts

# Commit the artifacts
git add test-artifacts/
git commit -m "Update test artifacts"
```

This approach ensures:
- **Faster CI builds** (no cross-compilation overhead)
- **Deterministic tests** (same artifacts across all CI runs)
- **Easier debugging** (you can test with the exact same artifacts locally)

## Troubleshooting

### Missing Test Artifacts
If you see errors about missing test binaries:
```
Test binary not found at: /path/to/artifact
Please run `cargo xtask build-test-artifacts` to generate test binaries.
```

Run the suggested command to build the artifacts.

### Platform-Specific Issues
- **macOS**: Requires code signing for debugging. The xtask automatically handles this.
- **Linux cross-compilation**: Requires Docker or cross-rs setup for building non-native targets.
- **Windows**: Not currently supported (PRs welcome!)

### Environment Variables
- `RUDY_TEST_ARTIFACTS_DIR`: Override the default test artifacts directory
- `CARGO_WORKSPACE_DIR`: Automatically set by Cargo, used to locate artifacts