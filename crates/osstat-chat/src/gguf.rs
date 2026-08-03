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

/// Why a header could not be read out of a prefix of a file.
///
/// The distinction is the whole point of [`parse_prefix`]. A caller reads a
/// prefix because a model file is gigabytes, and it has no way to know in
/// advance how long the header is — the tokenizer vocabulary alone is several
/// megabytes for a current model. Told only "this did not parse", the caller
/// must either give up on a good file or keep reading a bad one. Told which of
/// the two happened, it can grow the read exactly when growing can help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GgufNeed {
    /// The header ran off the end of the prefix. A longer prefix may parse.
    NeedMoreBytes,
    /// These bytes are not a header this crate can read, and no quantity more
    /// of them would change that.
    Malformed,
}

/// A read that either produced a value or said why it could not.
type Need<T> = Result<T, GgufNeed>;

/// A bounds-checked cursor over the header bytes.
///
/// Every method distinguishes the two failures: running past the end of the
/// slice is [`GgufNeed::NeedMoreBytes`], and anything the bytes themselves get
/// wrong is [`GgufNeed::Malformed`].
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Need<&'a [u8]> {
        // An offset that overflows came from a declared length no file could
        // ever satisfy, so it is the file being wrong rather than short.
        let end = self.at.checked_add(count).ok_or(GgufNeed::Malformed)?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(GgufNeed::NeedMoreBytes)?;
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Need<u32> {
        let bytes = self.take(4)?;
        bytes
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| GgufNeed::Malformed)
    }

    fn u64(&mut self) -> Need<u64> {
        let bytes = self.take(8)?;
        bytes
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| GgufNeed::Malformed)
    }

    fn string(&mut self) -> Need<String> {
        let length = usize::try_from(self.u64()?).map_err(|_| GgufNeed::Malformed)?;
        let bytes = self.take(length)?.to_vec();
        String::from_utf8(bytes).map_err(|_| GgufNeed::Malformed)
    }

    /// Skips a value of the given type without interpreting it.
    ///
    /// Most of a GGUF header is metadata this crate has no use for —
    /// tokenizer vocabularies especially, which are large arrays. Skipping
    /// still has to walk them, because the next key's position depends on
    /// this value's length.
    fn skip_value(&mut self, kind: u32) -> Need<()> {
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
                Ok(())
            }
            // A type tag GGUF does not define. Reading on would be guessing at
            // the width of a value, and every following offset would be wrong.
            _ => Err(GgufNeed::Malformed),
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
    parse_prefix(bytes).map_err(|_| ChatError::NotAGguf {
        file: std::path::PathBuf::new(),
        reason: "the header is truncated, malformed, or missing a required field",
    })
}

