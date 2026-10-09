use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
// Verify dry-run lists kept items and deletion targets without modifying files.
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
    assert!(output.contains("PREVIEW ONLY  Nothing will be removed"));
    assert!(output.contains("Items to keep (1):"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    temp.child("remove.txt").assert("remove");
}

#[test]
fn keep_patterns_preserve_leading_whitespace() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child(" keep.txt").write_str("leading").unwrap();
    temp.child("keep.txt").write_str("plain").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", " keep.txt"])
        .assert()
        .success();

    assert!(temp.path().join(" keep.txt").is_file());
    assert!(!temp.path().join("keep.txt").exists());
}

#[cfg(unix)]
#[test]
fn keep_patterns_preserve_trailing_whitespace() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt ").write_str("trailing").unwrap();
    temp.child("keep.txt").write_str("plain").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "keep.txt "])
        .assert()
        .success();

    assert!(temp.path().join("keep.txt ").is_file());
    assert!(!temp.path().join("keep.txt").exists());
}

#[test]
fn keep_patterns_normalize_dot_components_and_repeated_separators() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("nested/keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    for pattern in ["nested/./keep.txt", "nested//keep.txt"] {
        let output = Command::cargo_bin("remove-except")
            .unwrap()
            .current_dir(temp.path())
            .args(["--dry-run", "--layout", "flat", pattern])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("Items to keep (2):"), "pattern: {pattern}");
        assert!(output.contains("nested/keep.txt"), "pattern: {pattern}");
        assert!(output.contains("remove.txt"), "pattern: {pattern}");
    }
}

#[cfg(unix)]
#[test]
fn preview_escapes_control_characters_in_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    std::fs::write(temp.path().join("bad\n\u{1b}[31mname.txt"), "remove").unwrap();

    for layout in [None, Some("flat")] {
        let mut command = Command::cargo_bin("remove-except").unwrap();
        command.current_dir(temp.path()).arg("--dry-run");
        if let Some(layout) = layout {
            command.args(["--layout", layout]);
        }
        let output = command
            .arg("keep.txt")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8_lossy(&output);

        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains("bad\n"));
        assert!(output.contains(r"bad\n\u{1b}[31mname.txt"));
    }
}

#[test]
// Verify relative roots resolve from the startup directory and do not target items outside it.
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
    let root_line = output
        .lines()
        .find(|line| line.starts_with("Root"))
        .unwrap();
    assert!(
        root_line.ends_with(
            canonical_root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    #[cfg(windows)]
    assert!(!root_line.contains(r"\\?\"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("remove.txt"));
    assert!(!output.contains("outside.txt"));
    temp.child("target/remove.txt").assert("remove");
    temp.child("outside.txt").assert("outside");
}

#[test]
// Verify an absolute processing root is accepted.
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
// Verify absolute keep patterns are limited to the selected root and outside paths are rejected.
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
// Verify --force preserves the selected root itself and its sibling items.
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
// Verify --force with no matches preserves the root and removes everything beneath it.
fn force_with_no_matches_removes_all_items_under_selected_root() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("target/nested/remove.txt")
        .write_str("remove")
        .unwrap();
    let root = temp.path().join("target");

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "target", "missing"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert!(String::from_utf8_lossy(&output.stderr).contains("No items matched"));
    assert!(root.is_dir());
    assert!(!root.join("nested").exists());
}

#[test]
// Verify filesystem roots are rejected even when --force is specified.
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

#[cfg(windows)]
#[test]
fn drive_relative_roots_and_patterns_are_rejected_by_cli() {
    let temp = assert_fs::TempDir::new().unwrap();

    let root_error = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "C:folder", "keep.txt"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&root_error).contains("Drive-relative root"));

    let pattern_error = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "C:folder"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&pattern_error).contains("Drive-relative pattern"));
}

