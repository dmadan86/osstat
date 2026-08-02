//! A stand-in for `llama-server`, for tests.
//!
//! Speaks `/health`, `/props` and a canned `/v1/chat/completions` stream. Its
//! reason for existing is ADR-012's rule that CI neither downloads a runtime
//! nor runs inference — and the practical one that a real model takes minutes
//! to load, which would make every iteration of the session code unbearable.
//!
//! Behaviour is driven by arguments so one binary covers every case:
//!
//! - `--port N`          which port to listen on (required)
//! - `--slow-start MS`   answer /health with 503 for this long first (2000)
//! - `--die-after MS`    exit(1) this long after the first completion request
//! - `--fail-to-start`   print a message to stderr and exit(1) immediately
//!
//! The session passes the model path straight through as `-m <path>`, so the
//! integration tests smuggle these flags in as the model path. A bare flag with
//! no parsable value after it therefore has to mean something sensible, which
//! is why each one has a default rather than being ignored.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Writes a `200 OK` carrying `body` as JSON.
fn respond_ok(stream: &mut TcpStream, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// Writes the `503` `llama-server` answers with while the weights are loading.
fn respond_loading(stream: &mut TcpStream) {
    let body = r#"{"error":{"code":503,"message":"Loading model"}}"#;
    let _ = write!(
        stream,
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .and_then(|value| value.parse::<u64>().ok())
    };
    let present = |name: &str| args.iter().any(|arg| arg == name);

    if present("--fail-to-start") {
        eprintln!("stub: refusing to start, as requested");
        std::process::exit(1);
    }

    let Some(port) = flag("--port").and_then(|value| u16::try_from(value).ok()) else {
        eprintln!("stub: --port is required");
        std::process::exit(2);
    };

    // Each flag may arrive bare, with the next argument belonging to the
    // session rather than to it. Presence decides; the value is a refinement.
    //
    // The bare default is two seconds, not the couple of hundred milliseconds
    // it takes to observe one 503. The test asserts on elapsed wall time, and
    // spawning this binary and reaching it over a socket already costs ~500 ms
    // on Windows -- a shorter delay would be indistinguishable from that
    // overhead, so the assertion would hold against a `start` that never
    // waited for /health at all.
    let slow_start = if present("--slow-start") {
        flag("--slow-start").unwrap_or(2000)
    } else {
        0
    };
    let die_after = if present("--die-after") {
        Some(flag("--die-after").unwrap_or(50))
    } else {
        None
    };

    let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
        eprintln!("stub: could not bind port {port}");
        std::process::exit(3);
    };

    let started = Instant::now();

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };

        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }

        if line.starts_with("GET /health") {
            if started.elapsed() < Duration::from_millis(slow_start) {
                respond_loading(&mut stream);
            } else {
                respond_ok(&mut stream, r#"{"status":"ok"}"#);
            }
        } else if line.starts_with("GET /props") {
            respond_ok(
                &mut stream,
                r#"{"default_generation_settings":{"n_ctx":4096}}"#,
            );
        } else if line.starts_with("POST /v1/chat/completions") {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                  Connection: close\r\n\r\n",
            );
            let _ =
                stream.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"stub\"}}]}\n\n");
            let _ = stream.flush();

            if let Some(delay) = die_after {
                std::thread::sleep(Duration::from_millis(delay));
                eprintln!("stub: out of memory (simulated)");
                std::process::exit(1);
            }

            let _ = stream.write_all(
                b"data: {\"choices\":[{\"delta\":{}}],\"usage\":\
                  {\"prompt_tokens\":7,\"completion_tokens\":1},\"timings\":\
                  {\"prompt_per_second\":100.0,\"predicted_per_second\":50.0}}\n\n",
            );
            let _ = stream.write_all(b"data: [DONE]\n\n");
            let _ = stream.flush();
        } else {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
}
