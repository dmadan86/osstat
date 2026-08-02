//! Reading a GGUF file's header.
//!
//! Only the header is read — a model file is gigabytes and nothing here needs
//! the weights. What it yields is the architecture the launch arithmetic in
//! [`crate::plan`] needs, taken from the file rather than from a registry
//! estimate.
//!
//! Every read is bounds-checked and every failure is total. A partial parse
//! would produce a confident layer count for the wrong model, and the `-ngl`
//! computed from it would be wrong in a way nothing downstream could detect.

use crate::ChatError;

/// A model file's header, as the file declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFile {
    /// `general.architecture`, e.g. `llama`.
    pub architecture: String,
    /// Transformer block count. The upper bound on `-ngl`.
    pub block_count: u32,
    /// The model's native maximum context. The cap on `-c`.
    pub context_length: u32,
    /// Model dimension.
    pub embedding_length: u32,
    /// Query head count.
    pub head_count: u32,
    /// Key/value head count.
    pub head_count_kv: u32,
    /// Per-head dimension. The KV cache scales with this.
    pub head_dim: u32,
    /// Whether `head_dim` was derived rather than declared — see
    /// [`ModelFile::head_dim`] and the risk noted in the design.
    pub head_dim_derived: bool,
    /// `general.file_type`, the quantization tag, where the file declares one.
    pub file_type: Option<u32>,
    /// Parameter count, summed from the tensor shapes.
    pub parameters: u64,
}

/// GGUF's magic number, `"GGUF"` little-endian.
const MAGIC: u32 = 0x4655_4747;

/// A bounds-checked cursor over the header bytes.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let length = usize::try_from(self.u64()?).ok()?;
        String::from_utf8(self.take(length)?.to_vec()).ok()
    }

    /// Skips a value of the given type without interpreting it.
    ///
    /// Most of a GGUF header is metadata this crate has no use for —
    /// tokenizer vocabularies especially, which are large arrays. Skipping
    /// still has to walk them, because the next key's position depends on
    /// this value's length.
    fn skip_value(&mut self, kind: u32) -> Option<()> {
        match kind {
            0 | 1 | 7 => self.take(1).map(|_| ()),
            2 | 3 => self.take(2).map(|_| ()),
            4..=6 => self.take(4).map(|_| ()),
            10..=12 => self.take(8).map(|_| ()),
            8 => self.string().map(|_| ()),
            9 => {
                let element = self.u32()?;
                let count = self.u64()?;
                for _ in 0..count {
                    self.skip_value(element)?;
                }
                Some(())
            }
            _ => None,
        }
    }
}

/// One metadata value, in the two shapes this crate reads.
enum Value {
    Unsigned(u32),
    Text(String),
}

/// Parses a GGUF header.
///
/// # Errors
///
/// [`ChatError::NotAGguf`] for anything that is not a readable GGUF header,
/// including a truncation, a bad magic, a declared count that overruns the
/// buffer, and a header missing a field the launch arithmetic requires. The
/// `file` field is filled by the caller, which knows the path.
pub fn parse(bytes: &[u8]) -> Result<ModelFile, ChatError> {
    read(bytes).ok_or(ChatError::NotAGguf {
        file: std::path::PathBuf::new(),
        reason: "the header is truncated, malformed, or missing a required field",
    })
}

