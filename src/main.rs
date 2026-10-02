//! カレントディレクトリ配下の保持・削除計画を表示し、確認後に削除します。

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;
use dialoguer::Confirm;
use remove_except::{ItemType, PlannedItem, build_plan};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Keep items matching any pattern and remove the rest under the current directory"
)]
struct Args {
    #[arg(
        short = 'n',
        long,
        help = "Preview the deletion plan without confirming or deleting anything"
    )]
    dry_run: bool,

    #[arg(short = 'f', long, help = "Delete without the confirmation prompt")]
    force: bool,

    #[arg(
        required = true,
        num_args = 1..,
        help = "Paths or glob patterns to keep (* matches one level, ** recurses); any match keeps an item"
    )]
    patterns: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    let plan = build_plan(&current_dir, &args.patterns)?;

    if plan.direct_match_count == 0 {
        eprintln!(
            "WARNING: No items matched the keep patterns. Everything under the current directory would be removed."
        );
    }

    print_plan(&plan.keep_items, &plan.delete_items);

    if args.dry_run || plan.delete_roots.is_empty() {
        return Ok(());
    }

    if !args.force && !confirm_deletion()? {
        println!("Aborted.");
        return Ok(());
    }

    for item in &plan.delete_roots {
        remove_item(item).with_context(|| format!("Failed to remove {}", item.path.display()))?;
    }

    Ok(())
}

fn confirm_deletion() -> Result<bool> {
    if io::stdin().is_terminal() {
        return Confirm::new()
            .with_prompt("Are you sure you want to delete the above items?")
            .default(false)
            .interact()
            .context("Failed to read confirmation");
    }

    print!("Are you sure you want to delete the above items? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn print_plan(keep_items: &[PlannedItem], delete_items: &[PlannedItem]) {
    print_item_section("Items to keep", keep_items);
    println!();
    print_item_section("Items to delete", delete_items);
}

fn print_item_section(title: &str, items: &[PlannedItem]) {
    println!("{title} ({}):", items.len());
    if items.is_empty() {
        println!("(None)");
        return;
    }

    println!("{:<12}  {:<40}  FullName", "ItemType", "RelativePath");
    println!("{:-<12}  {:-<40}  {:-<8}", "", "", "");
    for item in items {
        let item_type = match item.item_type {
            ItemType::File => "File",
            ItemType::Directory => "Directory",
            ItemType::Symlink => "Symlink",
        };
        println!(
            "{item_type:<12}  {:<40}  {}",
            item.relative_path,
            item.path.display()
        );
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
