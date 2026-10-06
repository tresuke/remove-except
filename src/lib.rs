//! パターンに一致した項目を保持し、それ以外を削除する計画を作成します。
//!
//! [`build_plan`] はルートディレクトリ自体を残したまま、その配下を走査します。
//! 保持パターンは複数指定でき、いずれかに一致した項目とその祖先を保持します。
//! 計画作成はファイルシステムを変更しません。返される計画は走査時点の情報であり、
//! その後のファイルシステム変更を反映したり、削除を実行したりはしません。
//! ケース照合はWindowsではUnicode単純ケースフォールド（非トルコ語）、Ubuntuでは
//! 大文字小文字を区別します。Windowsのディレクトリ単位のケースセンシティブ設定には追従しません。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

mod model;
mod path;
mod pattern;

pub use model::{ItemType, PlannedItem, RemovalPlan};
use path::{display_path, is_drive_relative_path, is_filesystem_root};
use pattern::compile_patterns;

/// ルート配下を走査し、保持・削除対象の計画を作成します。
///
/// パターンは保持対象を指定します。複数のパターンを指定した場合はいずれかに
/// 一致した項目を保持し、その祖先も保持します。`*` と `?` はパス区切りをまたがず、
/// `**` は複数階層に一致します。リテラルのディレクトリ指定は配下も再帰的に保持します。
/// 相対ルートはプロセスのカレントディレクトリ基準で解決し、相対パターンはルート基準です。
///
/// シンボリックリンクはたどらず、リンク自体を1項目として扱います。ルートディレクトリ
/// 自体は保持・削除一覧に含まれません。計画作成はファイルを変更せず、結果は走査時点の
/// スナップショットです。計画後にファイルシステムが変更されても自動で再検証されません。
///
/// # Errors
///
/// パターンが未指定または空、無効なglob、親ディレクトリへの移動を含む、またはルート外の
/// 絶対パスを指す場合にエラーを返します。filesystem rootやディレクトリ以外をルートに
/// 指定した場合、ルートの解決や走査に失敗した場合もエラーになります。詳細なエラー文言は
/// 安定したAPI契約ではありません。
///
/// # Examples
///
/// ```no_run
/// use remove_except::build_plan;
/// use std::path::Path;
///
/// let plan = build_plan(Path::new("work"), &["*.md".to_owned()])
///     .expect("the root and pattern should be valid");
/// for item in plan.delete_roots() {
///     println!("{}", item.relative_path());
/// }
/// ```
pub fn build_plan(root: &Path, patterns: &[String]) -> Result<RemovalPlan> {
    if patterns.is_empty() {
        bail!("At least one pattern is required");
    }
    if is_drive_relative_path(&root.to_string_lossy()) {
        bail!(
            "Drive-relative root paths are not supported: {}",
            root.display()
        );
    }

    let root = std::fs::canonicalize(root)
        .with_context(|| format!("Failed to resolve root directory: {}", root.display()))?;
    if is_filesystem_root(&root) {
        bail!(
            "Filesystem roots cannot be used as processing roots: {}",
            root.display()
        );
    }
    if !root.is_dir() {
        bail!("Root path is not a directory: {}", root.display());
    }

    let compiled_patterns = compile_patterns(&root, patterns)?;
    let mut entries = Vec::new();
    // WalkDirは深さ優先で項目を返すため、現在項目の祖先ディレクトリのindexを保持する。
    let mut directory_stack = Vec::new();
    let mut direct_match_count = 0;

    for entry in WalkDir::new(&root).follow_links(false).min_depth(1) {
        let entry = entry.with_context(|| "Failed to walk root directory")?;
        let depth = entry.depth();
        directory_stack.truncate(depth.saturating_sub(1));
        // 親は子より先に訪れるため、そのindexは常に現在項目より小さい。
        let parent_index = directory_stack.last().copied();
        let file_type = entry.file_type();
        let path = entry.path().to_path_buf();
        let relative = path
            .strip_prefix(&root)
            .context("Walked path escaped the root directory")?;
        let relative_path = display_path(relative);
        let matched = compiled_patterns.matches(&relative_path);

        if matched {
            direct_match_count += 1;
        }

        let item_type = if file_type.is_symlink() {
            ItemType::Symlink
        } else if file_type.is_dir() {
            ItemType::Directory
        } else {
            ItemType::File
        };
        let item_index = entries.len();
        entries.push(IndexedItem {
            item: PlannedItem {
                path,
                relative_path,
                item_type,
            },
            parent_index,
            keep: matched,
            is_delete_root: false,
        });
        if item_type == ItemType::Directory {
            directory_stack.push(item_index);
        }
    }

    Ok(finalize_plan(root, direct_match_count, entries))
}

