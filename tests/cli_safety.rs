//! Runs the real binary to check the safety defaults of disk-changing commands.
//! These run on any OS; on Linux there is no PowerShell, so a scan fails, which must never
//! lead to a change.

use std::process::Command;

fn linuxify(args: &[&str]) -> (bool, String) {
    let home = std::env::temp_dir().join(format!("linuxify-cli-safety-{}", std::process::id()));
    linuxify_in(&home, args)
}

fn linuxify_in(home: &std::path::Path, args: &[&str]) -> (bool, String) {
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_linuxify")).args(args).env("LOCALAPPDATA", home).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn execute_and_dry_run_contradict() {
    let (ok, text) = linuxify(&["install", "linux-mint", "--dry-run", "--execute"]);
    assert!(!ok);
    assert!(text.contains("contradict"), "{text}");
}

#[test]
fn undo_without_a_recorded_install_does_nothing() {
    let (ok, text) = linuxify(&["install", "--undo", "--execute"]);
    assert!(!ok);
    assert!(text.contains("nothing to undo") || text.contains("no Linuxify install setup"), "{text}");
}

#[test]
fn install_without_execute_never_reaches_the_typed_confirmation() {
    // No scan is possible here, so this ends with an error or a plan; either way it must not ask to
    // type a confirmation phrase, which only appears on a real (--execute) run.
    let (_, text) = linuxify(&["install", "linux-mint"]);
    assert!(!text.contains("Type exactly"), "{text}");
}

#[test]
fn quiz_result_with_bad_input_is_refused() {
    let dir = std::env::temp_dir().join(format!("linuxify-quiz-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("r.json");
    std::fs::write(&f, r#"{"version":1,"distro":"../../etc/passwd"}"#).unwrap();
    let (ok, text) = linuxify(&["choose", "--quiz-result", f.to_str().unwrap()]);
    assert!(!ok);
    assert!(text.contains("invalid distro id"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn quiz_spec_lists_every_question_with_options() {
    let (ok, text) = linuxify(&["quiz-spec"]);
    assert!(ok, "{text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("quiz-spec prints JSON");
    let qs = v["questions"].as_array().unwrap();
    assert_eq!(qs.len(), 14);
    for q in qs {
        assert!(q["id"].as_str().is_some_and(|s| !s.is_empty()), "{q}");
        assert!(!q["options"].as_array().unwrap().is_empty(), "{q}");
    }
}

#[test]
fn quiz_spec_first_answers_are_accepted_and_saved() {
    // Build an answer set from the spec itself: the first option of every question.
    let (_, spec) = linuxify(&["quiz-spec"]);
    let v: serde_json::Value = serde_json::from_str(&spec).unwrap();
    let answers: serde_json::Map<String, serde_json::Value> = v["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| (q["id"].as_str().unwrap().to_string(), q["options"][0]["key"].clone()))
        .collect();
    let dir = std::env::temp_dir().join(format!("linuxify-quiz-ok-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("r.json");
    std::fs::write(&f, serde_json::json!({"version": 1, "answers": answers}).to_string()).unwrap();
    let (ok, text) = linuxify_in(&dir, &["choose", "--no-scan", "--quiz-result", f.to_str().unwrap()]);
    assert!(ok, "{text}");
    assert!(dir.join("Linuxify").join("choice.json").is_file(), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn quiz_result_with_a_known_distro_is_saved() {
    let dir = std::env::temp_dir().join(format!("linuxify-quiz-distro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("r.json");
    std::fs::write(&f, r#"{"version":1,"distro":"linux-mint"}"#).unwrap();
    let (ok, text) = linuxify_in(&dir, &["choose", "--no-scan", "--quiz-result", f.to_str().unwrap()]);
    assert!(ok, "{text}");
    assert!(text.contains("Linux Mint"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}
