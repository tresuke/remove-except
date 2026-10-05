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
            plan.root().display()
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
        remove_item(item).with_context(|| format!("Failed to remove {}", item.path().display()))?;
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
    let mode_text = if dry_run {
        "PREVIEW ONLY - nothing will be removed"
    } else if plan.delete_roots().is_empty() {
        "No removal operations are needed"
    } else if force {
        "DELETION ENABLED (--force)"
    } else {
        "DELETION AFTER CONFIRMATION"
    };
    let mode = if colors_enabled {
        if dry_run {
            style(mode_text).cyan().bold().to_string()
        } else if plan.delete_roots().is_empty() {
            style(mode_text).green().bold().to_string()
        } else if force {
            style(mode_text).red().bold().to_string()
        } else {
            style(mode_text).yellow().bold().to_string()
        }
    } else {
        mode_text.to_owned()
    };
    let heading = if colors_enabled {
        style("Deletion plan:").bold().to_string()
    } else {
        "Deletion plan:".to_owned()
    };

    writeln!(output, "{heading} {mode}")?;
    writeln!(output, "Root: {}", plan.root().display())?;
    writeln!(output)?;
    if (!keep_summary && !plan.keep_items().is_empty())
        || (!delete_summary && !plan.delete_items().is_empty())
    {
        let legend = if flat {
            "Legend: blue directory, cyan symlink; without color: / directory, @ symlink"
        } else {
            "Legend: blue directory, cyan symlink, green kept parent; without color: / directory, @ symlink, + kept parent"
        };
        writeln!(output, "{legend}")?;
        writeln!(output)?;
    }
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
            "Summary sections hide item paths. Run --dry-run without --summary to inspect all paths before deleting."
        )?;
    }

    Ok(())
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
        writeln!(output, "{title} summary:")?;
        if is_keep_section {
            writeln!(
                output,
                "  {} kept items (includes ancestor directories); {} direct pattern matches",
                items.len(),
                options.direct_match_count
            )?;
        } else {
            writeln!(
                output,
                "  {} delete items; {} top-level removal operations",
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
            let relative_path = format!("{}{suffix}", item.relative_path());
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
        let display_name = format!("{name}{suffix}");
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
    if fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
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

    #[test]
    // ツリー表示用の接尾辞がディレクトリとシンボリックリンクを識別することを確認する。
    fn tree_item_suffix_identifies_directories_and_symlinks() {
        assert_eq!(tree_item_suffix(ItemType::Directory), "/");
        assert_eq!(tree_item_suffix(ItemType::File), "");
        assert_eq!(tree_item_suffix(ItemType::Symlink), "@");
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
        assert!(plain_output.starts_with("Deletion plan: PREVIEW ONLY"));
        assert!(plain_output.contains("group/+"));
        assert!(!plain_output.contains('\u{1b}'));

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
        assert!(colored_output.contains("\u{1b}[1mDeletion plan:"));
        assert!(colored_output.contains("\u{1b}[36m\u{1b}[1mPREVIEW ONLY"));
        assert!(colored_output.contains("\u{1b}[32m\u{1b}[1mgroup/"));
        assert!(colored_output.contains("remove.txt"));
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
