# Development Guide

This guide is the English translation of [DEVELOPMENT.ja.md](DEVELOPMENT.ja.md), the primary development guide. It covers repository development, validation, manual fixtures, the public library API, releases, and performance measurements. User-facing CLI behavior is documented in [README.ja.md](README.ja.md) and [README.md](README.md).

## Project Structure

- `src/main.rs`: CLI argument parsing, stdin pattern input, plan display (human-readable and JSON), confirmation, deletion, and exit codes.
- `src/lib.rs`: Public API exports and coordination of scanning and `RemovalPlan` creation through `build_plan`.
- `src/model.rs`: Public plan types and read-only getters.
- `src/pattern.rs`, `src/path.rs`, and `src/error.rs`: Pattern matching, path validation and normalization, and `ValidationError`.
- `tests/cli.rs`: Process-level CLI behavior in temporary directories. `tests/library.rs`: Public API and platform-specific matching. Binary unit tests cover rendering.

## Run and Validate

Run the CLI from the repository root during development:

```sh
cargo run -- --dry-run keep.txt keep '**/*.md'
```

The project uses Edition 2024 and has an MSRV of Rust 1.88. The MSRV reflects the highest declared Rust-version requirement among the currently locked dependencies, not Edition 2024 alone. CI tests Rust 1.88 and stable on Windows and Ubuntu. Its standard checks are:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

CI uses `--locked` for tests and Clippy. Also run these checks locally when practical. For Rust Doc or public API documentation changes, verify documentation generation:

```sh
cargo doc --no-deps
```

To run the Ubuntu checks from Windows, use the Docker runner from the repository root:

```powershell
.\tools\Test-Rust-Docker.ps1
```

If Docker is not on `PATH`, pass its executable path:

```powershell
.\tools\Test-Rust-Docker.ps1 -DockerPath 'C:\path\to\docker.exe'
```

The connected Docker Engine must run Linux containers and expose the Windows repository drive inside the engine as `/mnt/<drive>/...`. For engines using a different mount mapping, pass the repository root's absolute Linux path with `-DockerSourcePath`. The runner builds an Ubuntu 26.04 image and runs formatting, tests, and Clippy separately with Rust 1.88.0 and stable. It mounts the repository read-only, excludes local-only files, and copies the source into a fixed temporary Linux directory. Cargo build output is kept in the `remove-except-cargo-target` volume, with separate directories per toolchain; the Windows `target` directory is not used. The Cargo registry cache is kept in the separate `remove-except-cargo-registry` volume. Remove the target cache with `docker volume rm remove-except-cargo-target`; remove the registry cache with `docker volume rm remove-except-cargo-registry`.

The target volume remains on the Docker Engine after `docker run --rm` exits and may contain source-derived build artifacts and debug information. Remove it with the command above when it is no longer needed. The target cache is shared on that Docker Engine, so run this runner sequentially, not concurrently. Docker Ubuntu checks complement, rather than replace, Windows-native checks. Changes to Windows console confirmation still require separate verification on Windows.

Build a release binary when needed:

```sh
cargo build --release
```

If a dependency update or code change requires a newer compiler, reassess `rust-version`, the CI toolchain matrix, and both READMEs in the same change. Use the Docker runner above for local Ubuntu validation. When using Ubuntu or WSL directly, run Linux-native Cargo from a repository copy under the Linux home directory, not from a Windows-mounted worktree.

## Manual Fixtures

`Prepare-Manual-Test.ps1` and `prepare-manual-test.sh` create fixtures in `manual-test-workspace`. Run the platform-appropriate script from the repository root, then enter the fixture directory and run the CLI with `--manifest-path`:

```powershell
.\Prepare-Manual-Test.ps1
Set-Location .\manual-test-workspace
cargo run --manifest-path ..\Cargo.toml -- --dry-run keep.txt keep '*.md'
```

```bash
bash ./prepare-manual-test.sh
cd manual-test-workspace
cargo run --manifest-path ../Cargo.toml -- --dry-run keep.txt keep '*.md'
```

The preview should keep `keep.txt`, `release-notes.md`, and the `keep` directory and its contents. Other items are deletion targets. To try a real deletion, omit `--dry-run`, review the full plan, and answer `n` to cancel. After a deletion, return to the repository root and rerun the fixture script.

The scripts create or overwrite known fixture files. They replace `keep-link` only when it is the expected symbolic link to `keep.txt`; they do not delete unrelated files or links. If a regular file or a different link conflicts with `keep-link`, they report an error. Use `-Path` in PowerShell or the optional output-path argument in Bash to create fixtures elsewhere. On Windows, creating symbolic links may require Developer Mode or administrator privileges. When running from a custom fixture location, pass the absolute path to the repository's `Cargo.toml` using `--manifest-path`.

## Public Library API

Publication of the public library API reference on docs.rs is planned, but it is not available yet. See “Run and Validate” for generating the API documentation locally. The crate's `build_plan(root, patterns)` function creates a `RemovalPlan` for a processing directory and keep patterns.

