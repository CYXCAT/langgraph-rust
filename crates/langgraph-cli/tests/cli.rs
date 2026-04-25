use std::io::Write;
use std::process::{Command, Stdio};

fn bin_path() -> String {
    std::env::var("CARGO_BIN_EXE_langgraph-cli")
        .expect("cargo should provide CARGO_BIN_EXE_langgraph-cli for integration tests")
}

#[test]
fn cli_run_with_input_file_outputs_json() {
    let dir = std::env::temp_dir();
    let input_path = dir.join(format!(
        "langgraph-cli-input-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be monotonic enough for tests")
            .as_nanos()
    ));
    std::fs::write(&input_path, r#"{"text":""}"#).expect("should write input file");

    let output = Command::new(bin_path())
        .args(["run", "--input"])
        .arg(input_path.as_os_str())
        .output()
        .expect("cli should run");

    std::fs::remove_file(&input_path).expect("should cleanup temp file");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be json");
    assert_eq!(parsed.get("text"), Some(&serde_json::json!("ab")));
}

#[test]
fn cli_run_accepts_stdin_json() {
    let mut child = Command::new(bin_path())
        .arg("run")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cli should spawn");

    let mut stdin = child.stdin.take().expect("stdin should be available");
    stdin.write_all(br#"{"text":"prefix-"}"#).expect("should write stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("should collect output");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be json");
    assert_eq!(parsed.get("text"), Some(&serde_json::json!("prefix-ab")));
}

#[test]
fn cli_run_with_pretty_print_formats_json() {
    let output =
        Command::new(bin_path()).args(["run", "--pretty"]).output().expect("cli should run");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains('\n'));
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be json");
    assert_eq!(parsed.get("text"), Some(&serde_json::json!("ab")));
}

#[test]
fn cli_run_with_metadata_emits_state_and_execution_metadata() {
    let output =
        Command::new(bin_path()).args(["run", "--metadata"]).output().expect("cli should run");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be json");

    assert_eq!(parsed.pointer("/state/text"), Some(&serde_json::json!("ab")));
    assert_eq!(parsed.pointer("/metadata/supersteps"), Some(&serde_json::json!(2)));
    assert!(
        parsed.pointer("/metadata/channel_versions").is_some(),
        "channel_versions should exist"
    );
    assert!(parsed.pointer("/metadata/versions_seen").is_some(), "versions_seen should exist");
    let command_trace = parsed
        .pointer("/metadata/command_trace")
        .and_then(serde_json::Value::as_array)
        .expect("command_trace should be array");
    assert!(command_trace.is_empty(), "append_ab should not emit commands");
}

#[test]
fn cli_run_with_stream_emits_jsonl_events_and_completed_state() {
    let output =
        Command::new(bin_path()).args(["run", "--stream"]).output().expect("cli should run");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let mut saw_node_started = false;
    let mut completed_state: Option<serde_json::Value> = None;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: serde_json::Value =
            serde_json::from_str(line).expect("each line should be json");
        match event.get("event").and_then(serde_json::Value::as_str) {
            Some("node_started") => saw_node_started = true,
            Some("completed") => completed_state = event.get("state").cloned(),
            _ => {}
        }
    }
    assert!(saw_node_started, "stream should include node_started event");
    assert_eq!(
        completed_state.as_ref().and_then(|state| state.get("text")),
        Some(&serde_json::json!("ab"))
    );
}

#[test]
fn cli_run_rejects_pretty_with_stream() {
    let output = Command::new(bin_path())
        .args(["run", "--stream", "--pretty"])
        .output()
        .expect("cli should run");

    assert!(!output.status.success(), "command should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    assert!(stderr.contains("option conflict"), "stderr: {stderr}");
}
