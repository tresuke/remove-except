use std::path::Path;

use anyhow::{Context, Result, bail};
use globset::GlobBuilder;
#[cfg(not(windows))]
use globset::{GlobSet, GlobSetBuilder};
#[cfg(windows)]
use regex_automata::meta::Regex;

use crate::path::{
    is_absolute_pattern, is_drive_relative_path, normalize_absolute_for_match, path_matches_prefix,
    paths_equal,
};

#[cfg(windows)]
fn windows_glob_regex(glob: &globset::Glob) -> Result<String> {
    let byte_regex = glob
        .regex()
        .strip_prefix("(?-u)")
        .context("Glob regex did not use the expected byte mode prefix")?;
    let unicode_regex = decode_utf8_regex_literals(byte_regex)?;
    Ok(format!("(?i:{unicode_regex})"))
}

#[cfg(windows)]
fn decode_utf8_regex_literals(regex: &str) -> Result<String> {
    let bytes = regex.as_bytes();
    let mut decoded = String::with_capacity(regex.len());
    let mut offset = 0;

    while offset < bytes.len() {
        if let Some((byte, next_offset)) = regex_hex_escape(bytes, offset)
            && byte >= 0x80
        {
            let mut utf8 = vec![byte];
            offset = next_offset;
            while let Some((next_byte, after_escape)) = regex_hex_escape(bytes, offset) {
                if next_byte < 0x80 {
                    break;
                }
                utf8.push(next_byte);
                offset = after_escape;
            }
            let literal = std::str::from_utf8(&utf8)
                .context("Glob regex contained an invalid UTF-8 literal")?;
            decoded.push_str(literal);
            continue;
        }

        let character = regex[offset..]
            .chars()
            .next()
            .context("Glob regex ended unexpectedly")?;
        decoded.push(character);
        offset += character.len_utf8();
    }

    Ok(decoded)
}

#[cfg(windows)]
fn regex_hex_escape(bytes: &[u8], offset: usize) -> Option<(u8, usize)> {
    let escape = bytes.get(offset..offset + 4)?;
    if escape[0] != b'\\' || escape[1] != b'x' {
        return None;
    }
    let high = hex_digit(escape[2])?;
    let low = hex_digit(escape[3])?;
    Some(((high << 4) | low, offset + 4))
}

#[cfg(windows)]
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 保持パターンをGlobSetに変換し、リテラルのディレクトリ指定を記録します。
pub(super) fn compile_patterns(root: &Path, patterns: &[String]) -> Result<CompiledPatterns> {
    let root_text = normalize_absolute_for_match(&root.to_string_lossy());
    #[cfg(not(windows))]
    let mut builder = GlobSetBuilder::new();
    let mut prefixes = Vec::with_capacity(patterns.len());
    #[cfg(windows)]
    let mut regexes = Vec::with_capacity(patterns.len());

    for raw in patterns {
        let raw = raw.trim();
        if raw.is_empty() {
            bail!("Pattern must not be empty");
        }
        if raw.split(['/', '\\']).any(|component| component == "..") {
            bail!("Parent directory traversal is not allowed in pattern: {raw}");
        }

        let pattern = normalize_pattern(&root_text, raw)?;
        let glob = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("Invalid glob pattern: {raw}"))?;
        #[cfg(windows)]
        regexes.push(windows_glob_regex(&glob)?);
        #[cfg(not(windows))]
        builder.add(glob);

        if !pattern.contains(['*', '?', '[']) {
            prefixes.push(pattern.trim_end_matches('/').to_owned());
        }
    }

    #[cfg(windows)]
    let matcher = Regex::new_many(&regexes).context("Failed to compile keep patterns")?;
    #[cfg(not(windows))]
    let matcher = builder.build()?;

    Ok(CompiledPatterns { matcher, prefixes })
}

pub(super) struct CompiledPatterns {
    #[cfg(windows)]
    matcher: Regex,
    #[cfg(not(windows))]
    matcher: GlobSet,
    prefixes: Vec<String>,
}

impl CompiledPatterns {
    pub(super) fn matches(&self, path: &str) -> bool {
        #[cfg(windows)]
        if self.matcher.is_match(path.as_bytes()) {
            return true;
        }
        #[cfg(not(windows))]
        if self.matcher.is_match(path) {
            return true;
        }

        self.prefixes
            .iter()
            .any(|prefix| path_matches_prefix(path, prefix))
    }
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
        let Some(relative) = strip_case_insensitive_prefix(&normalized, boundary_root) else {
            bail!("Absolute pattern is outside the processing root: {pattern}");
        };
        Ok(relative.to_owned())
    } else {
        let mut relative = normalized.as_str();
        while let Some(stripped) = relative.strip_prefix("./") {
            relative = stripped;
        }
        let relative = relative.trim_matches('/');
        Ok(if relative == "." {
            String::new()
        } else {
            relative.to_owned()
        })
    }
}

