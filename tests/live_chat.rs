use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use llmwire::codec::{Chat, ProtocolCodec};
use llmwire::ir::{Conversation, Part, Role, Sampling, Turn};

#[test]
#[ignore = "requires .env live API settings"]
fn live_chat_nonstream_roundtrip() {
    let env = load_env().expect(".env is required for live API tests");
    let url = required(&env, "LLMWIRE_LIVE_CHAT_URL");
    let key = required(&env, "LLMWIRE_LIVE_CHAT_API_KEY");
    let model = required(&env, "LLMWIRE_LIVE_CHAT_MODEL");

    let conversation = Conversation {
        system: vec![Part::Text("You are terse.".to_owned())],
        turns: vec![Turn {
            role: Role::User,
            parts: vec![Part::Text("Reply with exactly: llmwire-ok".to_owned())],
        }],
        sampling: Sampling {
            max_output_tokens: Some(32),
            temperature: Some(0.0),
            ..Sampling::default()
        },
        ..Conversation::default()
    };

    let mut request: serde_json::Value =
        serde_json::from_slice(&Chat.encode_request(&conversation).unwrap()).unwrap();
    request["model"] = serde_json::Value::String(model);
    request["stream"] = serde_json::Value::Bool(false);
    let request_body = serde_json::to_vec(&request).unwrap();

    let mut child = Command::new("curl")
        .args([
            "--fail-with-body",
            "--silent",
            "--show-error",
            "--request",
            "POST",
            &url,
            "--header",
            "Content-Type: application/json",
            "--header",
            &format!("Authorization: Bearer {key}"),
            "--data-binary",
            "@-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start curl");

    child
        .stdin
        .as_mut()
        .expect("curl stdin")
        .write_all(&request_body)
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "curl failed with status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let response = Chat.decode_response(&output.stdout).unwrap();

    assert!(!response.choices.is_empty());
    assert!(response
        .choices
        .iter()
        .flat_map(|choice| &choice.parts)
        .any(|part| matches!(part, Part::Text(_))));
}

fn load_env() -> Option<BTreeMap<String, String>> {
    let path = project_root().join(".env");
    let text = fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (key, value) = line.split_once('=')?;
                Some((
                    key.trim().to_owned(),
                    value.trim().trim_matches('"').trim_matches('\'').to_owned(),
                ))
            })
            .collect(),
    )
}

fn required(env: &BTreeMap<String, String>, key: &str) -> String {
    env.get(key)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("missing {key} in .env"))
        .clone()
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
