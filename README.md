# remove-except

[English](README.md) | [日本語](README.ja.md)

Have you ever wanted to keep only a few items among many files in a directory and remove everything else, including items in nested directories? I have. Of course, a script can do that. But I thought it would be more convenient to have a command that only requires the paths or patterns to keep. That is why I made `remove-except`.

`remove-except` is a command-line tool that keeps items matching any supplied path or glob pattern and deletes the other items under the selected directory.

## Safety Notice

This tool permanently deletes files and directories. Back up important data before running it. Before deleting anything, use `--dry-run` and review every item. `--summary` omits paths, so do not use it for the pre-deletion review. If none of the supplied keep patterns matches any item, everything under the selected directory becomes a deletion target. Especially when using `--force`, verify that the intended patterns match items. When using `--root`, verify that the displayed directory is the intended location. A filesystem's top-level directory cannot be selected for processing.

## Installation

To install from source, install Rust 1.88 or later and run this command from the top-level directory of the repository.

```sh
cargo install --path .
```

## Quick Start

The syntax is `remove-except [OPTIONS] <KEEP_PATTERN>...`. Provide one or more keep patterns as positional arguments. When you specify multiple patterns, an item is kept if it matches any of them.

First, use `--dry-run` to review which items will be kept and deleted.

```sh
remove-except --dry-run keep.txt keep '**/*.md'
```

To perform deletion, omit `--dry-run`. In a normal run, the tool displays the kept items and deletion targets, then asks whether to proceed. Enter `y` or `yes` to delete; enter `n` or press Enter to cancel (the default is No). With `--force`, deletion proceeds without prompting. Invalid patterns or an invalid directory still produce an error.

```sh
remove-except keep.txt keep '**/*.md'
remove-except --force keep.txt keep '**/*.md'
```

To process a directory other than the one where the command was started, specify it with `--root`. Relative paths are resolved from the current directory when the command starts.

```sh
remove-except --root work --dry-run keep.txt '**/*.md'
```

## Options

| Option | Description |
| --- | --- |
| `<KEEP_PATTERN>...` | Paths or glob patterns to keep. At least one is required; multiple patterns use OR semantics. |
| `--root <PATH>` | Selects the directory to process. Defaults to the current directory when the command starts. Relative paths are resolved from that directory. |
| `-n`, `--dry-run` | Displays kept items and deletion targets without deleting anything. |
| `-f`, `--force` | Deletes without showing the confirmation prompt. Pattern and directory validation still apply. |
| `--layout <LAYOUT>` | Output layout: `tree` (default) or `flat`. |
| `--sort-by-type` | Flat layout only. Sorts by Directory, File, then Symlink. |
| `--summary <SECTION>` | Shows counts for `keep`, `delete`, or `both` instead of paths in the selected sections. |
| `-h`, `--help` | Displays help. |
| `-V`, `--version` | Displays the version. |

## Patterns and Processing Rules

### Example

The following example assumes `work` is the current directory.

```text
work/
├── keep.txt
├── readme.md
├── todo.txt
├── reports/
│   ├── summary.txt
│   ├── report1.csv
│   ├── report2.csv
│   └── 2025/
│       ├── annual.csv
│       └── notes.txt
└── archive/
    └── old.csv
```

### Glob Patterns

You can use glob patterns to specify paths to keep. Characters such as `*` and `?` represent parts of item names or directory levels. Patterns can match files, directories, and symbolic links. Separate multiple patterns with spaces.

```sh
remove-except --dry-run '**/*.md'
```

In this example tree, `readme.md` is kept, while `keep.txt`, `todo.txt`, `reports`, and `archive` are shown as deletion targets. The preview displays both kept items and deletion targets. Since `**/` matches zero or more directory levels, `**/*.md` matches paths ending in `.md` directly under the processing directory and in nested directories.

Common glob syntax:

| Pattern | Meaning |
| --- | --- |
| `*.md` | Item paths ending in `.md` directly under the processing directory. `*` does not cross `/`. |
| `**/*.md` | Item paths ending in `.md` directly under the processing directory or in nested directories. `**/` matches zero or more directory levels. |
| `reports/report?.csv` | `?` matches any single character except `/`. In this example, it matches `report1.csv` and `report2.csv`. |
| `reports/report[12].csv` | `[]` matches one of the listed characters. In this example, it matches `report1.csv` and `report2.csv`. |

To limit a glob to a particular subdirectory, include its path relative to the processing directory in the pattern.

```sh
remove-except --dry-run 'reports/**/*.csv'
```

In this example tree, the pattern keeps `reports/report1.csv`, `reports/report2.csv`, and `reports/2025/annual.csv`. Because `**/` matches zero or more directory levels, `reports/**/*.csv` matches paths ending in `.csv` directly under `reports` and at deeper levels. Quote globs to prevent the shell from expanding them before the command runs.

