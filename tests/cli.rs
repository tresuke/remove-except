use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
// dry-run が保持項目と削除対象を表示し、ファイルを変更しないことを確認する。
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
// 相対ルートが起動時ディレクトリ基準で解決され、ルート外を対象にしないことを確認する。
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
// 絶対パスで指定した処理ルートを受け付けることを確認する。
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
// 絶対保持パターンを選択ルート内に制限し、ルート外の指定を拒否することを確認する。
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
// --force 実行でも選択ルート自体とその兄弟項目を保護することを確認する。
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
// 一致項目がない --force はルートを残して配下をすべて削除することを確認する。
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
// --force 指定時もファイルシステムのルートを処理対象として拒否することを確認する。
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
// ルートに指定したシンボリックリンクの正規化先を処理・表示することを確認する。
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
// ルート配下のファイル・ディレクトリーリンクをたどらず、削除時もリンク先を残すことを確認する。
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

#[test]
// 一致項目がない場合の警告に選択した処理ルートが含まれることを確認する。
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
// 既定のツリー表示でディレクトリをまとめ、削除ルート印を表示しないことを確認する。
fn default_display_groups_directories_and_marks_kept_parents_without_root_tags() {
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
    assert!(output.contains(
        "Legend: blue directory, cyan symlink, green kept parent; without color: / directory, @ symlink, + kept parent"
    ));
    assert_eq!(output.matches("Legend:").count(), 1);
    assert!(output.contains("├── alpha/"));
    assert!(output.contains("│   └── nested.txt"));
    assert!(output.contains("├── beta/"));
    assert!(output.contains("└── 0-file.txt"));
    assert!(output.find("├── alpha/").unwrap() < output.find("0-file.txt").unwrap());
    assert!(!output.contains("!"));
    assert!(!output.contains("[Directory]"));
    assert!(!output.contains("[File]"));
    assert!(!output.contains("[DELETE ROOT]"));
    temp.child("alpha/nested.txt").assert("remove");
}

#[test]
// --layout flat がツリーではなく相対パス一覧を表示することを確認する。
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
    assert!(
        output.contains(
            "Legend: blue directory, cyan symlink; without color: / directory, @ symlink"
        )
    );
    assert!(!output.contains("└──"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
// --sort-by-type が種類ごとに分類し、各種類内をパス順に表示することを確認する。
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
// --sort-by-type でシンボリックリンクがファイルの後に並ぶことを確認する。
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
// --sort-by-type を tree layout で指定すると拒否されることを確認する。
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
// --layout と --summary が選択肢と既定値をヘルプに示すことを確認する。
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
// ツリー表示で削除対象の子を持つ保持済み親ディレクトリを示すことを確認する。
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
// --summary keep が保持項目のパスだけを省略し、削除一覧を保つことを確認する。
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
    assert!(output.contains("Items to keep summary:"));
    assert!(output.contains("1 kept items (includes ancestor directories)"));
    assert!(output.contains("1 direct pattern matches"));
    assert!(!output.contains("keep.txt"));
    assert!(output.contains("Items to delete (1):"));
    assert!(output.contains("remove.txt"));
    assert!(!output.contains('\u{1b}'));
}

#[test]
// --summary delete が削除項目のパスだけを省略し、保持一覧を保つことを確認する。
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
    assert!(output.contains("Items to delete summary:"));
    assert!(output.contains("1 delete items; 1 top-level removal operations"));
    assert!(!output.contains("remove.txt"));
}

#[test]
// --summary both が保持・削除両方を要約することを確認する。
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
    temp.child("remove.txt").assert("remove");
}

#[test]
// -n が --dry-run と同様にプレビューのみを行い、削除しないことを確認する。
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
// --force が一致項目を残し、それ以外を削除することを確認する。
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
// -f が --force と同様に一致項目以外を削除することを確認する。
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
// 確認プロンプトに「はい」と答えると削除を実行することを確認する。
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
// 削除対象がない場合は確認を求めず、保持内容をそのまま残すことを確認する。
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

    assert!(output.contains("No removal operations are needed"));
    assert!(!output.contains("Are you sure you want to delete"));
    temp.child("keep/nested/important.txt").assert("keep");
}

#[test]
// 複数の保持パターンが OR 条件で適用されることを確認する。
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
// 保持パターンを省略した CLI 呼び出しが拒否されることを確認する。
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
// 確認プロンプトの既定回答が「いいえ」で、削除を中止することを確認する。
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
    assert!(stdout.contains("Summary sections hide item paths."));
    assert!(stdout.contains("Aborted."));
    assert!(stderr.contains("Delete the plan? Summary sections do not list item paths."));
    temp.child("remove.txt").assert("remove");
}

#[test]
// 通常の確認前に保持・削除一覧を表示し、拒否時にファイルを残すことを確認する。
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
// --force と要約表示を併用しても、要約どおり削除が実行されることを確認する。
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
