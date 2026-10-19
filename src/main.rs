//! Displays a keep/deletion plan beneath the current directory and deletes items after confirmation.

use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::builder::styling::{AnsiColor, Styles};
use clap::{CommandFactory, Parser, ValueEnum};
use console::style;
use remove_except::{ItemType, PlannedItem, RemovalPlan, ValidationError, build_plan};
use serde::Serialize;

const EXIT_RUNTIME_ERROR: u8 = 1;
const EXIT_INVALID_INPUT: u8 = 2;
const EXIT_NOT_EXECUTED: u8 = 3;
const JSON_SCHEMA_VERSION: u32 = 1;

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
    after_long_help = "Output:\n  Tree: directories end with / and symlinks with @. With color, directories are blue, symlinks cyan, and kept parent directories in the delete tree green. Without color, kept parents end with +.\n  Flat: ItemType identifies each entry. Without color, directory and symlink paths end with / and @. Flat output has no kept-parent marker.\n  Status labels: NO REMOVALS (green) when there are no deletion operations; otherwise PREVIEW ONLY (cyan) for dry runs, CONFIRM TO DELETE (yellow) when confirmation is required, FORCE DELETE (red) with --force.\n  Colors are used automatically when output is a terminal and color is available.\n\nPipelines:\n  --patterns-from-stdin reads one UTF-8 pattern per line (LF or CRLF). A leading BOM is removed, empty lines are ignored, other whitespace is kept, and # has no special meaning. Invalid UTF-8 or NUL characters (for example, UTF-16 input) are rejected. Confirmation is then read from the terminal; without one, nothing is deleted.\n  --format json writes exactly one JSON object (schema_version 1) with root, direct_match_count, keep_items, delete_items, and delete_roots to stdout. Messages and prompts go to stderr. New fields may be added without changing schema_version.\n\nExit codes:\n  0 plan shown or deletion completed; 1 runtime error; 2 invalid arguments, input, root, or patterns; 3 nothing deleted because confirmation was declined or unavailable.",
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
        help = "Also read keep patterns from standard input, one per line"
    )]
    patterns_from_stdin: bool,

    #[arg(
        long,
        value_enum,
        value_name = "FORMAT",
        default_value = "human",
        help = "Plan output format: human or json"
    )]
    format: OutputFormat,

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
        help = "Display paths as a tree or a flat list [default: tree]"
    )]
    layout: Option<Layout>,

    #[arg(
        long,
        help = "Sort flat output by item type, then relative path (requires --layout flat)"
    )]
    sort_by_type: bool,

    #[arg(
        required_unless_present = "patterns_from_stdin",
        num_args = 1..,
        value_name = "KEEP_PATTERN",
        help = "Paths or glob patterns to keep (* matches one level, ** recurses); any match keeps an item"
    )]
    patterns: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
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

/// Result of a run that did not fail.
#[derive(Debug, Eq, PartialEq)]
enum Outcome {
    /// The plan was shown, or deletion completed.
    Completed,
    /// Nothing was deleted because confirmation was declined or unavailable.
    NotExecuted,
}

/// Invalid command-line input detected by the CLI itself rather than by the library.
#[derive(Debug)]
struct InputError(String);

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for InputError {}

fn main() -> ExitCode {
    let args = Args::parse();
    validate_option_combinations(&args);

    match run(args) {
        Ok(Outcome::Completed) => ExitCode::SUCCESS,
        Ok(Outcome::NotExecuted) => ExitCode::from(EXIT_NOT_EXECUTED),
        Err(error) => {
            eprintln!("Error: {error:?}");
            ExitCode::from(exit_code_for_error(&error))
        }
    }
}

fn validate_option_combinations(args: &Args) {
    let conflict = if args.format == OutputFormat::Json {
        if args.summary.is_some() {
            Some("--format json cannot be used with --summary; JSON always contains the full plan")
        } else if args.layout.is_some() {
            Some("--format json cannot be used with --layout")
        } else if args.sort_by_type {
            Some("--format json cannot be used with --sort-by-type")
        } else {
            None
        }
    } else {
        None
    };
    if let Some(message) = conflict {
        Args::command()
            .error(clap::error::ErrorKind::ArgumentConflict, message)
            .exit();
    }
    if args.sort_by_type && args.layout != Some(Layout::Flat) {
        Args::command()
            .error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "--sort-by-type requires --layout flat",
            )
            .exit();
    }
}

