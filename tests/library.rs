use assert_fs::prelude::*;
use remove_except::{ItemType, ValidationError, build_plan};

mod support;

#[test]
fn non_unicode_entries_are_runtime_errors_without_mutation() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("nested").create_dir_all().unwrap();
    let first = temp
        .path()
        .join("nested")
        .join(support::non_unicode_name(0xff));
    let second = temp
        .path()
        .join("nested")
        .join(support::non_unicode_name(0xfe));
    std::fs::write(&first, "first").unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(second.join("child.txt"), "child").unwrap();

    for pattern in ["keep.txt", ".", "nested/bad-\u{fffd}.txt"] {
        let error = build_plan(temp.path(), &[pattern.to_owned()]).unwrap_err();
        assert!(error.downcast_ref::<ValidationError>().is_none());
        assert!(
            error
                .to_string()
                .contains("cannot be represented as Unicode")
        );
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(second.join("child.txt")).unwrap(), b"child");
    }
}

#[test]
fn non_unicode_roots_are_runtime_errors_even_when_empty() {
    let temp = assert_fs::TempDir::new().unwrap();
    let root = temp.path().join(support::non_unicode_name(0xff));
    std::fs::create_dir(&root).unwrap();

    let error = build_plan(&root, &[".".to_owned()]).unwrap_err();
    assert!(error.downcast_ref::<ValidationError>().is_none());
    assert!(
        error
            .to_string()
            .contains("cannot be represented as Unicode")
    );
    assert!(root.is_dir());
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
}

#[test]
fn valid_unicode_replacement_characters_remain_distinct_paths() {
    let temp = assert_fs::TempDir::new().unwrap();
    let root = temp.child("root-\u{fffd}");
    root.child("bad-\u{fffd}.txt")
        .write_str("replacement")
        .unwrap();
    root.child("bad-é.txt").write_str("accent").unwrap();

    let plan = build_plan(root.path(), &["bad-\u{fffd}.txt".to_owned()]).unwrap();
    assert_eq!(plan.direct_match_count(), 1);
    assert_eq!(plan.keep_items().len(), 1);
    assert_eq!(plan.keep_items()[0].relative_path(), "bad-\u{fffd}.txt");
    assert_eq!(plan.delete_items().len(), 1);
    assert_eq!(plan.delete_items()[0].relative_path(), "bad-é.txt");
    root.child("bad-\u{fffd}.txt").assert("replacement");
    root.child("bad-é.txt").assert("accent");
}

#[cfg(unix)]
#[test]
fn unicode_links_do_not_require_unicode_targets() {
    let temp = assert_fs::TempDir::new().unwrap();
    let outside = assert_fs::TempDir::new().unwrap();
    let target = outside.path().join(support::non_unicode_name(0xff));
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join(support::non_unicode_name(0xfe)), "outside").unwrap();
    std::os::unix::fs::symlink(&target, temp.path().join("link")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join(support::non_unicode_name(0xfd)),
        temp.path().join("dangling"),
    )
    .unwrap();

    let plan = build_plan(temp.path(), &["link".to_owned()]).unwrap();
    assert_eq!(plan.keep_items().len(), 1);
    assert_eq!(plan.keep_items()[0].item_type(), ItemType::Symlink);
    assert_eq!(plan.delete_items().len(), 1);
    assert_eq!(plan.delete_items()[0].relative_path(), "dangling");
    assert_eq!(
        std::fs::read(target.join(support::non_unicode_name(0xfe))).unwrap(),
        b"outside"
    );
    let alias = outside.path().join(support::non_unicode_name(0xfc));
    std::os::unix::fs::symlink(temp.path(), &alias).unwrap();
    let aliased_plan = build_plan(&alias, &["link".to_owned()]).unwrap();
    assert_eq!(
        aliased_plan.root(),
        std::fs::canonicalize(temp.path()).unwrap()
    );
}

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
fn validation_errors_are_identifiable_by_type() {
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("file.txt").write_str("file").unwrap();
    let root = temp.path();

    for (root, patterns) in [
        (root.to_path_buf(), vec![]),
        (root.to_path_buf(), vec![String::new()]),
        (root.to_path_buf(), vec!["[".to_owned()]),
        (root.to_path_buf(), vec!["../outside".to_owned()]),
        (root.join("missing"), vec!["*".to_owned()]),
        (root.join("file.txt"), vec!["*".to_owned()]),
    ] {
        let error = build_plan(&root, &patterns).unwrap_err();
        assert!(
            error.downcast_ref::<ValidationError>().is_some(),
            "root: {}, patterns: {patterns:?}",
            root.display()
        );
    }
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