#[test]
// Verify a symbolic link used as the root is processed and displayed through its canonical target.
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
    let root_line = output
        .lines()
        .find(|line| line.starts_with("Root"))
        .unwrap();
    assert!(
        root_line.ends_with(
            canonical_target
                .file_name()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    #[cfg(windows)]
    assert!(!root_line.contains(r"\\?\"));
    assert!(output.contains("remove.txt"));
    temp.child("target/remove.txt").assert("remove");
}

#[test]
// Verify file and directory links beneath the root are not followed and their targets survive deletion.
fn child_symlinks_are_not_traversed_or_removed_with_their_targets() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("root/keep.txt").write_str("keep").unwrap();
    temp.child("outside/nested/secret.txt")
        .write_str("secret")
        .unwrap();
    temp.child("outside-file.txt")
        .write_str("file target")
        .unwrap();
    let root = temp.path().join("root");
    let target = temp.path().join("outside");
    let directory_link = root.join("external");
    let file_target = temp.path().join("outside-file.txt");
    let file_link = root.join("external-file");

    #[cfg(unix)]
    let directory_link_result = std::os::unix::fs::symlink(&target, &directory_link);
    #[cfg(windows)]
    let directory_link_result = std::os::windows::fs::symlink_dir(&target, &directory_link);

    #[cfg(unix)]
    let file_link_result = std::os::unix::fs::symlink(&file_target, &file_link);
    #[cfg(windows)]
    let file_link_result = std::os::windows::fs::symlink_file(&file_target, &file_link);

    if directory_link_result.is_err() || file_link_result.is_err() {
        eprintln!("Skipping child symlink test because symlink creation is unavailable");
        return;
    }

    let preview = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--root", "root", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let preview = String::from_utf8_lossy(&preview);
    assert!(preview.contains("external@"));
    assert!(preview.contains("external-file@"));
    assert!(!preview.contains("secret.txt"));

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "root", "keep.txt"])
        .assert()
        .success();

    assert!(std::fs::symlink_metadata(&directory_link).is_err());
    assert!(std::fs::symlink_metadata(&file_link).is_err());
    temp.child("outside/nested/secret.txt").assert("secret");
    temp.child("outside-file.txt").assert("file target");
}

#[cfg(windows)]
#[test]
fn dangling_directory_symlink_is_removed_without_following_its_target() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("root/keep.txt").write_str("keep").unwrap();
    let target = temp.path().join("missing-target");
    let link = temp.path().join("root/dangling-directory-link");

    if std::os::windows::fs::symlink_dir(&target, &link).is_err() {
        eprintln!(
            "Skipping dangling directory symlink test because symlink creation is unavailable"
        );
        return;
    }

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--root", "root", "keep.txt"])
        .assert()
        .success();

    assert!(std::fs::symlink_metadata(&link).is_err());
    assert!(!target.exists());
    temp.child("root/keep.txt").assert("keep");
}

#[test]
// Verify the no-match warning names the selected processing root.
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

    assert!(
        String::from_utf8_lossy(&output).contains(
            canonical_root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    temp.child("target/remove.txt").assert("remove");
}

#[test]
// Verify default tree output sorts by relative path and does not show deletion-root markers.
fn default_display_sorts_by_relative_path_and_marks_kept_parents_without_root_tags() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("0-file.txt").write_str("remove").unwrap();
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
    assert!(output.contains("Markers: / directory; @ symlink; + kept parent."));
    assert_eq!(output.matches("Markers:").count(), 1);
    assert!(output.contains("├── 0-file.txt"));
    assert!(output.contains("├── alpha/"));
    assert!(output.contains("│   └── nested.txt"));
    assert!(output.contains("└── beta/"));
    assert!(output.find("0-file.txt").unwrap() < output.find("├── alpha/").unwrap());
    assert!(!output.contains("!"));
    assert!(!output.contains("[Directory]"));
    assert!(!output.contains("[File]"));
    assert!(!output.contains("[DELETE ROOT]"));
    temp.child("alpha/nested.txt").assert("remove");
}

