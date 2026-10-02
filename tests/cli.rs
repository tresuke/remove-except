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
    assert!(output.contains("PREVIEW ONLY - nothing will be removed"));
    assert!(output.contains("Items to keep (1):"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn relative_root_is_resolved_from_starting_directory() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    temp.child("outside.txt").write_str("outside").unwrap();
    let canonical_root = std::fs::canonicalize(temp.path().join("target")).unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "target", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains(&format!("Root: {}", canonical_root.display())));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("remove.txt"));
    assert!(!output.contains("outside.txt"));
    temp.child("target/remove.txt").assert("remove");
    temp.child("outside.txt").assert("outside");
}

#[test]
fn absolute_root_is_accepted() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    let root = std::fs::canonicalize(temp.path().join("target")).unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", root.to_str().unwrap(), "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Items to delete (1):"));
}

#[test]
fn absolute_keep_pattern_is_relative_to_selected_root_boundary() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    temp.child("target-sibling/outside.txt")
        .write_str("outside")
        .unwrap();
    let selected_root = std::fs::canonicalize(temp.path().join("target")).unwrap();
    let keep_pattern = selected_root
        .join("keep.txt")
        .to_string_lossy()
        .into_owned();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--root",
            selected_root.to_str().unwrap(),
            &keep_pattern,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Items to delete (1):"));

    let outside_pattern = temp
        .path()
        .join("target-sibling/*")
        .to_string_lossy()
        .into_owned();
    let stderr = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--root",
            selected_root.to_str().unwrap(),
            &outside_pattern,
        ])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&stderr).contains("outside the processing root"));
}

#[test]
fn force_with_root_preserves_selected_root_and_siblings() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    temp.child("outside.txt").write_str("outside").unwrap();
    let root = temp.path().join("target");

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "target", "keep.txt"])
        .assert()
        .success();

    assert!(root.is_dir());
    temp.child("target/keep.txt").assert("keep");
    assert!(!root.join("remove.txt").exists());
    temp.child("outside.txt").assert("outside");
}

#[test]
fn filesystem_root_is_rejected_even_with_force() {
    let temp = assert_fs::TempDir::new().unwrap();
    let filesystem_root = temp.path().ancestors().last().unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--force",
            "--root",
            filesystem_root.to_str().unwrap(),
            "keep-nothing",
        ])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Filesystem roots"));
}

#[test]
fn root_symlink_processes_and_displays_its_canonical_target() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/keep.txt").write_str("keep").unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    let target = temp.path().join("target");
    let link = temp.path().join("target-link");

    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_dir(&target, &link);

    if link_result.is_err() {
        eprintln!("Skipping root symlink test because symlink creation is unavailable");
        return;
    }

    let canonical_target = std::fs::canonicalize(&target).unwrap();
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "target-link", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains(&format!("Root: {}", canonical_target.display())));
    assert!(output.contains("remove.txt"));
    temp.child("target/remove.txt").assert("remove");
}

#[test]
fn no_match_warning_names_selected_root() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/remove.txt").write_str("remove").unwrap();
    let canonical_root = std::fs::canonicalize(temp.path().join("target")).unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "target", "missing"])
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();

    assert!(String::from_utf8_lossy(&output).contains(&canonical_root.display().to_string()));
    temp.child("target/remove.txt").assert("remove");
}

#[test]
fn default_display_groups_directories_and_marks_kept_parents_without_root_tags() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("z-file.txt").write_str("remove").unwrap();
    temp.child("alpha/nested.txt").write_str("remove").unwrap();
    temp.child("beta/nested.txt").write_str("remove").unwrap();

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
    assert!(output.contains("Items to delete (5):"));
    assert!(output.contains(
        "Legend: blue directory, cyan symlink, green kept parent; without color: / directory, @ symlink, + kept parent"
    ));
    assert_eq!(output.matches("Legend:").count(), 1);
    assert!(output.contains("├── alpha/"));
    assert!(output.contains("│   └── nested.txt"));
    assert!(output.contains("├── beta/"));
    assert!(output.contains("└── z-file.txt"));
    assert!(!output.contains("!"));
    assert!(!output.contains("[Directory]"));
    assert!(!output.contains("[File]"));
    assert!(!output.contains("[DELETE ROOT]"));
    temp.child("alpha/nested.txt").assert("remove");
}

#[test]
fn flat_option_displays_relative_paths_instead_of_a_tree() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("nested/remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--flat", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("RelativePath"));
    assert!(output.contains("nested/remove.txt"));
    assert!(!output.contains("└──"));
}

#[test]
fn tree_display_labels_retained_parent_for_delete_child() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("group/keep.txt").write_str("keep").unwrap();
    temp.child("group/remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "group/keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("└── group/+"));
    assert!(output.contains("    └── remove.txt"));
    assert!(!output.contains('\u{1b}'));
    temp.child("group/remove.txt").assert("remove");
}

#[test]
fn keep_summary_hides_only_keep_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--keep-summary", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Items to keep summary:"));
    assert!(output.contains("1 kept items (includes ancestor directories)"));
    assert!(output.contains("1 direct pattern matches"));
    assert!(!output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
fn delete_summary_hides_only_delete_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--delete-summary", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Items to keep (1):"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("Items to delete summary:"));
    assert!(output.contains("1 delete items; 1 top-level removal operations"));
    assert!(!output.contains("remove.txt"));
}

#[test]
fn summary_only_matches_both_section_summary_options() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let summary_only = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary-only", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let both_summaries = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--keep-summary",
            "--delete-summary",
            "keep.txt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(summary_only, both_summaries);
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
        .args(["--keep-summary", "--delete-summary", "keep-nothing"])
        .write_stdin("n\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Summary sections do not list item paths."));
    assert!(output.contains("Aborted."));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn force_with_summary_options_still_removes_unmatched_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--summary-only", "keep.txt"])
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}