/// Parses a GGUF header out of a prefix of a file, saying which way it failed.
///
/// This is [`parse`] with the one distinction its caller needs kept: whether a
/// longer prefix could succeed. A caller reading a multi-gigabyte file cannot
/// read it whole and cannot know the header's length in advance, so it reads a
/// prefix and grows it — and it can only do that safely if a file that will
/// never parse says so on the first read.
///
/// # Errors
///
/// [`GgufNeed::NeedMoreBytes`] if the header runs past the end of `bytes`, and
/// [`GgufNeed::Malformed`] for a bad magic number, an undefined value type, a
/// declared length that could not fit any file, a key that is not UTF-8, or a
/// complete metadata block missing a field the launch arithmetic requires.
///
/// One case is deliberately [`GgufNeed::NeedMoreBytes`] though it can never be
/// satisfied: a corrupt count that claims more pairs or tensors than the file
/// holds is indistinguishable from a header that simply continues past the
/// prefix. The caller's ceiling is what bounds that, not this.
pub fn parse_prefix(bytes: &[u8]) -> Need<ModelFile> {
    let mut reader = Reader::new(bytes);

    if reader.u32()? != MAGIC {
        return Err(GgufNeed::Malformed);
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

    // Every pair the file declared has been read by now, so a field that is
    // still missing is missing from the file rather than from the prefix.
    let missing = || GgufNeed::Malformed;

    let architecture = text("general.architecture").ok_or_else(missing)?;
    let key = |suffix: &str| format!("{architecture}.{suffix}");

    let block_count = unsigned(&key("block_count")).ok_or_else(missing)?;
    let context_length = unsigned(&key("context_length")).ok_or_else(missing)?;
    let embedding_length = unsigned(&key("embedding_length")).ok_or_else(missing)?;
    let head_count = unsigned(&key("attention.head_count")).ok_or_else(missing)?;
    // Models without grouped-query attention omit head_count_kv; it equals the
    // query head count there.
    let head_count_kv = unsigned(&key("attention.head_count_kv")).unwrap_or(head_count);

    let declared = unsigned(&key("attention.key_length"));
    let head_dim = match declared {
        Some(value) => value,
        // A file declaring no attention heads is a file that cannot describe a
        // transformer, not a file that is short.
        None => embedding_length
            .checked_div(head_count)
            .ok_or_else(missing)?,
    };

    let mut parameters = 0_u64;
    for _ in 0..tensor_count {
        let _name = reader.string()?;
        let dimensions = reader.u32()?;
        let mut product = 1_u64;
        for _ in 0..dimensions {
            product = product
                .checked_mul(reader.u64()?)
                .ok_or(GgufNeed::Malformed)?;
        }
        let _kind = reader.u32()?;
        let _offset = reader.u64()?;
        parameters = parameters.checked_add(product).ok_or(GgufNeed::Malformed)?;
    }

    Ok(ModelFile {
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
    fn a_prefix_that_stops_mid_header_asks_for_more_rather_than_condemning_the_file() {
        // The distinction the adaptive read in `chat.rs` is built on. Every one
        // of these cuts is a good file read too early, and answering
        // `Malformed` for any of them would tell the caller to give up on a
        // model that is perfectly fine.
        let full = complete()
            .tensor("token_embd.weight", &[4096, 32_000])
            .build();

        for cut in [4, 12, 20, full.len() / 2, full.len() - 1] {
            assert_eq!(
                parse_prefix(&full[..cut]),
                Err(GgufNeed::NeedMoreBytes),
                "a header cut at {cut} bytes should have asked for more"
            );
        }

        assert!(parse_prefix(&full).is_ok(), "the whole header should parse");
    }

    #[test]
    fn a_bad_magic_is_malformed_however_few_bytes_there_are() {
        // The case that must never grow a read. A file whose first four bytes
        // are not "GGUF" is not a GGUF at any length, and treating it as a
        // short read would turn a wrong file into a 64 MiB one.
        assert_eq!(
            parse_prefix(b"not a model at all"),
            Err(GgufNeed::Malformed)
        );

        // Even where the rest of the header is impeccable.
        let mut bytes = complete().build();
        bytes[..4].copy_from_slice(&0xDEAD_BEEF_u32.to_le_bytes());
        assert_eq!(parse_prefix(&bytes), Err(GgufNeed::Malformed));
    }

    #[test]
    fn a_complete_header_missing_a_required_field_is_malformed_rather_than_short() {
        // Every declared pair was read, so the field is absent from the file.
        // Asking for more bytes here would read the weights looking for a key
        // that the metadata block has already finished without.
        let bytes = Builder::new()
            .kv_string("general.architecture", "llama")
            .kv_u32("llama.context_length", 8192)
            .build();

        assert_eq!(parse_prefix(&bytes), Err(GgufNeed::Malformed));
    }

    #[test]
    fn an_undefined_value_type_is_malformed() {
        // GGUF defines types 0..=12. Anything else means the width of the
        // value is unknown, so every offset after it would be a guess.
        let mut bytes = Builder::new().kv_u32("llama.block_count", 32).build();
        let kind_at = bytes.len() - 8;
        bytes[kind_at..kind_at + 4].copy_from_slice(&99_u32.to_le_bytes());

        assert_eq!(parse_prefix(&bytes), Err(GgufNeed::Malformed));
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
