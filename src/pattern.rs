use std::path::Path;

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::path::{is_absolute_pattern, is_drive_relative_path, normalize_absolute_for_match};

pub(super) struct PatternSpec {
    pub(super) glob_index: usize,
    pub(super) prefix: Option<String>,
}

/// 保持パターンをGlobSetに変換し、リテラルのディレクトリ指定を記録します。
pub(super) fn compile_patterns(
    root: &Path,
    patterns: &[String],
) -> Result<(GlobSet, Vec<PatternSpec>)> {
    let root_text = normalize_absolute_for_match(&root.to_string_lossy());
    let mut builder = GlobSetBuilder::new();
    let mut specs = Vec::with_capacity(patterns.len());

    for raw in patterns {
        let raw = raw.trim();
        if raw.is_empty() {
            bail!("Pattern must not be empty");
        }
        if raw.split(['/', '\\']).any(|component| component == "..") {
            bail!("Parent directory traversal is not allowed in pattern: {raw}");
        }

        let pattern = normalize_pattern(&root_text, raw)?;
        let matcher = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .case_insensitive(cfg!(windows))
            .build()
            .with_context(|| format!("Invalid glob pattern: {raw}"))?;
        let glob_index = specs.len();
        builder.add(matcher);

        let prefix = if !pattern.contains(['*', '?', '[']) {
            Some(pattern.trim_end_matches('/').to_owned())
        } else {
            None
        };
        specs.push(PatternSpec { glob_index, prefix });
    }

    Ok((builder.build()?, specs))
}

/// 絶対パターンをルート相対へ変換し、ルート外の指定を拒否します。
pub(super) fn normalize_pattern(root_text: &str, pattern: &str) -> Result<String> {
    if is_drive_relative_path(pattern) {
        bail!("Drive-relative patterns are not supported: {pattern}");
    }

    let normalized = normalize_absolute_for_match(pattern);
    let root_text = normalize_absolute_for_match(root_text);

    if is_absolute_pattern(pattern) {
        let boundary_root = root_text.trim_end_matches('/');
        let root_matches = |candidate: &str| {
            if cfg!(windows) {
                candidate.eq_ignore_ascii_case(boundary_root)
            } else {
                candidate == boundary_root
            }
        };
        let is_within_root = root_matches(&normalized)
            || normalized.get(..boundary_root.len()).is_some_and(|prefix| {
                root_matches(prefix) && normalized[boundary_root.len()..].starts_with('/')
            });
        if !is_within_root {
            bail!("Absolute pattern is outside the processing root: {pattern}");
        }
        let relative = normalized[boundary_root.len()..].trim_start_matches('/');
        Ok(relative.to_owned())
    } else {
        let mut relative = normalized.as_str();
        while let Some(stripped) = relative.strip_prefix("./") {
            relative = stripped;
        }
        let relative = relative.trim_matches('/');
        Ok(relative.to_owned())
    }
}
