//! Naming a pinned model file, and where it is fetched from.
//!
//! A model is identified by the cell it occupies in the advisor's fit matrix —
//! a model and a quantization — because that is what the user pointed at. The
//! same model at two quantizations is two different files of very different
//! sizes, and conflating them would let one download overwrite another.
//!
//! The host is a constant here rather than a field in `models.json`. A per-model
//! URL would let one row point somewhere else entirely: a wider trust surface,
//! widened in a file whose diffs are read for hashes rather than hostnames.

use serde::{Deserialize, Serialize};

/// Where every pinned model file is fetched from.
///
/// Hugging Face serves `/resolve/main/` as a redirect to its CDN, which is why
/// the client that fetches these must follow redirects.
const HUGGING_FACE: &str = "https://huggingface.co";

/// Which cell of the fit matrix a file belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelKey {
    /// The model id from `osstat-llm`'s registry, e.g. `qwen2.5-7b`.
    pub model_id: String,
    /// The quantization id, e.g. `Q4_K_M`.
    pub quant_id: String,
}

/// Where a pinned file lives on Hugging Face.
///
/// `repo` is `owner/name`; `file` is the name within it. Both come from the
/// pinned registry, so neither is user input.
#[must_use]
pub fn download_url(repo: &str, file: &str) -> String {
    format!("{HUGGING_FACE}/{repo}/resolve/main/{file}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_url_is_derived_and_cannot_vary_by_host() {
        // Storing a full URL per entry would let the host differ per model --
        // a wider trust surface, widened somewhere nobody would look.
        assert_eq!(
            download_url("bartowski/Qwen2.5-7B-Instruct-GGUF", "a.gguf"),
            "https://huggingface.co/bartowski/Qwen2.5-7B-Instruct-GGUF/resolve/main/a.gguf"
        );
    }

    #[test]
    fn a_key_distinguishes_two_quantizations_of_one_model() {
        let small = ModelKey {
            model_id: "qwen2.5-7b".to_owned(),
            quant_id: "Q4_K_M".to_owned(),
        };
        let large = ModelKey {
            model_id: "qwen2.5-7b".to_owned(),
            quant_id: "Q8_0".to_owned(),
        };

        assert_ne!(
            small, large,
            "a key ignoring the quantization would let one download overwrite another"
        );
    }
}
