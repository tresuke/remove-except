//! カレントディレクトリ配下の保持・削除計画を表示し、確認後に削除します。

use std::cmp::Ordering;
use std::fs;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::builder::styling::{AnsiColor, Styles};
use clap::{CommandFactory, Parser, ValueEnum};
use console::style;
use dialoguer::Confirm;
use remove_except::{ItemType, PlannedItem, build_plan};

const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().bold())
    .usage(AnsiColor::Cyan.on_default().bold())
    .literal(AnsiColor::Green.on_default().bold())
    .placeholder(AnsiColor::Yellow.on_default());

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Keep matching items and remove the rest under a processing root",
    long_about = "Keep items matching one or more paths or glob patterns and remove the rest under the selected root. Multiple patterns are combined with OR. Always review the plan before deleting items.",
    after_long_help = "Output:\n  Tree: directories end with / and symlinks with @. With color, directories are blue, symlinks cyan, and kept parent directories in the delete tree green. Without color, kept parents end with +.\n  Flat: ItemType identifies each entry. Without color, directory and symlink paths end with / and @. Flat output has no kept-parent marker.\n  Status labels: PREVIEW ONLY (cyan), CONFIRM TO DELETE (yellow), FORCE DELETE (red), NO REMOVALS (green).\n  Colors are used automatically when output is a terminal and color is available.",
    styles = HELP_STYLES
)]
struct Args {
    #[arg(
        long,
        value_name = "PATH",
        help = "Directory to process (defaults to the current directory)"
    )]
    root: Option<PathBuf>,

    #[arg(
        short = 'n',
        long,
        help = "Preview the deletion plan without confirming or deleting anything"
    )]
    dry_run: bool,

    #[arg(short = 'f', long, help = "Delete without the confirmation prompt")]
    force: bool,

    #[arg(
        long,
        value_enum,
        value_name = "SECTION",
        help = "Summarize paths in SECTION: keep, delete, or both"
    )]
    summary: Option<SummarySection>,

    #[arg(
        long,
        value_enum,
        value_name = "LAYOUT",
        default_value = "tree",
        help = "Display paths as a tree or a flat list"
    )]
    layout: Layout,

    #[arg(
        long,
        help = "Sort flat output by item type, then relative path (requires --layout flat)"
    )]
    sort_by_type: bool,

    #[arg(
        required = true,
        num_args = 1..,
        value_name = "KEEP_PATTERN",
        help = "Paths or glob patterns to keep (* matches one level, ** recurses); any match keeps an item"
    )]
    patterns: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Layout {
    Tree,
    Flat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SummarySection {
    Keep,
    Delete,
    Both,
}

struct PlanDisplayOptions {
    flat: bool,
    sort_by_type: bool,
    direct_match_count: usize,
    delete_root_count: usize,
    colors_enabled: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.sort_by_type && args.layout != Layout::Flat {
        Args::command()
            .error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "--sort-by-type requires --layout flat",
            )
            .exit();
    }
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    let root = resolve_root(&current_dir, args.root)?;
    let plan = build_plan(&root, &args.patterns)?;

    if plan.direct_match_count() == 0 {
        eprintln!(
            "WARNING: No items matched the keep patterns. Everything under {} would be removed.",
            escape_control_characters(&display_root_path(plan.root()))
        );
    }

    let (keep_summary, delete_summary) = match args.summary {
        Some(SummarySection::Keep) => (true, false),
        Some(SummarySection::Delete) => (false, true),
        Some(SummarySection::Both) => (true, true),
        None => (false, false),
    };
    print_plan(
        &plan,
        args.dry_run,
        args.force,
        keep_summary,
        delete_summary,
        args.layout == Layout::Flat,
        args.sort_by_type,
    )?;

    if args.dry_run || plan.delete_roots().is_empty() {
        return Ok(());
    }

    if !args.force && !confirm_deletion(keep_summary || delete_summary)? {
        println!("Aborted.");
        return Ok(());
    }

    for item in plan.delete_roots() {
        remove_item(item).with_context(|| {
            format!(
                "Failed to remove {}",
                escape_control_characters(&item.path().to_string_lossy())
            )
        })?;
    }

    println!("Removal complete.");
    Ok(())
}

