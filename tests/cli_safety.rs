//! Runs the real binary to check the safety defaults of disk-changing commands.
//! These run on any OS; on Linux there is no PowerShell, so a scan fails, which must never
//! lead to a change.

use std::process::Command;

fn linuxify(args: &[&str]) -> (bool, String) {
    let home = std::env::temp_dir().join(format!("linuxify-cli-safety-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_linuxify")).args(args).env("LOCALAPPDATA", &home).output().unwrap();
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