### Selecting Subdirectories

To keep an entire subdirectory, specify its path without a glob.

```sh
remove-except --dry-run reports/2025
```

`reports/2025` and all of its contents are kept. For example, `annual.csv` and `notes.txt` remain, while `reports/summary.txt` and `reports/report1.csv` are deletion targets.

To keep only one file inside a subdirectory, specify its relative path.

```sh
remove-except --dry-run reports/2025/annual.csv
```

This keeps `annual.csv` and its parent directories, `reports` and `reports/2025`. The sibling file `notes.txt` is a deletion target.

To keep only certain file types within a subdirectory, combine its path with a glob.

```sh
remove-except --dry-run 'reports/2025/*.csv'
```

This example keeps `reports/2025/annual.csv`; `reports/2025/notes.txt` is a deletion target. Since `*` matches only within one level, `reports/2025/*.csv` is limited to CSV files directly under `reports/2025`. To include deeper levels, use `reports/2025/**/*.csv`. To keep the entire directory, specify its path, such as `reports/2025`, without a glob.

Use `--dry-run` in any example to review the kept items and deletion targets. Omit `--dry-run` only when you intend to delete. In a normal run, the tool displays both lists and then asks whether to proceed with deletion. The default answer is No.

### Processing Scope and Paths

By default, the tool recursively processes the current directory from which it was started. Use `--root` to select another processing directory. A relative path supplied to `--root` is resolved from the startup directory. The selected directory itself is never listed or deleted. The filesystem's top-level directory is rejected for safety.

At least one keep pattern is required. Patterns are positional arguments. With multiple patterns, an item is kept if it matches any one of them. Leading or trailing spaces are part of the item name. Quote patterns containing spaces so the shell passes each pattern as one argument.

Relative patterns are resolved from the selected processing directory. Absolute patterns are accepted only when they are inside that directory. Relative patterns and the portion of an absolute pattern after the matching directory prefix normalize `.` components and repeated separators. For absolute patterns, separators are converted to `/` before matching the directory prefix; the prefix itself is not normalized. Patterns outside the selected directory, patterns containing `..` as a path component, empty patterns, and ambiguous Windows drive-relative paths (for example, `C:folder`) are errors.

### Matching and Retention

`*`, `**`, `?`, and character classes `[]` are supported. `*` and `?` do not cross `/`; `**` matches across multiple levels. `**/` matches zero or more directory levels. Patterns can match files, directories, and symbolic links. Quote globs to prevent shell expansion.

A matched item and its parent directories are kept, but unmatched descendants are not. A directory path without a glob keeps that directory and all of its contents. `dir/*` matches only items directly under `dir`; it does not recursively keep the contents of matched subdirectories. `.` and `./` keep everything under the processing directory.

### Filesystem Behavior

Windows uses the tool's Unicode simple case folding (non-Turkic); Ubuntu is case-sensitive. On Windows, `ä` and `Ä`, and `ß` and `ẞ`, are treated as equal, but `ß` and `SS` are not. The tool does not follow per-directory NTFS case-sensitivity settings. Windows and Ubuntu are supported.

A symbolic link supplied to `--root` is resolved to its target. Links under the processing directory are not followed and are treated as individual items. If a link is a deletion target, only the link is removed, not its target.

### Display and Execution

The default `--layout tree` shows kept items and deletion targets as a tree; `--layout flat` shows `ItemType` and `RelativePath` columns. Pattern matches and required ancestors are shown separately from deletion targets. Both layouts use relative-path order by default. `--sort-by-type` requires `--layout flat` and sorts by Directory, File, then Symlink, with relative-path order within each type. Sorting affects display only, not the displayed lists or deletion order. See “Output and Options” for display details.

In a normal run, the tool displays both lists and asks whether to proceed with deletion. Enter `y` or `yes` to delete; enter `n` or press Enter to cancel (the default is No). With `--force`, deletion proceeds without prompting. Invalid patterns or an invalid processing directory still produce an error.

Kept items, deletion targets, and execution results go to standard output. Warnings, errors, and confirmation prompts go to standard error. Output is human-readable text; pattern input from standard input and stable machine-readable output are not provided.

If none of the supplied keep patterns matches any item, a warning is shown and everything under the processing directory becomes a deletion target. Review the targets with a dry run before deleting.

## Output and Options

The output starts with a status label and the processing directory. When there are no deletion targets, the label is `NO REMOVALS`, including in a dry run. Otherwise, dry runs show `PREVIEW ONLY`, normal runs that require confirmation show `CONFIRM TO DELETE`, and `--force` shows `FORCE DELETE`. On a colored TTY, these labels are highlighted in green, cyan, yellow, and red, respectively. The labels and descriptions remain when color is disabled.

Preview header example:

