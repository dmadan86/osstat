//! Reading a GGUF header out of a file that is still on a server.
//!
//! A GGUF header sits at the *start* of the file, and HTTP has had a way to ask
//! for the start of a file since 1999. So the architecture a model declares —
//! layer count, head counts, context length — can be read before a single
//! gigabyte of weights is fetched, which means a search result can be priced by
//! exactly the arithmetic a downloaded one is priced by.
//!
//! This is the network twin of the growing read `src-tauri/src/chat.rs` does
//! over a local file, and it is deliberately the same shape:
//!
//! - it starts modest, because most headers are small;
//! - it grows only when [`crate::parse_prefix`] says more bytes could help,
//!   and stops at once when the bytes are not a GGUF at all;
//! - it stops growing at [`MAX_HEADER_FETCH`], the same ceiling the local read
//!   uses.
//!
//! # Why the ceiling is load-bearing here and not merely tidy
//!
//! Over a local file the ceiling saves time. Over the network it is the only
//! thing standing between osstat and a 30 GB download. A server or CDN is free
//! to ignore `Range` and answer `200` with the whole body, and plenty do. That
//! case is detected from the status rather than hoped against: anything other
//! than `206` is treated as the whole file arriving, read to the ceiling and no
//! further, and the connection is then dropped mid-body. If the header has not
//! appeared by then the result is reported as unpriced, which is the honest
//! answer and costs 64 MiB rather than the file.
//!
//! # What this does not do
//!
//! It does not verify anything. The bytes are read to learn a shape, never to
//! be executed or kept, and nothing here writes to disk. Verification belongs
//! to the download, which checks a SHA256 over the whole file — see
//! `osstat_inference::download`.

use futures_util::StreamExt as _;

use crate::gguf::{GgufNeed, ModelFile};

/// How much of the file the first request asks for.
///
/// Larger than the local read's first megabyte on purpose: a round trip costs
/// far more here than a page of memory does, and 4 MiB covers the header of
/// every model shipping today in one request. The local read can afford to be
/// stingier because growing costs it nothing but a `read`.
const FIRST_HEADER_FETCH: u64 = 4 * 1024 * 1024;

/// The most of a file that will ever be pulled looking for a header.
///
/// The same 64 MiB `src-tauri/src/chat.rs` stops its local read at, and for the
/// same reason: a tokenizer vocabulary runs to several megabytes — Qwen2.5
/// declares about 152k tokens — so no single fixed size is right, but a header
/// several times larger than anything published is a file this cannot price.
///
/// One constant, two readers. If a model ever ships a header larger than this,
/// the local read and this one must fail together rather than one of them
/// quietly pricing a model the other refuses.
pub const MAX_HEADER_FETCH: u64 = 64 * 1024 * 1024;

/// Why a remote header could not be read.
///
/// Separate from [`crate::ChatError`] because every variant of that one names a
/// [`std::path::PathBuf`], and there is no path here — the file has not been
/// downloaded and may never be. Squeezing a URL into `NotAGguf`'s `file` field
/// would put a repository name into a type whose `Display` the chat surface
/// already shows.
#[derive(Debug, thiserror::Error)]
pub enum RemoteHeaderError {
    /// The host could not be reached, or the body stopped part-way.
    #[error("the file could not be reached")]
    Network(#[source] reqwest::Error),

    /// The host answered, refusing.
    #[error("the host answered {status}")]
    HttpStatus {
        /// The status code, which is the whole of what was said.
        status: u16,
    },

    /// Bytes arrived and are not a GGUF header.
    #[error("that file is not a readable GGUF: {reason}")]
    NotAGguf {
        /// Which invariant it broke, for the message shown to the user.
        reason: &'static str,
    },

    /// The header had not finished within [`MAX_HEADER_FETCH`] bytes.
    ///
    /// The case a server ignoring `Range` lands in, and the reason the ceiling
    /// exists. Never a reason to read further.
    #[error("the header did not appear in the first {read_bytes} bytes of the file")]
    HeaderTooLarge {
        /// How much was read before giving up.
        read_bytes: u64,
    },
}

impl RemoteHeaderError {
    /// A stable, user-data-free name for this variant, safe to write to a log.
    ///
    /// Same contract as [`crate::ChatError::kind`] and
    /// `osstat_inference::AcquireError::kind`: a log line gets this and never
    /// [`Display`](std::fmt::Display), because what someone went looking for is
    /// the most obviously personal thing this app touches and it is half of the
    /// URL every request here is made against.
    ///
    /// The match has no wildcard arm on purpose. A new variant must fail to
    /// compile rather than quietly logging as something it is not.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Network(_) => "network",
            Self::HttpStatus { .. } => "http_status",
            Self::NotAGguf { .. } => "not_a_gguf",
            Self::HeaderTooLarge { .. } => "header_too_large",
        }
    }
}

