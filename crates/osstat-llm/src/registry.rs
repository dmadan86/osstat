//! The model registry: real, sourced facts about popular open models, and the
//! quantization levels the calculator prices them at.
//!
//! ROADMAP.md's M4 gate calls this "data, not code": the JSON in
//! `crates/osstat-llm/registry/models.json` (validated against
//! `models.schema.json` in this module's tests) is the entire registry, and
//! this file only parses it into the types [`crate::calculator`] consumes. A
//! new model or a corrected parameter count is a data change, not a Rust
//! change — the point of a separate file the roadmap says can "ship without a
//! release".
//!
//! Every fact in the seed data is meant to be checkable: each
//! [`ModelEntry::source_note`] says where its numbers came from, and a model
//! this crate's authors were not confident about was left out rather than
//! filled in with a guess. An invented parameter count would make every
//! verdict computed from it wrong in a way nobody could see, which is worse
//! than a smaller, honest registry.

use serde::{Deserialize, Serialize};

/// One quantization level the calculator evaluates every model at.
///
/// Bit widths are a property of the quantization scheme, not of any one
/// model — GGUF k-quants mix precisions across tensor types, so the figure
/// here is the commonly measured average, not an exact constant. That is why
/// this lives once per registry rather than being repeated on every
/// [`ModelEntry`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct QuantLevel {
    /// Stable identifier matching the llama.cpp quantization name, e.g.
    /// `Q4_K_M`.
    pub id: String,
    /// What the UI shows for this level.
    pub label: String,
    /// Average bits per weight, used to derive a file size from a parameter
    /// count rather than storing an exact byte count nobody could verify.
    pub bits_per_weight: f64,
    /// What this level trades off, for the explanation drawer.
    pub description: String,
}

/// The architectural dimensions the KV-cache formula needs.
///
/// `hidden_size` is kept for documentation even though the calculator never
/// reads it directly — the cache scales with `head_dim` and `num_kv_heads`,
/// which do not always divide evenly out of `hidden_size` (Gemma 2's `head_dim`
/// of 256 is wider than `hidden_size / num_attention_heads` on the 9B and 2B
/// variants), so both are stored rather than derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelArchitecture {
    /// Transformer block count (`num_hidden_layers`).
    pub num_layers: u32,
    /// Model dimension (`hidden_size`).
    pub hidden_size: u32,
    /// Query head count (`num_attention_heads`).
    pub num_attention_heads: u32,
    /// Key/value head count (`num_key_value_heads`) — equal to
    /// `num_attention_heads` on a model with no grouped-query attention.
    pub num_kv_heads: u32,
    /// Per-head dimension. The KV cache scales with this and `num_kv_heads`.
    pub head_dim: u32,
    /// The model's native maximum context (`max_position_embeddings`), used
    /// to cap the context-length control in the UI.
    pub max_context_length: u32,
}

/// One downloadable GGUF file: a specific model at a specific quantization,
/// pinned by hash.
///
/// The URL is derived from `repo` and `file` rather than stored, so the host
/// cannot vary per entry — a wider trust surface than this feature needs, and
/// one nobody would notice widening.
///
/// `publisher` is recorded because these are community re-quantizations rather
/// than the model vendors' own uploads. Llama and Gemma are gated on Hugging
/// Face, and pinning the official repositories would mean nothing downloads
/// without an account and a stored token. Trusting a third-party re-quantiser
/// is a real trade, so the UI names who published every file rather than
/// presenting a re-upload as though it were the vendor's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelDownload {
    /// The [`QuantLevel::id`] this file is a build of, e.g. `Q4_K_M`.
    pub quant_id: String,
    /// Hugging Face repository, e.g. `bartowski/Qwen2.5-7B-Instruct-GGUF`.
    pub repo: String,
    /// Who published the re-quantization, shown beside the download control.
    pub publisher: String,
    /// File name within the repository.
    pub file: String,
    /// SHA256 of the file, lowercase hex. Pinned here rather than fetched
    /// alongside the file: a digest from the same origin proves only that the
    /// transfer was not corrupted, not that the bytes are the ones anybody
    /// reviewed.
    pub sha256: String,
    /// Exact size in bytes, from the Hugging Face API. The free-space check
    /// depends on this being real rather than an estimate.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub size_bytes: u64,
}

/// One model in the registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    /// Stable slug, used as a React key and to address a row in the fit
    /// matrix.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Model family, for grouping in the UI.
    pub family: String,
    /// Total parameter count in billions, as published by the model's own
    /// card or technical report.
    pub parameters_billion: f64,
    /// The dimensions the KV-cache formula needs.
    pub architecture: ModelArchitecture,
    /// Where the numbers above came from, so a reviewer can check them
    /// without re-deriving anything.
    pub source_note: String,
    /// The pinned GGUF files available for this model, at most one per
    /// quantization. Defaults to empty so a model nobody has pinned yet needs
    /// no change to the JSON — it simply has no download control.
    #[serde(default)]
    pub downloads: Vec<ModelDownload>,
}

/// The whole registry: quantization levels once, models many times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelRegistry {
    /// Bumped whenever the shape of this file changes.
    pub version: u32,
    /// Quantization levels every model is priced at.
    pub quant_levels: Vec<QuantLevel>,
    /// The models themselves.
    pub models: Vec<ModelEntry>,
}

/// The seed registry, embedded at compile time.
///
/// Embedding rather than reading from disk at runtime means the registry
/// travels with the binary — there is no install-time path to get wrong —
/// and it is still a plain-data file a contributor edits without touching
/// Rust.
const SEED_REGISTRY_JSON: &str = include_str!("../registry/models.json");