/// 走査時に記録した親indexから保持祖先と最上位の削除項目を確定します。
fn finalize_plan(
    root: PathBuf,
    direct_match_count: usize,
    mut entries: Vec<IndexedItem>,
) -> RemovalPlan {
    // 子は親より後ろにあるため、逆順にたどって一致項目から祖先へ保持状態を伝播する。
    for index in (0..entries.len()).rev() {
        if entries[index].keep
            && let Some(parent_index) = entries[index].parent_index
        {
            entries[parent_index].keep = true;
        }
    }

    // 保持されない項目の親が保持対象か処理ルートなら、その項目を削除ルートとする。
    for index in 0..entries.len() {
        let parent_is_kept = entries[index]
            .parent_index
            .is_none_or(|parent_index| entries[parent_index].keep);
        entries[index].is_delete_root = !entries[index].keep && parent_is_kept;
    }

    entries.sort_unstable_by(|left, right| left.item.relative_path.cmp(&right.item.relative_path));
    let mut keep_items = Vec::new();
    let mut delete_items = Vec::new();
    let mut delete_roots = Vec::new();
    for entry in entries {
        if entry.keep {
            keep_items.push(entry.item);
        } else {
            if entry.is_delete_root {
                delete_roots.push(entry.item.clone());
            }
            delete_items.push(entry.item);
        }
    }

    RemovalPlan {
        root,
        direct_match_count,
        keep_items,
        delete_items,
        delete_roots,
    }
}

/// 走査順序を使って保持祖先と最上位の削除項目を導出するための情報です。
struct IndexedItem {
    /// 公開計画に含める項目です。
    item: PlannedItem,
    /// 処理ルート直下でない場合、走査順序ベクター内にある親ディレクトリーのindexです。
    parent_index: Option<usize>,
    /// 直接一致で初期化し、一致項目の祖先へ保持状態を伝播します。
    keep: bool,
    /// 実際の削除処理に渡す最上位項目の場合にtrueです。
    is_delete_root: bool,
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::path::is_drive_relative_path;
    #[cfg(windows)]
    use super::pattern::normalize_pattern;
    use super::{ItemType, build_plan, is_filesystem_root};
    use assert_fs::prelude::*;
    use std::path::Path;

    #[test]
    // UnixとWindowsのファイルシステムルートを正しく判定することを確認する。
    fn identifies_filesystem_root_paths() {
        assert!(is_filesystem_root(Path::new("/")));

        #[cfg(windows)]
        {
            assert!(is_filesystem_root(Path::new("C:\\")));
            assert!(is_filesystem_root(Path::new("\\\\server\\share\\")));
            assert!(!is_filesystem_root(Path::new("C:\\folder")));
        }
    }

    #[cfg(windows)]
    #[test]
    // Windowsのドライブ相対パスをルートと保持パターンの両方で拒否することを確認する。
    fn rejects_drive_relative_roots_and_patterns() {
        assert!(is_drive_relative_path("C:folder"));
        assert!(!is_drive_relative_path("C:\\folder"));
        assert!(!is_drive_relative_path("\\\\server\\share\\folder"));

        let root_error = build_plan(Path::new("C:folder"), &["*".to_owned()]).unwrap_err();
        assert!(root_error.to_string().contains("Drive-relative root"));

        let pattern_error = normalize_pattern("C:/root", "C:folder").unwrap_err();
        assert!(pattern_error.to_string().contains("Drive-relative pattern"));
    }

    #[test]
    // ファイルシステムルートを走査開始前に拒否することを確認する。
    fn rejects_filesystem_root_before_walking_it() {
        let error = build_plan(Path::new("/"), &["*".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("Filesystem roots"));
    }

    #[test]
    // パターン未指定、空パターン、不正なglobを拒否することを確認する。
    fn rejects_empty_and_invalid_patterns() {
        let temp = assert_fs::TempDir::new().unwrap();

        let missing_error = build_plan(temp.path(), &[]).unwrap_err();
        assert!(missing_error.to_string().contains("At least one pattern"));

        let empty_error = build_plan(temp.path(), &[String::new()]).unwrap_err();
        assert!(
            empty_error
                .to_string()
                .contains("Pattern must not be empty")
        );

        let invalid_error = build_plan(temp.path(), &["[".to_owned()]).unwrap_err();
        assert!(invalid_error.to_string().contains("Invalid glob pattern"));
    }

    #[test]
    // ディレクトリ以外を処理ルートとして指定すると拒否することを確認する。
    fn rejects_non_directory_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        let root_file = temp.child("root.txt");
        root_file.write_str("not a directory").unwrap();

        let error = build_plan(root_file.path(), &["*".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("Root path is not a directory"));
    }

    #[test]
    // パターン一致項目と祖先を保持し、それ以外を削除計画に含めることを確認する。
    fn keeps_matching_item_and_its_ancestors() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep/nested/file.txt")
            .write_str("keep")
            .unwrap();
        temp.child("remove.txt").write_str("remove").unwrap();

        let plan = build_plan(temp.path(), &["keep/nested/file.txt".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 1);
        assert_eq!(plan.keep_items.len(), 3);
        assert!(
            plan.keep_items
                .iter()
                .any(|item| item.relative_path == "keep/nested/file.txt")
        );
        assert_eq!(plan.delete_items.len(), 1);
        assert_eq!(plan.delete_items[0].relative_path, "remove.txt");
        assert_eq!(plan.delete_items[0].item_type, ItemType::File);
        assert_eq!(plan.delete_roots.len(), 1);
    }

    #[test]
    // リテラルのディレクトリ指定が配下全体を保持することを確認する。
    fn directory_prefix_keeps_its_contents() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep/deep/file.txt").write_str("keep").unwrap();
        temp.child("remove/file.txt").write_str("remove").unwrap();

        let plan = build_plan(temp.path(), &["keep".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 3);
        assert_eq!(plan.keep_items.len(), 3);
        assert!(
            plan.keep_items
                .iter()
                .any(|item| item.relative_path == "keep/deep/file.txt")
        );
        assert_eq!(plan.delete_items.len(), 2);
        assert_eq!(plan.delete_roots.len(), 1);
        assert_eq!(plan.delete_roots[0].relative_path, "remove");
    }

    #[test]
    // * は直下、** は複数階層のパスに一致することを確認する。
    fn globstar_matches_nested_paths_while_star_matches_one_level() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("root.md").write_str("keep").unwrap();
        temp.child("nested/child.md").write_str("keep").unwrap();
        temp.child("nested/deep/annual.md")
            .write_str("keep")
            .unwrap();

        let single_level = build_plan(temp.path(), &["*.md".to_owned()]).unwrap();
        assert_eq!(single_level.direct_match_count, 1);
        assert!(
            single_level
                .delete_items
                .iter()
                .any(|item| item.relative_path == "nested/child.md")
        );

        let all_levels = build_plan(temp.path(), &["**/*.md".to_owned()]).unwrap();
        assert_eq!(all_levels.direct_match_count, 3);
        assert!(all_levels.delete_items.is_empty());
    }

