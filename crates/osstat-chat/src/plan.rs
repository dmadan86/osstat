//! Choosing what to launch a model with, from the file and this machine.
//!
//! The advisor cannot supply these figures. `calculator::evaluate` takes a
//! context length rather than producing one, and it needs a registry entry — so
//! a model the registry has never heard of would have no verdict and no layer
//! count. Reading `-ngl` off the fit matrix would work for the fifteen models
//! in the registry and silently not for anything else.
//!
//! So the arithmetic runs over the file instead: its real size on disk, and the
//! architecture its own header declares, against the VRAM the ADR-008 probe
//! measured. The calculator's terms are reused rather than reimplemented, so
//! there is one formula in the codebase and not two.

use crate::gguf::ModelFile;
use osstat_llm::calculator::{GpuBudget, kv_cache_bytes, per_layer_weight_bytes};
use osstat_llm::registry::ModelArchitecture;

/// The context length osstat asks for when a model would allow more.
///
/// Models increasingly declare 128k or more. Allocating that much KV cache can
/// exhaust VRAM on a machine where the weights themselves fit comfortably, and
/// almost no chat session uses it. 8192 is generous for conversation and cheap.
pub const DEFAULT_CONTEXT_CEILING: u32 = 8192;

/// What to pass `llama-server` for one model on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchPlan {
    /// Layers to offload to the GPU — `-ngl`.
    pub gpu_layers: u32,
    /// Context window to allocate — `-c`.
    pub context_length: u32,
    /// Whether the model is expected to fit in GPU and system memory together.
    ///
    /// `false` is a warning the UI shows, never a refusal: the arithmetic is an
    /// estimate, and refusing on an estimate would make osstat wrong in a way
    /// the user cannot override.
    pub fits: bool,
}

/// Chooses `-ngl` and `-c` for a model file on this machine.
///
/// `file_size` is the model's actual size on disk, which is a better figure
/// than the fit matrix has: the matrix derives a size from a parameter count
/// and a bits-per-weight average, while this is the number the loader will
/// actually read.
#[must_use]
pub fn plan_launch(model: &ModelFile, file_size: u64, gpu: GpuBudget) -> LaunchPlan {
    let architecture = architecture_of(model);

    // A GPU whose memory could not be measured is treated as no GPU. Guessing
    // a budget would invent the figure the probe deliberately refuses to.
    let budget = if gpu.present {
        gpu.vram_bytes.unwrap_or(0)
    } else {
        0
    };

    let context_length = fit_context(model, &architecture, budget);
    let kv_cache = kv_cache_bytes(context_length, &architecture);

    let gpu_layers = if model.block_count == 0 {
        0
    } else {
        largest_offload(file_size, model.block_count, budget, kv_cache)
    };

    LaunchPlan {
        gpu_layers,
        context_length,
        fits: gpu_layers == model.block_count,
    }
}

/// Builds the calculator's architecture type from the file's own header.
fn architecture_of(model: &ModelFile) -> ModelArchitecture {
    ModelArchitecture {
        num_layers: model.block_count,
        hidden_size: model.embedding_length,
        num_attention_heads: model.head_count,
        num_kv_heads: model.head_count_kv,
        head_dim: model.head_dim,
        max_context_length: model.context_length,
    }
}

/// The largest context that fits its own KV cache, capped by the model.
fn fit_context(model: &ModelFile, architecture: &ModelArchitecture, budget: u64) -> u32 {
    let mut context = model.context_length.min(DEFAULT_CONTEXT_CEILING);

    // With no GPU budget the KV cache lives in system memory, which this
    // function does not police -- the cap is all that applies.
    if budget == 0 {
        return context;
    }

    while context > 512 && kv_cache_bytes(context, architecture) >= budget {
        context /= 2;
    }

    context
}

