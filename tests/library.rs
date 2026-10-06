use assert_fs::prelude::*;
use remove_except::{ItemType, build_plan};

#[test]
fn public_getters_describe_a_non_mutating_removal_plan() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep/important.txt").write_str("keep").unwrap();
    temp.child("keep/remove.txt").write_str("remove").unwrap();
    temp.child("other.md").write_str("keep").unwrap();
    temp.child("remove.txt").write_str("remove").unwrap();
    let patterns = ["keep/important.txt".to_owned(), "*.md".to_owned()];

    let plan = build_plan(temp.path(), &patterns).unwrap();

    assert_eq!(plan.root(), std::fs::canonicalize(temp.path()).unwrap());
    assert_eq!(plan.direct_match_count(), 2);
    assert_eq!(plan.keep_items().len(), 3);
    assert_eq!(plan.delete_items().len(), 2);
    assert_eq!(plan.delete_roots().len(), 2);
    assert!(plan.keep_items().iter().any(|item| {
        item.relative_path() == "keep/important.txt" && item.item_type() == ItemType::File
    }));
    assert!(
        plan.delete_items()
            .iter()
            .any(|item| item.path() == plan.root().join("keep/remove.txt"))
    );

    temp.child("keep/important.txt").assert("keep");
    temp.child("keep/remove.txt").assert("remove");
    temp.child("other.md").assert("keep");
    temp.child("remove.txt").assert("remove");
}

#[test]
fn public_api_reports_invalid_patterns_without_relying_on_error_text() {
    let temp = assert_fs::TempDir::new().unwrap();

    assert!(build_plan(temp.path(), &[String::new()]).is_err());
}

#[test]
fn public_api_rejects_parent_traversal_and_absolute_paths_outside_root() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("root/keep.txt").write_str("keep").unwrap();
    temp.child("sibling/outside.txt")
        .write_str("outside")
        .unwrap();
    let root = temp.path().join("root");

    assert!(build_plan(&root, &["../sibling/*".to_owned()]).is_err());

    let outside_pattern = temp.path().join("sibling/*").to_string_lossy().into_owned();
    assert!(build_plan(&root, &[outside_pattern]).is_err());

    temp.child("root/keep.txt").assert("keep");
    temp.child("sibling/outside.txt").assert("outside");
}

#[test]
fn public_delete_roots_contain_only_top_level_delete_items() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("keep.txt").write_str("keep").unwrap();
    temp.child("remove/nested/deep.txt")
        .write_str("remove")
        .unwrap();

    let plan = build_plan(temp.path(), &["keep.txt".to_owned()]).unwrap();

    assert_eq!(plan.delete_items().len(), 3);
    assert_eq!(plan.delete_roots().len(), 1);
    assert_eq!(plan.delete_roots()[0].relative_path(), "remove");
    assert!(plan.delete_items().iter().any(|item| {
        item.relative_path() == "remove/nested/deep.txt" && item.item_type() == ItemType::File
    }));
    temp.child("remove/nested/deep.txt").assert("remove");
}

#[test]
fn dot_patterns_keep_the_entire_processing_root() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("top.txt").write_str("top").unwrap();
    temp.child("nested/child.txt").write_str("child").unwrap();

    for pattern in [".", "./"] {
        let plan = build_plan(temp.path(), &[pattern.to_owned()]).unwrap();

        assert_eq!(plan.keep_items().len(), 3, "pattern: {pattern}");
        assert_eq!(plan.delete_items().len(), 0, "pattern: {pattern}");
        assert_eq!(plan.delete_roots().len(), 0, "pattern: {pattern}");
    }

    temp.child("top.txt").assert("top");
    temp.child("nested/child.txt").assert("child");
}

#[test]
fn case_matching_uses_windows_simple_fold_and_ubuntu_sensitive_rules() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("Ä.txt").write_str("umlaut").unwrap();
    temp.child("Keep/kept.txt").write_str("kelvin").unwrap();
    temp.child("ß.txt").write_str("sharp s").unwrap();
    let patterns = [
        "ä.txt".to_owned(),
        "Keep/kept.txt".to_owned(),
        "SS.txt".to_owned(),
    ];

    let plan = build_plan(temp.path(), &patterns).unwrap();

    assert_eq!(plan.direct_match_count(), if cfg!(windows) { 2 } else { 0 });
    assert_eq!(plan.keep_items().is_empty(), !cfg!(windows));
    assert!(
        plan.keep_items()
            .iter()
            .any(|item| { item.relative_path() == "Ä.txt" && item.item_type() == ItemType::File })
            == cfg!(windows)
    );
    assert!(
        plan.delete_items()
            .iter()
            .any(|item| item.relative_path() == "ß.txt")
    );
    temp.child("Ä.txt").assert("umlaut");
    temp.child("Keep/kept.txt").assert("kelvin");
    temp.child("ß.txt").assert("sharp s");
}