fn resolve_root(current_dir: &Path, root: Option<PathBuf>) -> Result<PathBuf> {
    let Some(root) = root else {
        return Ok(current_dir.to_path_buf());
    };

    if is_drive_relative_path(&root) {
        bail!(
            "Drive-relative root paths are not supported: {}",
            root.display()
        );
    }

    if root.is_absolute() {
        Ok(root)
    } else {
        Ok(current_dir.join(root))
    }
}

fn is_drive_relative_path(path: &Path) -> bool {
    let path = path.as_os_str().to_string_lossy();
    let bytes = path.as_bytes();
    cfg!(windows)
        && bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && !bytes
            .get(2)
            .is_some_and(|separator| matches!(separator, b'/' | b'\\'))
}

fn display_root_path(path: &Path) -> String {
    let path_text = path.to_string_lossy();

    #[cfg(windows)]
    {
        if let Some(extended_path) = path_text.strip_prefix("\\\\?\\") {
            let unc_path = extended_path.strip_prefix("UNC\\").or_else(|| {
                extended_path
                    .get(..4)
                    .filter(|prefix| prefix.eq_ignore_ascii_case("UNC\\"))
                    .map(|_| &extended_path[4..])
            });
            if let Some(unc_path) = unc_path.filter(|unc_path| {
                is_plain_windows_display_path(unc_path)
                    && unc_path.split('\\').filter(|part| !part.is_empty()).count() >= 2
            }) {
                return format!("\\\\{unc_path}");
            }

            let bytes = extended_path.as_bytes();
            if bytes.first().is_some_and(u8::is_ascii_alphabetic)
                && bytes.get(1) == Some(&b':')
                && bytes.get(2) == Some(&b'\\')
                && is_plain_windows_display_path(extended_path)
            {
                return extended_path.to_owned();
            }
        }
    }

    path_text.into_owned()
}

