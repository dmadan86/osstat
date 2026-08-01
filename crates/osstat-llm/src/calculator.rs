//! The runnability calculator: ADR-008's arithmetic as pure functions.
//!
//! ```text
//! required_memory ≈ model_file_size × 1.1 + kv_cache(context_length, layers, heads)
//! ```
//!
//! Every function here takes plain numbers or the registry's own types and
//! returns plain numbers or a verdict — nothing here reads a clock, a file or
//! a device. That is deliberate, not incidental: ROADMAP.md's M4 gate is
//! "the runnability calculator has 100% branch coverage", which is only
//! achievable at all because there is no OS call anywhere in this module to
//! make a branch untestable.
//!
//! # Showing the arithmetic
//!
//! The roadmap requires the explanation drawer to show the working, not just
//! a verdict — "a verdict of 'won't fit' with no shown working is the failure
//! mode this feature exists to avoid". [`Breakdown`] exists so the UI never
//! has to recompute a single term: every number the drawer displays is a
//! field this module already produced while deciding the verdict.
//!
//! # The three states VRAM data can be in
//!
//! [`GpuBudget`] is the IPC-facing, `ts-rs`-exported shape (a flat struct, to
//! match this crate's other exported types). Internally [`GpuBudget::memory`]
//! turns it into [`GpuMemory`], which makes the three real states impossible
//! to confuse:
//!
//! - [`GpuMemory::Known`] — NVML, or Apple unified memory: a real number.
//! - [`GpuMemory::Unknown`] — a GPU is present (`wgpu` found an adapter) but
//!   its memory cannot be read; see the `wgpu` row in `probe`'s module docs.
//! - [`GpuMemory::Absent`] — no GPU at all, the ordinary state of a CI runner.
//!
//! `Unknown` and `Absent` are handled identically below: neither is a number
//! the calculator can compare against, so both fall back to the
//! system-memory-only verdict. That fallback is what "hardware probing
//! degrades gracefully with no GPU present" means in code — a machine with no
//! GPU is not an error state, it is the input that produces
//! [`VerdictKind::FitsOnCpuOnly`] or [`VerdictKind::WontFit`] and nothing
//! else.

use osstat_core::GpuDevice;
use serde::{Deserialize, Serialize};

use crate::registry::{ModelArchitecture, ModelEntry, ModelRegistry, QuantLevel};

/// The `× 1.1` in ADR-008's formula, as a divisor: 10% of the quantized
/// weight size, for tensors quantizers commonly leave at higher precision
/// (embeddings, layer norms) plus file-format and runtime overhead.
///
/// Integer division of `/10` is deliberately coarse. This is an estimate, and
/// implying more precision than the formula actually has would be its own
/// small dishonesty.
const fn weight_overhead_bytes(quantized_weight_bytes: u64) -> u64 {
    quantized_weight_bytes / 10
}

/// Bytes per KV-cache element. llama.cpp defaults the KV cache to fp16
/// regardless of how the weights themselves are quantized, so this does not
/// vary with [`QuantLevel`].
const KV_CACHE_BYTES_PER_ELEMENT: u64 = 2;

/// How much VRAM is available to weigh a model against, as it crosses IPC.
///
/// A flat struct rather than a tagged enum, matching every other `ts-rs`
/// export in this codebase: `present: false` is "no GPU", `present: true`
/// with `vram_bytes: None` is "a GPU is present but its memory is unknown",
/// and `present: true` with a value is a real figure. [`GpuBudget::memory`]
/// turns this into the three-way [`GpuMemory`] the calculator actually
/// reasons over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct GpuBudget {
    /// Whether any GPU was found at all.
    pub present: bool,
    /// Its VRAM in bytes, where the source could measure or confidently
    /// derive it. `None` while `present` is `true` means a GPU exists but
    /// `wgpu` could not report its memory — see the module docs.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub vram_bytes: Option<u64>,
}

impl GpuBudget {
    /// No GPU was found.
    #[must_use]
    pub const fn absent() -> Self {
        Self {
            present: false,
            vram_bytes: None,
        }
    }

