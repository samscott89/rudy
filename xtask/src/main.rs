use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

mod benchmark_binaries;

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
    BuildExamples {
        /// Only build for current platform
        #[arg(long)]
        current_platform: bool,
    },
    /// Build benchmark binaries (small, medium, large)
    BuildBenchmarkBinaries {
        /// Only build for current platform
        #[arg(long)]
        current_platform: bool,
    },
    /// Clean up Docker volumes and images
    CleanDocker {
        /// Also remove Docker images
        #[arg(long)]
        images: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::BuildExamples { current_platform } => {
            build_examples(current_platform)?;
        }
        Commands::BuildBenchmarkBinaries { current_platform } => {
            build_bench_binaries(current_platform)?;
        }
        Commands::CleanDocker { images } => {
            clean_docker(images)?;
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

fn artifact_dir() -> Result<PathBuf> {
    let root = workspace_root()?;
    let target = current_target();
    let artifacts_dir = root.join("rudy-db/test-artifacts").join(target);

    // Create artifacts directory if it doesn't exist
    if !artifacts_dir.exists() {
        fs::create_dir_all(&artifacts_dir).context("Failed to create test artifacts directory")?;
    }

    Ok(artifacts_dir)
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

fn build_examples(current_platform_only: bool) -> Result<()> {
    let workspace_root = workspace_root()?;

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

    println!("🛠️  Building example artifacts...");

    for target in targets {
        if target.contains("linux") && std::env::consts::OS == "macos" {
            // run Linux targets on macOS using Docker
            println!("🐳 Building Linux target {target} using Docker");

            run_in_docker(target, "cargo xtask build-examples --current-platform")?;
        } else if target.contains("darwin") && std::env::consts::OS == "linux" {
            // Skip macOS targets on Linux
            println!("⚠️ Skipping macOS target {target} on Linux");
        } else {
            println!("🛠️  Generating example binaries for {target}");

            build_examples_for_target(&workspace_root, target)?;

            println!("  ✅ Generated binaries for {target}");
        }
    }

    Ok(())
}

fn build_bench_binaries(current_platform_only: bool) -> Result<()> {
    println!("\n🔨 Generating benchmark binaries...");
    let artifacts_dir = artifact_dir()?;

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

    for target in targets {
        if target.contains("linux") && std::env::consts::OS == "macos" {
            // run Linux targets on macOS using Docker
            println!("🐳 Building Linux target {target} using Docker");

            run_in_docker(
                target,
                "cargo xtask build-bench-binaries --current-platform",
            )?;
        } else if target.contains("darwin") && std::env::consts::OS == "linux" {
            // Skip macOS targets on Linux
            println!("⚠️ Skipping macOS target {target} on Linux");
        } else {
            println!("🛠️  Generating benchmark binaries for {target}");

            // Generate small, medium, and large test programs
            benchmark_binaries::generate(&artifacts_dir, "small", 10, 5)?;
            benchmark_binaries::generate(&artifacts_dir, "medium", 100, 20)?;
            benchmark_binaries::generate(&artifacts_dir, "large", 500, 50)?;
            println!("  ✅ Generated binaries for {target}");
        }
    }

    println!("✅ All benchmark binaries generated successfully");
    Ok(())
}

fn build_examples_for_target(workspace_root: &std::path::Path, target: &str) -> Result<()> {
    println!("📦 Building examples for {target}");

    // Check if target is installed
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("Failed to run rustup")?;

    let installed_targets = String::from_utf8_lossy(&installed.stdout);
    if !installed_targets.contains(target) {
        println!("  ⚠️  Target {target} not installed. Run: rustup target add {target}");
        return Ok(());
    }

    let examples = ["simple_test", "lldb_demo"];
    let artifacts_dir = artifact_dir()?;

    for example in &examples {
        println!("  🎯 Building {example}");

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
            let source = workspace_root
                .join("target")
                .join(target)
                .join("debug")
                .join("examples")
                .join(example);

            let dest = artifacts_dir.join(example);

            if source.exists() {
                std::fs::copy(&source, &dest).context("Failed to copy binary")?;
                println!("    ✅ Built and copied to {}", dest.display());
            } else {
                println!("    ❌ Binary not found at {}", source.display());
            }
        } else {
            println!("    ❌ Build failed for {example}");
        }
    }

    Ok(())
}

fn build_docker_image(target: &str) -> Result<()> {
    let arch = match target {
        "x86_64-unknown-linux-gnu" => "x86_64",
        "aarch64-unknown-linux-gnu" => "aarch64",
        _ => anyhow::bail!("Unsupported target: {target}"),
    };

    println!("🐳 Building Docker image for `{target}` builds...");

    let status = Command::new("docker")
        .args([
            "build",
            "--load",
            "--platform",
            &format!("linux/{arch}"),
            "-f",
            "testing/Dockerfile",
            // context is the testing folder
            // (we'll mount the workspace root later)
            "testing",
        ])
        .arg("-t")
        .arg(format!("rudy-builder.{arch}"))
        .status()
        .context("Failed to execute docker build")?;

    if !status.success() {
        anyhow::bail!("Docker build failed");
    }

    println!("✅ Docker image built successfully");
    Ok(())
}

fn run_in_docker(target: &str, command: &str) -> Result<()> {
    println!("🐳 Running `{command}` for `{target}` using Docker...");

    let arch = match target {
        "x86_64-unknown-linux-gnu" => "x86_64",
        "aarch64-unknown-linux-gnu" => "aarch64",
        _ => anyhow::bail!("Unsupported target: {target}"),
    };

    let image = format!("rudy-builder.{arch}");

    // First ensure Docker image exists
    let output = Command::new("docker")
        .args(["images", "-q", &image])
        .output()
        .context("Failed to check Docker images")?;

    // build the image if it doesn't exist
    if output.stdout.is_empty() {
        build_docker_image(target)?;
    }

    // Ensure Docker volumes exist for caching
    let volume_name = format!("rudy-linux-cache-{arch}");
    ensure_docker_volume_exists(&volume_name)?;

    let command_args = command.split_whitespace().collect::<Vec<_>>();

    let status = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--platform",
            &format!("linux/{arch}"),
            "-v",
            &format!("{}:/app", workspace_root()?.display()),
            // Mount volume for target cache
            "--mount",
            &format!("src={volume_name},dst=/app/target,volume-subpath=target"),
            // Mount volume for cargo cache
            "--mount",
            &format!("src={volume_name},dst=/root/.cargo,volume-subpath=cargo-cache"),
            &image,
        ])
        .args(command_args)
        .status()
        .context("Failed to execute docker run")?;

    if !status.success() {
        anyhow::bail!("Docker build failed for `{target}`: `{command}`");
    }

    Ok(())
}

