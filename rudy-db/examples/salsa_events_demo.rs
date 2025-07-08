//! Demonstrates how salsa's incremental computation works in Rudy
//!
//! Run with: cargo run --example salsa_events_demo
//! For detailed salsa logs: RUST_LOG=salsa=info cargo run --example salsa_events_demo
//! For even more detail: RUST_LOG=salsa=debug cargo run --example salsa_events_demo

use rudy_db::{DebugDb, DebugInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use test_utils::{artifacts_dir, source_map};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// Simple structure to capture salsa events
#[derive(Debug, Clone)]
struct SalsaEvent {
    target: String,
    message: String,
}

// Custom tracing layer to capture salsa events
struct SalsaCapture {
    events: Arc<Mutex<Vec<SalsaEvent>>>,
}

impl SalsaCapture {
    fn new() -> (Self, Arc<Mutex<Vec<SalsaEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let capture = Self {
            events: events.clone(),
        };
        (capture, events)
    }
}

impl<S> tracing_subscriber::Layer<S> for SalsaCapture
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // Only capture meaningful salsa events, filter out internal bookkeeping
        let target = event.metadata().target();
        if target.starts_with("salsa::") && !is_internal_salsa_event(target) {
            let mut visitor = EventVisitor::new();
            event.record(&mut visitor);

            if let Some(message) = visitor.message {
                let mut events = self.events.lock().unwrap();
                events.push(SalsaEvent {
                    target: target.to_string(),
                    message,
                });
            }
        }
    }
}

// Filter out internal salsa events that aren't meaningful for users
fn is_internal_salsa_event(target: &str) -> bool {
    // Filter out internal tracking and bookkeeping
    matches!(target, "salsa::zalsa_local")
}

// Helper to extract message from tracing events
struct EventVisitor {
    message: Option<String>,
}