It returns an error for missing or empty patterns, invalid globs, absolute patterns outside the selected directory, parent traversal, filesystem roots, non-directory roots, Windows drive-relative paths, and roots that cannot be resolved. Input validation errors can be identified with `anyhow::Error::downcast_ref::<ValidationError>()`; filesystem errors during scanning are not `ValidationError`s. `.` and `./` represent the selected root and keep everything beneath it.

Read `RemovalPlan` and `PlannedItem` through their getters. They return references; plan contents cannot be mutated through them. Code that used public fields should migrate to getter calls.

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

- `root()`: Normalized processing directory.
- `direct_match_count()`: Number of items directly matched by patterns.
- `keep_items()`: Kept items, including required ancestors, sorted by relative path.
- `delete_items()`: All deletion targets, sorted by relative path.
- `delete_roots()`: Top-level items passed to deletion; descendants of deleted directories are excluded.

`PlannedItem` exposes `path()`, `relative_path()`, and `item_type()`. Relative paths use `/` separators. `path()` returns an absolute path below the normalized root; on Windows it may include the standard library's `\\?\` extended-path prefix.

The scan does not follow symbolic links. The CLI deletes `delete_roots` after plan creation. A deletion error does not restore items already removed.

## Preparing a Release

During the 0.x series, bug fixes that restore documented behavior use patch releases. Backward-compatible feature additions, breaking CLI or public library API changes, and MSRV increases use minor releases. For a release containing multiple changes, use the highest applicable increment. Bump the version once the release contents are settled, updating the root package version in `Cargo.toml` and `Cargo.lock` together. Documentation-only changes do not normally require a version bump.

The published `v0.2.4` source-only release is immutable. For a future version, update the package version in `Cargo.toml` and the root package entry in `Cargo.lock` in a PR, merge after required checks pass, and verify CI on the resulting `main`. After release contents are approved, create and push a new version tag:

```sh
git tag -a vX.Y.Z -m "remove-except vX.Y.Z"
git push origin vX.Y.Z
```

Run the manually dispatched **Prepare Release** workflow from that exact version tag, for example:

```sh
gh workflow run prepare-release.yml --ref vX.Y.Z
```

Verify that the tag points to a commit on `main` and matches the package version. The workflow validates Windows x64 and Linux x64 binaries, audits the lockfile and binaries with `cargo audit`, builds with a pinned `cargo-auditable` version, packages binaries with both READMEs and licenses, calculates SHA-256 checksums, attests archives, and creates or updates a draft GitHub Release. Embedded dependency metadata identifies crate names and versions; it is not a vulnerability scan or a defense against supply-chain attacks. The Linux binary is built in an Ubuntu 22.04 container and requires glibc 2.35 or later.

A tag push does not publish a release. Review the draft's source commit, generated notes, archives, checksums, and attestations before manually publishing it. Do not replace published contents under an existing version.

Before preparing a crate distribution, inspect and validate the package:

```sh
cargo package --list --locked
cargo package --locked
cargo publish --dry-run --locked
```

Keep the Cargo `include` allowlist limited to distributable source, user documentation, licenses, and required build targets; exclude local instructions, tests, fixtures, and developer-only utilities. A successful dry run does not publish the crate. crates.io publication is a separate manually approved action using maintainer-local credentials. Never store a crates.io token in the repository or release workflow.

## Performance Benchmarks

The `plan_generation` benchmark creates temporary fixtures before timing; fixture generation and the initial warm-up are outside the measurement interval. Its cases compare a broad 2,080-item tree with one glob, multiple globs, a literal directory, and all items kept; an 8,208-item tree with both sparse-match and all-items-kept cases; and matches in a deep 48-item tree. Each case reports the median, sample range, number of scanned items, number of patterns, and approximate items per second. Emphasize relevant cases when changing keep ratios or glob counts.

```sh
cargo bench --bench plan_generation
cargo bench --bench plan_generation -- --samples 25 --iterations 10
```

The default 9 samples × 3 iterations are for quick checks. For comparisons, use 25 samples × 10 iterations with the same OS, machine, and arguments. Measure on native Windows and Ubuntu; on Ubuntu, use a synchronized repository copy under the Linux home directory and record the Ubuntu release. Record target cases and measurement conditions. Review sample ranges as well as medians; if ranges overlap and the difference matters, repeat with 50 samples × 20 iterations. Do not combine absolute timings or improvement percentages across operating systems or unrelated cases. Do not claim a small change is an improvement when ranges overlap.

For workloads above 8,208 items, first add and measure a broad 50,000-item case on both operating systems. Consider 100,000 items only when needed. Continue excluding fixture-generation time.

To measure the full CLI, use a release binary and `hyperfine`. This dry-run example reads manual fixtures and does not delete them:

```powershell
cargo build --release
hyperfine --warmup 3 --runs 10 'target/release/remove-except.exe --dry-run --summary both --root manual-test-workspace keep.txt keep'
```

CLI measurements include process startup, planning, and rendering; do not compare them as if they measured `build_plan` alone. Keep fixtures identical between runs.
