use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
fn dry_run_lists_items_without_removing_them() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Items to keep (1):"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn short_dry_run_option_previews_without_deleting_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["-n", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("remove.txt"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn force_removes_unmatched_items_and_keeps_matching_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "keep.txt"])
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
fn short_force_option_removes_unmatched_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["-f", "keep.txt"])
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
fn multiple_patterns_keep_items_matching_any_pattern() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep-a.txt").write_str("keep").unwrap();
    temp.child("keep-b.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "keep-a.txt", "keep-b.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    let (keep_section, delete_section) = output.split_once("Items to delete").unwrap();
    assert!(keep_section.contains("keep-a.txt"));
    assert!(keep_section.contains("keep-b.txt"));
    assert!(delete_section.contains("remove.txt"));
    assert!(!delete_section.contains("keep-a.txt"));
    assert!(!delete_section.contains("keep-b.txt"));
}

#[test]
fn missing_patterns_are_rejected() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--dry-run")
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("required"));
}

#[test]
fn confirmation_defaults_to_no() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["keep-nothing"])
        .write_stdin("n\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Aborted."));
    temp.child("remove.txt").assert("remove");
}
