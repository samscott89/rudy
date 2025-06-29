use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(name = "cargo xtask")]
#[command(about = "Development tasks for rudy", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build test artifacts for all platforms
    BuildTestArtifacts {
        /// Only build for current platform
        #[arg(long)]
        current_platform: bool,
    },
    /// Generate test binaries (small, medium, large)
    GenerateTestBinaries,
    /// Run tests with proper artifact setup
    Test {
        /// Target platform (defaults to current)
        #[arg(long)]
        target: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::BuildTestArtifacts { current_platform } => {
            build_test_artifacts(current_platform)?;
        }
        Commands::GenerateTestBinaries => {
            generate_test_binaries()?;
        }
        Commands::Test { target } => {
            run_tests(target)?;
        }
    }

    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let root = std::env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .or_else(|_| {
            // If not in cargo environment, find workspace root
            let mut path = std::env::current_dir()?;
            while !path.join("Cargo.toml").exists() || !path.join("xtask").exists() {
                path = path
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Could not find workspace root"))?
                    .to_path_buf();
            }
            Ok::<_, anyhow::Error>(path)
        })?;
    Ok(root)
}

fn current_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => panic!(
            "Unsupported OS/ARCH: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    }
}

fn build_test_artifacts(current_platform_only: bool) -> Result<()> {
    let workspace_root = workspace_root()?;
    let artifacts_dir = workspace_root.join("rudy-db/test-artifacts");

    // Create artifacts directory
    std::fs::create_dir_all(&artifacts_dir).context("Failed to create artifacts directory")?;

    let targets = if current_platform_only {
        vec![current_target()]
    } else {
        vec![
            "aarch64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ]
    };

    let examples = vec!["simple_test", "lldb_demo"];

    println!("🛠️  Building test artifacts...");

    for example in &examples {
        println!("\n📦 Building example: {example}");

        for target in &targets {
            if !current_platform_only
                && std::env::consts::OS != "macos"
                && target.contains("darwin")
            {
                println!("  ⏭️  Skipping {target} (not on macOS)");
                continue;
            }

            // Check if target is installed
            let installed = Command::new("rustup")
                .args(["target", "list", "--installed"])
                .output()
                .context("Failed to run rustup")?;

            let installed_targets = String::from_utf8_lossy(&installed.stdout);
            if !installed_targets.contains(target) {
                println!("  ⚠️  Target {target} not installed. Run: rustup target add {target}");
                continue;
            }

            println!("  🎯 Target: {target}");

            // For Linux targets on macOS, we'd need Docker setup
            if std::env::consts::OS == "macos" && target.contains("linux") {
                println!("    ℹ️  Linux builds on macOS require Docker setup (skipping for now)");
                continue;
            }

            // Build the example
            let mut cmd = Command::new("cargo");
            cmd.args([
                "build",
                "--example",
                example,
                "--target",
                target,
                "--manifest-path",
                workspace_root.join("rudy-db/Cargo.toml").to_str().unwrap(),
            ]);

            // Add frame pointers for better debugging
            cmd.env("RUSTFLAGS", "-Cforce-frame-pointers=yes");

            let status = cmd.status().context("Failed to run cargo build")?;

            if status.success() {
                // Copy artifact to test-artifacts directory
                let binary_name = if target.contains("windows") {
                    format!("{example}.exe")
                } else {
                    example.to_string()
                };

                let source = workspace_root
                    .join("target")
                    .join(target)
                    .join("debug")
                    .join("examples")
                    .join(&binary_name);

                let target_dir = artifacts_dir.join(target);
                std::fs::create_dir_all(&target_dir)
                    .context("Failed to create target directory")?;

                let dest = target_dir.join(&binary_name);

                if source.exists() {
                    std::fs::copy(&source, &dest).context("Failed to copy binary")?;
                    println!("    ✅ Built and copied to {}", dest.display());
                } else {
                    println!("    ❌ Binary not found at {}", source.display());
                }
            } else {
                println!("    ❌ Build failed");
            }
        }
    }

    // Also build generated test binaries
    generate_test_binaries()?;

    // Copy generated binaries to artifacts
    let generated_dir = artifacts_dir.join("generated");
    std::fs::create_dir_all(&generated_dir)?;

    for size in ["small", "medium", "large"] {
        let source = workspace_root.join("rudy-db/bin/test_binaries").join(size);
        if source.exists() {
            let dest = generated_dir.join(size);
            std::fs::copy(&source, &dest)?;
            println!("✅ Copied {size} to artifacts");
        }
    }

    println!("\n✨ Test artifacts built successfully!");
    println!("📁 Artifacts location: {}", artifacts_dir.display());

    Ok(())
}

fn generate_test_binaries() -> Result<()> {
    println!("\n🔨 Generating test binaries...");

    let workspace_root = workspace_root()?;
    let status = Command::new("cargo")
        .args([
            "run",
            "--bin",
            "generate_test_binaries",
            "--manifest-path",
            workspace_root.join("rudy-db/Cargo.toml").to_str().unwrap(),
        ])
        .status()
        .context("Failed to run generate_test_binaries")?;

    if !status.success() {
        anyhow::bail!("Failed to generate test binaries");
    }

    println!("✅ Test binaries generated successfully");
    Ok(())
}

fn run_tests(target: Option<String>) -> Result<()> {
    let target = target.unwrap_or_else(|| current_target().to_string());
    let workspace_root = workspace_root()?;

    println!("🧪 Running tests for target: {target}");

    // First ensure test artifacts exist
    println!("📦 Checking test artifacts...");
    let artifacts_dir = workspace_root.join("rudy-db/test-artifacts");
    if !artifacts_dir.exists() {
        println!("⚠️  Test artifacts not found. Building...");
        build_test_artifacts(true)?;
    }

    // Set environment variable to point to artifacts
    let status = Command::new("cargo")
        .args([
            "test",
            "--target",
            &target,
            "--manifest-path",
            workspace_root.join("rudy-db/Cargo.toml").to_str().unwrap(),
        ])
        .env("RUDY_TEST_ARTIFACTS_DIR", artifacts_dir)
        .status()
        .context("Failed to run tests")?;

    if !status.success() {
        anyhow::bail!("Tests failed");
    }

    println!("✅ Tests passed!");
    Ok(())
}
