# remove-except

[English](README.md) | [日本語](README.ja.md)

A command-line tool that keeps only items matching the specified paths or glob patterns and removes everything else under the selected root.

## Safety Notice

This tool permanently deletes files and directories. Before running it on a processing root, always use `--dry-run` and review the full list. `--summary` omits paths from the selected sections, so before deleting anything, run `--dry-run` without `--summary` to review every item. If no item matches a pattern, everything under the selected root becomes a deletion target. When using `--root`, verify that the normalized path shown is the intended location. A filesystem root cannot be selected as the processing root.

## Installation

To install from source, install Rust 1.85 or later and run this command from the repository root.

```sh
cargo install --path .
```

After installation, run the `remove-except` command. To build a release binary, use the following command. The binary is created under `target/release`.

```sh
cargo build --release
```

## Usage

The syntax is `remove-except [OPTIONS] <KEEP_PATTERN>...`. Provide one or more keep patterns as positional arguments. When you specify multiple patterns, an item is kept if it matches any of them.

Review the plan with `--dry-run` first.

```sh
remove-except --dry-run keep.txt keep '**/*.md'
```

To perform deletion, omit `--dry-run`. Normally, the tool asks for confirmation after showing the plan; the default answer is No. `--force` skips confirmation only. It does not skip pattern or root validation.

```sh
remove-except keep.txt keep '**/*.md'
remove-except --force keep.txt keep '**/*.md'
```

Use `--root` to select a different processing root. A relative root is resolved from the current directory when the command starts.

```sh
remove-except --root work --dry-run keep.txt '**/*.md'
```

Choose the output layout with `--layout tree|flat`; the default is `tree`. Use `--summary keep|delete|both` to show counts instead of paths in the selected sections. To sort by item type, combine `--sort-by-type` with `--layout flat`.

```sh
remove-except --dry-run --layout flat keep.txt
remove-except --dry-run --layout flat --sort-by-type keep.txt
remove-except --dry-run --summary delete keep.txt
remove-except --dry-run --summary both keep.txt
```

The short forms of `--dry-run` and `--force` are `-n` and `-f`. See all options with `remove-except --help` and the version with `remove-except --version`.

The keep count includes required ancestor directories. The deletion-item count includes descendants of deletion-target directories, while the deletion-operation count is the number of top-level items passed to the deletion routine.

### Display and Command Integration

The plan starts with the execution status and processing root. On a TTY, the status labels `PREVIEW ONLY`, `CONFIRM TO DELETE`, `FORCE DELETE`, and `NO REMOVALS` are highlighted in cyan, yellow, red, and green, respectively. The same labels and descriptions identify each state when color is disabled. For human-readable output, ordinary Windows drive and UNC paths omit the `\\?\` prefix. Paths that would become ambiguous in ordinary notation, such as components ending in a space or period, retain the extended form. Control characters in file names and roots are rendered visibly to prevent line or terminal-decoration spoofing. Display formatting does not change the internal root path or deletion targets.

Preview header example:

```text
PREVIEW ONLY  Nothing will be removed
Root: S:\work\project
```

In full listings, tree layout appends `/` to directory names and `@` to symbolic-link names. A retained parent directory that appears only in the deletion list is shown in green when color is enabled and marked with `+` when color is disabled. Flat layout uses `ItemType` and `RelativePath` columns; without color, directory and link paths end in `/` and `@`, respectively. Flat layout does not mark retained parent directories. In full listings, a brief legend appears immediately after the root and reflects the selected layout and color state. With color enabled, the names `blue`, `cyan`, and `green` following `Colors:` use their respective colors. Without color, `Markers:` explains the symbols.

The color and marker meanings and status labels are also documented in the `Output` section of `remove-except --help`. When color is disabled, the same markers are used and help output has no ANSI styling. Summary output omits item paths and briefly reports counts in the `KEEP` and `DELETE` sections. The keep count includes required ancestors and separately reports direct pattern matches. The number of deletion items and the number of top-level deletion operations are also shown separately. Before deleting, always review every item with `--dry-run` and without `--summary`.

Example with summaries for both sections:

```text
KEEP:
    1 kept item (including required ancestors); 1 direct pattern match
DELETE:
    14 deletion items; 8 top-level deletion operations

