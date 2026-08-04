//! Talking to a running `llama-server` over its OpenAI-compatible endpoint.
//!
//! This is where ADR-012's "all egress is in Rust" is kept. The webview issues
//! no HTTP request of its own, so a compromised webview still cannot reach the
//! network — and the per-session API key never leaves this process.
//!
//! Server-sent events do not align with TCP writes: one frame can arrive in two
//! reads, and two frames can arrive in one. The buffer below exists for that,
//! and the test that splits a frame across writes is what keeps it honest.

use crate::ChatError;
use futures_util::StreamExt as _;

/// One turn in the conversation, in the shape the endpoint expects.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Message {
    /// `system`, `user` or `assistant`.
    pub role: String,
    /// The turn itself: bare text, or text with an image beside it.
    pub content: Content,
}

/// What one turn carries.
///
/// The OpenAI-compatible endpoint accepts a string or an array of typed parts,
/// and `llama-server` implements both. A text turn stays a string rather than
/// becoming a one-element array: that is the shape every server has always
/// accepted, and every stored conversation replayed as history goes through
/// here.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum Content {
    /// A turn that is only words.
    Text(String),
    /// A turn with an image in it.
    Parts(Vec<Part>),
}

/// One piece of a multi-part turn.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum Part {
    /// Words.
    #[serde(rename = "text")]
    Text {
        /// The words.
        text: String,
    },
    /// An image, as a `data:` URL.
    ///
    /// The field is named `image_url` and holds a `data:` URL, which reads
    /// oddly and is what the endpoint specifies: llama.cpp's server documents
    /// `image_url.url` as accepting a remote URL, a base64 payload, or a local
    /// path. osstat only ever sends the middle one — a URL would have the
    /// *server* fetch something, and a path would have it read the disk.
    #[serde(rename = "image_url")]
    Image {
        /// The `{ "url": "data:image/png;base64,…" }` object.
        image_url: ImageUrl,
    },
}

/// The object wrapping one image's data URL.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImageUrl {
    /// `data:image/png;base64,…`
    pub url: String,
}

impl Message {
    /// A turn of plain text.
    #[must_use]
    pub fn text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Content::Text(text.into()),
        }
    }

    /// The same turn with an image attached.
    ///
    /// The image goes first. Both orders work, and models trained on
    /// image-then-question handle it more reliably than the reverse.
    #[must_use]
    pub fn with_image(self, data_url: String) -> Self {
        let text = match self.content {
            Content::Text(text) => text,
            // Already multi-part. Nothing in this crate produces that today;
            // returning it unchanged is better than dropping either the parts
            // it has or the image it was just handed.
            Content::Parts(_) => return self,
        };

        Self {
            role: self.role,
            content: Content::Parts(vec![
                Part::Image {
                    image_url: ImageUrl { url: data_url },
                },
                Part::Text { text },
            ]),
        }
    }
}

/// Token counts for one exchange.
///
/// `Serialize` as well as `Deserialize`: these counts are read off the wire,
/// then stored with the message they belong to and shown in the UI.
/// Written in camelCase and read in either case. Everything crossing the IPC
/// boundary is camelCase (ADR-002), but `llama-server` writes `snake_case` on the
/// wire — so the alias accepts the server's spelling without making the webview
/// the one type that speaks differently from every other payload. The aliases
/// also mean conversations stored before this rename still load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Tokens in the prompt, including the whole conversation so far.
    #[serde(alias = "prompt_tokens")]
    pub prompt_tokens: u32,
    /// Tokens generated in reply.
    #[serde(alias = "completion_tokens")]
    pub completion_tokens: u32,
}

/// Generation speeds, as `llama-server` reports them.
///
/// Same two-spelling arrangement as [`Usage`], and for the same reason: these
/// figures are read off the wire and shown in the UI.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    /// Prompt processing rate. Absent until the prompt has been evaluated.
    #[serde(alias = "prompt_per_second")]
    pub prompt_per_second: Option<f64>,
    /// Generation rate. Absent until at least one token has been produced.
    #[serde(alias = "predicted_per_second")]
    pub predicted_per_second: Option<f64>,
}

