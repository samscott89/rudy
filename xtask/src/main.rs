use std::{fs, io::Write, path::PathBuf, process::Command};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod benchmarks;

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
    /// Generate example code for benchmarks
    GenerateBenchmarkExamples,
    /// Clean up Docker volumes and images
    CleanDocker {
        /// Also remove Docker images
        #[arg(long)]
        images: bool,
    },
    /// Publish test artifacts to GitHub releases
    PublishArtifacts {
        /// Force publish even if version already exists
        #[arg(long)]
        force: bool,
    },
    /// Download test artifacts from GitHub releases
    DownloadArtifacts {
        /// Specific version to download (defaults to latest)
        #[arg(long)]
        version: Option<String>,
        /// Force download even if already have latest
        #[arg(long)]
        force: bool,
    },
    /// Run Linux tests using Docker
    TestLinux,
    /// Applies an unstable/aggressive formatting option that groups imports together
    MergeImports,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::BuildExamples { current_platform } => build_examples(current_platform),
        Commands::GenerateBenchmarkExamples => generate_bench_examples(),
        Commands::CleanDocker { images } => clean_docker(images),
        Commands::PublishArtifacts { force } => publish_artifacts(force),
        Commands::DownloadArtifacts { version, force } => download_artifacts(version, force),
        Commands::TestLinux => run_in_docker("x86_64-unknown-linux-gnu", "cargo nextest run"),
        Commands::MergeImports => {
            // run `cargo fmt -- --config group_imports=StdExternalCrate,imports_granularity=Crate`
            Command::new("cargo")
                .arg("+nightly")
                .arg("fmt")
                .arg("--")
                .arg("--config")
                .arg("group_imports=StdExternalCrate,imports_granularity=Crate")
                .status()
                .context("Failed to format imports")
                .map(drop)
        }
    }
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

fn root_artifact_dir() -> Result<PathBuf> {
    let root = workspace_root()?;
    let artifacts_dir = root.join("test-artifacts");

    // Create artifacts directory if it doesn't exist
    if !artifacts_dir.exists() {
        fs::create_dir_all(&artifacts_dir).context("Failed to create test artifacts directory")?;
    }

    Ok(artifacts_dir)
}