/// The parse proper, as an `Option` so every bounds check is a `?`.
fn read(bytes: &[u8]) -> Option<ModelFile> {
    let mut reader = Reader::new(bytes);

    if reader.u32()? != MAGIC {
        return None;
    }
    let _version = reader.u32()?;
    let tensor_count = reader.u64()?;
    let kv_count = reader.u64()?;

    let mut values: Vec<(String, Value)> = Vec::new();
    for _ in 0..kv_count {
        let key = reader.string()?;
        let kind = reader.u32()?;
        match kind {
            4 => {
                let value = reader.u32()?;
                values.push((key, Value::Unsigned(value)));
            }
            8 => {
                let value = reader.string()?;
                values.push((key, Value::Text(value)));
            }
            other => reader.skip_value(other)?,
        }
    }

    let text = |name: &str| {
        values.iter().find_map(|(key, value)| match value {
            Value::Text(text) if key == name => Some(text.clone()),
            _ => None,
        })
    };
    let unsigned = |name: &str| {
        values.iter().find_map(|(key, value)| match value {
            Value::Unsigned(number) if key == name => Some(*number),
            _ => None,
        })
    };

    let architecture = text("general.architecture")?;
    let key = |suffix: &str| format!("{architecture}.{suffix}");

    let block_count = unsigned(&key("block_count"))?;
    let context_length = unsigned(&key("context_length"))?;
    let embedding_length = unsigned(&key("embedding_length"))?;
    let head_count = unsigned(&key("attention.head_count"))?;
    // Models without grouped-query attention omit head_count_kv; it equals the
    // query head count there.
    let head_count_kv = unsigned(&key("attention.head_count_kv")).unwrap_or(head_count);

    let declared = unsigned(&key("attention.key_length"));
    let head_dim = match declared {
        Some(value) => value,
        None => embedding_length.checked_div(head_count)?,
    };

    let mut parameters = 0_u64;
    for _ in 0..tensor_count {
        let _name = reader.string()?;
        let dimensions = reader.u32()?;
        let mut product = 1_u64;
        for _ in 0..dimensions {
            product = product.checked_mul(reader.u64()?)?;
        }
        let _kind = reader.u32()?;
        let _offset = reader.u64()?;
        parameters = parameters.checked_add(product)?;
    }

    Some(ModelFile {
        architecture,
        block_count,
        context_length,
        embedding_length,
        head_count,
        head_count_kv,
        head_dim,
        head_dim_derived: declared.is_none(),
        file_type: unsigned("general.file_type"),
        parameters,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Builds a GGUF header byte by byte.
    ///
    /// A real model file is gigabytes; every test here needs only the header,
    /// so the fixture is constructed rather than checked in. That also means a
    /// malformed case can be built precisely, which a captured file cannot.
    struct Builder {
        bytes: Vec<u8>,
        kv_count: u64,
        tensor_count: u64,
        kv: Vec<u8>,
        tensors: Vec<u8>,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                kv_count: 0,
                tensor_count: 0,
                kv: Vec::new(),
                tensors: Vec::new(),
            }
        }

        fn string(target: &mut Vec<u8>, value: &str) {
            target.extend_from_slice(&(value.len() as u64).to_le_bytes());
            target.extend_from_slice(value.as_bytes());
        }

        fn kv_string(mut self, key: &str, value: &str) -> Self {
            Self::string(&mut self.kv, key);
            self.kv.extend_from_slice(&8_u32.to_le_bytes());
            Self::string(&mut self.kv, value);
            self.kv_count += 1;
            self
        }

        fn kv_u32(mut self, key: &str, value: u32) -> Self {
            Self::string(&mut self.kv, key);
            self.kv.extend_from_slice(&4_u32.to_le_bytes());
            self.kv.extend_from_slice(&value.to_le_bytes());
            self.kv_count += 1;
            self
        }

        /// One tensor with the given dimensions, contributing their product to
        /// the parameter count.
        fn tensor(mut self, name: &str, dims: &[u64]) -> Self {
            Self::string(&mut self.tensors, name);
            let dimension_count = u32::try_from(dims.len()).unwrap();
            self.tensors
                .extend_from_slice(&dimension_count.to_le_bytes());
            for dim in dims {
                self.tensors.extend_from_slice(&dim.to_le_bytes());
            }
            self.tensors.extend_from_slice(&0_u32.to_le_bytes()); // type
            self.tensors.extend_from_slice(&0_u64.to_le_bytes()); // offset
            self.tensor_count += 1;
            self
        }

        fn build(mut self) -> Vec<u8> {
            self.bytes.extend_from_slice(&0x4655_4747_u32.to_le_bytes());
            self.bytes.extend_from_slice(&3_u32.to_le_bytes());
            self.bytes
                .extend_from_slice(&self.tensor_count.to_le_bytes());
            self.bytes.extend_from_slice(&self.kv_count.to_le_bytes());
            self.bytes.extend_from_slice(&self.kv);
            self.bytes.extend_from_slice(&self.tensors);
            self.bytes
        }
    }

    /// A header with everything `plan` needs, and nothing more.
    fn complete() -> Builder {
        Builder::new()
            .kv_string("general.architecture", "llama")
            .kv_u32("llama.block_count", 32)
            .kv_u32("llama.context_length", 8192)
            .kv_u32("llama.embedding_length", 4096)
            .kv_u32("llama.attention.head_count", 32)
            .kv_u32("llama.attention.head_count_kv", 8)
            .kv_u32("llama.attention.key_length", 128)
            .kv_u32("general.file_type", 15)
    }

    #[test]
    fn a_complete_header_parses() {
        let model = parse(&complete().build()).unwrap();

        assert_eq!(model.architecture, "llama");
        assert_eq!(model.block_count, 32);
        assert_eq!(model.context_length, 8192);
        assert_eq!(model.head_count_kv, 8);
        assert_eq!(model.head_dim, 128);
        assert!(!model.head_dim_derived);
        assert_eq!(model.file_type, Some(15));
    }

    #[test]
    fn a_missing_key_length_is_derived_and_says_so() {
        // Not every architecture writes attention.key_length. The fallback is
        // correct for standard attention and wrong for models that diverge, so
        // which route was taken has to be recoverable -- a mis-sized KV cache
        // is otherwise a mystery rather than a diagnosis.
        let mut builder = Builder::new()
            .kv_string("general.architecture", "llama")
            .kv_u32("llama.block_count", 32)
            .kv_u32("llama.context_length", 8192)
            .kv_u32("llama.embedding_length", 4096)
            .kv_u32("llama.attention.head_count", 32)
            .kv_u32("llama.attention.head_count_kv", 8);
        builder = builder.kv_u32("general.file_type", 15);

        let model = parse(&builder.build()).unwrap();

        assert_eq!(model.head_dim, 4096 / 32);
        assert!(model.head_dim_derived);
    }

    #[test]
    fn parameters_come_from_the_tensor_shapes() {
        let bytes = complete()
            .tensor("token_embd.weight", &[4096, 32_000])
            .tensor("blk.0.attn_q.weight", &[4096, 4096])
            .build();

        let model = parse(&bytes).unwrap();

        assert_eq!(model.parameters, 4096 * 32_000 + 4096 * 4096);
    }

    #[test]
    fn a_file_that_is_not_gguf_is_refused() {
        assert!(matches!(
            parse(b"not a model at all"),
            Err(ChatError::NotAGguf { .. })
        ));
    }

    #[test]
    fn a_truncated_header_is_refused_rather_than_half_read() {
        // A partial parse would produce a confident layer count for the wrong
        // model, and -ngl computed from it would be silently wrong. Same rule
        // parse_luid_instance follows on the GPU side.
        let full = complete().build();

        for cut in [4, 12, 20, full.len() / 2, full.len() - 1] {
            assert!(
                matches!(parse(&full[..cut]), Err(ChatError::NotAGguf { .. })),
                "a header cut at {cut} bytes should not have parsed"
            );
        }
    }

    #[test]
    fn a_kv_count_that_overruns_the_buffer_is_refused() {
        // A hostile or corrupt file can claim more pairs than it contains.
        // Reading on would walk off the end of the slice.
        let mut bytes = complete().build();
        bytes[12..20].copy_from_slice(&u64::MAX.to_le_bytes());

        assert!(matches!(parse(&bytes), Err(ChatError::NotAGguf { .. })));
    }

    #[test]
    fn a_header_missing_the_layer_count_is_refused() {
        // block_count is the one field -ngl cannot be computed without.
        let bytes = Builder::new()
            .kv_string("general.architecture", "llama")
            .kv_u32("llama.context_length", 8192)
            .build();

        assert!(matches!(parse(&bytes), Err(ChatError::NotAGguf { .. })));
    }
}
