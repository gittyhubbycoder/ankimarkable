//! Live OpenRouter API tests — run only with a real key in the environment:
//!
//! ```sh
//! OPENROUTER_API_KEY="$(cat key.txt)" cargo test --locked --test openrouter_live
//! ```
//!
//! Without the variable each test prints a skip note and passes, so plain
//! `cargo test` never touches the network. Nothing here prints the key.
//!
//! The runtime is built manually (no `tokio/macros` dependency) on a
//! current-thread runtime, mirroring the app's own network worker.

use openrouter_chat::openrouter::{Message, OpenRouter};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test tokio runtime")
}

fn live_client() -> Option<OpenRouter> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => match OpenRouter::from_env() {
            Ok(api) => Some(api),
            Err(e) => panic!("key set but client failed: {e:#}"),
        },
        _ => {
            eprintln!("skipping live OpenRouter test: OPENROUTER_API_KEY not set");
            None
        }
    }
}

#[test]
fn live_free_model_list_is_nonempty() {
    let Some(api) = live_client() else { return };
    let models = runtime()
        .block_on(api.models())
        .expect("live /models request");
    assert!(!models.is_empty(), "expected at least one free model");
    for model in &models {
        assert!(
            !model.id.is_empty() && !model.name.is_empty(),
            "unexpected empty model entry"
        );
    }
    eprintln!(
        "live /models: {} free models (e.g. {} / {})",
        models.len(),
        models.first().map(|m| m.id.as_str()).unwrap_or("?"),
        models.get(1).map(|m| m.id.as_str()).unwrap_or("?")
    );
}

#[test]
fn live_chat_completion_returns_text() {
    let Some(api) = live_client() else { return };
    let rt = runtime();
    let models = rt.block_on(api.models()).expect("live /models request");
    let model = models
        .iter()
        .find(|m| m.id.contains("gpt-4o-mini"))
        .or_else(|| models.first())
        .expect("at least one free model")
        .id
        .clone();
    eprintln!("live chat model: {model}");
    let reply = rt
        .block_on(api.complete(
            &model,
            &[Message {
                role: "user".into(),
                content: "Reply with exactly: OK".into(),
            }],
        ))
        .expect("live /chat/completions request");
    assert!(
        !reply.trim().is_empty(),
        "expected non-empty assistant reply"
    );
    eprintln!("live chat reply ({} chars)", reply.len());
}

#[test]
fn live_error_path_reports_http_status() {
    // /models is public (no auth needed), so exercise the authed endpoint:
    // a bogus key must produce a readable HTTP 401 error, never a panic,
    // proving the new error-surfacing path against the real API.
    let bogus =
        OpenRouter::with_key("sk-or-v1-bogus-key-for-error-path-test").expect("bogus client");
    let err = format!(
        "{:#}",
        runtime()
            .block_on(bogus.complete(
                "openai/gpt-4o-mini",
                &[Message {
                    role: "user".into(),
                    content: "hi".into(),
                }],
            ))
            .expect_err("bogus key must fail chat completions")
    );
    assert!(
        err.contains("401"),
        "expected an HTTP 401 style error, got: {err}"
    );
    eprintln!("live error path OK: {err}");
}