fn ensure_docker_volume_exists(volume_name: &str) -> Result<()> {
    // Check if volume exists
    let output = Command::new("docker")
        .args(["volume", "ls", "-q", "-f", &format!("name={volume_name}")])
        .output()
        .context("Failed to list Docker volumes")?;

    let volume_exists = !output.stdout.is_empty();

    if !volume_exists {
        println!("📦 Creating Docker volume: {volume_name}");
        let status = Command::new("docker")
            .args(["volume", "create", volume_name])
            .status()
            .context("Failed to create Docker volume")?;

        if !status.success() {
            anyhow::bail!("Failed to create Docker volume: {volume_name}");
        }

        // Initialize the volume with required subdirectories
        println!("📁 Initializing volume subdirectories...");
        let status = Command::new("docker")
            .args([
                "run",
                "--rm",
                "-v",
                &format!("{volume_name}:/vol"),
                "alpine:latest",
                "sh",
                "-c",
                "mkdir -p /vol/target /vol/cargo-cache && chmod 755 /vol/target /vol/cargo-cache",
            ])
            .status()
            .context("Failed to initialize Docker volume subdirectories")?;

        if !status.success() {
            anyhow::bail!("Failed to initialize Docker volume subdirectories: {volume_name}");
        }

        println!("✅ Created and initialized Docker volume: {volume_name}");
    }

    Ok(())
}

fn clean_docker(also_remove_images: bool) -> Result<()> {
    println!("🧹 Cleaning up Docker resources...");

    // List and remove rudy-related volumes
    let output = Command::new("docker")
        .args(["volume", "ls", "-q", "-f", "name=rudy-linux-cache-"])
        .output()
        .context("Failed to list Docker volumes")?;

    let volumes = String::from_utf8_lossy(&output.stdout);

    if !volumes.trim().is_empty() {
        println!("🗑️  Removing Docker volumes...");
        for volume in volumes.lines() {
            let volume = volume.trim();
            if !volume.is_empty() {
                println!("  Removing volume: {volume}");
                let status = Command::new("docker")
                    .args(["volume", "rm", volume])
                    .status()
                    .context("Failed to remove Docker volume")?;

                if status.success() {
                    println!("    ✅ Removed");
                } else {
                    println!("    ❌ Failed to remove");
                }
            }
        }
    } else {
        println!("  No rudy-related volumes found");
    }

    if also_remove_images {
        println!("🗑️  Removing Docker images...");

        // List and remove rudy-related images
        let output = Command::new("docker")
            .args(["images", "-q", "rudy-builder.*"])
            .output()
            .context("Failed to list Docker images")?;

        let images = String::from_utf8_lossy(&output.stdout);

        if !images.trim().is_empty() {
            for image_id in images.lines() {
                let image_id = image_id.trim();
                if !image_id.is_empty() {
                    println!("  Removing image: {image_id}");
                    let status = Command::new("docker")
                        .args(["rmi", image_id])
                        .status()
                        .context("Failed to remove Docker image")?;

                    if status.success() {
                        println!("    ✅ Removed");
                    } else {
                        println!("    ❌ Failed to remove");
                    }
                }
            }
        } else {
            println!("  No rudy-related images found");
        }
    }

    println!("✅ Docker cleanup complete");
    Ok(())
}