fn escape_control_characters(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            escaped.extend(character.escape_debug());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

#[cfg(windows)]
fn is_plain_windows_display_path(path: &str) -> bool {
    path.split(['\\', '/']).all(|component| {
        component.is_empty() || (!component.ends_with(' ') && !component.ends_with('.'))
    })
}

fn confirm_deletion(has_summaries: bool) -> Result<bool> {
    let prompt = if has_summaries {
        "Delete the plan? Summary sections do not list item paths."
    } else {
        "Are you sure you want to delete the above items?"
    };

    if io::stdin().is_terminal() {
        return Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()
            .context("Failed to read confirmation");
    }

    let stderr = io::stderr();
    let mut prompt_output = stderr.lock();
    write!(prompt_output, "{prompt} [y/N] ")?;
    prompt_output.flush()?;
    drop(prompt_output);
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn print_plan(
    plan: &remove_except::RemovalPlan,
    dry_run: bool,
    force: bool,
    keep_summary: bool,
    delete_summary: bool,
    flat: bool,
    sort_by_type: bool,
) -> io::Result<()> {
    let colors_enabled = io::stdout().is_terminal() && console::colors_enabled();
    let mut output = BufWriter::new(io::stdout().lock());
    render_plan(
        &mut output,
        plan,
        dry_run,
        force,
        keep_summary,
        delete_summary,
        flat,
        sort_by_type,
        colors_enabled,
    )?;
    output.flush()
}

#[allow(clippy::too_many_arguments)]
fn render_plan(
    mut output: &mut impl Write,
    plan: &remove_except::RemovalPlan,
    dry_run: bool,
    force: bool,
    keep_summary: bool,
    delete_summary: bool,
    flat: bool,
    sort_by_type: bool,
    colors_enabled: bool,
) -> io::Result<()> {
    let (status_label, status_detail) = if dry_run {
        ("PREVIEW ONLY", "Nothing will be removed")
    } else if plan.delete_roots().is_empty() {
        ("NO REMOVALS", "No deletion operations are needed")
    } else if force {
        (
            "FORCE DELETE",
            "Confirmation is disabled; validation still applies",
        )
    } else {
        ("CONFIRM TO DELETE", "Review both lists before confirming")
    };
    let mode = if colors_enabled {
        if dry_run {
            style(status_label).cyan().bold().to_string()
        } else if plan.delete_roots().is_empty() {
            style(status_label).green().bold().to_string()
        } else if force {
            style(status_label).red().bold().to_string()
        } else {
            style(status_label).yellow().bold().to_string()
        }
    } else {
        status_label.to_owned()
    };

    writeln!(output, "{mode}  {status_detail}")?;
    writeln!(
        output,
        "Root: {}",
        escape_control_characters(&display_root_path(plan.root()))
    )?;
    let show_legend = (!keep_summary && !plan.keep_items().is_empty())
        || (!delete_summary && !plan.delete_items().is_empty());
    if show_legend {
        print_legend(output, flat, colors_enabled)?;
    }
    writeln!(output)?;
    let options = PlanDisplayOptions {
        flat,
        sort_by_type,
        direct_match_count: plan.direct_match_count(),
        delete_root_count: plan.delete_roots().len(),
        colors_enabled,
    };
    print_item_section(
        &mut output,
        "Items to keep",
        plan.keep_items(),
        keep_summary,
        &options,
    )?;
    writeln!(output)?;
    print_item_section(
        &mut output,
        "Items to delete",
        plan.delete_items(),
        delete_summary,
        &options,
    )?;

    if keep_summary || delete_summary {
        writeln!(output)?;
        writeln!(
            output,
            "Summary sections omit paths. Run --dry-run without --summary to review every item before deleting."
        )?;
    }

    Ok(())
}

fn print_legend(output: &mut impl Write, flat: bool, colors_enabled: bool) -> io::Result<()> {
    if colors_enabled {
        write!(
            output,
            "Colors: directories ({}); symlinks ({})",
            style("blue").blue(),
            style("cyan").cyan()
        )?;
        if !flat {
            write!(output, "; kept parents ({})", style("green").green())?;
        }
        writeln!(output, ".")
    } else if flat {
        writeln!(output, "Markers: / directory; @ symlink.")
    } else {
        writeln!(output, "Markers: / directory; @ symlink; + kept parent.")
    }
}

fn print_item_section(
    output: &mut impl Write,
    title: &str,
    items: &[PlannedItem],
    summary_only: bool,
    options: &PlanDisplayOptions,
) -> io::Result<()> {
    let is_keep_section = title == "Items to keep";
    let title = if options.colors_enabled {
        if is_keep_section {
            style(title).green().bold().to_string()
        } else {
            style(title).red().bold().to_string()
        }
    } else {
        title.to_owned()
    };

    if summary_only {
        let summary_title = if is_keep_section { "KEEP" } else { "DELETE" };
        writeln!(output, "{summary_title}:")?;
        if is_keep_section {
            let item_label = if items.len() == 1 { "item" } else { "items" };
            let match_label = if options.direct_match_count == 1 {
                "match"
            } else {
                "matches"
            };
            writeln!(
                output,
                "  {} kept {item_label} (including required ancestors); {} direct pattern {match_label}",
                items.len(),
                options.direct_match_count
            )?;
        } else {
            let item_label = if items.len() == 1 { "item" } else { "items" };
            let operation_label = if options.delete_root_count == 1 {
                "operation"
            } else {
                "operations"
            };
            writeln!(
                output,
                "  {} deletion {item_label}; {} top-level deletion {operation_label}",
                items.len(),
                options.delete_root_count
            )?;
        }
        return Ok(());
    }

    writeln!(output, "{title} ({}):", items.len())?;
    if items.is_empty() {
        writeln!(output, "(None)")?;
        return Ok(());
    }

    if options.flat {
        writeln!(output, "{:<12}  RelativePath", "ItemType")?;
        writeln!(output, "{:-<12}  {:-<12}", "", "")?;
        let mut ordered_items: Vec<_> = items.iter().collect();
        if options.sort_by_type {
            ordered_items.sort_unstable_by(|left, right| {
                flat_item_type_rank(left.item_type())
                    .cmp(&flat_item_type_rank(right.item_type()))
                    .then_with(|| left.relative_path().cmp(right.relative_path()))
            });
        }
        for item in ordered_items {
            let item_type = format!("{:<12}", item_type_name(item.item_type()));
            let suffix = if options.colors_enabled {
                ""
            } else {
                tree_item_suffix(item.item_type())
            };
            let relative_path =
                escape_control_characters(&format!("{}{suffix}", item.relative_path()));
            writeln!(
                output,
                "{}  {}",
                style_flat_item(&item_type, item.item_type(), options.colors_enabled),
                style_flat_item(&relative_path, item.item_type(), options.colors_enabled)
            )?;
        }
    } else {
        let mut ordered_items: Vec<_> = items.iter().collect();
        ordered_items.sort_unstable_by(|left, right| compare_tree_order(left, right));
        print_tree_items(output, &ordered_items, "", "", options.colors_enabled)?;
    }
    Ok(())
}

fn print_tree_items(
    output: &mut impl Write,
    items: &[&PlannedItem],
    parent_path: &str,
    branch_prefix: &str,
    colors_enabled: bool,
) -> io::Result<()> {
    let mut start = 0;
    while start < items.len() {
        let remainder = if parent_path.is_empty() {
            items[start].relative_path()
        } else {
            items[start]
                .relative_path()
                .strip_prefix(parent_path)
                .and_then(|path| path.strip_prefix('/'))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Invalid tree path prefix")
                })?
        };
        let name = remainder.split('/').next().unwrap_or(remainder);
        let child_path = if parent_path.is_empty() {
            name.to_owned()
        } else {
            format!("{parent_path}/{name}")
        };
        let child_prefix = format!("{child_path}/");
        let mut end = start + 1;
        while end < items.len()
            && (items[end].relative_path() == child_path
                || items[end].relative_path().starts_with(&child_prefix))
        {
            end += 1;
        }

        let item = (items[start].relative_path() == child_path).then_some(items[start]);
        let descendants_start = start + usize::from(item.is_some());
        let is_directory = item.is_none_or(|item| item.item_type() == ItemType::Directory)
            || descendants_start < end;
        let is_last = end == items.len();
        let connector = if is_last { "└──" } else { "├──" };
        let next_prefix = format!("{branch_prefix}{}", if is_last { "    " } else { "│   " });
        let suffix = item
            .map(|item| tree_item_suffix(item.item_type()))
            .unwrap_or(if is_directory { "/" } else { "" });
        let is_kept_parent = item.is_none();
        let display_name = escape_control_characters(&format!("{name}{suffix}"));
        let display_name = if colors_enabled {
            match item.map(|item| item.item_type()) {
                Some(ItemType::Directory) => style(display_name).blue().bold().to_string(),
                Some(ItemType::Symlink) => style(display_name).cyan().bold().to_string(),
                _ if is_kept_parent => style(display_name).green().bold().to_string(),
                _ => display_name,
            }
        } else {
            display_name
        };
        let kept_parent_marker = if is_kept_parent && !colors_enabled {
            "+"
        } else {
            ""
        };
        writeln!(
            output,
            "{branch_prefix}{connector} {display_name}{kept_parent_marker}"
        )?;

        print_tree_items(
            output,
            &items[descendants_start..end],
            &child_path,
            &next_prefix,
            colors_enabled,
        )?;
        start = end;
    }
    Ok(())
}