```text
PREVIEW ONLY  Nothing will be removed
Root: S:\work\project
```

For human-readable output, ordinary Windows drive and UNC paths omit the `\\?\` prefix. Paths that would become ambiguous in ordinary notation, such as components ending in a space or period, retain the extended form. Control characters in file names and the processing-directory path are rendered visibly. Display formatting does not change the internal path or deletion targets.

### Layouts

| Layout | With color | Without color |
| --- | --- | --- |
| Tree (default) | Directories blue, symbolic links cyan, retained parents that appear only in the deletion list green | `/` for directories, `@` for symbolic links, `+` for retained parents |
| Flat | `ItemType` and `RelativePath` columns; directories blue, symbolic links cyan, files uncolored | `/` on directory paths, `@` on symbolic-link paths, no file marker |

Every entry in the deletion list is a deletion target; top-level deletion items are not marked separately. Flat layout has no retained-parent color or marker. `--sort-by-type` requires `--layout flat` and sorts by Directory, File, then Symlink, with relative-path order within each type. Without type sorting, items are displayed in relative-path order. Sorting affects display only, not the displayed lists or deletion order.

Examples:

```sh
remove-except --dry-run --layout flat keep.txt
remove-except --dry-run --layout flat --sort-by-type keep.txt
remove-except --dry-run --summary both keep.txt
```

### Legends and Summaries

A legend appears after the processing-directory path only when a full path listing is shown. With color, Tree legends list blue directories, cyan symlinks, and green retained parents; Flat legends list blue directories and cyan symlinks. Without color, `Markers:` explains `/`, `@`, and `+` for Tree, or `/` and `@` for Flat. On a TTY, `--help` subtly styles headings, Usage, option names, and values; it emits no ANSI styling when color is disabled.

`--summary keep|delete|both` replaces selected sections with counts and no paths. Keep counts include required ancestors and separately report direct pattern matches. Deletion-item counts and the number of top-level deletion operations are distinct. Since summaries omit paths, review every item with `--dry-run` and without `--summary` before deleting.

Kept items, deletion targets, and execution results are written to standard output. Warnings, errors, and confirmation prompts are written to standard error. Keep patterns are positional arguments. Reading patterns from standard input and stable machine-readable output are not supported. Parsing standard output from another program is not a supported use case.

## License

This project is dual-licensed. You may choose either `LICENSE-MIT` or `LICENSE-APACHE`. Dependencies are covered by their respective licenses.

## Developer Documentation

### Project Structure

- `src/main.rs`: Parses CLI arguments, creates and displays the kept-item and deletion-target lists, asks for confirmation, and deletes filesystem items.
- `src/lib.rs`: Re-exports the crate's public API and coordinates scanning and `RemovalPlan` creation through `build_plan`.
- `src/model.rs`: Defines the public data types and read-only getters. `src/pattern.rs` handles glob matching; `src/path.rs` handles path validation and normalization.
- `tests/cli.rs`: Runs the CLI in temporary directories and verifies deletion, confirmation, and display. `tests/library.rs` tests the public library API and platform-specific case matching. Binary unit tests cover colored and uncolored rendering.

### Run from Source

During development, run the tool from the top-level directory of the repository with `cargo run --`.

```sh
cargo run -- --dry-run keep.txt keep '**/*.md'
```

### Manual Testing

`Prepare-Manual-Test.ps1` creates `manual-test-workspace` and fixture files for manually checking the Rust CLI. `keep-link` is a symbolic link to `keep.txt` used to verify the `@` marker in previews. On Windows, creating symbolic links may require Developer Mode or administrator privileges. Run the script from the top-level directory of the repository.

```powershell
.\Prepare-Manual-Test.ps1
Set-Location .\manual-test-workspace
cargo run --manifest-path ..\Cargo.toml -- --dry-run keep.txt keep '*.md'
```

The preview lists `keep.txt`, `release-notes.md`, the `keep` directory, and its contents as kept; all other items are deletion targets. To try an actual deletion, omit `--dry-run` and answer `n` at the prompt to cancel. If you perform a deletion, return to the top-level directory of the repository with `Set-Location ..` and rerun the script to restore the known fixtures.

On Ubuntu (including WSL), use `prepare-manual-test.sh` to create the same fixtures. In WSL, work in a repository copy under the Linux home directory rather than under `/mnt/...`. Run these commands from the top-level directory of that copy; ensure that Cargo is available on `PATH`.

```bash
bash ./prepare-manual-test.sh
cd manual-test-workspace
cargo run --manifest-path ../Cargo.toml -- --dry-run keep.txt keep '*.md'
```

To cancel at the confirmation prompt, omit `--dry-run` and answer `n`. After an actual deletion, return to the top-level directory of the repository copy with `cd ..` and rerun the generator.

Both scripts create or overwrite only known fixture files. They replace `keep-link` only when that known symbolic link points to `keep.txt`. They do not delete other files or links in the directory. Use `-Path` in PowerShell or the optional workspace-path argument in Bash to create fixtures elsewhere. If a regular file or a different link conflicts with `keep-link`, the script reports an error instead of overwriting it.

```powershell
.\Prepare-Manual-Test.ps1 -Path 'C:\work\manual-fixtures'
```

```bash
bash ./prepare-manual-test.sh "$HOME/manual-fixtures"
```

For a custom workspace location, use the absolute path to the repository's `Cargo.toml` with `--manifest-path` when running Cargo from the fixture directory.

### Library API

The library function `build_plan(root, patterns)` returns a `RemovalPlan` for the selected processing directory and keep patterns. It returns an error if no patterns are provided, a pattern is empty or contains an invalid glob, an absolute pattern is outside the selected directory, a pattern traverses to a parent directory, the selected directory is a filesystem's top-level directory or is not a directory, or a Windows drive-relative path is used. Resolving or scanning the selected directory can also fail. `.` and `./` both represent the selected directory and keep everything beneath it.

Read `RemovalPlan` and `PlannedItem` data through their getters. Getters return references and do not allow the `RemovalPlan` contents to be modified. Code that accessed public fields directly should be updated to call the getters.

```rust,no_run
use remove_except::build_plan;
use std::path::Path;

