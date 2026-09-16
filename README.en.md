# llmwire

[简体中文](README.md) | English

`llmwire` is a **bidirectional, stateless-by-default, embeddable LLM wire protocol conversion kernel**.

A host passes raw request or response bytes to a request-scoped `Converter`, which handles:

- SSE framing and cross-chunk reassembly
- Source protocol decoding
- Conversion through a protocol-neutral IR
- Target protocol serialization
- Explicit `Report`s for downgrades and unsupported cases

`llmwire` does not send HTTP requests, manage connections, retain cross-request state, or bind to an async runtime.

> Status: `M0` through `M5` are complete, and the `P0` through `P2` verification work is complete. The current version is `0.1.0`.
> See `docs/TESTING.md` for the verified scope and the real API scenarios that remain uncovered.

## Supported Scope

| Capability | Status |
|---|---|
| OpenAI Chat Completions | Requests, responses, and streaming |
| Anthropic Messages | Requests, responses, and streaming |
| OpenAI Responses | Stateless requests, responses, and streaming |
| Bidirectional text conversion across all three protocols | Supported |
| Response `id` / `model` | Passes through the target-reported values; a missing `model` is normalized to an empty string |
| Tool calls / tool results | Supported with byte-exact IDs |
| Thinking / signatures / encrypted reasoning | Supported or explicitly downgraded; never fabricated |
| Usage / cache usage | Preserves the distinction between unknown and zero |
| Image input as URL / Base64 | Supported and mapped across the three protocols |
| Image `file_id` | Explicitly unsupported |
| Stateful Responses | Explicitly unsupported |

Core dependency policy: no runtime dependencies beyond `serde`, `serde_json`, `thiserror`, and `bitflags`.

## Installation

Install from crates.io:

```bash
cargo add llmwire
```

Or add it directly to `Cargo.toml`:

```toml
[dependencies]
llmwire = "0.1"
```

For local development, you can reference the repository path:

```toml
[dependencies]
llmwire = { path = "../llmwire" }
```

Requires Rust `edition 2021`.

## Non-Streaming Usage

In `converter(src, dst, caps)`, the direction convention is:

- `src`: the protocol used by the client
- `dst`: the protocol used by the upstream backend

```rust
use llmwire::{converter, resolve, ProtocolId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-model"),
    )?;

    let client_request = br#"{
        "model": "claude-model",
        "messages": [{"role": "user", "content": "hello"}],
        "max_completion_tokens": 32
    }"#;

    let mut upstream_request = Vec::new();
    converter.request(client_request, &mut upstream_request)?;
    // The host sends upstream_request to the Messages backend.

    let backend_response = br#"{
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "world"}],
        "model": "claude-model",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 3, "output_tokens": 2}
    }"#;

    let mut client_response = Vec::new();
    converter.response(backend_response, &mut client_response)?;

    let report = converter.take_report();
    println!(
        "report: {} unmapped, {} warnings",
        report.unmapped.len(),
        report.warnings.len()
    );

    println!("{}", String::from_utf8_lossy(&client_response));
    Ok(())
}
```

Run the complete example:

```powershell
cargo run --example nonstream
```

## Streaming Usage

In streaming mode, the host must:

1. Call `request` first so the SDK can record `stream=true` and the request context.
2. Call `feed` for every upstream chunk and immediately forward the appended bytes.
3. Call `finish` after the upstream ends, forward the tail frame, and inspect the returned `Termination`.
4. Call `take_report` at an appropriate point.

```rust
use llmwire::{converter, resolve, ProtocolId, Termination};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut converter = converter(
        ProtocolId::Chat,
        ProtocolId::Messages,
        resolve(ProtocolId::Chat, ProtocolId::Messages, "claude-model"),
    )?;

    let client_request = br#"{
        "model": "claude-model",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}],
        "max_completion_tokens": 32
    }"#;
    let mut upstream_request = Vec::new();
    converter.request(client_request, &mut upstream_request)?;

    let upstream_chunk = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-model\"}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );

    let mut client_stream = Vec::new();
    converter.feed(upstream_chunk.as_bytes(), &mut client_stream)?;

    let mut tail = Vec::new();
    let termination = converter.finish(&mut tail)?;
    client_stream.extend_from_slice(&tail);
    assert_eq!(termination, Termination::Explicit);

    println!("{}", String::from_utf8_lossy(&client_stream));
    Ok(())
}
```

