#!/bin/bash
set -e

# Script to generate platform-specific test artifacts
# These artifacts will be checked into version control for use in tests

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ARTIFACTS_DIR="$PROJECT_ROOT/test-artifacts"
EXAMPLES_DIR="$PROJECT_ROOT/examples"

# List of targets to build for
TARGETS=(
    "aarch64-unknown-linux-gnu"
    "x86_64-unknown-linux-gnu"
    "aarch64-apple-darwin"
    "x86_64-apple-darwin"
)

# List of example programs to build
EXAMPLES=(
    "simple_test"
    "lldb_demo"
)

echo "Generating test artifacts..."
echo "Project root: $PROJECT_ROOT"
echo "Artifacts will be saved to: $ARTIFACTS_DIR"

# Create artifacts directory if it doesn't exist
mkdir -p "$ARTIFACTS_DIR"

# Build each example for each target
for example in "${EXAMPLES[@]}"; do
    echo "Building $example..."
    
    for target in "${TARGETS[@]}"; do
        echo "  Target: $target"
        
        # Create target directory
        TARGET_DIR="$ARTIFACTS_DIR/$target"
        mkdir -p "$TARGET_DIR"
        
        # Check if target is installed
        if ! rustup target list --installed | grep -q "$target"; then
            echo "    Warning: Target $target is not installed. Skipping..."
            echo "    Run: rustup target add $target"
            continue
        fi
        
        # Build the example
        if cargo build --example "$example" --target "$target" 2>/dev/null; then
            # Copy the built binary to artifacts directory
            BINARY_PATH="$PROJECT_ROOT/target/$target/debug/examples/$example"
            if [ -f "$BINARY_PATH" ]; then
                cp "$BINARY_PATH" "$TARGET_DIR/$example"
                echo "    ✓ Built and copied to $TARGET_DIR/$example"
            else
                echo "    ✗ Binary not found at expected location: $BINARY_PATH"
            fi
        else
            echo "    ✗ Build failed for $target"
        fi
    done
done

# Also build the generated test binaries (small, medium, large)
echo "Building generated test binaries..."
cd "$PROJECT_ROOT"
if cargo run --bin generate_test_binaries 2>/dev/null; then
    echo "  ✓ Generated test binaries built successfully"
    
    # Copy generated binaries to artifacts directory
    for size in small medium large; do
        if [ -f "bin/test_binaries/$size" ]; then
            mkdir -p "$ARTIFACTS_DIR/generated"
            cp "bin/test_binaries/$size" "$ARTIFACTS_DIR/generated/$size"
            echo "  ✓ Copied $size to artifacts"
        fi
    done
else
    echo "  ✗ Failed to generate test binaries"
fi

echo ""
echo "Artifact generation complete!"
echo "The following files should be added to version control:"
echo "  git add $ARTIFACTS_DIR"