fn compare_tree_order(left: &PlannedItem, right: &PlannedItem) -> Ordering {
    let mut left_parts = left.relative_path().split('/').peekable();
    let mut right_parts = right.relative_path().split('/').peekable();

    loop {
        match (left_parts.next(), right_parts.next()) {
            (Some(left_part), Some(right_part)) if left_part == right_part => {}
            (Some(left_part), Some(right_part)) => {
                let left_is_directory =
                    left_parts.peek().is_some() || left.item_type() == ItemType::Directory;
                let right_is_directory =
                    right_parts.peek().is_some() || right.item_type() == ItemType::Directory;
                return right_is_directory
                    .cmp(&left_is_directory)
                    .then_with(|| left_part.cmp(right_part));
            }
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

fn item_type_name(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::File => "File",
        ItemType::Directory => "Directory",
        ItemType::Symlink => "Symlink",
    }
}

fn flat_item_type_rank(item_type: ItemType) -> u8 {
    match item_type {
        ItemType::Directory => 0,
        ItemType::File => 1,
        ItemType::Symlink => 2,
    }
}

fn style_flat_item(text: &str, item_type: ItemType, colors_enabled: bool) -> String {
    if !colors_enabled {
        return text.to_owned();
    }

    match item_type {
        ItemType::Directory => style(text).blue().bold().to_string(),
        ItemType::File => text.to_owned(),
        ItemType::Symlink => style(text).cyan().bold().to_string(),
    }
}

fn tree_item_suffix(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Directory => "/",
        ItemType::File => "",
        ItemType::Symlink => "@",
    }
}

fn remove_item(item: &PlannedItem) -> std::io::Result<()> {
    match item.item_type() {
        ItemType::Directory => fs::remove_dir_all(item.path()),
        ItemType::File | ItemType::Symlink => remove_file_or_directory_link(item.path()),
    }
}

#[cfg(windows)]
fn remove_file_or_directory_link(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::FileTypeExt;

    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_dir() || file_type.is_symlink_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(not(windows))]
fn remove_file_or_directory_link(path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)
}