    #[test]
    // ? と文字クラスがそれぞれ1文字に一致することを確認する。
    fn question_mark_and_character_class_match_one_character() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("file1.c").write_str("keep").unwrap();
        temp.child("file2.h").write_str("keep").unwrap();
        temp.child("file12.c").write_str("remove").unwrap();
        temp.child("file3.txt").write_str("remove").unwrap();

        let plan = build_plan(temp.path(), &["file?.[ch]".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 2);
        assert_eq!(plan.delete_items.len(), 2);
        assert!(
            plan.delete_items
                .iter()
                .any(|item| item.relative_path == "file12.c")
        );
        assert!(
            plan.delete_items
                .iter()
                .any(|item| item.relative_path == "file3.txt")
        );
    }

    #[test]
    // dir/* が直下の項目だけに一致し、子孫までは保持しないことを確認する。
    fn single_level_glob_does_not_keep_nested_directory_contents() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("dir/top.txt").write_str("keep").unwrap();
        temp.child("dir/sub/nested.txt")
            .write_str("remove")
            .unwrap();

        let plan = build_plan(temp.path(), &["dir/*".to_owned()]).unwrap();

        assert_eq!(plan.direct_match_count, 2);
        assert_eq!(plan.delete_items.len(), 1);
        assert_eq!(plan.delete_items[0].relative_path, "dir/sub/nested.txt");
    }

    #[test]
    // 親ディレクトリへの移動を含む保持パターンを拒否することを確認する。
    fn rejects_parent_traversal() {
        let temp = assert_fs::TempDir::new().unwrap();
        let error = build_plan(temp.path(), &["../outside".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("Parent directory traversal"));
    }

    #[test]
    // ルート外を指す絶対保持パターンを拒否することを確認する。
    fn absolute_pattern_must_stay_within_root_boundary() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep.txt").write_str("keep").unwrap();
        let sibling = temp.path().with_file_name(format!(
            "{}-sibling",
            temp.path().file_name().unwrap().to_string_lossy()
        ));
        let outside_pattern = sibling.join("*").to_string_lossy().into_owned();

        let error = build_plan(temp.path(), &[outside_pattern]).unwrap_err();

        assert!(error.to_string().contains("outside the processing root"));
    }

    #[test]
    // ルート内の絶対保持パターンで対象項目を保持できることを確認する。
    fn absolute_keep_pattern_matches_inside_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep.txt").write_str("keep").unwrap();
        temp.child("remove.txt").write_str("remove").unwrap();
        let keep = temp.path().join("keep.txt").to_string_lossy().into_owned();

        let plan = build_plan(temp.path(), &[keep]).unwrap();

        assert_eq!(plan.delete_items.len(), 1);
        assert_eq!(plan.delete_items[0].relative_path, "remove.txt");
    }

    #[test]
    // Globの大文字・小文字の照合がプラットフォームごとの仕様に従うことを確認する。
    fn glob_case_sensitivity_matches_platform() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("Keep.txt").write_str("keep").unwrap();

        let plan = build_plan(temp.path(), &["keep.txt".to_owned()]).unwrap();

        assert_eq!(plan.delete_items.is_empty(), cfg!(windows));
    }

    #[cfg(windows)]
    #[test]
    // Windowsのルート相対パターンが選択ルート外なら拒否することを確認する。
    fn rejects_windows_root_relative_pattern_outside_current_root() {
        let temp = assert_fs::TempDir::new().unwrap();

        let error = build_plan(temp.path(), &["/outside/*".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("outside the processing root"));
    }
}