/// Something that happened while a reply was streaming.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// More text.
    Delta(String),
    /// The reply finished, with whatever figures the server reported.
    Complete {
        /// Token counts, where the server sent them.
        usage: Option<Usage>,
        /// Speeds, where the server sent them.
        timings: Option<Timings>,
    },
}

/// A client bound to one running server.
#[derive(Debug, Clone)]
pub struct ChatClient {
    base: String,
    api_key: String,
    http: reqwest::Client,
}

/// The subset of a streamed chunk this crate reads.
#[derive(serde::Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    timings: Option<Timings>,
    #[serde(default)]
    error: Option<ServerError>,
}

#[derive(serde::Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
}

#[derive(serde::Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(serde::Deserialize)]
struct ServerError {
    message: String,
}

/// What the running server says about itself.
///
/// Both figures come from the server rather than from anything osstat decided.
/// The context window because the server may round or clamp what it was asked
/// for, and vision because the server is the only thing that knows whether a
/// projector actually loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerProps {
    /// The context window the server actually allocated.
    pub context_length: u32,
    /// Whether the server will accept images.
    pub vision: bool,
}

/// `/props`, narrowed to the fields this crate reads.
#[derive(serde::Deserialize)]
struct Props {
    default_generation_settings: GenerationSettings,
    /// Absent on an older `llama-server` build that predates the field.
    ///
    /// Defaulting to "no modalities" is the only safe reading. Absent must
    /// never mean "assume yes": an attach control offered against a server that
    /// cannot see would take the image, send it, and get back an answer about
    /// nothing — the user would have no way to tell it had been ignored.
    #[serde(default)]
    modalities: Modalities,
}

#[derive(serde::Deserialize, Default)]
struct Modalities {
    #[serde(default)]
    vision: bool,
}

#[derive(serde::Deserialize)]
struct GenerationSettings {
    n_ctx: u32,
}

impl ChatClient {
    /// Binds a client to `base`, e.g. `http://127.0.0.1:52413`.
    #[must_use]
    pub fn new(base: String, api_key: String) -> Self {
        Self {
            base,
            api_key,
            http: reqwest::Client::new(),
        }
    }

    /// What the server reports about itself.
    ///
    /// Read from the server rather than assumed. The context window because the
    /// server may round or clamp what it was asked for, and a context meter
    /// whose denominator is a request rather than a fact would mislead
    /// precisely when the window is nearly full.
    ///
    /// Vision for a stronger reason: it is the only authority there is. A
    /// model's name is not evidence — plenty of vision models are not named
    /// after it and plenty of text models are — and even a correct name says
    /// nothing about whether the projector this launch passed actually loaded.
    ///
    /// # Errors
    ///
    /// [`ChatError::BadChunk`] if the request fails or the response does not
    /// contain the context length.
    pub async fn props(&self) -> Result<ServerProps, ChatError> {
        let response = self
            .http
            .get(format!("{}/props", self.base))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|error| ChatError::BadChunk(error.to_string()))?;

        let props: Props = response
            .json()
            .await
            .map_err(|error| ChatError::BadChunk(error.to_string()))?;