/// The most layers whose weights plus the KV cache fit the budget.
fn largest_offload(file_size: u64, block_count: u32, budget: u64, kv_cache: u64) -> u32 {
    let Some(remaining) = budget.checked_sub(kv_cache) else {
        return 0;
    };

    let per_layer = per_layer_weight_bytes(file_size, block_count);
    if per_layer == 0 {
        return 0;
    }

    let affordable = remaining / per_layer;
    u32::try_from(affordable)
        .unwrap_or(u32::MAX)
        .min(block_count)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A 7B-shaped model: 32 layers, GQA with 8 KV heads, 8k native context.
    fn model() -> ModelFile {
        ModelFile {
            architecture: "llama".to_owned(),
            block_count: 32,
            context_length: 8192,
            embedding_length: 4096,
            head_count: 32,
            head_count_kv: 8,
            head_dim: 128,
            head_dim_derived: false,
            file_type: Some(15),
            parameters: 7_000_000_000,
        }
    }

    const FOUR_GB: u64 = 4 * 1024 * 1024 * 1024;
    const TWELVE_GB: u64 = 12 * 1024 * 1024 * 1024;

    fn gpu(vram: u64) -> GpuBudget {
        GpuBudget {
            present: true,
            vram_bytes: Some(vram),
        }
    }

    #[test]
    fn a_model_that_fits_comfortably_gets_every_layer() {
        let plan = plan_launch(&model(), FOUR_GB, gpu(TWELVE_GB));

        assert_eq!(plan.gpu_layers, 32);
        assert!(plan.fits);
    }

    #[test]
    fn a_model_larger_than_vram_gets_a_partial_offload() {
        // The case the whole function exists for: some layers on the GPU, the
        // rest on the CPU, rather than refusing or pretending it all fits.
        let plan = plan_launch(&model(), TWELVE_GB, gpu(FOUR_GB));

        assert!(plan.gpu_layers > 0, "nothing was offloaded at all");
        assert!(plan.gpu_layers < 32, "the whole model cannot fit in 4 GB");
    }

    #[test]
    fn no_gpu_means_no_offloaded_layers() {
        let plan = plan_launch(&model(), FOUR_GB, GpuBudget::absent());

        assert_eq!(plan.gpu_layers, 0);
    }

    #[test]
    fn a_gpu_whose_memory_is_unknown_offloads_nothing() {
        // present: true with vram_bytes: None means wgpu found a card but could
        // not size it. Guessing a budget would be inventing the number the
        // whole probe refuses to invent.
        let plan = plan_launch(
            &model(),
            FOUR_GB,
            GpuBudget {
                present: true,
                vram_bytes: None,
            },
        );

        assert_eq!(plan.gpu_layers, 0);
    }

    #[test]
    fn context_never_exceeds_what_the_model_declares() {
        // Asking for more context than the model was trained for is not a
        // performance trade, it is a broken model.
        let small = ModelFile {
            context_length: 2048,
            ..model()
        };

        let plan = plan_launch(&small, FOUR_GB, gpu(TWELVE_GB));

        assert_eq!(plan.context_length, 2048);
    }

    #[test]
    fn context_never_exceeds_the_default_ceiling() {
        let long = ModelFile {
            context_length: 131_072,
            ..model()
        };

        let plan = plan_launch(&long, FOUR_GB, gpu(TWELVE_GB));

        assert_eq!(plan.context_length, DEFAULT_CONTEXT_CEILING);
    }

    #[test]
    fn a_context_whose_kv_cache_alone_will_not_fit_is_reduced() {
        // A 128k context on a small card can exhaust VRAM before a single
        // weight is loaded. A context that cannot fit its own KV cache is not
        // a context.
        let long = ModelFile {
            context_length: 131_072,
            ..model()
        };
        let tiny = 512 * 1024 * 1024;

        let plan = plan_launch(&long, FOUR_GB, gpu(tiny));

        assert!(
            plan.context_length < DEFAULT_CONTEXT_CEILING,
            "context was not reduced to fit a {tiny}-byte budget"
        );
        assert!(plan.context_length > 0, "context collapsed to nothing");
    }

    #[test]
    fn a_model_that_cannot_fit_at_all_says_so_rather_than_refusing() {
        // fits: false is a warning the UI shows. It is never a refusal --
        // the arithmetic is an estimate, and refusing on an estimate would make
        // osstat wrong in a way the user cannot override.
        let huge = ModelFile {
            block_count: 126,
            ..model()
        };

        let plan = plan_launch(&huge, 400 * 1024 * 1024 * 1024, gpu(FOUR_GB));

        assert!(!plan.fits);
        assert_eq!(plan.gpu_layers, 0, "nothing fits, so nothing is offloaded");
    }

    #[test]
    fn a_header_declaring_no_layers_does_not_divide_by_zero() {
        let broken = ModelFile {
            block_count: 0,
            ..model()
        };

        let plan = plan_launch(&broken, FOUR_GB, gpu(TWELVE_GB));

        assert_eq!(plan.gpu_layers, 0);
    }
}