fn strip_case_insensitive_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let mut path_chars = path.char_indices();
    let mut prefix_bytes = 0;
    for _ in prefix.chars() {
        let (index, character) = path_chars.next()?;
        prefix_bytes = index + character.len_utf8();
    }
    if !paths_equal(&path[..prefix_bytes], prefix) {
        return None;
    }
    let suffix = &path[prefix_bytes..];
    if suffix.is_empty() {
        Some(suffix)
    } else {
        suffix.strip_prefix('/')
    }
}

#[cfg(test)]
mod normalization_tests {
    use super::normalize_pattern;

    #[test]
    fn root_patterns_normalize_to_an_empty_relative_pattern() {
        assert_eq!(normalize_pattern("/work", ".").unwrap(), "");
        assert_eq!(normalize_pattern("/work", "./").unwrap(), "");
        assert_eq!(normalize_pattern("/work", "./.").unwrap(), "");
    }

    #[test]
    fn absolute_sibling_with_shared_text_prefix_is_outside_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        let root = temp.path().join("work");
        let sibling_pattern = temp.path().join("work-sibling").join("*");

        let error = normalize_pattern(&root.to_string_lossy(), &sibling_pattern.to_string_lossy())
            .unwrap_err();

        assert!(error.to_string().contains("outside the processing root"));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use anyhow::Context;
    use globset::GlobBuilder;
    use regex_automata::meta::Regex;

    use super::{decode_utf8_regex_literals, normalize_pattern, windows_glob_regex};

    fn compile_windows_glob(pattern: &str) -> Regex {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .unwrap();
        Regex::new(&windows_glob_regex(&glob).unwrap()).unwrap()
    }

    #[test]
    fn windows_glob_uses_unicode_simple_case_folding() {
        let umlaut = compile_windows_glob("ä.txt");
        assert!(umlaut.is_match("Ä.txt".as_bytes()));

        let sharp_s = compile_windows_glob("ß.txt");
        assert!(sharp_s.is_match("ẞ.txt".as_bytes()));
        assert!(!sharp_s.is_match("SS.txt".as_bytes()));
    }

    #[test]
    fn windows_question_mark_matches_one_unicode_scalar() {
        let matcher = compile_windows_glob("?.txt");
        assert!(matcher.is_match("é.txt".as_bytes()));
        assert!(!matcher.is_match("ee.txt".as_bytes()));
    }

    #[test]
    fn windows_character_classes_keep_glob_syntax_under_case_folding() {
        let matcher = compile_windows_glob("[Ä].txt");
        assert!(matcher.is_match("ä.txt".as_bytes()));
        assert!(!matcher.is_match("b.txt".as_bytes()));
    }

    #[test]
    fn windows_globs_preserve_recursive_and_single_level_separators() {
        let recursive = compile_windows_glob("root/**/*.txt");
        assert!(recursive.is_match("ROOT/Ä.txt".as_bytes()));
        assert!(recursive.is_match("root/nested/ä.txt".as_bytes()));

        let single_level = compile_windows_glob("root/*.txt");
        assert!(single_level.is_match("ROOT/Ä.txt".as_bytes()));
        assert!(!single_level.is_match("root/nested/ä.txt".as_bytes()));
    }

    #[test]
    fn absolute_pattern_boundary_uses_simple_fold_without_byte_length_assumptions() {
        assert_eq!(
            normalize_pattern("C:/Keep", "c:/Keep/file.txt").unwrap(),
            "file.txt"
        );
        assert!(normalize_pattern("C:/keep", "c:/keeping/file.txt").is_err());
    }

    #[test]
    fn decodes_non_ascii_literals_without_rewriting_regex_syntax() {
        let glob = GlobBuilder::new("[Ä]?.txt")
            .literal_separator(true)
            .build()
            .unwrap();
        let decoded = decode_utf8_regex_literals(
            glob.regex()
                .strip_prefix("(?-u)")
                .context("expected byte regex mode")
                .unwrap(),
        )
        .unwrap();
        assert!(decoded.contains("[Ä]"));
        assert!(decoded.contains("[^/]"));
    }
}