Run the complete example:

```powershell
cargo run --example streaming
```

## Capabilities and Policies

For straightforward protocol-pair configurations, use:

```rust
let caps = llmwire::resolve(
    llmwire::ProtocolId::Chat,
    llmwire::ProtocolId::Messages,
    "claude-model",
);
```

For model-specific policy overrides, use `StaticHost`:

```rust
use llmwire::{
    Host, ModelProfile, ProtocolId, StaticHost, ThinkingPolicy, UnknownModelPolicy,
};

let host = StaticHost::with_unknown_policy(UnknownModelPolicy::Reject)
    .with_profile(
        "claude-model",
        ModelProfile {
            thinking: Some(ThinkingPolicy::Passthrough),
            max_output_tokens: Some(8192),
            ..ModelProfile::default()
        },
    );

let caps = host.capabilities(
    ProtocolId::Chat,
    ProtocolId::Messages,
    "claude-model",
)?;
# Ok::<(), llmwire::Error>(())
```

Capability policies describe which conversion strategies are allowed for a route. They are not a protocol capability table.

## Errors and Reports

`llmwire` uses two channels to communicate problems:

- `Result::Err`: failures that occur before a successful response has been sent to the client.
- In-stream failures from `Converter::feed`: emitted as target-protocol error events and recorded in the `Report`; they are not mixed into `Ok(Termination::Explicit)`.
- `Report`: field downgrades, inexpressible content, and policy blocks.

A typical handling pattern:

```rust
let report = converter.take_report();
for item in report.unmapped {
    eprintln!(
        "unmapped field={} reason={:?} severity={:?}",
        item.field, item.reason, item.severity
    );
}
```

`take_report` clears the accumulated report.

## Host Responsibilities

`llmwire` only handles wire bytes. The host remains responsible for:

- HTTP requests, authentication, timeouts, and retries
- Route selection and upstream connections
- Extracting the model name from the request body and selecting a backend
- Writing `out` bytes to the client or upstream
- Disabling proxy buffering, gzip, and gateway aggregation for SSE responses
- Client disconnects, network failures, and upstream cancellation

## Known Limitations

- It never fabricates `signature`, `redacted_thinking`, or thinking content.
- It never recalculates or rewrites client-provided `cache_control` breakpoints.
- Stateful Responses requests such as `previous_response_id` and `store=true` are unsupported.
- Streaming Chat with `n > 1` currently returns `Unsupported` explicitly.
- Images support remote `http(s)` URLs and Base64 payloads only; `file_id` is unsupported.
- The core never downloads images, uploads images, or generates hosted URLs.
- The live matrix currently focuses on a local gateway; the official endpoint matrix has not yet been executed.
- The Messages thinking/signature live tests remain skipped, so that path cannot be considered verified against the real API.

## Documentation

- `docs/DESIGN.md`: architecture, public API, and non-negotiable constraints
- `docs/spec/IR.md`: protocol-neutral IR
- `docs/spec/STREAMING.md`: streaming events and the FSM
- `docs/spec/chat.md`, `docs/spec/messages.md`, `docs/spec/responses.md`: protocol mappings
- `docs/spec/IMAGE.md`: image input rules
- `docs/TESTING.md`: test strategy, live API usage, and the current verification matrix
- `docs/PLAN.md`: milestones and definitions of done
- `docs/README.md`: complete documentation map

## License

MIT. See `LICENSE`.

## Development

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -p llmwire
pwsh -NoProfile -File scripts/check-core-deps.ps1
pwsh -NoProfile -File scripts/check-live-gate.ps1
```

Live API tests are ignored by default and only access the network when run explicitly with `--ignored`. See `docs/TESTING.md §7` for configuration.