#[cfg(test)]
mod tests {
    use super::{Args, flat_item_type_rank, render_plan, style_flat_item, tree_item_suffix};
    use assert_fs::prelude::*;
    use clap::CommandFactory;
    use remove_except::{ItemType, build_plan};

    static COLOR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(windows)]
    use super::display_root_path;

    #[test]
    // ツリー表示用の接尾辞がディレクトリとシンボリックリンクを識別することを確認する。
    fn tree_item_suffix_identifies_directories_and_symlinks() {
        assert_eq!(tree_item_suffix(ItemType::Directory), "/");
        assert_eq!(tree_item_suffix(ItemType::File), "");
        assert_eq!(tree_item_suffix(ItemType::Symlink), "@");
    }

    #[test]
    fn escape_control_characters_keeps_paths_on_one_terminal_line() {
        assert_eq!(
            super::escape_control_characters("bad\n\u{1b}[31mname.txt"),
            r"bad\n\u{1b}[31mname.txt"
        );
    }

    #[cfg(windows)]
    #[test]
    fn display_root_path_shortens_standard_extended_drive_and_unc_paths() {
        assert_eq!(
            display_root_path(std::path::Path::new(r"\\?\S:\work\project")),
            r"S:\work\project"
        );
        assert_eq!(
            display_root_path(std::path::Path::new(r"\\?\UNC\server\share\project")),
            r"\\server\share\project"
        );
    }

    #[cfg(windows)]
    #[test]
    fn display_root_path_preserves_extended_prefix_for_special_components() {
        let special_path = r"\\?\S:\work\name.";
        assert_eq!(
            display_root_path(std::path::Path::new(special_path)),
            special_path
        );
    }

    #[test]
    // フラット表示の種類順がディレクトリ、ファイル、シンボリックリンクであることを確認する。
    fn flat_item_type_rank_orders_directories_files_and_symlinks() {
        assert!(flat_item_type_rank(ItemType::Directory) < flat_item_type_rank(ItemType::File));
        assert!(flat_item_type_rank(ItemType::File) < flat_item_type_rank(ItemType::Symlink));
    }

    #[test]
    // フラット表示でディレクトリとリンクに色を付け、ファイルは無色にすることを確認する。
    fn flat_item_styles_directories_and_symlinks_but_not_files() {
        let _color_test_guard = COLOR_TEST_LOCK.lock().unwrap();
        let colors_were_enabled = console::colors_enabled();
        console::set_colors_enabled(true);

        let directory = style_flat_item("Directory", ItemType::Directory, true);
        let file = style_flat_item("File", ItemType::File, true);
        let symlink = style_flat_item("Symlink", ItemType::Symlink, true);

        console::set_colors_enabled(colors_were_enabled);
        assert!(directory.contains("34m"));
        assert_eq!(file, "File");
        assert!(symlink.contains("36m"));
    }

    #[test]
    // ツリー表示で実ディレクトリ、シンボリックリンク、保持済み親を色分けすることを確認する。
    fn tree_item_styles_directories_symlinks_and_kept_parents() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("real-dir/file.txt").write_str("file").unwrap();
        temp.child("kept-parent/keep.txt")
            .write_str("keep")
            .unwrap();
        temp.child("kept-parent/remove.txt")
            .write_str("remove")
            .unwrap();
        let link = temp.path().join("real-dir/link");
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink("file.txt", &link);
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file("file.txt", &link);
        if link_result.is_err() {
            eprintln!("Skipping tree symlink style test because symlink creation is unavailable");
            return;
        }
        let plan = build_plan(temp.path(), &["kept-parent/keep.txt".to_owned()]).unwrap();

        let _color_test_guard = COLOR_TEST_LOCK.lock().unwrap();
        let colors_were_enabled = console::colors_enabled();
        console::set_colors_enabled(true);

        let options = super::PlanDisplayOptions {
            flat: false,
            sort_by_type: false,
            direct_match_count: 1,
            delete_root_count: 1,
            colors_enabled: true,
        };
        let mut output = Vec::new();
        super::print_item_section(
            &mut output,
            "Items to delete",
            plan.delete_items(),
            false,
            &options,
        )
        .unwrap();

        console::set_colors_enabled(colors_were_enabled);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\u{1b}[34m\u{1b}[1mreal-dir/"));
        assert!(output.contains("\u{1b}[36m\u{1b}[1mlink@"));
        assert!(output.contains("\u{1b}[32m\u{1b}[1mkept-parent/"));
        assert!(output.contains("remove.txt"));
    }

    #[test]
    fn render_plan_supports_colored_and_uncolored_output() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("group/keep.txt").write_str("keep").unwrap();
        temp.child("group/remove.txt").write_str("remove").unwrap();
        let plan = build_plan(temp.path(), &["group/keep.txt".to_owned()]).unwrap();

        let mut plain_output = Vec::new();
        render_plan(
            &mut plain_output,
            &plan,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
        )
        .unwrap();
        let plain_output = String::from_utf8(plain_output).unwrap();
        assert!(plain_output.starts_with("PREVIEW ONLY  Nothing will be removed"));
        let plain_lines: Vec<_> = plain_output.lines().collect();
        assert!(plain_lines[1].starts_with("Root: "));
        assert_eq!(
            plain_lines[2],
            "Markers: / directory; @ symlink; + kept parent."
        );
        assert!(plain_lines[3].is_empty());
        assert!(plain_output.contains("Markers: / directory; @ symlink; + kept parent."));
        assert!(plain_output.contains("group/+"));
        assert!(!plain_output.contains('\u{1b}'));

        let _color_test_guard = COLOR_TEST_LOCK.lock().unwrap();
        let colors_were_enabled = console::colors_enabled();
        console::set_colors_enabled(true);
        let mut colored_output = Vec::new();
        render_plan(
            &mut colored_output,
            &plan,
            true,
            false,
            false,
            false,
            false,
            false,
            true,
        )
        .unwrap();
        console::set_colors_enabled(colors_were_enabled);
        let colored_output = String::from_utf8(colored_output).unwrap();
        assert!(colored_output.contains("\u{1b}[36m\u{1b}[1mPREVIEW ONLY"));
        assert!(colored_output.contains("Colors: directories ("));
        assert!(colored_output.contains("blue"));
        assert!(colored_output.contains("symlinks ("));
        assert!(colored_output.contains("cyan"));
        assert!(colored_output.contains("kept parents ("));
        assert!(colored_output.contains("green"));
        assert!(colored_output.contains("directories (\u{1b}[34mblue"));
        assert!(colored_output.contains("symlinks (\u{1b}[36mcyan"));
        assert!(colored_output.contains("kept parents (\u{1b}[32mgreen"));
        let colored_lines: Vec<_> = colored_output.lines().collect();
        assert!(colored_lines[1].starts_with("Root: "));
        assert!(colored_lines[2].starts_with("Colors: "));
        assert!(colored_lines[3].is_empty());
        assert!(colored_output.contains("\u{1b}[32m\u{1b}[1mgroup/"));
        assert!(colored_output.contains("remove.txt"));

        let mut flat_output = Vec::new();
        render_plan(
            &mut flat_output,
            &plan,
            true,
            false,
            false,
            false,
            true,
            false,
            true,
        )
        .unwrap();
        let flat_output = String::from_utf8(flat_output).unwrap();
        assert!(flat_output.contains("Colors: directories ("));
        assert!(flat_output.contains("blue"));
        assert!(flat_output.contains("symlinks ("));
        assert!(flat_output.contains("cyan"));
        assert!(flat_output.contains("directories (\u{1b}[34mblue"));
        assert!(flat_output.contains("symlinks (\u{1b}[36mcyan"));
        assert!(!flat_output.contains("kept parents green"));
    }

    #[test]
    fn render_plan_uses_distinct_status_labels_for_each_execution_mode() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("keep.txt").write_str("keep").unwrap();
        temp.child("remove.txt").write_str("remove").unwrap();
        let plan_with_deletions = build_plan(temp.path(), &["keep.txt".to_owned()]).unwrap();
        let plan_without_deletions = build_plan(temp.path(), &[".".to_owned()]).unwrap();

        let render_status = |plan: &remove_except::RemovalPlan, dry_run, force, colors_enabled| {
            let mut output = Vec::new();
            render_plan(
                &mut output,
                plan,
                dry_run,
                force,
                false,
                false,
                false,
                false,
                colors_enabled,
            )
            .unwrap();
            String::from_utf8(output).unwrap()
        };

        assert!(
            render_status(&plan_with_deletions, true, false, false)
                .starts_with("PREVIEW ONLY  Nothing will be removed")
        );
        assert!(
            render_status(&plan_with_deletions, false, false, false)
                .starts_with("CONFIRM TO DELETE  Review both lists before confirming")
        );
        assert!(
            render_status(&plan_with_deletions, false, true, false)
                .starts_with("FORCE DELETE  Confirmation is disabled; validation still applies")
        );
        assert!(
            render_status(&plan_without_deletions, false, false, false)
                .starts_with("NO REMOVALS  No deletion operations are needed")
        );

        let _color_test_guard = COLOR_TEST_LOCK.lock().unwrap();
        let colors_were_enabled = console::colors_enabled();
        console::set_colors_enabled(true);
        let confirm_output = render_status(&plan_with_deletions, false, false, true);
        let force_output = render_status(&plan_with_deletions, false, true, true);
        let no_removals_output = render_status(&plan_without_deletions, false, false, true);
        console::set_colors_enabled(colors_were_enabled);

        assert!(confirm_output.contains("\u{1b}[33m"));
        assert!(force_output.contains("\u{1b}[31m"));
        assert!(no_removals_output.contains("\u{1b}[32m"));
    }

    #[test]
    fn help_styles_can_be_forced_for_tty_rendering() {
        let mut command = Args::command().color(clap::ColorChoice::Always);
        let help = command.render_long_help().ansi().to_string();

        assert!(help.contains("\u{1b}["));
        assert!(help.contains("Usage:"));
        assert!(help.contains("--dry-run"));
    }
}