/// Parses the embedded seed registry.
///
/// A malformed embedded file would be a build-time mistake this crate's own
/// tests catch (see the `tests` module below), not something that can happen
/// to a shipped binary — so rather than surface a `Result` a caller would
/// have nothing useful to do with, a parse failure degrades to an empty
/// registry. An empty "no models known" advisor is a safe, honest fallback;
/// panicking over embedded data the tests already guard is not.
#[must_use]
pub fn seeded_registry() -> ModelRegistry {
    serde_json::from_str(SEED_REGISTRY_JSON).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const SCHEMA_JSON: &str = include_str!("../registry/models.schema.json");

    #[test]
    fn the_seed_registry_parses_into_the_expected_shape() {
        let registry = seeded_registry();

        assert!(
            !registry.models.is_empty(),
            "the registry must not be empty"
        );
        assert!(!registry.quant_levels.is_empty());
    }

    #[test]
    fn the_seed_registry_conforms_to_its_own_json_schema() {
        // The schema and the data are two files that can drift apart
        // silently; this is the test that notices.
        let schema: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
        let instance: serde_json::Value = serde_json::from_str(SEED_REGISTRY_JSON).unwrap();

        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|error| error.to_string())
            .collect();

        assert!(errors.is_empty(), "schema violations: {errors:?}");
    }

    #[test]
    fn roughly_the_promised_fifteen_models_across_four_quantizations() {
        let registry = seeded_registry();

        // ROADMAP.md M4: "~15 popular models x 4 quantizations". Not an exact
        // gate — a model correctly left out for lack of a sourced parameter
        // count should never make this test the thing standing in the way.
        assert!(registry.models.len() >= 12);
        assert_eq!(registry.quant_levels.len(), 4);
    }

    #[test]
    fn every_model_id_is_unique() {
        let registry = seeded_registry();
        let mut ids: Vec<&str> = registry.models.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            before,
            "a duplicate model id would alias two rows in the UI"
        );
    }

    #[test]
    fn every_model_cites_where_its_numbers_came_from() {
        let registry = seeded_registry();
        for model in &registry.models {
            assert!(
                !model.source_note.trim().is_empty(),
                "{} has no source note — an unsourced figure is indistinguishable from a guess",
                model.id
            );
        }
    }

    #[test]
    fn every_quant_level_has_a_positive_bit_width() {
        let registry = seeded_registry();
        for level in &registry.quant_levels {
            assert!(
                level.bits_per_weight > 0.0,
                "{} has a non-positive bit width",
                level.id
            );
        }
    }

    #[test]
    fn malformed_json_degrades_to_an_empty_registry_rather_than_panicking() {
        let result: ModelRegistry = serde_json::from_str("{ not json").unwrap_or_default();
        assert_eq!(result, ModelRegistry::default());
    }

    #[test]
    fn registry_round_trips_through_json() {
        let registry = seeded_registry();
        let encoded = serde_json::to_string(&registry).unwrap();
        let decoded: ModelRegistry = serde_json::from_str(&encoded).unwrap();
        assert_eq!(registry, decoded);
    }

    #[test]
    fn every_download_names_a_quantization_the_registry_defines() {
        // A quantId nobody prices is a download button on a cell that does not
        // exist. The two halves of the registry have to agree.
        let registry = seeded_registry();
        let known: Vec<&str> = registry
            .quant_levels
            .iter()
            .map(|q| q.id.as_str())
            .collect();

        for model in &registry.models {
            for download in &model.downloads {
                assert!(
                    known.contains(&download.quant_id.as_str()),
                    "{} pins unknown quantization {}",
                    model.id,
                    download.quant_id
                );
            }
        }
    }

    // The case-sensitive comparison is the point: a pin names one exact file in
    // one exact repository, and `.GGUF` would not be that file. Clippy's
    // case-insensitive suggestion would loosen the very thing being asserted.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    #[test]
    fn a_pinned_download_is_complete_or_absent() {
        // A half-filled entry is worse than none: it produces a button that
        // cannot verify what it fetched.
        for model in &seeded_registry().models {
            for download in &model.downloads {
                assert_eq!(download.sha256.len(), 64, "{} has no usable hash", model.id);
                assert!(download.sha256.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(download.size_bytes > 0, "{} has no size", model.id);
                assert!(!download.repo.is_empty() && !download.file.is_empty());
                assert!(!download.publisher.is_empty(), "provenance must be visible");
                assert!(download.file.ends_with(".gguf"));
            }
        }
    }

    #[test]
    fn at_least_one_model_is_actually_downloadable() {
        // Guards against the manifest silently emptying: the whole feature
        // renders as "not pinned" everywhere and looks like a bug.
        let count: usize = seeded_registry()
            .models
            .iter()
            .map(|m| m.downloads.len())
            .sum();
        assert!(count >= 5, "only {count} models are pinned");
    }

    #[test]
    fn no_two_downloads_claim_the_same_cell() {
        for model in &seeded_registry().models {
            let mut seen: Vec<&str> = Vec::new();
            for download in &model.downloads {
                assert!(
                    !seen.contains(&download.quant_id.as_str()),
                    "{} pins {} twice",
                    model.id,
                    download.quant_id
                );
                seen.push(&download.quant_id);
            }
        }
    }

    #[test]
    fn registry_serialises_with_camel_case_keys() {
        let registry = seeded_registry();
        let json = serde_json::to_value(&registry).unwrap();
        let object = json.as_object().unwrap();
        assert!(object.contains_key("quantLevels"));

        let model = json["models"][0].as_object().unwrap();
        assert!(model.contains_key("parametersBillion"));
        assert!(model.contains_key("sourceNote"));

        let architecture = json["models"][0]["architecture"].as_object().unwrap();
        assert!(architecture.contains_key("numKvHeads"));
        assert!(architecture.contains_key("maxContextLength"));
    }
}