    /// The three-way state this struct actually represents, collapsing the
    /// otherwise-representable-but-meaningless case of `present: false` with
    /// a `vram_bytes` figure into `Absent` — a caller cannot construct a
    /// state the calculator would need a fourth branch for.
    const fn memory(self) -> GpuMemory {
        if !self.present {
            GpuMemory::Absent
        } else if let Some(bytes) = self.vram_bytes {
            GpuMemory::Known(bytes)
        } else {
            GpuMemory::Unknown
        }
    }
}

/// The three states VRAM data can honestly be in. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GpuMemory {
    /// A real figure: NVML, or Apple unified memory.
    Known(u64),
    /// A GPU is present but `wgpu` cannot report its memory.
    Unknown,
    /// No GPU was found.
    Absent,
}

/// The three verdict shapes ADR-008 names: fits in VRAM, fits with CPU
/// offload, or will not run — plus the CPU-only case a machine with no GPU
/// produces, which the ADR groups under "fits with offload" in prose but
/// which this calculator gives its own tag because there is no GPU to
/// offload *from*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum VerdictKind {
    /// Weights, overhead and KV cache all fit inside VRAM alone.
    FitsOnGpu,
    /// Fits across VRAM and system memory together; some layers run on the
    /// CPU. See [`Verdict::gpu_layers`] and [`Verdict::cpu_layers`].
    FitsWithCpuOffload,
    /// No GPU with known memory to weigh against; fits in system memory.
    FitsOnCpuOnly,
    /// Exceeds every memory this machine can offer, GPU and system combined.
    WontFit,
}

/// A rough tokens-per-second tier — a classification, not a measurement.
///
/// Real throughput depends on the runtime, the quantization implementation,
/// memory bandwidth and thermal behaviour; ADR-008 is explicit that
/// presenting a heuristic as a measurement is the most damaging thing this
/// feature could do. This tier exists only so the UI can say "slower" rather
/// than a number nobody actually measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum SpeedTier {
    /// Entirely on the GPU.
    Fast,
    /// Mostly on the GPU, some layers offloaded.
    Moderate,
    /// Mostly or entirely on the CPU.
    Slow,
}

/// The verdict for one model at one quantization level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    /// Which of the four shapes this is.
    pub kind: VerdictKind,
    /// Layers estimated to stay resident on the GPU. Always `num_layers` for
    /// [`VerdictKind::FitsOnGpu`] and `0` for anything with no usable GPU.
    pub gpu_layers: u32,
    /// Layers estimated to run on the CPU instead. `gpu_layers + cpu_layers`
    /// always equals the model's layer count.
    pub cpu_layers: u32,
    /// A rough speed classification — see [`SpeedTier`].
    pub tier: SpeedTier,
}

/// Every term the arithmetic produced, for the explanation drawer.
///
/// ROADMAP.md M4 requires the drawer to show "the arithmetic ... rather than
/// hidden" — this struct is that arithmetic, held rather than recomputed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    /// The model's weights at the chosen quantization: `parameters ×
    /// bits_per_weight / 8`.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub quantized_weight_bytes: u64,
    /// The formula's `× 1.1` headroom, shown as its own term rather than
    /// folded silently into the weight figure.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub overhead_bytes: u64,
    /// `kv_cache(context_length, layers, heads)` from the ADR's formula.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub kv_cache_bytes: u64,
    /// The sum of the three terms above — what must fit somewhere.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub total_required_bytes: u64,
    /// VRAM this verdict was weighed against; `0` when no GPU memory figure
    /// was available (see [`GpuMemory`]).
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub available_vram_bytes: u64,
    /// System memory this verdict was weighed against.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub available_system_memory_bytes: u64,
    /// The context length the KV-cache term was computed at.
    pub context_length: u32,
}

/// One model, at one quantization, weighed against one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct FitResult {
    /// The model this row describes.
    pub model_id: String,
    /// The quantization level this row describes.
    pub quant_id: String,
    /// Whether — and how — it fits.
    pub verdict: Verdict,
    /// The arithmetic behind the verdict, for the explanation drawer.
    pub breakdown: Breakdown,
}

