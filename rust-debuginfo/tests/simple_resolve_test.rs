//! Simple test to debug the resolve_type performance issue

use anyhow::Result;
use rust_debuginfo::{DebugDb, DebugInfo};

#[test]
fn test_simple_resolve_debug() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info = DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    println!("Starting resolve_type call...");
    let start = std::time::Instant::now();
    
    // Try to resolve a simple type
    match debug_info.resolve_type("u32") {
        Ok(Some(typedef)) => {
            println!("Found u32 type in {:?}: {}", start.elapsed(), typedef.display_name());
        }
        Ok(None) => {
            println!("u32 type not found in {:?}", start.elapsed());
        }
        Err(e) => {
            println!("Error resolving u32 type in {:?}: {}", start.elapsed(), e);
        }
    }

    Ok(())
}