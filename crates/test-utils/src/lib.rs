use std::path::PathBuf;

use rstest_reuse::{self, *};

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive("info".parse().unwrap())
                .from_env()
                .unwrap()
                .add_directive("salsa=warn".parse().unwrap()),
        )
        .with_line_number(true)
        .try_init();
}

pub fn init_tracing_and_insta() -> insta::internals::SettingsBindDropGuard {
    init_tracing();
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    add_filters(&mut settings);
    settings.bind_to_scope()
}

fn rustup_home() -> String {
    std::env::var("RUSTUP_HOME").unwrap_or_else(|_| {
        // If RUST_HOME is not set, assume it's the default location
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}/.rustup")
    })
}

/// Filters to scrub machine-specific paths from snapshots.
pub fn add_filters(s: &mut insta::Settings) {
    let ws = workspace_dir();
    let ws = ws.to_str().unwrap();
    let rustup_home = rustup_home();
    let rustup_home = rustup_home.as_str();
    let filters = [
        (ws, "[CARGO_WORKSPACE_DIR]/"),
        (rustup_home, "[RUSTUP_HOME]"),
        // also need to set these for files generated
        // from my laptop or from the docker container
        ("/Users/sam/work/rudy/", "[CARGO_WORKSPACE_DIR]/"),
        ("/app/", "[CARGO_WORKSPACE_DIR]/"),
        ("/Users/sam/.rustup", "[RUSTUP_HOME]"),
        ("/root/.rustup", "[RUSTUP_HOME]"),
        (r"tv_sec: [0-9]+", "tv_sec: [ts]"),
        (r"tv_nsec: [0-9]+", "tv_nsec: [ts]"),
    ];

    for (from, to) in filters.iter() {
        s.add_filter(from, *to);
    }
}

pub fn workspace_dir() -> PathBuf {
    std::env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // If not in cargo environment, find workspace root
            let mut path = std::env::current_dir().expect("Failed to get current directory");
            while !path.join("Cargo.toml").exists() {
                path = path
                    .parent()
                    .expect("Could not find workspace root")
                    .to_path_buf();
            }
            path
        })
}

pub fn source_map(target: Option<&'static str>) -> Vec<(PathBuf, PathBuf)> {
    let subfolder = target.unwrap_or_else(current_arch);
    let old_target_dir = workspace_dir()
        .join("target")
        .join(subfolder)
        .join("debug")
        .join("examples");
    let new_artifacts = workspace_dir().join("test-artifacts").join(subfolder);
    let rustup_home = PathBuf::from(rustup_home());
    tracing::info!("RUSTUP HOME: {}", rustup_home.display());
    // we add some source maps to make our debug/source files work correctly
    // on whatever platform
    // NOTE: a better approach might be to use `--remap-path-prefix` rust flag
    // to standardize paths across platforms

    let sam_workspace = PathBuf::from("/Users/sam/work/rudy");

    vec![
        // first, replace `sam_workspace` with the current workspace root
        (sam_workspace.clone(), workspace_dir()),
        // TODO: This is probably not currently active
        // but if we use `remap-path-prefix` in the future, we can use this
        // to remap the workspace root to the current workspace root
        (PathBuf::from("/workspace"), workspace_dir()),
        // this is the path where the xtask is run from in docker
        (PathBuf::from("/app"), workspace_dir()),
        // then, remap any path in the generic target dir, into the target
        // artifacts directory
        (old_target_dir, new_artifacts.clone()),
        // also remap the rustup home directory
        (PathBuf::from("/Users/sam/.rustup"), rustup_home.clone()),
        (PathBuf::from("/root/.rustup"), rustup_home.clone()),
        (PathBuf::from("/rust_home"), rustup_home),
    ]
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

#[template]
#[export]
#[rstest]
#[case("aarch64-unknown-linux-gnu")]
#[case("x86_64-unknown-linux-gnu")]
// we can only really run these on macOS when we have the sources
// installed, since macos relies on debug symbols living
// alongside the standard libraries
// on mac we can run all of these, since the linux debug info is
// self-contained
#[cfg_attr(target_os = "macos", case("aarch64-apple-darwin"))]
#[cfg_attr(target_os = "macos", case("x86_64-apple-darwin"))]
pub fn binary_target(#[case] target: &'static str) {}