fn exit_code_for_error(error: &anyhow::Error) -> u8 {
    if error.downcast_ref::<ValidationError>().is_some()
        || error.downcast_ref::<InputError>().is_some()
    {
        EXIT_INVALID_INPUT
    } else {
        EXIT_RUNTIME_ERROR
    }
}

fn run(args: Args) -> Result<Outcome> {
    let json = args.format == OutputFormat::Json;
    let mut patterns = args.patterns;
    if args.patterns_from_stdin {
        let stdin = io::stdin();
        if stdin.is_terminal() {
            let end_of_input = if cfg!(windows) {
                "Ctrl+Z, then Enter"
            } else {
                "Ctrl+D"
            };
            eprintln!(
                "Reading keep patterns from the terminal, one per line. Finish with {end_of_input}."
            );
        }
        patterns.extend(read_patterns(stdin.lock())?);
    }
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    let root = resolve_root(&current_dir, args.root)?;
    let plan = build_plan(&root, &patterns)?;
    // Serialize before printing anything so an unrepresentable path never yields partial JSON.
    let json_plan = if json {
        Some(json_plan_text(&plan)?)
    } else {
        None
    };

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
    if let Some(json_plan) = json_plan {
        let mut output = io::stdout().lock();
        output.write_all(json_plan.as_bytes())?;
        output.flush()?;
    } else {
        print_plan(
            &plan,
            args.dry_run,
            args.force,
            keep_summary,
            delete_summary,
            args.layout == Some(Layout::Flat),
            args.sort_by_type,
        )?;
    }

    if args.dry_run || plan.delete_roots().is_empty() {
        return Ok(Outcome::Completed);
    }

    if !args.force {
        let has_summaries = keep_summary || delete_summary;
        let confirmed = if args.patterns_from_stdin {
            match confirm_on_terminal(confirmation_prompt(has_summaries)) {
                Some(confirmed) => confirmed,
                None => {
                    eprintln!(
                        "No terminal is available to confirm deletion; nothing was removed. Use --dry-run to preview or --force to delete without confirmation."
                    );
                    return Ok(Outcome::NotExecuted);
                }
            }
        } else {
            confirm_deletion(has_summaries)?
        };
        if !confirmed {
            report_status(json, "Aborted.");
            return Ok(Outcome::NotExecuted);
        }
    }

    for item in plan.delete_roots() {
        remove_item(item).with_context(|| {
            format!(
                "Failed to remove {}",
                escape_control_characters(&item.path().to_string_lossy())
            )
        })?;
    }

    report_status(json, "Removal complete.");
    Ok(Outcome::Completed)
}

/// Writes a final status message, keeping stdout reserved for JSON in JSON mode.
fn report_status(json: bool, message: &str) {
    if json {
        eprintln!("{message}");
    } else {
        println!("{message}");
    }
}

/// Reads keep patterns, one per LF- or CRLF-terminated UTF-8 line.
///
/// A single leading byte-order mark is removed and completely empty lines are ignored.
/// Whitespace and `#` are kept as literal pattern characters. Invalid UTF-8 and NUL
/// characters, which typically indicate UTF-16 or binary input, are rejected.
fn read_patterns(mut reader: impl Read) -> Result<Vec<String>> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .context("Failed to read keep patterns from standard input")?;
    let input = bytes
        .strip_prefix(b"\xEF\xBB\xBF".as_slice())
        .unwrap_or(&bytes);

    let mut patterns = Vec::new();
    let mut lines = input.split(|byte| *byte == b'\n').peekable();
    let mut line_number = 0;
    while let Some(line) = lines.next() {
        line_number += 1;
        // Only a carriage return that forms CRLF is a terminator; a final unterminated line keeps it.
        let line = if lines.peek().is_some() {
            line.strip_suffix(b"\r").unwrap_or(line)
        } else {
            line
        };
        let pattern = std::str::from_utf8(line).map_err(|_| {
            InputError(format!(
                "Standard input line {line_number} is not valid UTF-8"
            ))
        })?;
        if pattern.contains('\0') {
            return Err(InputError(format!(
                "Standard input line {line_number} contains a NUL character; input must be UTF-8 text"
            ))
            .into());
        }
        if !pattern.is_empty() {
            patterns.push(pattern.to_owned());
        }
    }
    Ok(patterns)
}

#[derive(Serialize)]
struct JsonPlan<'a> {
    schema_version: u32,
    root: &'a str,
    direct_match_count: usize,
    keep_items: Vec<JsonItem<'a>>,
    delete_items: Vec<JsonItem<'a>>,
    delete_roots: Vec<JsonItem<'a>>,
}