fn artifact_dir(target: &str) -> Result<PathBuf> {
    let root = root_artifact_dir()?;
    let artifacts_dir = root.join(target);

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

fn generate_bench_examples() -> Result<()> {
    let examples_folder = workspace_root()?.join("crates/rudy-test-examples/examples");
    println!("\n🔨 Generating benchmark examples...");
    benchmarks::generate(&examples_folder, "small", 10, 5)?;
    benchmarks::generate(&examples_folder, "medium", 100, 20)?;
    benchmarks::generate(&examples_folder, "large", 500, 50)?;

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

    let artifacts_dir = artifact_dir(target)?;

    println!("  🎯 Building examples for {target}");

    let mut cmd = Command::new("cargo");
    cmd.args([
        "build",
        "--examples",
        "--target",
        target,
        "-p",
        "rudy-test-examples",
    ]);

    // Add frame pointers for better debugging
    cmd.env("RUSTFLAGS", "-Cforce-frame-pointers=yes");

    let status = cmd.status().context("Failed to run cargo build")?;

    if status.success() {
        let source_folder = workspace_root
            .join("target")
            .join(target)
            .join("debug")
            .join("examples");

        // Copy contents of folder to artifacts directory
        let source = source_folder;
        let dest = artifacts_dir;

        if source.exists() {
            // remove dest
            if dest.exists() {
                fs::remove_dir_all(&dest)
                    .context("Failed to remove existing artifacts directory")?;
            }

            fs::create_dir_all(&dest)?;

            copy_dir(&source, &dest).context("Failed to copy binaries")?;

            println!("    ✅ Built and copied to {}", dest.display());
        } else {
            println!("    ❌ Binary not found at {}", source.display());
        }
    } else {
        println!("    ❌ Build failed for target {target}");
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

fn get_test_examples_version() -> Result<String> {
    let workspace_root = workspace_root()?;
    let manifest_path = workspace_root.join("crates/rudy-test-examples/Cargo.toml");

    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            manifest_path.to_str().unwrap(),
        ])
        .output()
        .context("Failed to run cargo metadata")?;

    if !output.status.success() {
        anyhow::bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let metadata_json = String::from_utf8_lossy(&output.stdout);

    // Use jq to extract version for rudy-test-examples package specifically
    let mut jq_process = Command::new("jq")
        .args([
            "-r",
            r#".packages[] | select(.name == "rudy-test-examples") | .version"#,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn jq")?;

    if let Some(stdin) = jq_process.stdin.as_mut() {
        stdin
            .write_all(metadata_json.as_bytes())
            .context("Failed to write to jq stdin")?;
    }

    let jq_result = jq_process
        .wait_with_output()
        .context("Failed to wait for jq process")?;

    if !jq_result.status.success() {
        anyhow::bail!("jq failed: {}", String::from_utf8_lossy(&jq_result.stderr));
    }

    let version = String::from_utf8_lossy(&jq_result.stdout)
        .trim()
        .to_string();
    Ok(version)
}

fn publish_artifacts(force: bool) -> Result<()> {
    let version = get_test_examples_version()?;
    let tag = format!("test-artifacts-v{version}");

    println!("🏷️  Test examples version: {version}");

    // Check if this version already exists
    if !force {
        let check_output = Command::new("gh")
            .args(["release", "view", &tag])
            .output()
            .context("Failed to check if release exists")?;

        if check_output.status.success() {
            println!("✅ Release {tag} already exists. Use --force to republish.");
            return Ok(());
        }
    }

    println!("🛠️  Building all artifacts for version {version}...");
    build_examples(false)?;

    // Create version metadata file
    let artifacts_dir = workspace_root()?.join("test-artifacts");
    let version_file = artifacts_dir.join("VERSION");
    fs::write(&version_file, &version).context("Failed to write version file")?;

    // Create archive
    let archive_name = format!("test-artifacts-{version}.tar.gz");
    println!("📦 Creating archive: {archive_name}");

    let workspace_root = workspace_root()?;
    let status = Command::new("tar")
        .args([
            "czf",
            &archive_name,
            "-C",
            workspace_root.to_str().unwrap(),
            "test-artifacts",
        ])
        .status()
        .context("Failed to create archive")?;

    if !status.success() {
        anyhow::bail!("Failed to create archive");
    }

    // Create GitHub release
    println!("🚀 Publishing release: {tag}");
    let status = Command::new("gh")
        .args([
            "release",
            "create",
            &tag,
            &archive_name,
            "--title",
            &format!("Test Artifacts v{version}"),
            "--notes",
            &format!(
                "Test artifacts for rudy-test-examples v{version}\n\nGenerated from commit: {}",
                get_git_sha()?
            ),
        ])
        .status()
        .context("Failed to create GitHub release")?;

    if !status.success() {
        anyhow::bail!("Failed to create GitHub release");
    }

    // Clean up local archive
    fs::remove_file(&archive_name).context("Failed to remove local archive")?;

    println!("✅ Published test artifacts v{version}");
    Ok(())
}

fn download_artifacts(version: Option<String>, force: bool) -> Result<()> {
    let workspace_root = workspace_root()?;
    let artifacts_dir = root_artifact_dir()?;

    // Determine target version
    let target_version = match version {
        Some(v) => v,
        None => {
            // Get latest release version
            let output = Command::new("gh")
                .args([
                    "release", "list",
                    "--limit", "50",
                    "--json", "tagName",
                    "--jq", r#".[].tagName | select(startswith("test-artifacts-v")) | ltrimstr("test-artifacts-v")"#
                ])
                .output()
                .context("Failed to list releases")?;

            if !output.status.success() {
                anyhow::bail!(
                    "Failed to list releases: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            let versions = String::from_utf8_lossy(&output.stdout);
            let latest = versions
                .lines()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No test artifact releases found"))?;
            latest.to_string()
        }
    };

    let tag = format!("test-artifacts-v{target_version}");
    println!("🎯 Target version: {target_version}");

    // Check if we already have this version
    if !force {
        if let Ok(current_version) = fs::read_to_string(artifacts_dir.join("VERSION")) {
            if current_version.trim() == target_version {
                println!("✅ Already have version {target_version}. Use --force to redownload.");
                return Ok(());
            }
        }
    }

    // Clean up existing artifacts
    if artifacts_dir.exists() {
        println!("🧹 Cleaning existing artifacts...");
        fs::remove_dir_all(&artifacts_dir).context("Failed to remove existing artifacts")?;
    }

    // Download and extract
    println!("⬇️  Downloading {tag}...");
    let archive_name = format!("test-artifacts-{target_version}.tar.gz");

    let status = Command::new("gh")
        .args(["release", "download", &tag, "--pattern", &archive_name])
        .current_dir(&workspace_root)
        .status()
        .context("Failed to download release")?;

    if !status.success() {
        anyhow::bail!("Failed to download release");
    }

    // Extract archive
    println!("📦 Extracting archive...");
    let status = Command::new("tar")
        .args(["xzf", &archive_name, "-C", workspace_root.to_str().unwrap()])
        .status()
        .context("Failed to extract archive")?;

    if !status.success() {
        anyhow::bail!("Failed to extract archive");
    }

    // Clean up downloaded archive
    fs::remove_file(workspace_root.join(&archive_name))
        .context("Failed to remove downloaded archive")?;

    // write version file
    let version_file = artifacts_dir.join("VERSION");
    fs::write(&version_file, &target_version).context("Failed to write version file")?;

    println!("✅ Downloaded and extracted test artifacts v{target_version}");
    Ok(())
}

fn get_git_sha() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .context("Failed to get git SHA")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get git SHA");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn copy_dir(source_folder: &std::path::Path, artifacts_dir: &std::path::Path) -> Result<()> {
    let mut copied_files = 0;

    // Read all files in the source folder
    for entry in fs::read_dir(source_folder).context("Failed to read source directory")? {
        let entry = entry.context("Failed to read directory entry")?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        let source_file = entry.path();
        let dest_file = artifacts_dir.join(&file_name);
        // Copy files
        if source_file.is_file() {
            fs::copy(&source_file, &dest_file)
                .with_context(|| format!("Failed to copy {file_name_str}"))?;
            copied_files += 1;
        }
    }

    if copied_files > 0 {
        println!("    ✅ Copied {copied_files} files");
    } else {
        println!("    ⚠️  No files found in {}", source_folder.display());
    }

    Ok(())
}
