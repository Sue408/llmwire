use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

pub fn load_env() -> Option<BTreeMap<String, String>> {
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

pub fn required(env: &BTreeMap<String, String>, key: &str) -> String {
    env.get(key)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("missing {key} in .env"))
        .clone()
}

#[allow(dead_code)]
pub fn optional(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key).filter(|value| !value.is_empty()).cloned()
}

pub fn post_json(url: &str, headers: &[(String, String)], body: &[u8]) -> Output {
    let mut command = Command::new("curl");
    command.args([
        "--fail-with-body",
        "--silent",
        "--show-error",
        "--request",
        "POST",
        url,
        "--header",
        "Content-Type: application/json",
    ]);
    for (name, value) in headers {
        command.arg("--header").arg(format!("{name}: {value}"));
    }
    let mut child = command
        .args(["--data-binary", "@-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start curl");

    child
        .stdin
        .as_mut()
        .expect("curl stdin")
        .write_all(body)
        .unwrap();
    child.wait_with_output().unwrap()
}

pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "curl failed with status {:?}: stderr={} body={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