#[derive(Serialize)]
struct JsonItem<'a> {
    relative_path: &'a str,
    #[serde(rename = "type")]
    item_type: &'static str,
}

/// Serializes the complete plan, failing if any path cannot be represented as Unicode.
fn json_plan_text(plan: &RemovalPlan) -> Result<String> {
    let root = unicode_path(plan.root())?;
    let json_plan = JsonPlan {
        schema_version: JSON_SCHEMA_VERSION,
        root,
        direct_match_count: plan.direct_match_count(),
        keep_items: json_items(plan.keep_items())?,
        delete_items: json_items(plan.delete_items())?,
        delete_roots: json_items(plan.delete_roots())?,
    };
    let mut text = serde_json::to_string_pretty(&json_plan).context("Failed to serialize plan")?;
    text.push('\n');
    Ok(text)
}

fn json_items(items: &[PlannedItem]) -> Result<Vec<JsonItem<'_>>> {
    items
        .iter()
        .map(|item| {
            unicode_path(item.path())?;
            Ok(JsonItem {
                relative_path: item.relative_path(),
                item_type: json_item_type(item.item_type()),
            })
        })
        .collect()
}

fn unicode_path(path: &Path) -> Result<&str> {
    path.to_str().with_context(|| {
        format!(
            "Path cannot be represented as Unicode in JSON output; nothing was printed or removed: {}",
            escape_control_characters(&path.to_string_lossy())
        )
    })
}

fn json_item_type(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Directory => "directory",
        ItemType::File => "file",
        ItemType::Symlink => "symlink",
    }
}