/// The raw quantized weight size: `parameters × bits_per_weight / 8`.
///
/// Negative, infinite or `NaN` inputs cannot occur from registry data — the
/// schema and this crate's tests both enforce positive values — but the cast
/// below is total regardless: Rust saturates a float-to-integer cast rather
/// than panicking (negative and `NaN` both land on `0`, infinity on
/// `u64::MAX`), so a pure function stays a pure function even fed adversarial
/// input.
#[must_use]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "float-to-u64 casts in Rust saturate rather than panic or produce \
              undefined behaviour; the registry's schema keeps both inputs positive \
              in practice, and the cast is total either way"
)]
pub fn quantized_weight_bytes(parameters_billion: f64, bits_per_weight: f64) -> u64 {
    (parameters_billion * 1_000_000_000.0 * bits_per_weight / 8.0) as u64
}

/// `kv_cache(context_length, layers, heads)` from ADR-008's formula: two
/// tensors (K and V), one fp16 element each, across every layer, every KV
/// head's dimension, and every token of context.
#[must_use]
pub fn kv_cache_bytes(context_length: u32, architecture: &ModelArchitecture) -> u64 {
    let per_token = u64::from(architecture.num_layers)
        .saturating_mul(2)
        .saturating_mul(u64::from(architecture.num_kv_heads))
        .saturating_mul(u64::from(architecture.head_dim))
        .saturating_mul(KV_CACHE_BYTES_PER_ELEMENT);

    per_token.saturating_mul(u64::from(context_length))
}

/// The weight bytes attributable to one transformer layer, assuming they are
/// distributed evenly — an approximation stated plainly here because the
/// offload estimate downstream inherits it: real models keep embeddings and
/// the output head outside the per-layer blocks, so this slightly overstates
/// each layer's true share on models with a large vocabulary.
///
/// A model with zero layers — which the registry schema forbids but this
/// function does not get to assume — yields `0` rather than dividing by
/// zero.
#[must_use]
pub fn per_layer_weight_bytes(quantized_weight_bytes: u64, num_layers: u32) -> u64 {
    if num_layers == 0 {
        0
    } else {
        quantized_weight_bytes / u64::from(num_layers)
    }
}

/// Picks the GPU budget to weigh a fit matrix against from the devices the
/// hardware probe found.
///
/// The largest known VRAM figure wins when more than one device reports one;
/// a device whose memory `wgpu` could not read (see `probe`'s module docs)
/// is present but contributes no number, which is exactly [`GpuMemory::Unknown`]
/// once no device offers a figure at all.
#[must_use]
pub fn select_gpu_budget(devices: &[GpuDevice]) -> GpuBudget {
    if devices.is_empty() {
        return GpuBudget::absent();
    }

    let best_known = devices.iter().filter_map(|device| device.vram_total).max();
    GpuBudget {
        present: true,
        vram_bytes: best_known,
    }
}

/// Weighs one model, at one quantization, against one machine's memory.
///
/// This is the function ROADMAP.md's M4 gate means by "the runnability
/// calculator": every branch below is enumerated and covered in the
/// `tests` module, and nothing here touches the OS.
#[must_use]
pub fn evaluate(
    model: &ModelEntry,
    quant: &QuantLevel,
    context_length: u32,
    gpu: GpuBudget,
    system_memory_bytes: u64,
) -> FitResult {
    let num_layers = model.architecture.num_layers;
    let quantized = quantized_weight_bytes(model.parameters_billion, quant.bits_per_weight);
    let overhead = weight_overhead_bytes(quantized);
    let kv_cache = kv_cache_bytes(context_length, &model.architecture);
    let total_required = quantized.saturating_add(overhead).saturating_add(kv_cache);
    let per_layer = per_layer_weight_bytes(quantized, num_layers);

    let (verdict, available_vram_bytes) = match gpu.memory() {
        GpuMemory::Known(vram) => (
            evaluate_against_gpu(
                total_required,
                vram,
                system_memory_bytes,
                per_layer,
                num_layers,
            ),
            vram,
        ),
        // Neither an unmeasured GPU nor no GPU at all gives the calculator a
        // number to compare VRAM against, so both take the same fallback:
        // the "degrade gracefully with no GPU present" path the M4 gate
        // requires.
        GpuMemory::Unknown | GpuMemory::Absent => (
            evaluate_cpu_only(total_required, system_memory_bytes, num_layers),
            0,
        ),
    };

    FitResult {
        model_id: model.id.clone(),
        quant_id: quant.id.clone(),
        verdict,
        breakdown: Breakdown {
            quantized_weight_bytes: quantized,
            overhead_bytes: overhead,
            kv_cache_bytes: kv_cache,
            total_required_bytes: total_required,
            available_vram_bytes,
            available_system_memory_bytes: system_memory_bytes,
            context_length,
        },
    }
}

