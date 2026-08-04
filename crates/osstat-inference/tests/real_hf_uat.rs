//! Live UAT against the real Hugging Face API.
//!
//! Every other test in this crate runs against a stub socket, which proves the
//! parsing but not the assumption underneath it: that real vision repositories
//! name their projector `mmproj-*.gguf` and ship it beside the weights. This
//! asks the actual API.
//!
//! `#[ignore]` because it needs the network and a third party's uptime; run it
//! with `cargo test -p osstat-inference --test real_hf_uat -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use osstat_inference::search;

#[tokio::test]
#[ignore = "hits the real Hugging Face API"]
async fn a_real_vision_repository_yields_models_carrying_a_projector() {
    let client = reqwest::Client::new();

    let started = Instant::now();
    let results = search(&client, "Qwen2.5-VL-3B-Instruct-GGUF", 3)
        .await
        .expect("the search itself failed");
    let elapsed = started.elapsed();

    println!(
        "\n=== vision search: {} results in {elapsed:?} ===",
        results.len()
    );

    let mut with_projector = 0;
    for result in &results {
        let projector = result.projector.as_ref().map_or_else(
            || "        (none)".to_owned(),
            |p| format!("        + {} ({} bytes)", p.file, p.size_bytes),
        );
        println!(
            "  {} [{}] {} bytes\n{projector}",
            result.file,
            result.quant_hint.as_deref().unwrap_or("-"),
            result.size_bytes
        );
        if result.projector.is_some() {
            with_projector += 1;
        }
    }

    assert!(!results.is_empty(), "the real API returned nothing at all");

    // The claim the whole feature rests on.
    assert!(
        with_projector > 0,
        "no real result carried a projector -- the mmproj-* convention assumption is wrong"
    );

    // The bug half: a projector must never be offered as a model.
    for result in &results {
        let name = result.file.rsplit('/').next().unwrap_or(&result.file);
        assert!(
            !name.to_ascii_lowercase().starts_with("mmproj"),
            "a projector was offered as a downloadable model: {}",
            result.file
        );
    }

    // Every result from one repository must agree on its projector.
    let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for result in &results {
        if let Some(projector) = &result.projector
            && let Some(previous) = seen.insert(&result.repo, &projector.file)
        {
            assert_eq!(
                previous, projector.file,
                "one repository handed out two different projectors"
            );
        }
    }

    // Everything the download path will re-check.
    for result in &results {
        assert!(
            result.is_well_formed(),
            "a live result would be refused at download time: {result:?}"
        );
    }
}

#[tokio::test]
#[ignore = "hits the real Hugging Face API"]
async fn a_real_text_only_repository_attaches_nothing() {
    let client = reqwest::Client::new();

    let started = Instant::now();
    let results = search(&client, "Meta-Llama-3-8B-Instruct-GGUF", 2)
        .await
        .expect("the search itself failed");
    let elapsed = started.elapsed();

    println!(
        "\n=== text-only search: {} results in {elapsed:?} ===",
        results.len()
    );
    for result in results.iter().take(5) {
        println!(
            "  {} -> projector: {:?}",
            result.file,
            result.projector.as_ref().map(|p| &p.file)
        );
    }

    assert!(!results.is_empty(), "the real API returned nothing at all");
    for result in &results {
        assert!(
            result.projector.is_none(),
            "a text-only model was given a projector: {} -> {:?}",
            result.file,
            result.projector
        );
    }
}
