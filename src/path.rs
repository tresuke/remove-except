use std::path::{Component, Path};

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
    let starts_with_prefix = if cfg!(windows) {
        path.get(..prefix.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
    } else {
        path.starts_with(prefix)
    };
    starts_with_prefix
        && (path.len() == prefix.len()
            || path
                .get(prefix.len()..)
                .is_some_and(|suffix| suffix.starts_with('/')))
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
