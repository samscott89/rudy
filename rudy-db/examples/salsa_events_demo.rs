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
    query_stack: Vec<String>, // Captured from salsa backtrace
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
                let query_stack = if let Some(bt) = salsa::Backtrace::capture() {
                    // Parse backtrace to extract query stack
                    parse_backtrace(&format!("{bt}"))
                } else {
                    Vec::new()
                };

                let mut events = self.events.lock().unwrap();
                events.push(SalsaEvent {
                    target: target.to_string(),
                    message,
                    query_stack,
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

// Parse salsa backtrace to extract query names
fn parse_backtrace(backtrace_str: &str) -> Vec<String> {
    let mut queries = Vec::new();

    for line in backtrace_str.lines() {
        let trimmed = line.trim();
        // Look for lines with query names like "0: lookup_address(Id(3000))"
        if let Some(colon_pos) = trimmed.find(':') {
            let after_colon = trimmed[colon_pos + 1..].trim();
            // Extract query name before the first '('
            if let Some(paren_pos) = after_colon.find('(') {
                let query_name = after_colon[..paren_pos].trim();
                if !query_name.is_empty() && !query_name.starts_with("at ") {
                    queries.push(query_name.to_string());
                }
            }
        }
    }

    queries
}

// Tree structure for dependency analysis
#[derive(Debug, Clone)]
struct QueryNode {
    query_name: String,
    miss_count: usize,
    hit_count: usize,
    children: Vec<QueryNode>,
}

impl QueryNode {
    fn new(query_name: String, is_cache_miss: bool) -> Self {
        Self {
            query_name,
            miss_count: if is_cache_miss { 1 } else { 0 },
            hit_count: if is_cache_miss { 0 } else { 1 },
            children: Vec::new(),
        }
    }

    fn merge_or_add_child(&mut self, child: QueryNode) {
        // Try to find existing child with same name (regardless of cache status)
        if let Some(existing) = self
            .children
            .iter_mut()
            .find(|c| c.query_name == child.query_name)
        {
            existing.miss_count += child.miss_count;
            existing.hit_count += child.hit_count;
            // Merge children recursively
            for grandchild in child.children {
                existing.merge_or_add_child(grandchild);
            }
        } else {
            self.children.push(child);
        }
    }

    fn total_count(&self) -> usize {
        self.miss_count + self.hit_count
    }
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

// Build a dependency tree from events using query stack information
fn build_query_tree(events: &[&SalsaEvent]) -> Vec<QueryNode> {
    let mut roots: Vec<QueryNode> = Vec::new();

    for event in events {
        if let Some(query_name) = extract_query_from_message(&event.message) {
            let is_cache_miss = event.target.contains("function::execute");
            let is_cache_hit = event.target.contains("maybe_changed_after");

            if is_cache_miss || is_cache_hit {
                let stack = &event.query_stack;

                if stack.is_empty() {
                    // Root level query
                    let node = QueryNode::new(query_name, is_cache_miss);
                    if let Some(existing_root) =
                        roots.iter_mut().find(|r| r.query_name == node.query_name)
                    {
                        existing_root.miss_count += node.miss_count;
                        existing_root.hit_count += node.hit_count;
                    } else {
                        roots.push(node);
                    }
                } else {
                    // Build tree from stack (stack[0] is current query, stack[1] is parent, etc.)
                    let current_query = stack.first().unwrap_or(&query_name);
                    let node = QueryNode::new(current_query.clone(), is_cache_miss);

                    if stack.len() == 1 {
                        // Direct root query
                        if let Some(existing_root) =
                            roots.iter_mut().find(|r| r.query_name == node.query_name)
                        {
                            existing_root.miss_count += node.miss_count;
                            existing_root.hit_count += node.hit_count;
                        } else {
                            roots.push(node);
                        }
                    } else {
                        // Find or create parent and add as child
                        let parent_query = &stack[1];
                        let parent_node = roots.iter_mut().find(|r| r.query_name == *parent_query);

                        if let Some(parent) = parent_node {
                            parent.merge_or_add_child(node);
                        } else {
                            // Create parent if it doesn't exist (assume it's a hit for parents we haven't seen execute)
                            let mut parent = QueryNode::new(parent_query.clone(), false);
                            parent.merge_or_add_child(node);
                            roots.push(parent);
                        }
                    }
                }
            }
        }
    }

    // Keep chronological order - don't sort
    roots
}

// Display the query tree
fn display_query_tree(nodes: &[QueryNode], indent: usize) {
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        let prefix = "  ".repeat(indent);
        let tree_char = if is_last { "└─" } else { "├─" };

        // Format the cache performance display
        let performance = match (node.miss_count, node.hit_count) {
            (0, hits) if hits > 1 => format!("hit ({hits}×)"),
            (misses, 0) if misses > 1 => format!("miss ({misses}×)"),
            (misses, hits) if misses > 0 && hits > 0 => {
                format!("{misses} miss, {hits} hit")
            }
            (1, 0) => "miss".to_string(),
            (0, 1) => "hit".to_string(),
            _ => format!("{}×", node.total_count()),
        };

        println!(
            "{}{} {}: {}",
            prefix, tree_char, node.query_name, performance
        );

        if !node.children.is_empty() {
            let child_prefix = if is_last { "  " } else { "│ " };
            display_query_tree_with_prefix(&node.children, indent + 1, child_prefix);
        }
    }
}

// Helper function for better tree formatting with proper vertical lines
fn display_query_tree_with_prefix(nodes: &[QueryNode], indent: usize, line_prefix: &str) {
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        let base_prefix = "  ".repeat(indent.saturating_sub(1));
        let tree_char = if is_last { "└─" } else { "├─" };

        // Format the cache performance display
        let performance = match (node.miss_count, node.hit_count) {
            (0, hits) if hits > 1 => format!("hit ({hits}×)"),
            (misses, 0) if misses > 1 => format!("miss ({misses}×)"),
            (misses, hits) if misses > 0 && hits > 0 => {
                format!("{misses} miss, {hits} hit")
            }
            (1, 0) => "miss".to_string(),
            (0, 1) => "hit".to_string(),
            _ => format!("{}×", node.total_count()),
        };

        println!(
            "{}{}{} {}: {}",
            base_prefix, line_prefix, tree_char, node.query_name, performance
        );

        if !node.children.is_empty() {
            let child_prefix = if is_last { "  " } else { "│ " };
            display_query_tree_with_prefix(&node.children, indent + 1, child_prefix);
        }
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
                    "    {total_queries} total queries - {cache_misses} misses, {cache_hits} hits ({hit_rate:.0}% hit rate)"
                );

                // Show tree-based breakdown
                println!("    Query dependency tree:");
                let tree = build_query_tree(&recent_events);
                display_query_tree(&tree, 2);
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
    println!("Speedup: {speedup:.0}x faster");
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
