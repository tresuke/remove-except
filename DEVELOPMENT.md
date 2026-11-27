# Development Guide

This guide is the English translation of [DEVELOPMENT.ja.md](DEVELOPMENT.ja.md), the primary development guide. It covers repository development, validation, manual fixtures, the public library API, releases, and performance measurements. User-facing CLI behavior is documented in [README.ja.md](README.ja.md) and [README.md](README.md).

**Read by task:** [Run and validate](#run-and-validate) · [CodeQL](#codeql) · [Manual fixtures](#manual-fixtures) · [Public library API](#public-library-api) · [Releases](#preparing-and-publishing-a-release) · [Performance](#performance-benchmarks)

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

## CodeQL

The [CodeQL workflow](.github/workflows/codeql.yml) uses Advanced setup to run the official default Rust code-scanning suite separately on Windows and Ubuntu. It does not change the existing Rust CI, MSRV validation, or required checks. Do not enable Default setup alongside it.

| Event | Analyzed source |
| --- | --- |
| `pull_request` | Merge commit of a PR targeting main |
| `push` | main |
| `schedule` | main every Monday at 05:23 UTC |
| `workflow_dispatch` | Selected ref; use main to verify the baseline |

Verify scheduled and manual execution after the workflow exists on main. Only updates to the same PR cancel older in-progress runs. Concurrency groups separate event types and refs; main push, scheduled, and manual analyses do not cancel in-progress runs. Pending runs can still be replaced by later runs, so this does not guarantee an analysis history for every commit.

### Analysis Scope and Results

The workflow installs Rust stable and extracts with `build-mode: none`. There is no manual build step, but the extractor can run Cargo, `build.rs`, and procedural macros. The init step loads the [shared configuration](.github/codeql/codeql-config.yml) through `config-file`, excluding `target/**`. It does not exclude src, tests, or benches wholesale. Separate hosts help inspect conditional code; file extraction alone does not establish complete semantic analysis of every branch or macro.

Use GitHub **Actions → CodeQL** to inspect each OS's logs, extraction diagnostics, and upload processing. Review findings under **Security → Code scanning** and in PR checks and annotations. Categories are `/language:rust/os:ubuntu-latest` and `/language:rust/os:windows-latest`. Check job success, SARIF registration, and the Code scanning check result separately.

The initial setup PR may lack a main baseline. Evaluate new PR alerts after the post-merge main analysis is registered; do not assume that every initial finding is new or guarantee whether merging is allowed. Fork PRs follow GitHub's normal approval and token restrictions. Record restricted execution or registration as unverified or failed; do not bypass restrictions with `pull_request_target` or a PAT.

SARIF is saved to GitHub. `upload-database: false` disables automatic DB uploads, and the workflow adds no DB/debug/SARIF artifact storage. To maintain the no-DB-publication policy, separately assess reruns in debug mode, which can generate debugging artifacts.

### Local Investigation and Maintenance

Match the alert's commit and OS, compare CLI, bundle, and query pack versions with the run logs, and create a new local DB. A DB represents the source at extraction time and must be recreated after code changes. Save DBs and SARIF outside the repository at unused paths; do not overwrite existing DBs. After obtaining the compatible official Rust pack, use the shared exclusions as follows. Replace angle-bracket placeholders with actual paths and the compatible version.

```sh
codeql database create '<new-db-path>' --language=rust --build-mode=none --source-root=. --codescanning-config=.github/codeql/codeql-config.yml --threads=2 --ram=4096
codeql database analyze '<new-db-path>' 'codeql/rust-queries@<compatible-version>:codeql-suites/rust-code-scanning.qls' --format=sarif-latest --output='<new-sarif-path>' --threads=2 --ram=4096
```

Open the new DB in the VS Code CodeQL extension to inspect the relevant query, source, and paths. Diagnostic queries may have raw results even when alerts show zero results. Do not rely on automatically downloading GitHub DBs. Successful local analysis does not verify GitHub permissions, uploads, or PR annotations.

Permissions are limited to `contents: read`, plus `actions: read` and `security-events: write` in the CodeQL job, using the default `GITHUB_TOKEN`. Checkout does not persist credentials. No new secrets, PAT, `id-token: write`, or `contents: write` are needed.

Actions are pinned to full SHAs, with `tools: linked` selecting the corresponding bundle. Before accepting existing Dependabot update proposals, verify the selected-actions policy's exact repository/ref allowance, linked bundle and query compatibility, and analysis on both OSs. Explicitly allow `github/codeql-action/init@<full-sha>` and `github/codeql-action/analyze@<full-sha>` individually; do not assume that a repository-root reference permits both subactions. Separately approve any necessary allowance changes; do not broaden permissions without evidence. Weekly reruns alone do not update pinned analysis tools or queries.

CodeQL does not replace tests, Clippy, `cargo audit`, or deletion-specification review; zero findings do not prove safety. Treat alert fixes or dismissals, custom queries, DB publication, and additional Code scanning required checks as separate changes.

## Manual Fixtures

`Prepare-Manual-Test.ps1` and `prepare-manual-test.sh` create or overwrite known fixture files in `manual-test-workspace`. Make sure you have no work files there that need to be kept before running either script. Run the platform-appropriate script from the repository root, then enter the fixture directory and run the CLI with `--manifest-path`:

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

The scripts replace `keep-link` only when it is the expected symbolic link to `keep.txt`; they do not delete unrelated files or links. If a regular file or a different link conflicts with `keep-link`, they report an error. Use `-Path` in PowerShell or the optional output-path argument in Bash to create fixtures elsewhere. On Windows, creating symbolic links may require Developer Mode or administrator privileges. When running from a custom fixture location, pass the absolute path to the repository's `Cargo.toml` using `--manifest-path`.

## Public Library API

The [API documentation for the latest published version](https://docs.rs/remove-except/latest/remove_except/) is available on docs.rs. The development API may differ from the published API. See “Run and Validate” for generating API documentation locally. The crate's `build_plan(root, patterns)` function creates a `RemovalPlan` for a processing directory and keep patterns.

It returns an error for missing or empty patterns, invalid globs, absolute patterns outside the selected directory, parent traversal, filesystem roots, non-directory roots, Windows drive-relative paths, and roots that cannot be resolved. Input validation errors can be identified with `anyhow::Error::downcast_ref::<ValidationError>()`; filesystem errors during scanning are not `ValidationError`s. `.` and `./` represent the selected root and keep everything beneath it.

Read `RemovalPlan` and `PlannedItem` through their getters. Getters that return references do not allow plan contents to be mutated. `direct_match_count()` returns a `usize`, and `item_type()` returns an `ItemType` by value. Code that used public fields should migrate to getter calls.

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
- `direct_match_count()`: Number of items directly matching all specified keep conditions, including any patterns; required ancestors are not counted.
- `modified_range()` / `size_range()`: The modification-time / file-size range used to create the plan, or `None` when unspecified.
- `keep_items()`: Kept items, including required ancestors, sorted by relative path.
- `delete_items()`: All deletion targets, sorted by relative path.
- `delete_roots()`: Top-level items passed to deletion; descendants of deleted directories are excluded.

`PlannedItem` exposes `path()`, `relative_path()`, and `item_type()`. Relative paths use `/` separators. `path()` returns an absolute path below the normalized root; on Windows it may include the standard library's `\\?\` extended-path prefix.

The scan does not follow symbolic links. The CLI deletes `delete_roots` after plan creation. A deletion error does not restore items already removed.

`build_plan(root, patterns)` requires at least one pattern and does not accept time or size filters. Use `build_plan_with_options(root, patterns, options)` to apply modification-time or file-size bounds, including when no patterns are supplied. It requires at least one pattern or time/size bound. With an active time or size filter, only regular files matching every keep condition directly match. Without those filters, non-file items can also match patterns directly. The API may differ in published versions; check the API documentation for the version you use.

## Preparing and Publishing a Release

During the 0.x series, bug fixes that restore documented behavior use patch releases. Backward-compatible feature additions, breaking CLI or public library API changes, and MSRV increases use minor releases. For a release containing multiple changes, use the highest applicable increment. Bump the version once the release contents are settled, updating the root package version in `Cargo.toml` and `Cargo.lock` together. Documentation-only changes do not normally require a version bump.

GitHub source/binary Releases and crates.io are separate distribution channels. Record the target version, included changes, intended channels, and approval scope for each publication in the release plan. Approval for one channel does not authorize publication to another. An approval applies only to its recorded version, channel, and action. Do not request the same explicit approval again when its scope is unchanged; stop and confirm again if the scope changes.

### Ordered Release Procedure

| Stage | Steps |
| --- | --- |
| Common preparation | 1–4 |
| GitHub Release | 5–6, when selected |
| crates.io | 7, when selected |
| Post-publication verification | 8, for the selected channels |

Record each stage's results in the release preparation PR:

- Record commits/tags, workflow runs, Release/crate URLs, and open items. Also record the actual source commit for each channel.
- If the GitHub Release tag commit differs from the crate source commit, record the difference and reason, verify there are no unintended code changes, and record approval for any intentional differences.
- Classify each check as passed, failed, not run, or not applicable, with a reason. Do not treat not-applicable checks as passed.
- Unresolved failures or unrun required checks block the next publication gate. Do not waive checks required by repository policy.
- For individually permissible decisions, such as an alternative to an explicitly requested review condition, record the scope, reason, and approval.

1. **Freeze the contents and distribution scope.** Review the changes since the last formal release and select a version under the 0.x rules. Decide separately whether to provide a GitHub Release using the Prepare Release workflow (GitHub source archives and Windows x64/Linux x64 binaries) and a crates.io package. Assess whether any change affects keep/delete decisions, deletion results, safety, or compatibility.

   If a change is made after the contents are frozen, reassess the affected channels and gates and repeat only the relevant checks.

   - For documentation-only changes, recheck the documentation and package contents. For code, test, or build-environment changes, repeat the affected reviews and validation.
   - Changes to the version, channels, or release scope require renewed approval.
   - After tagging, assess impact per channel and do not change existing tags or published artifacts. If an existing binary must change, decide whether to use a new version/tag or cancel publication.
   - For an unpublished crate, check its difference from the tag commit, reason, code changes, approval scope, and package validation before deciding whether it can be published from a separate clean commit. Publishing the same version from a different commit is limited to differences within the approved scope and consistent with versioning policy.
   - Reassess the version and cross-channel consistency for code differences; stop until required approvals and validation are complete.
2. **Finalize documentation included in the distributions.** Treat `README.ja.md` as the source of truth and synchronize `README.md`; review release notes, tag-install commands, archive names, and crates.io instructions. Update development-version labels and version-specific availability notes so they accurately describe the target tag. **Prepare Release checks out the tag's two READMEs and copies them into the binary archives; Cargo includes the `README.md` from the source used to publish the crate. A post-publication edit on main does not update either an existing archive or a published crate.** Therefore merge documentation needed by a distribution before tagging or crate publication. Avoid time-sensitive claims that become false after publication; prefer version-neutral installation instructions that remain accurate both before and after a release.
3. **Merge the version/release PR.** Update only the root package versions in `Cargo.toml` and `Cargo.lock`; include finalized user documentation and developer release notes as needed. Merge after required CI and reviews pass, then verify Rust CI and CodeQL on the resulting `main`. If an explicitly requested review condition cannot be met, do not substitute silently; report the limitation and options and stop. Proceed with an alternative only after explicit user approval. When the release contents add or change environment-dependent code or tests, including OS-specific behavior or dependencies on time zones, external data, or build tools, run and record affected checks in the relevant release environment before tagging. Documentation-only changes do not require full environment testing for this reason. If publishing a crate, also verify the package inventory and dry run before creating a tag.
4. **Tag the finalized `main` commit.** Confirm the version tag is unused, `main` is current and its CI passes, and the package version matches the intended tag. Create and push an annotated tag only after explicit approval. Never move or reuse a published tag.

   ```sh
   git tag -a vX.Y.Z -m "remove-except vX.Y.Z"
   git push origin vX.Y.Z
   ```

5. **If a GitHub Release is in scope, build binaries from the tag.** Manually dispatch **Prepare Release** from the exact version tag. Skip this step and step 6 if no GitHub Release is planned:

   ```sh
   gh workflow run prepare-release.yml --ref vX.Y.Z
   ```

   Wait for every workflow job to finish successfully and for the draft to be created. Treat failures, skipped checks, and warnings explicitly; do not publish until blockers are resolved. The workflow verifies that the tag is on `main` and matches the package version; tests and audits Windows x64/Linux x64 binaries and the lockfile; builds with a pinned `cargo-auditable`; and creates archives containing the binaries, both READMEs, and licenses, with SHA-256 checksums and attestations. The Linux binary is built in an Ubuntu 22.04 container and requires glibc 2.35 or later. Embedded dependency metadata is not a vulnerability scan.
6. **If a GitHub Release is in scope, manually review the draft before publication.** Skip this step if no GitHub Release is planned. Do not select **Publish release** until all of these are confirmed:

   ```sh
   gh release view vX.Y.Z --repo tresuke/remove-except --json isDraft,tagName,targetCommitish,assets,body,url
   ```

   Download assets into a newly created temporary directory outside the repository. Replace `vX.Y.Z` with the target version.

   ```powershell
   $releaseDir = Join-Path $env:TEMP ("remove-except-vX.Y.Z-" + [guid]::NewGuid())
   New-Item -ItemType Directory -Path $releaseDir
   gh release download vX.Y.Z --repo tresuke/remove-except --dir $releaseDir
   ```

   ```sh
   release_dir="$(mktemp -d)"
   gh release download vX.Y.Z --repo tresuke/remove-except --dir "$release_dir"
   ```

   Also confirm that the tag points to the planned commit and is an ancestor of the current `main`. Compare the `git rev-parse` output with the planned commit:

   ```sh
   git fetch origin main --tags
   git rev-parse "vX.Y.Z^{commit}"
   git merge-base --is-ancestor "vX.Y.Z^{commit}" origin/main
   ```

   Verify each archive checksum, inspect its contents, and check the executable version. Perform each binary check on its target OS.

   ```powershell
   Get-FileHash -LiteralPath (Join-Path $releaseDir 'remove-except-vX.Y.Z-x86_64-pc-windows-msvc.zip') -Algorithm SHA256
   Get-Content -LiteralPath (Join-Path $releaseDir 'remove-except-vX.Y.Z-x86_64-pc-windows-msvc.zip.sha256')
   Expand-Archive -LiteralPath (Join-Path $releaseDir 'remove-except-vX.Y.Z-x86_64-pc-windows-msvc.zip') -DestinationPath (Join-Path $releaseDir 'windows')
   Get-ChildItem -LiteralPath (Join-Path $releaseDir 'windows') -Recurse
   & (Join-Path $releaseDir 'windows\remove-except-vX.Y.Z-x86_64-pc-windows-msvc\remove-except.exe') --version
   ```

   ```sh
   (cd "$release_dir" && sha256sum --check remove-except-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256)
   tar -tzf "$release_dir/remove-except-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz"
   mkdir "$release_dir/linux"
   tar -xzf "$release_dir/remove-except-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz" -C "$release_dir/linux"
   "$release_dir/linux/remove-except-vX.Y.Z-x86_64-unknown-linux-gnu/remove-except" --version
   ```

   On Windows, compare the computed SHA-256 with the checksum-file value, ignoring letter case; do not proceed if they differ. Remove the temporary directory after verification.

   - The draft tag, source commit, and package version match the plan; the tagged commit is an ancestor of `main`.
   - Release notes accurately describe user-facing changes since the last formal release. Review generated notes rather than assuming they are correct.
   - The Windows ZIP, Linux tar.gz, and both `.sha256` files are present. Download the assets and verify each checksum against its sidecar.
   - Each archive contains the correct-version executable, both READMEs, and both licenses. Check `--version` on both binaries.
   - Verify both archive attestations, for example: `gh attestation verify <archive-path> --repo tresuke/remove-except --signer-workflow tresuke/remove-except/.github/workflows/prepare-release.yml`.
   - Record the results and publish decision; explicitly approve publishing the GitHub Release. Pushing a tag or completing the workflow does not publish the Release.
7. **Publish to crates.io as a separate manual operation.** Do this only if crates.io is included in the release plan and its publication is explicitly approved. Confirm the packaged README will remain accurate after upload, the version is not already registered, and the package checks below pass. Use only maintainer-local Cargo credentials; never put tokens in the repository or Actions. A dry run and a real upload are separate operations. Verify the registry entry and an isolated install after upload. See “Crate Distribution Validation.”
8. **Complete post-publication verification.** If a GitHub Release is in scope, verify its publication status, four assets, checksums, and attestations. If crates.io is in scope, verify the registry version and installation result. Check that the README on `main` describes only distributions that are actually available, and synchronize it through the normal documentation PR process if needed. Record the final outcomes in the release preparation PR.

### Post-publication Documentation and Distributions

Post-publication checks verify and, if necessary, synchronize the user-facing README on `main`. They do not update READMEs embedded in already-published binaries or a crate. If an artifact's documentation is wrong, do not replace that artifact under the same version or move its tag. Record the impact and correct it in the next appropriate version. Documentation-only corrections do not normally require a version bump or new release.

After the next release, assess whether results and evidence are recorded for applicable procedures, unmet items remain clearly marked as unmet with their reasons and decisions, and progression followed required decisions and approvals. Mark exception handling that did not occur as not evaluated. Assess effectiveness along with operational burden.

- [ ] `README.ja.md` and `README.md` agree on Release links, asset names, checksums, source tags, and crates.io installation instructions.
- [ ] Only published distribution channels are described as available; unpublished assets are not presented as downloadable.
- [ ] Make any necessary repository README synchronization through a normal PR and verify the merged `main`. If the docs already match, make no change and record the verification.
- [ ] Record channel-specific validation, manual actions, approvals, and URLs in the release preparation PR before marking post-release work complete.

### Crate Distribution Validation

When crates.io is in scope, run these checks from the clean commit intended for publication and inspect the file inventory, including the README:

```sh
cargo audit --file Cargo.lock
cargo package --list --locked
cargo package --locked
cargo publish --dry-run --locked
```

Keep the Cargo `include` allowlist limited to distributable source, user documentation, licenses, and required build targets; exclude local instructions, tests, fixtures, and developer-only utilities. A successful dry run does not prove that credentials can publish or that an upload succeeded. Confirm that the version is not registered. `cargo owner --list remove-except` can confirm maintainer access but does not confirm permission to publish. If authentication or permission fails, stop and have the maintainer repair credentials locally.

Create a token from crates.io **Account Settings → API Tokens** with only the permissions needed to publish a new version of the existing `remove-except` crate. Restrict it to the crate and set a short expiration when available. On a maintainer-controlled local environment, check the configured Cargo credential provider. To use Cargo's prompt, run `cargo login` and enter the token only there. The default `cargo:token` provider stores the token in plain text in `$CARGO_HOME/credentials.toml`; prefer a secure OS credential store when available. Never put tokens in command arguments, environment variables, chat, source, logs, the repository, or Actions, and never record the token value.

After separate approval to publish, run this only in the maintainer's local environment:

```sh
cargo publish --locked
```

After upload, verify the crates.io version page/API and install the exact version to an isolated root with `cargo install remove-except --version '=X.Y.Z' --locked --root <temporary-directory>`. Run `--version` on the installed binary and remove the temporary installation after verification. A published version cannot be overwritten. Remember that the crate README, the README inside GitHub binary archives, and the README on `main` are distinct published artifacts.

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
