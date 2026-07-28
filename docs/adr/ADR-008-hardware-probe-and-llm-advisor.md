# ADR-008: Hardware probe and LLM runnability advisor

**Status:** Accepted

## Context

"Can my machine run this model?" is a question people currently answer by
downloading a 20 GB file and finding out. Answering it accurately requires
knowing VRAM, system RAM and CPU, and knowing how a given model and quantization
consume them.

## Decision

**GPU and VRAM detection**, in order of preference:

1. NVIDIA via `nvml-wrapper` — exact VRAM, including free and used
2. AMD, Intel and Apple via `wgpu` adapter enumeration — name plus memory heuristics
3. macOS unified memory reported _as_ unified memory, with VRAM derived from
   system RAM

**Model registry:** a versioned JSON file, updatable independently of releases,
listing popular open models with parameter counts, file sizes per quantization
(Q4_K_M, Q5, Q8, FP16) and minimum context memory footprint.

**Runnability arithmetic**, documented in-app rather than hidden:

```
required_memory ≈ model_file_size × 1.1 + kv_cache(context_length, layers, heads)
```

Each model × quantization is classified as: **fits in VRAM**, **fits with CPU
offload** (with an estimate of layers offloaded and a rough tokens-per-second
tier), or **will not run**.

The advisor also recommends runtimes (llama.cpp, Ollama, LM Studio) and detects
whether Ollama is installed, offering a "pull this model" deep link when it is.

## Rationale

Showing the arithmetic is the point. A verdict a user cannot check is a verdict
they cannot trust, and this one depends on assumptions (context length, offload
behaviour) that vary by runtime. An expandable explanation turns a black box into
something a user can argue with — and correct.

The registry as a separate JSON file means new models can ship without a release,
and it is a low-barrier contribution surface for the community.

## Consequences

- **These are estimates and must be labelled as such.** Real throughput depends
  on the runtime, quantization implementation, memory bandwidth and thermal
  behaviour. Presenting a heuristic as a measurement would be the most damaging
  thing this feature could do to the project's credibility.
- The calculator is pure functions with no I/O, so it can be tested
  exhaustively. The M4 gate requires 100% branch coverage on it.
- Probing must degrade gracefully when there is no GPU at all. CI runners have
  none, which makes that path continuously tested rather than an afterthought.
- `wgpu` is a heavy dependency for what is ultimately adapter enumeration. Its
  effect on the ~20 MB installer budget should be measured when M4 lands, and a
  lighter platform-specific probe considered if it proves significant.
