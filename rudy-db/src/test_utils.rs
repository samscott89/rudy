use std::{collections::HashMap, path::PathBuf};

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("salsa=warn".parse().unwrap()),
        )
        .try_init();
}

fn workspace_dir() -> PathBuf {
    std::env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // If not in cargo environment, find workspace root
            let mut path = std::env::current_dir().expect("Failed to get current directory");
            while !path.join("Cargo.toml").exists() || !path.join("xtask").exists() {
                path = path
                    .parent()
                    .expect("Could not find workspace root")
                    .to_path_buf();
            }
            path
        })
}

pub fn debug_db(target: Option<&'static str>) -> crate::DebugDb {
    let previous_root = workspace_dir().join("target/debug/examples/");
    let subfolder = target.unwrap_or_else(current_arch);
    let new_root = PathBuf::from(".").join("test-artifacts").join(subfolder);
    // we add some source maps to make our debug/source files work correctly
    // on whatever platform
    // NOTE: a better approach might be to use `--remap-path-prefix` rust flag
    // to standardize paths across platforms
    crate::DebugDb::new().with_source_map(HashMap::from_iter([
        (previous_root, new_root),
        (PathBuf::from("/workspace/"), PathBuf::from(".")),
        (
            workspace_dir().join("crates/rudy-test-examples/examples"),
            PathBuf::from("./crates/rudy-test-examples/examples"),
        ),
        (PathBuf::from("/app/"), PathBuf::from(".")),
    ]))
}

pub fn root_artifacts_dir() -> PathBuf {
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
        })
        .expect("could not get workspace root");

    let artifacts = root.join("test-artifacts");
    if !artifacts.exists() {
        panic!(
            "Test artifacts directory not found at: {}. Please run `cargo xtask build-test-artifacts` to generate it.",
            artifacts.display()
        );
    }
    artifacts
}

fn current_arch() -> &'static str {
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

pub fn artifacts_dir(target: Option<&'static str>) -> PathBuf {
    let subfolder = target.unwrap_or_else(current_arch);
    root_artifacts_dir().join(subfolder)
}