#[test]
// Verify --layout flat displays relative paths instead of a tree.
fn flat_option_displays_relative_paths_instead_of_a_tree() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("nested/remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--layout", "flat", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("RelativePath"));
    assert!(
        output
            .lines()
            .any(|line| { line.contains("Directory") && line.ends_with("nested/") })
    );
    assert!(output.contains("nested/remove.txt"));
    assert!(output.contains("Markers: / directory; @ symlink."));
    assert!(!output.contains("└──"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
// Verify --sort-by-type groups items by type and sorts paths within each type.
fn flat_type_sort_groups_types_and_sorts_paths_within_each_type() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("0-file.txt").write_str("remove").unwrap();
    temp.child("y-file.txt").write_str("remove").unwrap();
    temp.child("x-file.txt").write_str("remove").unwrap();
    temp.child("z-dir/remove.txt").write_str("remove").unwrap();

    let default_output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--layout", "flat", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let default_output = String::from_utf8_lossy(&default_output);
    assert!(default_output.find("0-file.txt").unwrap() < default_output.find("z-dir/").unwrap());

    let sorted_output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--layout",
            "flat",
            "--sort-by-type",
            "keep.txt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let sorted_output = String::from_utf8_lossy(&sorted_output);
    let delete_section = sorted_output.split("Items to delete").nth(1).unwrap();
    assert!(delete_section.find("Directory").unwrap() < delete_section.find("File").unwrap());
    assert!(
        delete_section.find("x-file.txt").unwrap() < delete_section.find("y-file.txt").unwrap()
    );
    assert!(
        delete_section.find("y-file.txt").unwrap()
            < delete_section.find("z-dir/remove.txt").unwrap()
    );
}

#[test]
// Verify --sort-by-type places symbolic links after files.
fn flat_type_sort_places_symlinks_after_files() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("z-file.txt").write_str("remove").unwrap();
    let target = temp.path().join("keep.txt");
    let link = temp.path().join("a-link");

    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&target, &link);
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_file(&target, &link);

    if link_result.is_err() {
        eprintln!("Skipping flat symlink sort test because symlink creation is unavailable");
        return;
    }

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args([
            "--dry-run",
            "--layout",
            "flat",
            "--sort-by-type",
            "keep.txt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);
    let delete_section = output.split("Items to delete").nth(1).unwrap();
    assert!(delete_section.find("File").unwrap() < delete_section.find("Symlink").unwrap());
    assert!(delete_section.contains("a-link@"));
}

#[test]
// Verify --sort-by-type is rejected with tree layout.
fn flat_type_sort_requires_flat_option() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .args(["--dry-run", "--sort-by-type", "keep.txt"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("--layout flat"));
    assert!(output.contains("Usage:"));
}

#[test]
// Verify help documents the choices and defaults for --layout and --summary.
fn help_documents_layout_and_summary_options() {
    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);

    assert!(output.contains("<KEEP_PATTERN>..."));
    assert!(output.contains("--layout <LAYOUT>"));
    assert!(output.contains("[default: tree]"));
    assert!(output.contains("--summary <SECTION>"));
    assert!(output.contains("keep, delete, or both"));
    assert!(output.contains("Output:"));
    assert!(output.contains("kept parent directories in the delete tree green"));
    assert!(output.contains("Flat output has no kept-parent marker"));
    assert!(output.contains("PREVIEW ONLY (cyan)"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
fn replaced_display_options_are_rejected() {
    for obsolete_option in [
        "--flat",
        "--keep-summary",
        "--delete-summary",
        "--summary-only",
    ] {
        Command::cargo_bin("remove-except")
            .unwrap()
            .args([obsolete_option, "keep.txt"])
            .assert()
            .failure();
    }
}

#[test]
// Verify tree output marks a kept parent directory that contains a deletion target.
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
// Verify --summary keep omits only kept-item paths and preserves the deletion list.
fn keep_summary_hides_only_keep_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary", "keep", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("KEEP:"));
    assert!(output.contains("1 kept item (including required ancestors)"));
    assert!(output.contains("1 direct pattern match"));
    assert!(!output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
// Verify --summary delete omits only deletion-target paths and preserves the keep list.
fn delete_summary_hides_only_delete_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary", "delete", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Items to keep (1):"));
    assert!(output.contains("keep.txt"));
    assert!(output.contains("DELETE:"));
    assert!(output.contains("1 deletion item; 1 top-level deletion operation"));
    assert!(!output.contains("remove.txt"));
}

#[test]
// Verify --summary both summarizes both kept items and deletion targets.
fn summary_only_matches_both_section_summary_options() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let summary_only = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary", "both", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let both_summaries = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--dry-run", "--summary", "both", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(summary_only, both_summaries);
    let summary_output = String::from_utf8_lossy(&summary_only);
    assert!(summary_output.contains("KEEP:"));
    assert!(summary_output.contains("DELETE:"));
    assert!(summary_output.contains("Summary sections omit paths."));
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify -n previews the plan like --dry-run without deleting anything.
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
// Verify --force keeps matching items and deletes everything else.
fn force_removes_unmatched_items_and_keeps_matching_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "keep.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert!(String::from_utf8_lossy(&output).contains("Removal complete."));
    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify -f deletes nonmatching items like --force.
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
// Verify answering yes to the confirmation prompt performs deletion.
fn confirmation_accepts_yes_and_removes_unmatched_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .arg("keep.txt")
        .write_stdin("y\n")
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}