/// The verdict when a real VRAM figure is available.
fn evaluate_against_gpu(
    total_required: u64,
    vram: u64,
    system_memory_bytes: u64,
    per_layer_bytes: u64,
    num_layers: u32,
) -> Verdict {
    if total_required <= vram {
        return Verdict {
            kind: VerdictKind::FitsOnGpu,
            gpu_layers: num_layers,
            cpu_layers: 0,
            tier: SpeedTier::Fast,
        };
    }

    let combined = vram.saturating_add(system_memory_bytes);
    if total_required > combined {
        return Verdict {
            kind: VerdictKind::WontFit,
            gpu_layers: 0,
            cpu_layers: num_layers,
            tier: SpeedTier::Slow,
        };
    }

    // Fits, but not in VRAM alone: estimate how many layers have to move to
    // system memory to close the gap.
    let excess = total_required.saturating_sub(vram);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the min() below clamps to num_layers, itself a u32"
    )]
    let cpu_layers = if per_layer_bytes == 0 {
        0
    } else {
        excess.div_ceil(per_layer_bytes).min(u64::from(num_layers)) as u32
    };
    let gpu_layers = num_layers.saturating_sub(cpu_layers);

    #[allow(
        clippy::cast_precision_loss,
        reason = "num_layers is a small integer count"
    )]
    let offloaded_fraction = if num_layers == 0 {
        1.0
    } else {
        f64::from(cpu_layers) / f64::from(num_layers)
    };
    let tier = if offloaded_fraction < 0.5 {
        SpeedTier::Moderate
    } else {
        SpeedTier::Slow
    };

    Verdict {
        kind: VerdictKind::FitsWithCpuOffload,
        gpu_layers,
        cpu_layers,
        tier,
    }
}

/// The verdict when there is no VRAM figure to weigh against at all — no
/// GPU, or one `wgpu` could not measure. This is the branch that makes a
/// headless CI runner produce a useful answer instead of an error.
fn evaluate_cpu_only(total_required: u64, system_memory_bytes: u64, num_layers: u32) -> Verdict {
    if total_required <= system_memory_bytes {
        Verdict {
            kind: VerdictKind::FitsOnCpuOnly,
            gpu_layers: 0,
            cpu_layers: num_layers,
            tier: SpeedTier::Slow,
        }
    } else {
        Verdict {
            kind: VerdictKind::WontFit,
            gpu_layers: 0,
            cpu_layers: num_layers,
            tier: SpeedTier::Slow,
        }
    }
}

