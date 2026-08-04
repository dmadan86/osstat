//! Live latency measurement against the real Hugging Face API.
//!
//! `search` consults each repository's tree sequentially, so the figure a user
//! waits for scales with `SEARCH_LIMIT` (8 in `src-tauri/src/models.rs`) rather
//! than with the size of any one repository. This measures that.
//!
//! `#[ignore]` because it needs the network; run with
//! `cargo test -p osstat-inference --test real_hf_perf -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use osstat_inference::search;

/// What `models_search` passes.
const APP_LIMIT: usize = 8;

async fn timed(client: &reqwest::Client, query: &str, limit: usize) -> (Duration, usize, usize) {
    let started = Instant::now();
    let results = search(client, query, limit).await.unwrap_or_default();
    let elapsed = started.elapsed();
    let vision = results.iter().filter(|r| r.projector.is_some()).count();
    (elapsed, results.len(), vision)
}

#[tokio::test]
#[ignore = "hits the real Hugging Face API"]
async fn search_latency_at_the_limit_the_app_actually_uses() {
    let client = reqwest::Client::new();

    println!("\n=== how latency scales with repositories consulted ===");
    for limit in [1_usize, 2, 4, APP_LIMIT] {
        let (elapsed, count, vision) = timed(&client, "Qwen2.5-VL", limit).await;
        println!(
            "  limit={limit:<2} {:>8.0} ms   {count:>3} results ({vision} with projector)",
            elapsed.as_secs_f64() * 1000.0
        );
    }

    println!("\n=== three runs at the app's own limit ({APP_LIMIT}) ===");
    let mut samples = Vec::new();
    for run in 1..=3 {
        let (elapsed, count, vision) = timed(&client, "Qwen2.5-VL", APP_LIMIT).await;
        println!(
            "  run {run}: {:>8.0} ms   {count:>3} results ({vision} with projector)",
            elapsed.as_secs_f64() * 1000.0
        );
        samples.push(elapsed);
    }
    samples.sort_unstable();
    println!(
        "  median: {:.0} ms   worst: {:.0} ms",
        samples[samples.len() / 2].as_secs_f64() * 1000.0,
        samples[samples.len() - 1].as_secs_f64() * 1000.0
    );

    println!("\n=== a text-only query, for comparison ===");
    let (elapsed, count, vision) = timed(&client, "llama 3", APP_LIMIT).await;
    println!(
        "  {:>8.0} ms   {count:>3} results ({vision} with projector)",
        elapsed.as_secs_f64() * 1000.0
    );
}
