use std::process::Command;

#[test]
fn test_cli_json_flag_runs_and_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_ai-usage");
    let output = Command::new(bin)
        .arg("--json")
        .output()
        .expect("ai-usage binary should execute");

    assert!(output.status.success(), "command failed with: {:?}", output);
    let stdout = String::from_utf8(output.stdout).expect("valid utf-8 output");
    assert!(!stdout.trim().is_empty(), "stdout should not be empty");

    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("output of --json must be valid JSON");
    assert!(parsed.is_object(), "expected JSON object at root");
}
