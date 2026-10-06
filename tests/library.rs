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
