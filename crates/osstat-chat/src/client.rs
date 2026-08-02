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
    /// The turn's text.
    pub content: String,
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

/// `/props`, narrowed to the one field the context meter needs.
#[derive(serde::Deserialize)]
struct Props {
    default_generation_settings: GenerationSettings,
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

    /// The context window the server actually allocated.
    ///
    /// Read from the server rather than assumed from the launch plan: the
    /// server may round or clamp what it was asked for, and a context meter
    /// whose denominator is a request rather than a fact would mislead
    /// precisely when the window is nearly full.
    ///
    /// # Errors
    ///
    /// [`ChatError::Io`] if the request fails, [`ChatError::BadChunk`] if the
    /// response does not contain the field.
    pub async fn context_length(&self) -> Result<u32, ChatError> {
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

        Ok(props.default_generation_settings.n_ctx)
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
                .stream(
                    vec![Message {
                        role: "user".to_owned(),
                        content: "hello".to_owned(),
                    }],
                    |event| events.push(event),
                )
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