/// Evaluates every model in the registry at every quantization level it
/// offers — the fit matrix a page renders as one table.
#[must_use]
pub fn compute_fit_matrix(
    registry: &ModelRegistry,
    context_length: u32,
    gpu: GpuBudget,
    system_memory_bytes: u64,
) -> Vec<FitResult> {
    registry
        .models
        .iter()
        .flat_map(|model| {
            registry
                .quant_levels
                .iter()
                .map(move |quant| evaluate(model, quant, context_length, gpu, system_memory_bytes))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use osstat_core::{GpuKind, GpuSource};

    use super::*;

    /// A small, deliberately round architecture so the arithmetic in tests is
    /// checkable by hand: 10 layers, 2 KV heads, 64-wide heads.
    fn architecture() -> ModelArchitecture {
        ModelArchitecture {
            num_layers: 10,
            hidden_size: 512,
            num_attention_heads: 8,
            num_kv_heads: 2,
            head_dim: 64,
            max_context_length: 8192,
        }
    }

    fn model() -> ModelEntry {
        ModelEntry {
            id: "test-model".into(),
            name: "Test Model".into(),
            family: "Test".into(),
            parameters_billion: 1.0,
            architecture: architecture(),
            source_note: "fixture, not a real model".into(),
        }
    }

    fn quant(bits_per_weight: f64) -> QuantLevel {
        QuantLevel {
            id: "TEST".into(),
            label: "TEST".into(),
            bits_per_weight,
            description: "fixture".into(),
        }
    }

    fn device(vram_total: Option<u64>) -> GpuDevice {
        GpuDevice {
            index: 0,
            name: "Fixture GPU".into(),
            vendor: None,
            backend: None,
            kind: GpuKind::Discrete,
            vram_total,
            shared_total: None,
            source: GpuSource::Wgpu,
            shared_source: None,
        }
    }

    // -- quantized_weight_bytes -------------------------------------------

    #[test]
    fn quantized_weight_bytes_matches_the_formula_by_hand() {
        // 1B params at 8 bits/weight = 1e9 bytes exactly.
        assert_eq!(quantized_weight_bytes(1.0, 8.0), 1_000_000_000);
    }

    #[test]
    fn zero_parameters_is_zero_bytes() {
        assert_eq!(quantized_weight_bytes(0.0, 16.0), 0);
    }

    #[test]
    fn a_fractional_bit_width_still_produces_a_sane_byte_count() {
        // 7B params at 4.85 bits/weight (Q4_K_M): roughly 4.24 GB.
        let bytes = quantized_weight_bytes(7.0, 4.85);
        assert!(bytes > 4_200_000_000 && bytes < 4_300_000_000);
    }

    #[test]
    fn adversarial_inputs_saturate_rather_than_panic() {
        assert_eq!(
            quantized_weight_bytes(-1.0, 8.0),
            0,
            "a negative size saturates to zero"
        );
        assert_eq!(
            quantized_weight_bytes(f64::NAN, 8.0),
            0,
            "NaN saturates to zero"
        );
        assert_eq!(
            quantized_weight_bytes(f64::INFINITY, 8.0),
            u64::MAX,
            "infinity saturates to the maximum representable size"
        );
    }

    // -- weight_overhead_bytes (private, exercised through evaluate/Breakdown) --

    #[test]
    fn overhead_is_one_tenth_of_the_weight_size() {
        let result = evaluate(&model(), &quant(8.0), 0, GpuBudget::absent(), u64::MAX);
        assert_eq!(
            result.breakdown.overhead_bytes,
            result.breakdown.quantized_weight_bytes / 10
        );
    }

    #[test]
    fn zero_weight_bytes_has_zero_overhead() {
        let result = evaluate(&model(), &quant(0.0), 0, GpuBudget::absent(), u64::MAX);
        assert_eq!(result.breakdown.overhead_bytes, 0);
    }

    // -- kv_cache_bytes -----------------------------------------------------

    #[test]
    fn kv_cache_matches_the_formula_by_hand() {
        // 10 layers * 2 (K and V) * 2 kv heads * 64 head_dim * 2 bytes/elem
        // * 100 tokens = 512,000 bytes.
        assert_eq!(kv_cache_bytes(100, &architecture()), 512_000);
    }

    #[test]
    fn zero_context_length_is_zero_kv_cache() {
        assert_eq!(kv_cache_bytes(0, &architecture()), 0);
    }

    #[test]
    fn kv_cache_saturates_rather_than_overflowing_on_extreme_inputs() {
        let huge = ModelArchitecture {
            num_layers: u32::MAX,
            hidden_size: 1,
            num_attention_heads: 1,
            num_kv_heads: u32::MAX,
            head_dim: u32::MAX,
            max_context_length: u32::MAX,
        };
        assert_eq!(kv_cache_bytes(u32::MAX, &huge), u64::MAX);
    }

    // -- per_layer_weight_bytes ---------------------------------------------

    #[test]
    fn per_layer_bytes_divides_evenly_across_layers() {
        assert_eq!(per_layer_weight_bytes(1000, 10), 100);
    }

    #[test]
    fn zero_layers_yields_zero_rather_than_dividing_by_zero() {
        assert_eq!(per_layer_weight_bytes(1000, 0), 0);
    }

    // -- select_gpu_budget ---------------------------------------------------

    #[test]
    fn no_devices_is_absent() {
        let budget = select_gpu_budget(&[]);
        assert_eq!(budget, GpuBudget::absent());
    }

    #[test]
    fn a_device_with_known_vram_is_reported() {
        let budget = select_gpu_budget(&[device(Some(8_000_000_000))]);
        assert_eq!(
            budget,
            GpuBudget {
                present: true,
                vram_bytes: Some(8_000_000_000)
            }
        );
    }

    #[test]
    fn the_largest_known_vram_wins_across_several_devices() {
        let budget =
            select_gpu_budget(&[device(Some(4_000_000_000)), device(Some(24_000_000_000))]);
        assert_eq!(budget.vram_bytes, Some(24_000_000_000));
    }

    #[test]
    fn a_present_gpu_with_no_known_vram_is_unknown_not_absent() {
        let budget = select_gpu_budget(&[device(None)]);
        assert_eq!(
            budget,
            GpuBudget {
                present: true,
                vram_bytes: None
            }
        );
        assert!(
            budget.present,
            "a card was found, even though its memory could not be read"
        );
    }

    // -- GpuBudget::memory (through evaluate's dispatch) ---------------------

    #[test]
    fn absent_and_unknown_reach_the_same_cpu_only_verdict() {
        let hardware_absent = evaluate(&model(), &quant(8.0), 0, GpuBudget::absent(), u64::MAX);
        let hardware_unknown = evaluate(
            &model(),
            &quant(8.0),
            0,
            GpuBudget {
                present: true,
                vram_bytes: None,
            },
            u64::MAX,
        );
        assert_eq!(hardware_absent.verdict.kind, VerdictKind::FitsOnCpuOnly);
        assert_eq!(hardware_unknown.verdict.kind, VerdictKind::FitsOnCpuOnly);
    }

    // -- evaluate: the full verdict matrix ------------------------------------

    #[test]
    fn fits_entirely_on_gpu_when_vram_covers_everything() {
        let result = evaluate(
            &model(),
            &quant(8.0),
            0,
            GpuBudget {
                present: true,
                vram_bytes: Some(u64::MAX),
            },
            0,
        );
        assert_eq!(result.verdict.kind, VerdictKind::FitsOnGpu);
        assert_eq!(result.verdict.gpu_layers, 10);
        assert_eq!(result.verdict.cpu_layers, 0);
        assert_eq!(result.verdict.tier, SpeedTier::Fast);
    }

    #[test]
    fn fits_with_cpu_offload_when_vram_alone_is_short_but_system_memory_covers_the_rest() {
        // 1B @ 8 bits = 1e9 bytes of weights, +10% overhead, +0 kv cache
        // (context 0) = 1.1e9 required. A little VRAM plus generous system
        // memory should produce a moderate offload.
        let result = evaluate(
            &model(),
            &quant(8.0),
            0,
            GpuBudget {
                present: true,
                vram_bytes: Some(900_000_000),
            },
            1_000_000_000,
        );
        assert_eq!(result.verdict.kind, VerdictKind::FitsWithCpuOffload);
        assert!(result.verdict.cpu_layers > 0);
        assert!(result.verdict.cpu_layers < 10);
        assert_eq!(result.verdict.gpu_layers + result.verdict.cpu_layers, 10);
    }

    #[test]
    fn a_zero_layer_model_offloads_without_dividing_by_zero() {
        // The registry's schema forbids a zero-layer model, but `evaluate` is
        // a pure function that does not get to assume its caller honoured the
        // schema — so both guards on the offload path (the per-layer share and
        // the offloaded fraction) have a zero case, and this is what reaches
        // them. 1B @ 8 bits = 1e9 of weights + 10% overhead = 1.1e9 required,
        // against 1e9 of VRAM: short of VRAM alone, comfortably inside VRAM
        // plus system memory, which is the offload branch.
        let mut zero_layer = model();
        zero_layer.architecture.num_layers = 0;

        let result = evaluate(
            &zero_layer,
            &quant(8.0),
            0,
            GpuBudget {
                present: true,
                vram_bytes: Some(1_000_000_000),
            },
            8_000_000_000,
        );

        assert_eq!(result.verdict.kind, VerdictKind::FitsWithCpuOffload);
        assert_eq!(result.verdict.cpu_layers, 0);
        assert_eq!(result.verdict.gpu_layers, 0);
        // A model with no layers has nothing resident on the GPU, so the
        // offloaded fraction is treated as total rather than as 0/0.
        assert_eq!(result.verdict.tier, SpeedTier::Slow);
    }

    #[test]
    fn offload_tier_is_moderate_under_half_and_slow_at_or_above_half() {
        // Required ~1.1e9 bytes. VRAM at 990,000,000 leaves ~110,000,000 to
        // move — under one layer's ~100,000,000-byte share once overhead is
        // included, well under half: moderate.
        let moderate = evaluate(
            &model(),
            &quant(8.0),
            0,
            GpuBudget {
                present: true,
                vram_bytes: Some(990_000_000),
            },
            1_000_000_000,
        );
        assert_eq!(moderate.verdict.tier, SpeedTier::Moderate);

        // VRAM at 100,000,000 leaves ~1,000,000,000 to move — nearly all ten
        // layers: slow.
        let slow = evaluate(
            &model(),
            &quant(8.0),
            0,
            GpuBudget {
                present: true,
                vram_bytes: Some(100_000_000),
            },
            1_100_000_000,
        );
        assert_eq!(slow.verdict.tier, SpeedTier::Slow);
    }

    #[test]
    fn wont_fit_when_gpu_and_system_memory_combined_are_still_short() {
        let result = evaluate(
            &model(),
            &quant(8.0),
            0,
            GpuBudget {
                present: true,
                vram_bytes: Some(1),
            },
            1,
        );
        assert_eq!(result.verdict.kind, VerdictKind::WontFit);
        assert_eq!(result.verdict.gpu_layers, 0);
        assert_eq!(result.verdict.cpu_layers, 10);
    }

    #[test]
    fn fits_on_cpu_only_with_no_gpu_when_system_memory_is_enough() {
        let result = evaluate(&model(), &quant(8.0), 0, GpuBudget::absent(), u64::MAX);
        assert_eq!(result.verdict.kind, VerdictKind::FitsOnCpuOnly);
        assert_eq!(result.verdict.gpu_layers, 0);
        assert_eq!(result.verdict.cpu_layers, 10);
        assert_eq!(result.verdict.tier, SpeedTier::Slow);
    }

    #[test]
    fn wont_fit_with_no_gpu_when_system_memory_alone_is_short() {
        // ROADMAP.md M4's other gate: "hardware probing degrades gracefully
        // with no GPU present" must still produce a verdict, not an error —
        // even when the honest verdict is no.
        let result = evaluate(&model(), &quant(16.0), 0, GpuBudget::absent(), 1);
        assert_eq!(result.verdict.kind, VerdictKind::WontFit);
        assert_eq!(result.verdict.gpu_layers, 0);
        assert_eq!(result.verdict.cpu_layers, 10);
    }

    #[test]
    fn the_breakdown_carries_every_term_the_drawer_needs() {
        let result = evaluate(
            &model(),
            &quant(8.0),
            4096,
            GpuBudget {
                present: true,
                vram_bytes: Some(2_000_000_000),
            },
            1_000_000_000,
        );
        let breakdown = result.breakdown;
        assert_eq!(
            breakdown.total_required_bytes,
            breakdown.quantized_weight_bytes + breakdown.overhead_bytes + breakdown.kv_cache_bytes
        );
        assert_eq!(breakdown.available_vram_bytes, 2_000_000_000);
        assert_eq!(breakdown.available_system_memory_bytes, 1_000_000_000);
        assert_eq!(breakdown.context_length, 4096);
    }

    #[test]
    fn fit_result_carries_the_model_and_quant_identity() {
        let result = evaluate(&model(), &quant(8.0), 0, GpuBudget::absent(), u64::MAX);
        assert_eq!(result.model_id, "test-model");
        assert_eq!(result.quant_id, "TEST");
    }

    // -- compute_fit_matrix ---------------------------------------------------

    #[test]
    fn the_fit_matrix_covers_every_model_times_every_quantization() {
        let registry = ModelRegistry {
            version: 1,
            quant_levels: vec![quant(4.0), quant(8.0), quant(16.0)],
            models: vec![model(), model()],
        };

        let matrix = compute_fit_matrix(&registry, 2048, GpuBudget::absent(), u64::MAX);

        assert_eq!(
            matrix.len(),
            registry.models.len() * registry.quant_levels.len()
        );
    }

    #[test]
    fn an_empty_registry_yields_an_empty_matrix_not_an_error() {
        let registry = ModelRegistry::default();
        let matrix = compute_fit_matrix(&registry, 2048, GpuBudget::absent(), u64::MAX);
        assert!(matrix.is_empty());
    }

    // -- serialisation shape --------------------------------------------------

    #[test]
    fn verdicts_serialise_with_camel_case_keys() {
        let result = evaluate(&model(), &quant(8.0), 0, GpuBudget::absent(), u64::MAX);
        let json = serde_json::to_value(result).unwrap();
        let object = json.as_object().unwrap();
        assert!(object.contains_key("modelId"));
        assert!(object.contains_key("quantId"));

        let verdict = json["verdict"].as_object().unwrap();
        assert!(verdict.contains_key("gpuLayers"));
        assert!(verdict.contains_key("cpuLayers"));

        let breakdown = json["breakdown"].as_object().unwrap();
        assert!(breakdown.contains_key("quantizedWeightBytes"));
        assert!(breakdown.contains_key("totalRequiredBytes"));
    }

    // -- shared memory isolation --------------------------------------------------

    #[test]
    fn shared_memory_never_counts_as_vram() {
        // Shared memory is system RAM reached across PCIe -- roughly 30-80 GB/s
        // against 400-1000 GB/s for on-card VRAM. A model that "fits" by
        // spilling into it does not fit in any sense the user means.
        //
        // The calculator already has the honest verdict for this: system memory
        // is weighed separately and the outcome is CpuOffload, with an estimate
        // of layers moved. Letting shared memory inflate vram_bytes would turn
        // that into a false "fits in VRAM" -- which ADR-008 names as the most
        // damaging thing this feature could do.
        let devices = vec![GpuDevice {
            vram_total: None,
            shared_total: Some(17_179_869_184),
            shared_source: Some(GpuSource::Dxgi),
            ..device(None)
        }];

        let budget = select_gpu_budget(&devices);

        assert_eq!(
            budget.vram_bytes, None,
            "a 16 GB shared pool is not 16 GB of VRAM"
        );
        assert!(
            budget.present,
            "the GPU is still there; only its VRAM is unknown"
        );
    }

    #[test]
    fn a_shared_pool_does_not_inflate_a_real_vram_figure() {
        let devices = vec![GpuDevice {
            vram_total: Some(8_589_934_592),
            shared_total: Some(17_179_869_184),
            shared_source: Some(GpuSource::Dxgi),
            ..device(Some(8_589_934_592))
        }];

        let budget = select_gpu_budget(&devices);

        assert_eq!(budget.vram_bytes, Some(8_589_934_592));
    }
}