let patterns = ["Cargo.toml".to_owned(), "src/**".to_owned()];
let plan = build_plan(Path::new("."), &patterns).expect("valid plan");

for item in plan.delete_roots() {
    println!("{}", item.relative_path());
}
```

`RemovalPlan` getters:

- `root()`: The normalized processing directory.
- `direct_match_count()`: The number of items directly matched by patterns; used to determine whether to warn that nothing matched.
- `keep_items()`: All kept items, including required ancestor directories, sorted by relative path.
- `delete_items()`: All deletion targets, sorted by relative path.
- `delete_roots()`: The top-level items passed to deletion. Descendants of deletion-target directories are excluded to avoid redundant recursive deletions.

Each `PlannedItem` can be read through `path()`, `relative_path()`, and `item_type()`. Relative paths use `/` separators. `path()` returns an absolute path under the normalized processing directory. On Windows, the standard library's canonicalized path may include the extended-path prefix `\\?\`.

The scan does not follow symbolic links. After `build_plan` returns, the CLI deletes the items in `delete_roots`. If a filesystem error occurs during deletion, the command exits with an error, but items already deleted before the error cannot be restored.

### Building and Validation

This project uses Edition 2024 and has an MSRV of Rust 1.88. The standard validation environment is GitHub Actions, which runs `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` on Rust 1.88 and stable for Windows and Ubuntu. You can run the same commands locally.

The MSRV is based on the highest declared minimum Rust version among the currently locked dependencies, not solely on Edition 2024's requirements. If dependency updates or code changes require a newer compiler, review the MSRV in `Cargo.toml`, the CI toolchains, and both READMEs in the same change. CI uses `--locked` for tests and Clippy to validate the dependency versions in the lockfile.

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
cargo doc --no-deps
```

For additional validation on Ubuntu or WSL, use a repository copy under the Linux home directory and run the desired commands there. Use the Linux-side copy rather than a working copy under `/mnt/...`.

### Performance Benchmarks

Benchmarks measure `build_plan` using only the standard library and existing development dependencies. They create temporary fixtures before measurement; fixture creation time is excluded. The benchmarks compare a broad 2,080-item tree with one glob, multiple globs, a literal directory, and all items kept; an 8,208-item tree with sparse matches and all items kept; and matches in a deeper tree.

```sh
cargo bench --bench plan_generation
cargo bench --bench plan_generation -- --samples 25 --iterations 10
```

Each case reports the median, sample range, number of scanned items, number of patterns, and approximate items per second. The default of 9 samples × 3 iterations is for quick checks. For performance comparisons, use 25 samples × 10 iterations with the same OS, machine, and arguments, and record the Release-mode results after the build completes.

To benchmark the full CLI, run a dry run of the release binary with `hyperfine`. The following example only reads the manual fixtures and does not delete anything.

```powershell
cargo build --release
hyperfine --warmup 3 --runs 10 'target/release/remove-except.exe --dry-run --summary both --root manual-test-workspace keep.txt keep'
```

CLI measurements include process startup, `build_plan`, and output rendering. Do not compare them as if they measured `build_plan` alone, and keep fixture contents identical between runs.

Dependencies are managed in `Cargo.toml`. The CLI uses `clap`, confirmation prompts use `dialoguer`, output styling uses `console`, glob matching uses `globset`, directory traversal uses `walkdir`, and error context uses `anyhow`. Windows Unicode glob matching uses `regex-automata` and `unicode-casefold`.
