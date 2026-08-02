//! The session, driven against the stub server.
//!
//! An integration test rather than a unit test because it needs the stub
//! binary, which only exists once Cargo has built the crate's targets.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use osstat_chat::plan::LaunchPlan;
use osstat_chat::session::{Launch, free_port, start};
use osstat_chat::{ChatClient, ChatError, Message};
use std::path::PathBuf;

/// The stub binary Cargo just built, beside the test executable.
fn stub() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // the test binary's own name
    if path.ends_with("deps") {
        path.pop();
    }
    let stub = path.join(format!("stub-llama-server{}", std::env::consts::EXE_SUFFIX));
    assert!(
        stub.is_file(),
        "the stub binary is missing at {}; `cargo test` should have built it",
        stub.display()
    );
    stub
}

fn launch() -> Launch {
    Launch {
        server: stub(),
        model: PathBuf::from("unused-by-the-stub.gguf"),
        plan: LaunchPlan {
            gpu_layers: 0,
            context_length: 4096,
            fits: true,
        },
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_session_starts_and_reports_where_to_reach_it() {
    let runtime = runtime();
    let session = runtime.block_on(start(launch())).unwrap();

    assert!(session.base.starts_with("http://127.0.0.1:"));
    assert!(!session.api_key.is_empty(), "no api key was generated");

    runtime.block_on(session.stop()).unwrap();
}

#[test]
fn a_session_waits_for_a_slow_model_rather_than_giving_up() {
    // The stub answers /health with 503 "Loading model" for two seconds. A
    // start that polled once and gave up would fail here -- and would fail far
    // worse on a real 30 GB model, where loading takes minutes.
    //
    // Two seconds rather than the few hundred milliseconds it takes to notice
    // a 503, because the threshold has to sit clear of process-spawn noise.
    // Measured on this Windows machine, a start against a stub with no delay
    // at all already takes ~517 ms -- so a 400 ms threshold would have been
    // met by spawn overhead alone, and the test would have passed against an
    // implementation that never waited for /health.
    let runtime = runtime();
    let began = std::time::Instant::now();

    let session = runtime
        .block_on(start(Launch {
            server: stub(),
            model: PathBuf::from("--slow-start"),
            ..launch()
        }))
        .unwrap();

    assert!(
        began.elapsed() >= std::time::Duration::from_secs(2),
        "start returned in {:?}, so it cannot have waited for /health",
        began.elapsed()
    );

    runtime.block_on(session.stop()).unwrap();
}

#[test]
fn a_server_that_refuses_to_start_reports_its_own_words() {
    // The difference between a fixable problem and a shrug. A missing CUDA
    // DLL says so on stderr; swallowing it loses the only useful information.
    let outcome = runtime().block_on(start(Launch {
        server: stub(),
        model: PathBuf::from("--fail-to-start"),
        ..launch()
    }));

    // Two `assert!`s rather than a `match` arm ending in `panic!`: the
    // workspace sets `clippy::panic = "warn"` and CI runs `-D warnings`, so a
    // `panic!` here fails the build even in a test.
    let reported = match &outcome {
        Err(ChatError::SpawnFailed(message)) => Some(message.as_str()),
        _ => None,
    };

    assert!(reported.is_some(), "expected SpawnFailed, got {outcome:?}");
    assert!(
        reported.is_some_and(|message| message.contains("refusing to start")),
        "the child's stderr was lost: {outcome:?}"
    );
}

#[test]
fn the_process_is_gone_after_stop() {
    let runtime = runtime();
    let session = runtime.block_on(start(launch())).unwrap();
    let key = session.record();
    let port = session.base.rsplit(':').next().unwrap().to_owned();

    runtime.block_on(session.stop()).unwrap();

    // The port being free again is the observable proof the child released it.
    let rebound = std::net::TcpListener::bind(format!("127.0.0.1:{port}"));
    assert!(
        rebound.is_ok(),
        "port {port} is still held; pid {} was not reaped",
        key.pid
    );
}

#[test]
fn a_child_that_dies_mid_stream_does_not_take_the_process_with_it() {
    // THE load-bearing test. ADR-012 chose a subprocess so that an inference
    // OOM would not be "a crash of the whole app -- taking down the tray and
    // the sampler with it". Without this, that is a claim rather than a
    // guarantee.
    let runtime = runtime();
    let session = runtime
        .block_on(start(Launch {
            server: stub(),
            model: PathBuf::from("--die-after"),
            ..launch()
        }))
        .unwrap();

    let client = ChatClient::new(session.base.clone(), session.api_key.clone());
    let outcome = runtime.block_on(async {
        client
            .stream(
                vec![Message {
                    role: "user".to_owned(),
                    content: "hello".to_owned(),
                }],
                |_| {},
            )
            .await
    });

    assert!(outcome.is_err(), "a dead server reported success");

    // The assertion that matters: this test process is still executing. If the
    // child's death could take osstat down, we would not reach this line.
    assert_eq!(2 + 2, 4);

    runtime.block_on(session.stop()).unwrap();
}

#[test]
fn two_sessions_do_not_share_an_api_key() {
    // A key derived from the clock would repeat across two sessions started
    // in the same instant, which is exactly when two sessions are started.
    use osstat_chat::session::random_api_key;

    let keys: std::collections::HashSet<String> = (0..16).map(|_| random_api_key()).collect();

    assert_eq!(keys.len(), 16, "keys repeated: {keys:?}");
    assert!(keys.iter().all(|key| key.len() >= 32));
}

#[test]
fn free_port_hands_out_a_port_that_can_actually_be_bound() {
    let port = free_port().unwrap();

    let bound = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(bound.is_ok(), "free_port returned an unusable port {port}");
}

#[test]
fn two_sessions_do_not_collide_on_a_port() {
    let runtime = runtime();
    let first = runtime.block_on(start(launch())).unwrap();
    let second = runtime.block_on(start(launch())).unwrap();

    assert_ne!(first.base, second.base);

    runtime.block_on(first.stop()).unwrap();
    runtime.block_on(second.stop()).unwrap();
}
