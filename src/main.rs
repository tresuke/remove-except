//! カレントディレクトリ配下の保持・削除計画を表示し、確認後に削除します。

use std::cmp::Ordering;
use std::fs;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use console::style;
use dialoguer::Confirm;
use remove_except::{ItemType, PlannedItem, build_plan};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Keep items matching any pattern and remove the rest under the selected root"
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

    #[arg(long, help = "Show summaries instead of both item lists")]
    summary_only: bool,

    #[arg(long, help = "Show a summary instead of the keep item list")]
    keep_summary: bool,

    #[arg(long, help = "Show a summary instead of the delete item list")]
    delete_summary: bool,

    #[arg(long, help = "Show flat path lists instead of the default tree view")]
    flat: bool,

    #[arg(
        required = true,
        num_args = 1..,
        help = "Paths or glob patterns to keep (* matches one level, ** recurses); any match keeps an item"
    )]
    patterns: Vec<String>,
}

struct PlanDisplayOptions {
    flat: bool,
    direct_match_count: usize,
    delete_root_count: usize,
    colors_enabled: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    let root = resolve_root(&current_dir, args.root)?;
    let plan = build_plan(&root, &args.patterns)?;

    if plan.direct_match_count == 0 {
        eprintln!(
            "WARNING: No items matched the keep patterns. Everything under {} would be removed.",
            plan.root.display()
        );
    }

    let keep_summary = args.summary_only || args.keep_summary;
    let delete_summary = args.summary_only || args.delete_summary;
    print_plan(
        &plan,
        args.dry_run,
        args.force,
        keep_summary,
        delete_summary,
        args.flat,
    )?;

    if args.dry_run || plan.delete_roots.is_empty() {
        return Ok(());
    }

    if !args.force && !confirm_deletion(keep_summary || delete_summary)? {
        println!("Aborted.");
        return Ok(());
    }

    for item in &plan.delete_roots {
        remove_item(item).with_context(|| format!("Failed to remove {}", item.path.display()))?;
    }

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

    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
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
) -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let colors_enabled = io::stdout().is_terminal() && console::colors_enabled();
    let mode = if dry_run {
        "PREVIEW ONLY - nothing will be removed"
    } else if plan.delete_roots.is_empty() {
        "No removal operations are needed"
    } else if force {
        "DELETION ENABLED (--force)"
    } else {
        "DELETION AFTER CONFIRMATION"
    };

    writeln!(output, "Deletion plan: {mode}")?;
    writeln!(output, "Root: {}", plan.root.display())?;
    writeln!(output)?;
    if !flat
        && ((!keep_summary && !plan.keep_items.is_empty())
            || (!delete_summary && !plan.delete_items.is_empty()))
    {
        writeln!(
            output,
            "Legend: blue directory, cyan symlink, green kept parent; without color: / directory, @ symlink, + kept parent"
        )?;
        writeln!(output)?;
    }
    let options = PlanDisplayOptions {
        flat,
        direct_match_count: plan.direct_match_count,
        delete_root_count: plan.delete_roots.len(),
        colors_enabled,
    };
    print_item_section(
        &mut output,
        "Items to keep",
        &plan.keep_items,
        keep_summary,
        &options,
    )?;
    writeln!(output)?;
    print_item_section(
        &mut output,
        "Items to delete",
        &plan.delete_items,
        delete_summary,
        &options,
    )?;

    if keep_summary || delete_summary {
        writeln!(output)?;
        writeln!(
            output,
            "Summary sections do not list item paths. Run --dry-run without summary options to inspect all paths before deleting."
        )?;
    }

    output.flush()
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
        for item in items {
            writeln!(
                output,
                "{:<12}  {}",
                item_type_name(item.item_type),
                item.relative_path
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
            items[start].relative_path.as_str()
        } else {
            items[start]
                .relative_path
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
            && (items[end].relative_path == child_path
                || items[end].relative_path.starts_with(&child_prefix))
        {
            end += 1;
        }

        let item = (items[start].relative_path == child_path).then_some(items[start]);
        let descendants_start = start + usize::from(item.is_some());
        let is_directory = item.is_none_or(|item| item.item_type == ItemType::Directory)
            || descendants_start < end;
        let is_last = end == items.len();
        let connector = if is_last { "└──" } else { "├──" };
        let next_prefix = format!("{branch_prefix}{}", if is_last { "    " } else { "│   " });
        let suffix = item
            .map(|item| tree_item_suffix(item.item_type))
            .unwrap_or(if is_directory { "/" } else { "" });
        let is_kept_parent = item.is_none();
        let display_name = format!("{name}{suffix}");
        let display_name = if colors_enabled {
            match item.map(|item| item.item_type) {
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
    let mut left_parts = left.relative_path.split('/').peekable();
    let mut right_parts = right.relative_path.split('/').peekable();

    loop {
        match (left_parts.next(), right_parts.next()) {
            (Some(left_part), Some(right_part)) if left_part == right_part => {}
            (Some(left_part), Some(right_part)) => {
                let left_is_directory =
                    left_parts.peek().is_some() || left.item_type == ItemType::Directory;
                let right_is_directory =
                    right_parts.peek().is_some() || right.item_type == ItemType::Directory;
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

fn tree_item_suffix(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Directory => "/",
        ItemType::File => "",
        ItemType::Symlink => "@",
    }
}

fn remove_item(item: &PlannedItem) -> std::io::Result<()> {
    match item.item_type {
        ItemType::Directory => fs::remove_dir_all(&item.path),
        ItemType::File | ItemType::Symlink => remove_file_or_directory_link(&item.path),
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
    use super::tree_item_suffix;
    use remove_except::ItemType;

    #[test]
    fn tree_item_suffix_identifies_directories_and_symlinks() {
        assert_eq!(tree_item_suffix(ItemType::Directory), "/");
        assert_eq!(tree_item_suffix(ItemType::File), "");
        assert_eq!(tree_item_suffix(ItemType::Symlink), "@");
    }
}