Summary sections omit paths. Run --dry-run without --summary to review every item before deleting.
```

Plans and execution results are written to standard output. Warnings, errors, and confirmation prompts are written to standard error. Inputs are currently positional arguments and output is human-readable text. Reading patterns from standard input and stable machine-readable output are not supported. Parsing standard output from another program is not a supported use case.

During development, run the tool from the repository root with `cargo run --`.

```sh
cargo run -- --dry-run keep.txt keep '**/*.md'
```

## General Specification

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

Specify glob patterns for paths you want to keep. Separate multiple patterns with spaces.

```sh
remove-except --dry-run '**/*.md'
```

`readme.md` is kept, while `keep.txt`, `todo.txt`, `reports`, and `archive` are shown as deletion targets. The preview displays both kept items and deletion targets. `**` matches paths containing `/`, so `**/*.md` matches `.md` files in the current directory and in subdirectories.

Common glob syntax:

| Pattern | Meaning |
| --- | --- |
| `*.md` | `.md` files directly in the current directory. `*` does not cross `/`. |
| `**/*.md` | Paths ending in `.md` at any depth. `**` matches paths containing `/`. |
| `reports/report?.csv` | `?` matches any single character except `/`. This example matches `report1.csv` and `report2.csv`. |
| `reports/report[12].csv` | `[]` matches one of the listed characters. This example matches `report1.csv` and `report2.csv`. |

To limit a glob to a particular subdirectory, prefix the pattern with a path relative to that directory.

```sh
remove-except --dry-run 'reports/**/*.csv'
```

This example keeps `reports/report1.csv`, `reports/report2.csv`, and `reports/2025/annual.csv`. Because `**/` matches zero or more directory levels, `reports/**/*.csv` applies to `.csv` files directly under `reports` and at deeper levels. Quote globs to prevent the shell from expanding them before the command runs.

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

Use `--dry-run` in any example to review the kept items and deletion targets. Omit `--dry-run` only when you intend to delete. Normal execution also shows both lists before asking for confirmation.

### Rules

- **Processing scope:** By default, the tool recursively processes the current directory from which it was started. Use `--root` to process another directory. A relative root is resolved from the startup directory. The root itself is never included in the plan or deleted. A filesystem root is rejected for safety.
- **Keep patterns:** At least one pattern is required. With multiple patterns, an item is kept if it matches any one of them. Patterns are positional arguments, and leading or trailing spaces are part of the name. Quote patterns containing spaces in your shell.
- **Paths:** Relative patterns are resolved from the selected processing root. Absolute patterns are accepted only when they are inside the normalized root. In relative patterns and in the part of an absolute pattern after its matching root prefix, `.` components and repeated separators are normalized. Absolute patterns have their separators converted to `/` before the root prefix is matched, but `.` components and repeated separators within that prefix are not normalized. If the prefix does not match, the pattern is rejected as outside the root. Patterns containing `..` as a path component, empty patterns, and ambiguous Windows drive-relative paths (for example, `C:folder`) are errors.
- **Globs:** `*`, `**`, `?`, and character classes `[]` are supported. `*` and `?` do not cross `/`; `**` matches paths containing `/`. For example, `*.md` matches `.md` files at the top level, while `**/*.md` matches them at any depth. Quote globs to prevent shell expansion.
- **Directories:** A matched item and its parent directories are kept. Specifying a directory path without a glob keeps that directory and all of its contents. `dir/*` matches only items directly under `dir`; it does not recursively keep the contents of matched subdirectories.
- **Keep the entire processing root:** `.` and `./` represent the processing root and keep everything under it.
- **Case sensitivity:** Windows uses the tool's Unicode simple case folding (non-Turkic); Ubuntu is case-sensitive. On Windows, `ä` and `Ä`, and `ß` and `ẞ`, are treated as equal, but `ß` and `SS` are not. The tool does not follow per-directory NTFS case-sensitivity settings. Windows and Ubuntu are supported.
- **Symbolic links:** A symbolic link specified as the root is resolved to its target. Links under the processing root are not followed and are treated as individual items. If a link is a deletion target, only the link is removed, not its target.
- **Display and deletion:** The default `--layout tree` shows kept items and deletion targets as a
  tree. `--layout flat` shows `ItemType` and `RelativePath` columns. The legend reflects the
  selected layout and color state. `--sort-by-type` requires `--layout flat` and sorts by Directory,
  File, then Symlink; items of each type are sorted by relative path. Without this option, items are
  sorted by relative path. Sorting affects display only and does not change the deletion plan or
  deletion order. `--summary keep|delete|both` replaces selected paths with concise counts and
  distinguishes kept items (including ancestors) from direct matches, and deletion items from
  top-level deletion operations. Because summaries omit paths, review every item with `--dry-run`
  and without `--summary` before deleting. Normal execution shows the plan before confirmation, and
  the default answer is No. `--force` skips confirmation only.
- **Standard input and output:** Plans and execution results go to standard output; warnings, errors, and confirmation prompts go to standard error. Output is currently human-readable text; machine-readable output and pattern input from standard input are not provided. Passing positional arguments from tools such as `xargs` may be useful for future integrations, but no stable pipeline interface is promised.
- **No matches:** If no item matches any pattern, a warning is shown and everything under the root becomes a deletion target. Always review the targets with a dry run.

## Developer Documentation

### Project Structure

- `src/main.rs`: Parses CLI arguments, creates a plan for the selected root, displays the lists, asks for confirmation, and deletes filesystem items.
- `src/lib.rs`: Re-exports the public API from the crate root and coordinates scanning and plan creation through `build_plan`.
- `src/model.rs`: Defines the public plan types and read-only getters. `src/pattern.rs` handles glob matching; `src/path.rs` handles path validation and normalization.
- `tests/cli.rs`: Runs the CLI in temporary directories and verifies deletion, confirmation, and display. `tests/library.rs` tests the public library API and platform-specific case matching. Binary unit tests cover colored and uncolored rendering.

### Manual Testing

`Prepare-Manual-Test.ps1` creates `manual-test-workspace` and fixture files for manually checking the Rust CLI. `keep-link` is a symbolic link to `keep.txt` used to verify the `@` marker in previews. On Windows, creating symbolic links may require Developer Mode or administrator privileges. Run the script from the repository root.

```powershell
.\Prepare-Manual-Test.ps1
Set-Location .\manual-test-workspace
cargo run --manifest-path ..\Cargo.toml -- --dry-run keep.txt keep '*.md'
```

The preview lists `keep.txt`, `release-notes.md`, the `keep` directory, and its contents as kept; all other items are deletion targets. To try an actual deletion, omit `--dry-run` and answer `n` at the prompt to cancel. If you perform a deletion, return to the repository root with `Set-Location ..` and rerun the script to restore the known fixtures.

On Ubuntu (including WSL), use `prepare-manual-test.sh` to create the same fixtures. In WSL, work in a repository copy under the Linux home directory rather than under `/mnt/...`. Run these commands from that copy's repository root; ensure that Cargo is available on `PATH`.

```bash
bash ./prepare-manual-test.sh
cd manual-test-workspace
cargo run --manifest-path ../Cargo.toml -- --dry-run keep.txt keep '*.md'
```

To cancel at the confirmation prompt, omit `--dry-run` and answer `n`. After an actual deletion, return to the repository root with `cd ..` and rerun the generator.

Both scripts create or overwrite only known fixture files. They replace `keep-link` only when that known symbolic link points to `keep.txt`. They do not delete other files or links in the directory. Use `-Path` in PowerShell or the optional workspace-path argument in Bash to create fixtures elsewhere. If a regular file or a different link conflicts with `keep-link`, the script reports an error instead of overwriting it.

```powershell
.\Prepare-Manual-Test.ps1 -Path 'C:\work\manual-fixtures'
```

```bash
bash ./prepare-manual-test.sh "$HOME/manual-fixtures"
```

For a custom workspace location, use the absolute path to the repository's `Cargo.toml` with `--manifest-path` when running Cargo from the fixture directory.

### Plan API

The library function `build_plan(root, patterns)` creates a `RemovalPlan` from the selected root and keep patterns. It returns an error if no patterns are provided, a pattern is empty or contains an invalid glob, an absolute pattern is outside the root, a pattern traverses to a parent directory, the root is a filesystem root or is not a directory, or a Windows drive-relative path is used. Resolving or scanning the root can also fail. `.` and `./` both represent the processing root and keep everything beneath it.

Read `RemovalPlan` and `PlannedItem` data through their getters. Getters return references and do not allow the plan to be modified. Code that accessed public fields directly should be updated to call the getters.

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

- `root()`: The normalized processing root.
- `direct_match_count()`: The number of items directly matched by patterns; used to determine whether to warn that nothing matched.
- `keep_items()`: All kept items, including required ancestor directories, sorted by relative path.
- `delete_items()`: All deletion targets, sorted by relative path.
- `delete_roots()`: The top-level items passed to deletion. Descendants of deletion-target directories are excluded to avoid redundant recursive deletions.

Each `PlannedItem` can be read through `path()`, `relative_path()`, and `item_type()`. Relative paths use `/` separators. `path()` returns an absolute path under the normalized root. On Windows, the standard library's canonicalized path may include the extended-path prefix `\\?\`.

The scan does not follow symbolic links. After creating the plan, the CLI deletes the items in `delete_roots`. If a filesystem error occurs during deletion, the command exits with an error, but items already deleted before the error cannot be restored.

### Building and Validation

This project uses Edition 2024. Use Rust 1.85 or later for development.

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
cargo doc --no-deps
```

To run the same Rust checks on Ubuntu from Windows, run this script from the repository root.

```powershell
.\local-test-tools\Test-Rust-WSL.ps1
```

### Performance Benchmarks

Plan-generation benchmarks use only the standard library and existing development dependencies. They create temporary fixtures before measuring `build_plan`; fixture creation time is excluded. The benchmarks compare a broad 2,080-item tree with one glob, multiple globs, a literal directory, and all items kept; an 8,208-item tree with sparse matches and all items kept; and matches in a deeper tree.

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

CLI measurements include process startup, plan generation, and output rendering. Do not compare them as if they measured plan generation alone, and keep fixture contents identical between runs.

Dependencies are managed in `Cargo.toml`. The CLI uses `clap`, confirmation prompts use `dialoguer`, output styling uses `console`, glob matching uses `globset`, directory traversal uses `walkdir`, and error context uses `anyhow`. Windows Unicode glob matching uses `regex-automata` and `unicode-casefold`.