fn resolve_root(current_dir: &Path, root: Option<PathBuf>) -> Result<PathBuf> {
    let Some(root) = root else {
        return Ok(current_dir.to_path_buf());
    };

    if is_drive_relative_path(&root) {
        return Err(InputError(format!(
            "Drive-relative root paths are not supported: {}",
            root.display()
        ))
        .into());
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

fn confirmation_prompt(has_summaries: bool) -> &'static str {
    if has_summaries {
        "Delete the plan? Summary sections do not list item paths."
    } else {
        "Are you sure you want to delete the above items?"
    }
}

fn confirm_deletion(has_summaries: bool) -> Result<bool> {
    let prompt = confirmation_prompt(has_summaries);
    write_prompt(prompt)?;
    read_confirmation(io::stdin().lock()).context("Failed to read confirmation")
}

fn write_prompt(prompt: &str) -> io::Result<()> {
    let mut prompt_output = io::stderr().lock();
    write!(prompt_output, "{prompt} [y/N] ")?;
    prompt_output.flush()
}

/// Reads one full answer line, so a partial `y` keypress never confirms on its own.
///
/// Input that is not valid UTF-8, such as a non-UTF-8 console code page, is decoded
/// lossily; anything other than `y` or `yes` declines.
fn read_confirmation(mut input: impl BufRead) -> io::Result<bool> {
    let mut answer = Vec::new();
    input.read_until(b'\n', &mut answer)?;
    Ok(is_affirmative(&String::from_utf8_lossy(&answer)))
}

fn is_affirmative(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Asks for confirmation on the controlling terminal instead of standard input.
///
/// Returns `None` when no terminal can be opened or read, so the caller can stop
/// without deleting anything.
fn confirm_on_terminal(prompt: &str) -> Option<bool> {
    #[cfg(windows)]
    const TERMINAL_INPUT: &str = "CONIN$";
    #[cfg(not(windows))]
    const TERMINAL_INPUT: &str = "/dev/tty";

    let terminal = fs::OpenOptions::new()
        .read(true)
        .open(TERMINAL_INPUT)
        .ok()?;
    write_prompt(prompt).ok()?;
    read_confirmation(BufReader::new(terminal)).ok()
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
    let (status_label, status_detail) = if plan.delete_roots().is_empty() {
        ("NO REMOVALS", "No deletion operations are needed")
    } else if dry_run {
        ("PREVIEW ONLY", "Nothing will be removed")
    } else if force {
        (
            "FORCE DELETE",
            "Confirmation is disabled; validation still applies",
        )
    } else {
        ("CONFIRM TO DELETE", "Review both lists before confirming")
    };
    let mode = if colors_enabled {
        if plan.delete_roots().is_empty() {
            style(status_label).green().bold().to_string()
        } else if dry_run {
            style(status_label).cyan().bold().to_string()
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
        ordered_items
            .sort_unstable_by(|left, right| left.relative_path().cmp(right.relative_path()));
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
    use super::{
        Args, InputError, flat_item_type_rank, read_confirmation, read_patterns, render_plan,
        style_flat_item, tree_item_suffix,
    };
    use assert_fs::prelude::*;
    use clap::CommandFactory;
    use remove_except::{ItemType, build_plan};

    static COLOR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(windows)]
    use super::display_root_path;

    #[test]
    // Verify only a whole `y` or `yes` line confirms; partial or other answers decline.
    fn read_confirmation_requires_a_full_affirmative_line() {
        for answer in ["y\n", "YES\r\n", " yes \n", "y"] {
            assert!(read_confirmation(answer.as_bytes()).unwrap(), "{answer:?}");
        }
        for answer in ["", "\n", "n\n", "maybe\n", "yes please\n", "ye\n", "y es\n"] {
            assert!(!read_confirmation(answer.as_bytes()).unwrap(), "{answer:?}");
        }
        assert!(!read_confirmation(b"\xff\xfe\n".as_slice()).unwrap());
        assert!(!read_confirmation("n\ny\n".as_bytes()).unwrap());
    }

    #[test]
    // Verify stdin patterns strip only a leading BOM and line terminators and skip empty lines.
    fn read_patterns_parses_lines_literally() {
        let input = "\u{feff}a.txt\r\n\r\n\n  \n#b\n\u{feff}c\n tail \r\nlast".as_bytes();
        assert_eq!(
            read_patterns(input).unwrap(),
            ["a.txt", "  ", "#b", "\u{feff}c", " tail ", "last"]
        );
        assert!(read_patterns("\n\r\n".as_bytes()).unwrap().is_empty());
    }

    #[test]
    // Verify invalid UTF-8 reports the failing line as an input error.
    fn read_patterns_rejects_invalid_utf8() {
        let error = read_patterns(b"ok\r\nbad\xff\n".as_slice()).unwrap_err();
        assert!(error.downcast_ref::<InputError>().is_some());
        assert!(error.to_string().contains("line 2"));
    }

    #[test]
    // Verify a carriage return is removed only as part of CRLF, not at the end of unterminated input.
    fn read_patterns_keeps_a_final_unterminated_carriage_return() {
        assert_eq!(read_patterns("a\r".as_bytes()).unwrap(), ["a\r"]);
        assert_eq!(read_patterns("\r".as_bytes()).unwrap(), ["\r"]);
        assert_eq!(read_patterns("a\r\r\nb".as_bytes()).unwrap(), ["a\r", "b"]);
    }

    #[test]
    // Verify NUL characters, such as those from UTF-16 input without a BOM, are rejected.
    fn read_patterns_rejects_nul_characters() {
        let error = read_patterns(b"ok\nk\0e\0e\0p\0\n".as_slice()).unwrap_err();
        assert!(error.downcast_ref::<InputError>().is_some());
        assert!(
            error
                .to_string()
                .contains("line 2 contains a NUL character")
        );
    }

    #[test]
    // Verify tree markers distinguish directories from symbolic links.
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
    // Verify flat layout sorts directories, files, and symbolic links by type.
    fn flat_item_type_rank_orders_directories_files_and_symlinks() {
        assert!(flat_item_type_rank(ItemType::Directory) < flat_item_type_rank(ItemType::File));
        assert!(flat_item_type_rank(ItemType::File) < flat_item_type_rank(ItemType::Symlink));
    }

    #[test]
    // Verify flat layout colors directories and links but leaves files uncolored.
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
    // Verify tree layout colors real directories, symbolic links, and kept parents distinctly.
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
        let plan_without_deletions = build_plan(temp.path(), &[".".to_owned()]).unwrap();

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

        let mut no_kept_parent_output = Vec::new();
        render_plan(
            &mut no_kept_parent_output,
            &plan_without_deletions,
            true,
            false,
            false,
            false,
            false,
            false,
            true,
        )
        .unwrap();
        let no_kept_parent_output = String::from_utf8(no_kept_parent_output).unwrap();
        assert!(no_kept_parent_output.contains("Colors: directories ("));
        assert!(no_kept_parent_output.contains("kept parents ("));
        console::set_colors_enabled(colors_were_enabled);
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
            render_status(&plan_without_deletions, true, false, false)
                .starts_with("NO REMOVALS  No deletion operations are needed")
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
        let no_removals_output = render_status(&plan_without_deletions, true, false, true);
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
        assert!(help.contains("NO REMOVALS (green) when there are no deletion operations"));
    }
}