/// How one response ended.
enum Filled {
    /// The header parsed out of what arrived.
    Parsed(ModelFile),
    /// These bytes are not a GGUF header, and no more of them would be.
    Malformed,
    /// The body ended before the ceiling, and the header is still unfinished.
    Ended,
    /// The ceiling was reached with the header still unfinished.
    Full,
}

/// Reads a GGUF header from `url` by asking for the start of the file.
///
/// Returns the architecture the file declares, which is the same value the
/// local read produces for the same bytes — so the same [`crate::plan_launch`]
/// prices both, and a searched model and a downloaded one cannot disagree.
///
/// # Errors
///
/// [`RemoteHeaderError::Network`] or [`RemoteHeaderError::HttpStatus`] if the
/// file could not be fetched, [`RemoteHeaderError::NotAGguf`] if what arrived is
/// not a header this crate reads, and [`RemoteHeaderError::HeaderTooLarge`] if
/// the header had not finished within [`MAX_HEADER_FETCH`] bytes — including the
/// case of a server that ignores `Range` and sends the whole file.
pub async fn fetch_header(
    client: &reqwest::Client,
    url: &str,
) -> Result<ModelFile, RemoteHeaderError> {
    let mut header: Vec<u8> = Vec::new();
    let mut limit = FIRST_HEADER_FETCH;

    loop {
        let held = u64::try_from(header.len()).unwrap_or(u64::MAX);
        // HTTP counts the last byte inclusively, and `limit` is a length. The
        // request is closed at both ends rather than left open: an open-ended
        // `bytes=N-` is what `download.rs` wants, because it means to read to
        // the end, and it is precisely what this must never ask for.
        let last = limit.saturating_sub(1);

        let response = client
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={held}-{last}"))
            .send()
            .await
            .map_err(RemoteHeaderError::Network)?;

        let status = response.status();
        if !status.is_success() {
            return Err(RemoteHeaderError::HttpStatus {
                status: status.as_u16(),
            });
        }

        // A `206` is the server agreeing to send the slice that was asked for.
        // Anything else is the whole file starting at byte zero, so what is
        // already held is not a prefix of what is arriving and has to go — the
        // same rule `download.rs` follows when a `200` answers a ranged
        // request, for the same reason: appending would produce bytes that are
        // simply wrong.
        let honoured = status == reqwest::StatusCode::PARTIAL_CONTENT;
        if !honoured {
            header.clear();
        }

        // The ignored-range case gets one pass at the ceiling rather than five
        // doubling requests that would each re-send the whole file.
        let ceiling = if honoured { limit } else { MAX_HEADER_FETCH };

        match fill(response, &mut header, ceiling).await? {
            Filled::Parsed(model) => return Ok(model),
            Filled::Malformed => {
                return Err(RemoteHeaderError::NotAGguf {
                    reason: "the header is malformed or missing a field the launch needs",
                });
            }
            // Fewer bytes than the ceiling arrived, so there are no more to be
            // had and the header does not fit inside its own file.
            Filled::Ended => {
                return Err(RemoteHeaderError::NotAGguf {
                    reason: "the header runs past the end of the file",
                });
            }
            Filled::Full if !honoured || limit >= MAX_HEADER_FETCH => {
                return Err(RemoteHeaderError::HeaderTooLarge {
                    read_bytes: ceiling,
                });
            }
            // The only branch that asks for more, and only because
            // `parse_prefix` said more could help.
            Filled::Full => limit = limit.saturating_mul(2).min(MAX_HEADER_FETCH),
        }
    }
}