impl EventVisitor {
    fn new() -> Self {
        Self { message: None }
    }
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

// Extract query name from salsa log message
fn extract_query_from_message(message: &str) -> Option<String> {
    // Messages look like "find_closest_function(Id(800)): executing query"
    if let Some(colon_pos) = message.find(':') {
        let before_colon = &message[..colon_pos];
        if let Some(paren_pos) = before_colon.find('(') {
            Some(before_colon[..paren_pos].trim_matches('"').to_string())
        } else {
            Some(before_colon.trim_matches('"').to_string())
        }
    } else {
        None
    }
}

fn main() -> anyhow::Result<()> {
    // Set up custom tracing to capture salsa events
    let (salsa_capture, captured_events) = SalsaCapture::new();

    tracing_subscriber::registry()
        .with(
            salsa_capture.with_filter(
                tracing_subscriber::filter::Targets::new()
                    // Enable the `DEBUG` level for anything in `salsa`
                    .with_target("salsa", tracing::Level::DEBUG),
            ),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
        )
        .init();

    println!("Salsa Incremental Computation Demo");
    println!("==================================\n");

    let binary_path = artifacts_dir(None).join("large");
    let db = DebugDb::new().with_source_map(source_map(None));

    // Helper to show salsa activity with cache hit/miss analysis
    let show_salsa_activity = |events: &Arc<Mutex<Vec<SalsaEvent>>>, since: usize| -> usize {
        let events_guard = events.lock().unwrap();
        let recent_events: Vec<_> = events_guard.iter().skip(since).collect();
        let new_count = events_guard.len();

        if !recent_events.is_empty() {
            let mut cache_misses = 0;
            let mut cache_hits = 0;
            let mut query_breakdown: HashMap<String, (usize, usize)> = HashMap::new(); // (misses, hits)

            for event in &recent_events {
                if let Some(query) = extract_query_from_message(&event.message) {
                    let is_cache_miss = event.target.contains("function::execute");
                    let is_cache_hit = event.target.contains("maybe_changed_after");

                    if is_cache_miss {
                        cache_misses += 1;
                        query_breakdown.entry(query).or_insert((0, 0)).0 += 1;
                    } else if is_cache_hit {
                        cache_hits += 1;
                        query_breakdown.entry(query).or_insert((0, 0)).1 += 1;
                    }
                }
            }

            let total_queries = cache_misses + cache_hits;
            if total_queries > 0 {
                let hit_rate = (cache_hits as f64 / total_queries as f64) * 100.0;
                println!("  Salsa cache performance:");
                println!(
                    "    {} total queries - {} misses, {} hits ({:.0}% hit rate)",
                    total_queries, cache_misses, cache_hits, hit_rate
                );

                // Show breakdown by query type
                if !query_breakdown.is_empty() {
                    println!("    Query breakdown:");
                    let mut sorted_queries: Vec<_> = query_breakdown.iter().collect();
                    sorted_queries.sort_by(|a, b| (b.1.0 + b.1.1).cmp(&(a.1.0 + a.1.1)));

                    for (query, (misses, hits)) in sorted_queries.iter() {
                        let query_total = misses + hits;
                        let query_hit_rate = if query_total > 0 {
                            (*hits as f64 / query_total as f64) * 100.0
                        } else {
                            0.0
                        };
                        println!(
                            "      {}: {} miss, {} hit ({:.0}%)",
                            query, misses, hits, query_hit_rate
                        );
                    }
                }
                println!();
            }
        }

        new_count
    };

    // === Phase 1: Initial database creation ===
    println!("Phase 1: Creating database");
    println!("--------------------------");
    let start = Instant::now();
    let event_count = captured_events.lock().unwrap().len();
    let debug_info = DebugInfo::new(&db, &binary_path)?;
    println!(
        "Database created in {:.2}ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let event_count = show_salsa_activity(&captured_events, event_count);

    // === Phase 2: First query (cold cache) ===
    println!("Phase 2: First function lookup (cold cache)");
    println!("--------------------------------------------");
    let start = Instant::now();
    let result = debug_info.find_function_by_name("main")?;
    let cold_time = start.elapsed();
    println!("Found: {:?}", result.map(|f| format!("{:#x}", f.address)));
    println!(
        "Cold lookup took: {:.2}ms",
        cold_time.as_secs_f64() * 1000.0
    );
    let event_count = show_salsa_activity(&captured_events, event_count);

    // === Phase 3: Repeated query (warm cache) ===
    println!("Phase 3: Same function lookup (warm cache)");
    println!("-------------------------------------------");
    let start = Instant::now();
    let result = debug_info.find_function_by_name("main")?;
    let warm_time = start.elapsed();
    println!("Found: {:?}", result.map(|f| format!("{:#x}", f.address)));
    println!(
        "Warm lookup took: {:.2}ms",
        warm_time.as_secs_f64() * 1000.0
    );

    let speedup = cold_time.as_secs_f64() / warm_time.as_secs_f64();
    println!("Speedup: {:.0}x faster", speedup);
    let event_count = show_salsa_activity(&captured_events, event_count);

    // === Phase 4: Different query ===
    println!("Phase 4: Different function lookup");
    println!("----------------------------------");
    let start = Instant::now();
    let result = debug_info.find_function_by_name("TestStruct0::method_0")?;
    let time = start.elapsed();
    println!("Found: {:?}", result.map(|f| format!("{:#x}", f.address)));
    println!("Lookup took: {:.2}ms", time.as_secs_f64() * 1000.0);
    let event_count = show_salsa_activity(&captured_events, event_count);

    // === Phase 5: Address resolution ===
    println!("Phase 5: Address to location resolution");
    println!("---------------------------------------");
    let start = Instant::now();
    let result = debug_info.address_to_location(0x100001000)?;
    let time = start.elapsed();
    println!(
        "Resolved: {:?}",
        result.map(|l| format!("{}:{}", l.file, l.line))
    );
    println!("Resolution took: {:.2}ms", time.as_secs_f64() * 1000.0);
    let event_count = show_salsa_activity(&captured_events, event_count);

    // === Phase 6: Bulk repeated queries ===
    println!("Phase 6: Bulk repeated queries (demonstrating cache hits)");
    println!("---------------------------------------------------------");
    let queries = [
        ("main", "find_function_by_name"),
        ("TestStruct0::method_0", "find_function_by_name"),
    ];

    let start = Instant::now();
    for _ in 0..10 {
        for (name, _desc) in &queries {
            let _ = debug_info.find_function_by_name(name)?;
        }
        let _ = debug_info.address_to_location(0x100001000)?;
    }
    let batch_time = start.elapsed();
    println!(
        "Performed 30 cached queries in {:.2}ms",
        batch_time.as_secs_f64() * 1000.0
    );
    println!(
        "Average per query: {:.3}ms",
        batch_time.as_secs_f64() * 1000.0 / 30.0
    );
    let _event_count = show_salsa_activity(&captured_events, event_count);

    // === Analysis ===
    println!("Salsa Database Statistics");
    println!("========================");
    // Note: queries_info() method varies by salsa version
    println!(
        "Total events captured: {}",
        captured_events.lock().unwrap().len()
    );

    println!("\nHow Salsa Incremental Computation Works:");
    println!("• First queries trigger expensive DWARF parsing and indexing");
    println!("• Results are memoized based on input arguments");
    println!("• Repeated queries with same arguments return cached results");
    println!("• When binaries change, only affected queries are invalidated");
    println!("• This enables fast incremental recompilation in debuggers");

    println!("\nTo see salsa's internal behavior:");
    println!("  RUST_LOG=salsa=info cargo run --example salsa_events_demo");

    Ok(())
}
