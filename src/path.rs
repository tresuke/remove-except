use std::path::{Component, Path};
#[cfg(windows)]
use unicode_casefold::{Locale, UnicodeCaseFold, Variant};

/// パスをGlob照合用に正規化します。Windowsでは区切りと拡張パス表記も正規化します。
pub(super) fn normalize_absolute_for_match(path: &str) -> String {
    let normalized = if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    if cfg!(windows) {
        if let Some(unc_path) = normalized.strip_prefix("//?/UNC/") {
            return format!("//{unc_path}");
        }
        if let Some(path) = normalized.strip_prefix("//?/") {
            return path.to_owned();
        }
    }
    normalized
}

/// パスが指定ディレクトリとその配下に含まれるか判定します。
pub(super) fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    strip_path_prefix(path, prefix)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
}

pub(super) fn paths_equal(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        simple_case_fold(left) == simple_case_fold(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn strip_path_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let mut path_chars = path.char_indices();
    let mut prefix_bytes = 0;
    for _ in prefix.chars() {
        let (index, character) = path_chars.next()?;
        prefix_bytes = index + character.len_utf8();
    }
    paths_equal(&path[..prefix_bytes], prefix).then_some(&path[prefix_bytes..])
}

#[cfg(windows)]
fn simple_case_fold(value: &str) -> String {
    value
        .case_fold_with(Variant::Simple, Locale::NonTurkic)
        .collect()
}

pub(super) fn is_absolute_pattern(pattern: &str) -> bool {
    Path::new(pattern).is_absolute()
        || (cfg!(windows)
            && (matches!(pattern.as_bytes().first(), Some(b'/' | b'\\'))
                || (pattern.as_bytes().get(1) == Some(&b':')
                    && pattern
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                    && pattern
                        .as_bytes()
                        .get(2)
                        .is_some_and(|separator| matches!(separator, b'/' | b'\\')))))
}

pub(super) fn is_drive_relative_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    cfg!(windows)
        && bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && !bytes
            .get(2)
            .is_some_and(|separator| matches!(separator, b'/' | b'\\'))
}

pub(super) fn is_filesystem_root(path: &Path) -> bool {
    path.components()
        .all(|component| !matches!(component, Component::Normal(_)))
}

/// 相対パスをプラットフォームに依存しない `/` 区切りの表示形式にします。
pub(super) fn display_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::{path_matches_prefix, paths_equal};

    #[test]
    fn path_case_comparison_matches_platform_policy() {
        assert_eq!(paths_equal("Ä", "ä"), cfg!(windows));
        assert_eq!(path_matches_prefix("Ä/file.txt", "ä"), cfg!(windows));
        assert!(!path_matches_prefix("Äother/file.txt", "ä"));
    }

    #[cfg(windows)]
    #[test]
    fn prefix_comparison_handles_different_utf8_byte_lengths() {
        assert!(path_matches_prefix("Keep/file.txt", "Keep"));
        assert!(path_matches_prefix("Keep", "Keep"));
        assert!(paths_equal("K", "k"));
        assert!(!paths_equal("ß", "SS"));
    }
}