/// Appends the body to `header` until it parses, ends, or reaches `ceiling`.
///
/// Parsing is attempted at each doubling boundary rather than after every chunk
/// — a parse walks the whole buffer, so doing it per 8 KiB chunk of a 64 MiB
/// read would be quadratic for no benefit. The boundaries are the same sizes
/// [`fetch_header`] would have requested, so a server that honours `Range` and
/// one that ignores it stop reading at the same points.
///
/// Nothing is appended past `ceiling`, and this returns the moment it is
/// reached rather than draining what is left. Returning drops `stream`, which
/// closes the connection — and that is what stops a server sending gigabytes it
/// was never asked for. Reading the body out politely first would download
/// precisely what the ceiling exists to avoid.
async fn fill(
    response: reqwest::Response,
    header: &mut Vec<u8>,
    ceiling: u64,
) -> Result<Filled, RemoteHeaderError> {
    let mut checkpoint = FIRST_HEADER_FETCH;
    let mut stream = response.bytes_stream();

    loop {
        let held = u64::try_from(header.len()).unwrap_or(u64::MAX);

        if held >= ceiling {
            return Ok(check(header).unwrap_or(Filled::Full));
        }

        if held >= checkpoint {
            if let Some(outcome) = check(header) {
                return Ok(outcome);
            }
            checkpoint = checkpoint.saturating_mul(2);
        }

        let Some(chunk) = stream.next().await else {
            return Ok(check(header).unwrap_or(Filled::Ended));
        };
        let chunk = chunk.map_err(RemoteHeaderError::Network)?;

        // Truncated rather than trusted. A server may send more than the range
        // it was asked for, and the ceiling has to hold whatever arrives.
        let room = usize::try_from(ceiling.saturating_sub(held)).unwrap_or(usize::MAX);
        header.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
}

/// Parses what is held, or says nothing if more bytes could still help.
///
/// The one place [`GgufNeed`] is turned into a decision, so "keep reading" has a
/// single definition rather than one per caller.
fn check(header: &[u8]) -> Option<Filled> {
    match crate::parse_prefix(header) {
        Ok(model) => Some(Filled::Parsed(model)),
        Err(GgufNeed::Malformed) => Some(Filled::Malformed),
        Err(GgufNeed::NeedMoreBytes) => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Builds a GGUF header byte by byte.
    ///
    /// The same fixture shape `gguf.rs` and `chat.rs` use. A real model file is
    /// gigabytes and every test here needs only the header, so it is
    /// constructed rather than checked in — which is also the only way to build
    /// a header of a chosen size.
    struct Builder {
        kv_count: u64,
        tensor_count: u64,
        kv: Vec<u8>,
        tensors: Vec<u8>,
    }

    impl Builder {
        fn new() -> Self {
            Self {
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

        fn tensor(mut self, name: &str, dims: &[u64]) -> Self {
            Self::string(&mut self.tensors, name);
            self.tensors
                .extend_from_slice(&u32::try_from(dims.len()).unwrap().to_le_bytes());
            for dim in dims {
                self.tensors.extend_from_slice(&dim.to_le_bytes());
            }
            self.tensors.extend_from_slice(&0_u32.to_le_bytes()); // type
            self.tensors.extend_from_slice(&0_u64.to_le_bytes()); // offset
            self.tensor_count += 1;
            self
        }

        fn build(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0x4655_4747_u32.to_le_bytes());
            bytes.extend_from_slice(&3_u32.to_le_bytes());
            bytes.extend_from_slice(&self.tensor_count.to_le_bytes());
            bytes.extend_from_slice(&self.kv_count.to_le_bytes());
            bytes.extend_from_slice(&self.kv);
            bytes.extend_from_slice(&self.tensors);
            bytes
        }
    }

    /// A header with everything `plan_launch` needs, and nothing more.
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

    /// A header whose metadata runs past the first request's 4 MiB.
    ///
    /// Not a contrivance: a current model's `tokenizer.ggml.tokens` is a
    /// multi-megabyte array, which is the whole reason the read grows at all.
    /// One long string stands in for it, because what is under test is the
    /// growth and not the vocabulary.
    fn oversized() -> Vec<u8> {
        let padding = "v".repeat(usize::try_from(FIRST_HEADER_FETCH).unwrap() + 512 * 1024);

        complete()
            .kv_string("tokenizer.ggml.model", &padding)
            .tensor("token_embd.weight", &[4096, 32_000])
            .build()
    }

    /// A model file: a header followed by something standing in for weights.
    fn with_weights(header: Vec<u8>, weight_bytes: usize) -> Vec<u8> {
        let mut file = header;
        file.extend(std::iter::repeat_n(0_u8, weight_bytes));
        file
    }

    /// How the fixture answers a `Range` request.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Answer {
        /// Honour it: `206`, with exactly the bytes asked for.
        Honour,
        /// Ignore it: `200` with the whole body, which is what a CDN with no
        /// range support does and the case the ceiling exists for.
        IgnoreRange,
        /// Refuse the request outright.
        Status(u16),
    }

    /// What the fixture served, for the assertions about how much was fetched.
    #[derive(Default)]
    struct Served {
        /// The `bytes=A-B` of each request, in order.
        ranges: Vec<(u64, u64)>,
        /// How many body bytes were written before the client hung up.
        written: usize,
    }

    /// A server on a loopback port that speaks `Range`.
    ///
    /// A real socket rather than a mocked client, for the same reason
    /// `download.rs` uses one: what is under test is how the request is built,
    /// what is done with the status that comes back, and — the point of the
    /// ceiling — when the connection is dropped. A mock would replace all three.
    struct Fixture {
        url: String,
        served: Arc<Mutex<Served>>,
    }

    impl Fixture {
        /// The `(first, last)` of every `Range` header that arrived.
        fn ranges(&self) -> Vec<(u64, u64)> {
            self.served.lock().unwrap().ranges.clone()
        }

        /// How many body bytes the server managed to write.
        fn written(&self) -> usize {
            self.served.lock().unwrap().written
        }
    }

    /// Serves `body` on a loopback port, answering each request in turn.
    ///
    /// `Connection: close` on every response so each request arrives on its own
    /// connection, the same arrangement `search.rs` uses to let one server
    /// answer a sequence of calls.
    fn serve(body: Vec<u8>, answers: Vec<Answer>) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = Arc::new(Mutex::new(Served::default()));
        let recorded = Arc::clone(&served);

        std::thread::spawn(move || {
            for (index, accepted) in listener.incoming().enumerate() {
                let Ok(mut stream) = accepted else { continue };

                // Drained before the response is written: replying to an unread
                // socket and closing it sends RST, which reaches the client as a
                // transport error rather than the body under test.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while let Ok(1) = stream.read(&mut byte) {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }

                let text = String::from_utf8_lossy(&request).into_owned();
                if let Some(range) = requested_range(&text)
                    && let Ok(mut held) = recorded.lock()
                {
                    held.ranges.push(range);
                }

                let answer = answers.get(index).copied().unwrap_or(Answer::Honour);
                let (header, slice) = response_for(answer, &body, requested_range(&text));

                let _ = stream.write_all(header.as_bytes());
                // Written in pieces so a client that hangs up part-way stops
                // this rather than every byte landing in one syscall — which is
                // what makes "the connection was dropped at the ceiling"
                // observable at all.
                let mut written = 0;
                for piece in slice.chunks(64 * 1024) {
                    if stream.write_all(piece).is_err() {
                        break;
                    }
                    written += piece.len();
                }
                if let Ok(mut held) = recorded.lock() {
                    held.written += written;
                }
            }
        });

        Fixture {
            url: format!("http://127.0.0.1:{port}/model.gguf"),
            served,
        }
    }

    /// Reads `A` and `B` out of a `Range: bytes=A-B` request header.
    fn requested_range(request: &str) -> Option<(u64, u64)> {
        let line = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("range:"))?;
        let spec = line.split_once('=')?.1.trim();
        let (first, last) = spec.split_once('-')?;

        Some((first.trim().parse().ok()?, last.trim().parse().ok()?))
    }

    /// The status line, headers and body slice one answer produces.
    fn response_for(answer: Answer, body: &[u8], range: Option<(u64, u64)>) -> (String, Vec<u8>) {
        match answer {
            Answer::Status(code) => (
                format!(
                    "HTTP/1.1 {code} Refused\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ),
                Vec::new(),
            ),
            Answer::IgnoreRange => (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ),
                body.to_vec(),
            ),
            Answer::Honour => {
                let (first, last) = range.unwrap_or((0, u64::MAX));
                let start = usize::try_from(first).unwrap_or(usize::MAX).min(body.len());
                let end = usize::try_from(last)
                    .unwrap_or(usize::MAX)
                    .saturating_add(1)
                    .min(body.len());
                let slice = body.get(start..end).unwrap_or_default().to_vec();

                (
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                        slice.len(),
                        start,
                        end.saturating_sub(1),
                        body.len()
                    ),
                    slice,
                )
            }
        }
    }

    #[tokio::test]
    async fn a_header_is_read_without_downloading_the_file() {
        // The premise of the whole module: the architecture is at the front of
        // the file, so it can be had for a few megabytes rather than a few
        // gigabytes. If this needed the weights the feature would not exist.
        let header = complete()
            .tensor("token_embd.weight", &[4096, 32_000])
            .build();
        let fixture = serve(with_weights(header, 8 * 1024 * 1024), vec![Answer::Honour]);

        let model = fetch_header(&reqwest::Client::new(), &fixture.url)
            .await
            .unwrap();

        assert_eq!(model.architecture, "llama");
        assert_eq!(model.block_count, 32);
        assert_eq!(model.context_length, 8192);
        assert_eq!(model.head_count_kv, 8);
        assert_eq!(model.head_dim, 128);
        assert_eq!(model.parameters, 4096 * 32_000);
    }

    #[tokio::test]
    async fn a_remote_header_reads_the_same_as_the_local_one() {
        // The property the single pricing path rests on. `plan_launch` cannot
        // tell where a `ModelFile` came from, so if these two ever disagreed a
        // searched model and the same file once downloaded would be priced
        // differently -- with nothing in either answer saying which was which.
        let header = complete()
            .tensor("token_embd.weight", &[4096, 32_000])
            .build();
        let fixture = serve(
            with_weights(header.clone(), 4 * 1024 * 1024),
            vec![Answer::Honour],
        );

        let remote = fetch_header(&reqwest::Client::new(), &fixture.url)
            .await
            .unwrap();
        let local = crate::parse(&header).unwrap();

        assert_eq!(remote, local);
    }

    #[tokio::test]
    async fn a_header_larger_than_the_first_request_grows_the_read() {
        // The reason `parse_prefix` distinguishes its two failures at all. A
        // real tokenizer vocabulary runs past 4 MiB, so a fetch that asked once
        // and gave up would refuse to price most of what a search returns.
        let fixture = serve(
            with_weights(oversized(), 2 * 1024 * 1024),
            vec![Answer::Honour, Answer::Honour, Answer::Honour],
        );

        let model = fetch_header(&reqwest::Client::new(), &fixture.url)
            .await
            .unwrap();

        assert_eq!(model.block_count, 32);
        assert_eq!(model.parameters, 4096 * 32_000);

        // Grown, and grown by asking for what was not already held rather than
        // by fetching the front of the file again.
        let ranges = fixture.ranges();
        assert!(
            ranges.len() > 1,
            "the read never grew: ranges were {ranges:?}"
        );
        assert_eq!(ranges[0], (0, FIRST_HEADER_FETCH - 1));
        assert_eq!(
            ranges[1].0, FIRST_HEADER_FETCH,
            "the second request re-fetched bytes already held: {ranges:?}"
        );
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_gguf_is_refused_on_the_first_request() {
        // The case that must never grow a read. Bytes that are not a header at
        // any length would otherwise turn a wrong file into 64 MiB of traffic,
        // sixteen times over.
        let fixture = serve(vec![b'x'; 6 * 1024 * 1024], vec![Answer::Honour]);

        let outcome = fetch_header(&reqwest::Client::new(), &fixture.url).await;

        assert!(matches!(outcome, Err(RemoteHeaderError::NotAGguf { .. })));
        assert_eq!(
            fixture.ranges().len(),
            1,
            "a file that is not a GGUF was asked for twice"
        );
    }

    #[tokio::test]
    async fn a_server_ignoring_range_is_cut_off_at_the_ceiling() {
        // A CDN with no range support answers 200 with the whole body. Reading
        // it out would download the model -- which is the one outcome this
        // module exists to prevent -- so the body is read to the ceiling and the
        // connection is dropped.
        let body_bytes = usize::try_from(MAX_HEADER_FETCH).unwrap() + 16 * 1024 * 1024;
        let fixture = serve(vec![b'x'; body_bytes], vec![Answer::IgnoreRange]);

        let outcome = fetch_header(&reqwest::Client::new(), &fixture.url).await;

        // `x` is not a magic number, so this is refused as soon as four bytes
        // have arrived -- the ceiling is the backstop, not the first line.
        assert!(matches!(outcome, Err(RemoteHeaderError::NotAGguf { .. })));
        assert!(
            fixture.written() < body_bytes,
            "the whole body was served: the client never hung up"
        );
    }

    #[tokio::test]
    async fn a_header_that_never_finishes_is_unpriced_rather_than_endless() {
        // A GGUF whose magic is right and whose metadata claims more pairs than
        // any header holds. It reads as "more bytes could help" at every length,
        // by design -- `parse_prefix` cannot tell that from a header that simply
        // continues -- so the ceiling is the only thing that ends this.
        let mut bytes = complete().build();
        bytes[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
        let padded = with_weights(bytes, usize::try_from(MAX_HEADER_FETCH).unwrap());

        let fixture = serve(padded, vec![Answer::IgnoreRange]);

        let outcome = fetch_header(&reqwest::Client::new(), &fixture.url).await;

        let stopped_at_the_ceiling = matches!(
            &outcome,
            Err(RemoteHeaderError::HeaderTooLarge { read_bytes }) if *read_bytes == MAX_HEADER_FETCH
        );
        assert!(
            stopped_at_the_ceiling,
            "the read did not stop at the ceiling: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_refusal_is_reported_by_status_rather_than_retried() {
        let fixture = serve(complete().build(), vec![Answer::Status(403)]);

        let outcome = fetch_header(&reqwest::Client::new(), &fixture.url).await;

        assert!(matches!(
            outcome,
            Err(RemoteHeaderError::HttpStatus { status: 403 })
        ));
        assert_eq!(fixture.ranges().len(), 1, "a refusal was asked again");
    }

    #[tokio::test]
    async fn a_file_shorter_than_its_own_header_is_refused() {
        // The header declares a tensor table the file stops before. There is
        // nothing more to fetch, so this must not grow the read.
        let truncated = {
            let full = complete()
                .tensor("token_embd.weight", &[4096, 32_000])
                .build();
            full[..full.len() - 8].to_vec()
        };
        let fixture = serve(truncated, vec![Answer::Honour, Answer::Honour]);

        let outcome = fetch_header(&reqwest::Client::new(), &fixture.url).await;

        assert!(matches!(outcome, Err(RemoteHeaderError::NotAGguf { .. })));
    }

    #[test]
    fn every_variant_has_a_distinct_kind() {
        // A copied-and-not-edited string makes two different failures look
        // identical in a log, which is worse than no log line.
        let kinds = [
            RemoteHeaderError::HttpStatus { status: 404 }.kind(),
            RemoteHeaderError::NotAGguf { reason: "" }.kind(),
            RemoteHeaderError::HeaderTooLarge { read_bytes: 0 }.kind(),
        ];

        let mut unique = kinds.to_vec();
        unique.sort_unstable();
        unique.dedup();

        assert_eq!(unique.len(), kinds.len());
    }

    #[test]
    fn no_message_names_the_file_it_failed_on() {
        // Display reaches the user, and kind() reaches the log. Neither may
        // carry a repository or a file name: what someone searched for is the
        // most personal thing this app touches, and it is half of every URL
        // this module builds.
        for error in [
            RemoteHeaderError::HttpStatus { status: 404 },
            RemoteHeaderError::NotAGguf {
                reason: "the header is malformed",
            },
            RemoteHeaderError::HeaderTooLarge { read_bytes: 64 },
        ] {
            let shown = error.to_string();
            assert!(
                !shown.contains("http"),
                "a URL reached the message: {shown}"
            );
            assert!(!shown.contains(".gguf"), "a file name reached it: {shown}");
        }
    }
}