#[test]
// Verify no confirmation is requested and kept items remain when there is nothing to delete.
fn no_removal_operations_do_not_prompt_for_confirmation() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep/nested/important.txt")
        .write_str("keep")
        .unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .arg("keep")
        .write_stdin("")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8_lossy(&output);

    assert!(output.contains("NO REMOVALS  No deletion operations are needed"));
    assert!(!output.contains("Are you sure you want to delete"));
    temp.child("keep/nested/important.txt").assert("keep");
}

#[test]
// Verify multiple keep patterns use OR semantics.
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
// Verify CLI invocations without keep patterns are rejected.
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
// Verify the confirmation prompt defaults to No and cancels deletion.
fn confirmation_defaults_to_no() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--summary", "both", "keep-nothing"])
        .write_stdin("n\n")
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Summary sections omit paths."));
    assert!(stdout.contains("Aborted."));
    assert!(stderr.contains("Delete the plan? Summary sections do not list item paths."));
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify normal execution lists kept items and deletion targets before confirmation and preserves files when declined.
fn confirmation_shows_both_item_lists_before_prompt_and_preserves_items_on_rejection() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    let output = Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .arg("keep.txt")
        .write_stdin("n\n")
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let keep_list = stdout.find("Items to keep (1):").unwrap();
    let delete_list = stdout.find("Items to delete (1):").unwrap();
    let prompt = stderr
        .find("Are you sure you want to delete the above items?")
        .unwrap();
    assert!(keep_list < delete_list);
    assert!(!stdout.contains("Are you sure you want to delete"));
    assert_eq!(prompt, 0);
    assert!(stdout.contains("Aborted."));
    temp.child("keep.txt").assert("keep");
    temp.child("remove.txt").assert("remove");
}

#[test]
// Verify --force performs deletion as summarized when used with summary output.
fn force_with_summary_options_still_removes_unmatched_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();

    Command::cargo_bin("remove-except")
        .unwrap()
        .current_dir(temp.path())
        .args(["--force", "--summary", "both", "keep.txt"])
        .assert()
        .success();

    temp.child("keep.txt").assert("keep");
    assert!(!temp.path().join("remove.txt").exists());
}