        Ok(ServerProps {
            context_length: props.default_generation_settings.n_ctx,
            vision: props.modalities.vision,
        })
    }

    /// Streams one reply, calling `on_event` as each piece arrives.
    ///
    /// # Errors
    ///
    /// [`ChatError::StreamBroken`] if the stream ends without a completion,
    /// [`ChatError::BadChunk`] for a malformed frame or a server error object.
    pub async fn stream(
        &self,
        messages: Vec<Message>,
        mut on_event: impl FnMut(StreamEvent),
    ) -> Result<(), ChatError> {
        let body = serde_json::json!({
            "messages": messages,
            "stream": true,
            // Without this, timings arrive only on the final chunk and the
            // tokens/sec readout could not update while generating.
            "timings_per_token": true,
        });

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| ChatError::BadChunk(error.to_string()))?;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut finished = false;

        while let Some(piece) = stream.next().await {
            let piece = piece.map_err(|_| ChatError::StreamBroken)?;
            buffer.push_str(&String::from_utf8_lossy(&piece));

            // Frames are separated by a blank line. Anything after the last
            // separator is an incomplete frame and stays buffered.
            while let Some(end) = buffer.find("\n\n") {
                let frame = buffer[..end].to_owned();
                buffer.drain(..end + 2);

                let Some(payload) = frame.strip_prefix("data: ") else {
                    continue;
                };
                let payload = payload.trim();

                if payload == "[DONE]" {
                    finished = true;
                    continue;
                }

                let chunk: Chunk = serde_json::from_str(payload)
                    .map_err(|error| ChatError::BadChunk(error.to_string()))?;

                if let Some(error) = chunk.error {
                    return Err(ChatError::BadChunk(error.message));
                }

                if let Some(text) = chunk
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.content.clone())
                    && !text.is_empty()
                {
                    on_event(StreamEvent::Delta(text));
                }

                if chunk.usage.is_some() || chunk.timings.is_some() {
                    on_event(StreamEvent::Complete {
                        usage: chunk.usage,
                        timings: chunk.timings,
                    });
                }
            }
        }

        if finished {
            Ok(())
        } else {
            Err(ChatError::StreamBroken)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// Serves one request, writing `chunks` as separate flushed writes.
    ///
    /// Separate writes matter: a stream delivered as one buffer would pass
    /// even if the parser only ever handled a complete body, which is the
    /// failure this whole module has to survive.
    fn serve_sse(chunks: Vec<String>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // The request must be drained -- headers *and* body -- before
                // the response is written. Replying to a socket that still
                // holds unread bytes and then closing it sends RST, which
                // discards the response the client has not yet read. This is a
                // POST, so unlike `download.rs`'s GET fixture, stopping at the
                // blank line would leave the JSON body sitting unread.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while let Ok(1) = stream.read(&mut byte) {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }

                let headers = String::from_utf8_lossy(&request).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0_u8; length];
                let _ = stream.read_exact(&mut body);

                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                );
                for chunk in chunks {
                    let _ = write!(stream, "{:x}\r\n{chunk}\r\n", chunk.len());
                    let _ = stream.flush();
                }
                let _ = stream.write_all(b"0\r\n\r\n");
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });

        format!("http://127.0.0.1:{port}")
    }

    /// Serves one GET with `body` as its whole JSON response.
    ///
    /// Separate from [`serve_sse`] because `/props` is a plain request with a
    /// plain answer: no chunking, no streaming, and no request body to drain.
    fn serve_props(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Drained before replying for the same reason `serve_sse`
                // drains: closing a socket that still holds unread bytes sends
                // RST, which discards the response.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while let Ok(1) = stream.read(&mut byte) {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }

                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });

        format!("http://127.0.0.1:{port}")
    }

    /// Asks a stub server for its properties.
    fn properties(base: String) -> Result<ServerProps, ChatError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async { ChatClient::new(base, "test-key".to_owned()).props().await })
    }

    fn delta(text: &str) -> String {
        format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n")
    }

    fn ask(base: String) -> (Vec<StreamEvent>, Result<(), ChatError>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let mut events = Vec::new();
        let outcome = runtime.block_on(async {
            let client = ChatClient::new(base, "test-key".to_owned());
            client
                .stream(vec![Message::text("user", "hello")], |event| {
                    events.push(event);
                })
                .await
        });

        (events, outcome)
    }

    #[test]
    fn deltas_arrive_one_at_a_time() {
        let base = serve_sse(vec![
            delta("Hel"),
            delta("lo"),
            "data: [DONE]\n\n".to_owned(),
        ]);

        let (events, outcome) = ask(base);

        assert!(outcome.is_ok());
        let text: String = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Delta(part) => Some(part.as_str()),
                StreamEvent::Complete { .. } => None,
            })
            .collect();
        assert_eq!(text, "Hello");
    }

    #[test]
    fn a_delta_split_across_two_writes_is_reassembled() {
        // The load-bearing case: SSE frames do not align with TCP writes. A
        // parser that assumed one write per event would pass every other test
        // here and corrupt real output.
        let whole = delta("Hello");
        let (first, second) = whole.split_at(whole.len() / 2);

        let base = serve_sse(vec![
            first.to_owned(),
            second.to_owned(),
            "data: [DONE]\n\n".to_owned(),
        ]);

        let (events, outcome) = ask(base);

        assert!(outcome.is_ok());
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::Delta(text) if text == "Hello")),
            "a frame split across writes was not reassembled: {events:?}"
        );
    }

    #[test]
    fn usage_and_timings_reach_the_caller() {
        let base = serve_sse(vec![
            delta("hi"),
            "data: {\"choices\":[{\"delta\":{}}],\
             \"usage\":{\"prompt_tokens\":44,\"completion_tokens\":48},\
             \"timings\":{\"prompt_per_second\":32.3,\"predicted_per_second\":52.9}}\n\n"
                .to_owned(),
            "data: [DONE]\n\n".to_owned(),
        ]);

        let (events, outcome) = ask(base);

        assert!(outcome.is_ok());
        let completion = events
            .iter()
            .find_map(|event| match event {
                StreamEvent::Complete { usage, timings } => Some((*usage, *timings)),
                StreamEvent::Delta(_) => None,
            })
            .expect("no completion event");

        assert_eq!(completion.0.unwrap().prompt_tokens, 44);
        assert_eq!(completion.0.unwrap().completion_tokens, 48);
        assert_eq!(completion.1.unwrap().predicted_per_second, Some(52.9));
    }

    #[test]
    fn a_stream_that_stops_mid_message_is_an_error_not_a_short_answer() {
        // Silently returning the partial text as if it were the whole answer
        // is the worst outcome available: the user cannot tell.
        let base = serve_sse(vec![delta("Once upon a")]);

        let (events, outcome) = ask(base);

        assert!(
            matches!(outcome, Err(ChatError::StreamBroken)),
            "a truncated stream reported {outcome:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::Delta(_))),
            "the partial text should still have reached the caller"
        );
    }

    #[test]
    fn a_chunk_that_is_not_json_is_reported() {
        let base = serve_sse(vec![
            "data: {not json at all}\n\n".to_owned(),
            "data: [DONE]\n\n".to_owned(),
        ]);

        let (_, outcome) = ask(base);

        assert!(matches!(outcome, Err(ChatError::BadChunk(_))));
    }

    #[test]
    fn a_server_error_object_is_reported_rather_than_treated_as_a_delta() {
        let base = serve_sse(vec![
            "data: {\"error\":{\"message\":\"context is full\"}}\n\n".to_owned(),
            "data: [DONE]\n\n".to_owned(),
        ]);

        let (_, outcome) = ask(base);

        // Two `assert!`s rather than a `match` arm ending in `panic!`: the
        // workspace sets `clippy::panic = "warn"` and CI runs `-D warnings`,
        // so a `panic!` here fails the build even inside `#[cfg(test)]`. This
        // module's opt-out is exactly `unwrap_used` and `expect_used`.
        let reported = match &outcome {
            Err(ChatError::BadChunk(message)) => Some(message.as_str()),
            _ => None,
        };

        assert!(
            reported.is_some(),
            "expected the server error to surface, got {outcome:?}"
        );
        assert!(
            reported.is_some_and(|message| message.contains("context is full")),
            "the server's own words were lost: {outcome:?}"
        );
    }

    #[test]
    fn a_text_turn_stays_a_bare_string_on_the_wire() {
        // Not a one-element array. Every stored conversation replayed as
        // history goes through here, and a string is the shape every
        // OpenAI-compatible server has always accepted.
        let json = serde_json::to_value(Message::text("user", "hello")).unwrap();

        assert_eq!(json["content"], serde_json::json!("hello"));
    }

    #[test]
    fn an_image_turn_is_the_shape_the_endpoint_documents() {
        // Asserted against llama.cpp's server README rather than a summary of
        // it: content becomes an array of typed parts, and the image part is
        // `image_url` wrapping an object with a `url`. Getting the nesting
        // wrong produces a 400 the user reads as "the model refused".
        let message = Message::text("user", "what is this?")
            .with_image("data:image/png;base64,AAAA".to_owned());

        let json = serde_json::to_value(message).unwrap();
        let parts = json["content"].as_array().expect("content is not an array");

        assert!(
            parts.iter().any(|part| {
                part["type"] == "image_url"
                    && part["image_url"]["url"] == "data:image/png;base64,AAAA"
            }),
            "the image never reached the payload: {json}"
        );
        assert!(
            parts
                .iter()
                .any(|part| part["type"] == "text" && part["text"] == "what is this?"),
            "the question was lost when the image was attached: {json}"
        );
    }

    #[test]
    fn a_server_reporting_vision_is_taken_at_its_word() {
        let base = serve_props(
            r#"{"default_generation_settings":{"n_ctx":8192},
                "modalities":{"vision":true,"audio":false}}"#,
        );

        let props = properties(base).unwrap();

        assert!(props.vision, "the server said it can see and was not heard");
        assert_eq!(props.context_length, 8192);
    }

    #[test]
    fn a_server_reporting_no_vision_reports_no_vision() {
        let base = serve_props(
            r#"{"default_generation_settings":{"n_ctx":4096},
                "modalities":{"vision":false,"audio":false}}"#,
        );

        let props = properties(base).unwrap();

        assert!(!props.vision);
        assert_eq!(props.context_length, 4096);
    }

    #[test]
    fn a_props_with_no_modalities_at_all_means_no_vision() {
        // An older llama-server build predates the field entirely. Absent has
        // to mean "no", never "assume yes": an attach control offered against a
        // server that cannot see would take the image, send it, and return an
        // answer about nothing at all -- with no way for the user to tell it
        // had been ignored. The context length still has to come through, or a
        // conservative reading of one field would break the other.
        let base = serve_props(r#"{"default_generation_settings":{"n_ctx":2048}}"#);

        let props = properties(base).unwrap();

        assert!(!props.vision, "a missing field was read as support");
        assert_eq!(props.context_length, 2048);
    }

    #[test]
    fn figures_are_read_in_the_servers_spelling_and_written_in_the_webviews() {
        // Two audiences, two spellings, one type. `llama-server` writes
        // snake_case; every payload the webview sees is camelCase (ADR-002).
        // Losing either direction is silent -- a renamed field deserialises to
        // a default or serialises to a key the UI never reads.
        let from_wire: Usage =
            serde_json::from_str(r#"{"prompt_tokens":44,"completion_tokens":48}"#).unwrap();
        assert_eq!(from_wire.prompt_tokens, 44);

        let to_webview = serde_json::to_value(from_wire).unwrap();
        let object = to_webview.as_object().unwrap();
        assert!(object.contains_key("promptTokens"));
        assert!(object.contains_key("completionTokens"));

        // And back again, so a conversation stored today reloads tomorrow.
        let round_trip: Usage = serde_json::from_value(to_webview).unwrap();
        assert_eq!(round_trip, from_wire);

        let timings: Timings =
            serde_json::from_str(r#"{"prompt_per_second":32.3,"predicted_per_second":52.9}"#)
                .unwrap();
        assert_eq!(timings.predicted_per_second, Some(52.9));
        assert!(
            serde_json::to_value(timings)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("predictedPerSecond")
        );
    }
}
