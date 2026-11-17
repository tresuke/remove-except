# 開発ガイド

このガイドは開発者向け文書の日本語正本です。[English translation](DEVELOPMENT.md) も参照できます。CLI の利用者向け仕様は [README.ja.md](README.ja.md) と [README.md](README.md) を参照してください。

## プロジェクト構成

- `src/main.rs`: CLI 引数の解析、標準入力からのパターン読み込み、計画の表示（人向け・JSON）、確認、削除、終了コード。
- `src/lib.rs`: 公開 API の再エクスポートと、`build_plan` による走査・`RemovalPlan` 作成の調整。
- `src/model.rs`: 公開計画型と読み取り用 getter。
- `src/pattern.rs`、`src/path.rs`、`src/error.rs`: パターン照合、パスの検証と正規化、`ValidationError`。
- `tests/cli.rs`: 一時ディレクトリーを使ったプロセス単位の CLI 動作。`tests/library.rs`: 公開 API と OS ごとのパターン照合。描画の単体テストはバイナリーにあります。

## 実行と検証

開発中はリポジトリーのルートから CLI を実行できます。

```sh
cargo run -- --dry-run keep.txt keep '**/*.md'
```

Edition 2024 を使用し、MSRV は Rust 1.88 です。この MSRV はロック済み依存クレートが宣言する最低 Rust バージョンの最大値に基づき、Edition 2024 だけを理由にしたものではありません。CI は Windows と Ubuntu で Rust 1.88 および stable を検証します。標準のチェックは次のとおりです。

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

CI ではテストと Clippy に `--locked` を指定します。可能であればローカルでも同じチェックを実行してください。Rust Doc または公開 API の文書を変更した場合は、次でドキュメント生成を確認します。

```sh
cargo doc --no-deps
```

Windows から Ubuntu のチェックを実行するには、リポジトリーのルートで Docker runner を使います。

```powershell
.\tools\Test-Rust-Docker.ps1
```

Docker CLI が `PATH` にない場合は、実行ファイルのパスを指定できます。

```powershell
.\tools\Test-Rust-Docker.ps1 -DockerPath 'C:\path\to\docker.exe'
```

接続先の Docker Engine は Linux コンテナーを実行でき、Windows の処理対象ドライブをコンテナー側から `/mnt/<drive>/...` として参照できる必要があります。別の mount 表記を使う Engine では `-DockerSourcePath <absolute-linux-path>` で repository root の見え方を指定できます。runner は Ubuntu 26.04 イメージをビルドし、Rust 1.88.0 と stable のそれぞれで fmt、テスト、Clippy を実行します。リポジトリーは読み取り専用でマウントされ、個人用ファイルなどを除外して固定パスの一時 Linux ディレクトリーへコピーします。Cargo のビルド出力は `remove-except-cargo-target` volume にtoolchain別に保持され、Windows 側の `target` は使いません。Cargo registry cache は別の `remove-except-cargo-registry` volume に保持されます。target cacheを削除する場合は `docker volume rm remove-except-cargo-target`、registry cacheを削除する場合は `docker volume rm remove-except-cargo-registry` を実行してください。

target volume はDocker Engine側に残り、`docker run --rm` では削除されません。ソース由来のビルド成果物やdebug情報が残るため、不要になったら上記コマンドで削除してください。target cacheは同じDocker Engine上で共有されるため、このrunnerは同時実行せず順番に実行してください。このUbuntu検証はWindowsネイティブのチェックを置き換えません。Windowsコンソールの確認入力を変更した場合は、Windowsで別途確認してください。

必要に応じて release バイナリーをビルドします。

```sh
cargo build --release
```

依存更新やコード変更でより新しいコンパイラーが必要になった場合は、`rust-version`、CI の toolchain matrix、両 README の記載を合わせて見直します。Ubuntu のローカル検証には上記 Docker runner を使います。Ubuntu または WSL を直接使う場合も、Windows マウント上ではなく Linux ホームディレクトリー配下のリポジトリーコピーで Linux ネイティブの Cargo を実行してください。

## CodeQL

[CodeQL workflow](.github/workflows/codeql.yml) は Advanced setup で公式の既定 Rust code-scanning suite を Windows / Ubuntu のそれぞれで実行します。既存の Rust CI、MSRV 検証、必須チェックは変更しません。Default setup を同時に有効にしないでください。

| イベント | 解析対象 |
| --- | --- |
| `pull_request` | main 宛て PR の merge commit |
| `push` | main |
| `schedule` | 毎週月曜日 05:23 UTC の main |
| `workflow_dispatch` | 選択した ref。基準確認には main を使用 |

週次・手動起動の導入確認は workflow が main に存在してから行います。古い進行中実行をキャンセルするのは同じ PR の更新時だけです。concurrency はイベント種別と ref で分離し、main push・週次・手動の進行中解析は中止しません。ただし保留中の実行は後続実行で置き換わり得るため、すべての commit の解析履歴を保証するものではありません。

### 解析対象と結果の確認

Rust stable を設置し、`build-mode: none` で抽出します。手動ビルド手順はありませんが、抽出器は Cargo、`build.rs`、手続きマクロを実行し得ます。[共有設定](.github/codeql/codeql-config.yml) を init の `config-file` 入力で読み込み、`target/**` を除外します。src・tests・benches は一律除外しません。各 host の条件付きコードを確認する補助であり、ファイル収録だけで全分岐・マクロの意味解析が完全とは判断しません。

GitHub の **Actions → CodeQL** で各 OS のログ・抽出診断・アップロード処理を確認し、**Security → Code scanning** と PR のチェック・注釈で問題候補を確認します。category は `/language:rust/os:ubuntu-latest` と `/language:rust/os:windows-latest` に分離します。ジョブ成功、SARIF 登録、Code scanning 側チェックの結果は別々に確認してください。

初回の導入 PR では main 基準が未確立の可能性があります。マージ後の main 解析が登録されてから PR の新規アラート表示を評価し、初回表示が全件新規になることやマージ可否を事前に断定しないでください。fork PR は GitHub の通常の承認・トークン制限に従い、起動や登録が制限された場合は未確認または失敗として記録します。`pull_request_target` や PAT で回避しません。

GitHub に保存するのは SARIF です。`upload-database: false` により DB の自動アップロードを無効にし、追加の DB/debug/SARIF artifact 保存は行いません。DB 非公開の方針を維持するため、デバッグ artifact を生成する debug モードでの再実行は別途検討してください。

### ローカル調査と保守

アラートの commit と OS を揃え、CLI / bundle / query pack の版を実行ログと照合して新しいローカル DB を作成します。DB は収録時点のソースであり、コード変更後は再作成が必要です。DB・SARIF はリポジトリー外の未使用パスへ保存し、既存 DB を上書きしないでください。対応する公式 Rust パックを取得した後、次の形式で共有除外設定を使えます。山括弧のプレースホルダーは実際のパス・互換版に置き換えます。

```sh
codeql database create '<new-db-path>' --language=rust --build-mode=none --source-root=. --codescanning-config=.github/codeql/codeql-config.yml --threads=2 --ram=4096
codeql database analyze '<new-db-path>' 'codeql/rust-queries@<compatible-version>:codeql-suites/rust-code-scanning.qls' --format=sarif-latest --output='<new-sarif-path>' --threads=2 --ram=4096
```

VS Code の CodeQL 拡張で新規 DB を開き、該当クエリー・ソース・経路を調べます。診断クエリーは alerts が0件でも raw results に結果がある場合があります。GitHub DB の自動取得に依存しません。ローカル解析成功は GitHub の権限・アップロード・PR 注釈を検証したことにはなりません。

権限は `contents: read`、CodeQL job の `actions: read` と `security-events: write` に限定し、既定の `GITHUB_TOKEN` を使用します。checkout の認証情報保存は無効です。新しい secrets、PAT、`id-token: write`、`contents: write` は不要です。

action は full SHA に固定し、`tools: linked` で対応する bundle を利用します。既存 Dependabot の更新候補を採用する前に、selected-actions の exact repository/ref 許可、linked bundle・クエリーの互換性、両 OS の解析を確認してください。CodeQL は `github/codeql-action/init@<full-sha>` と `github/codeql-action/analyze@<full-sha>` を個別に許可し、リポジトリー直下の参照だけで許可されると仮定しないでください。必要な許可設定変更は別途承認し、根拠なく許可範囲を広げないでください。週次再実行だけでは固定した解析ツール・クエリーは更新されません。

CodeQL はテスト・Clippy・`cargo audit`・削除仕様レビューの代替ではなく、検出0件は安全性の証明ではありません。アラート修正・dismiss、独自クエリー、DB 公開、Code scanning による必須チェック追加は別の変更として判断します。

## 手動 fixture

`Prepare-Manual-Test.ps1` と `prepare-manual-test.sh` は `manual-test-workspace` に fixture を作成します。OS に合ったスクリプトをリポジトリーのルートから実行し、その後 fixture ディレクトリーに移動して `--manifest-path` 付きで CLI を実行します。

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

プレビューでは `keep.txt`、`release-notes.md`、`keep` ディレクトリーとその中身が保持され、それ以外が削除対象になります。実際の削除を試す場合は `--dry-run` を外し、全件を確認してからプロンプトに `n` と入力して中止できます。削除を実行した後はリポジトリーのルートに戻り、fixture スクリプトを再実行してください。

スクリプトは既知の fixture ファイルを作成または上書きします。`keep-link` は `keep.txt` を指す既知のシンボリックリンクである場合に限り置き換え、無関係なファイルやリンクは削除しません。通常ファイルまたは別のリンクが `keep-link` と競合する場合はエラーになります。別の場所に作成するには、PowerShell の `-Path` または Bash の省略可能な出力先引数を使います。Windows でシンボリックリンクを作成するには Developer Mode または管理者権限が必要な場合があります。任意の場所の fixture から Cargo を実行する場合は、リポジトリーの `Cargo.toml` を絶対パスで `--manifest-path` に指定します。

## 公開ライブラリー API

[公開されている最新バージョンの API ドキュメント](https://docs.rs/remove-except/latest/remove_except/)は docs.rs で閲覧できます。開発版の API は公開版と異なる場合があります。ローカルで API ドキュメントを生成する方法は「実行と検証」を参照してください。crate の `build_plan(root, patterns)` 関数は、処理対象ディレクトリーと保持パターンから `RemovalPlan` を作成します。

パターンがない、または空である場合、glob が無効な場合、処理対象外の絶対パターンや親ディレクトリーへの移動がある場合、処理対象がファイルシステム最上位のディレクトリーまたはディレクトリー以外の場合、Windows の drive-relative path が指定された場合、処理対象を解決できない場合はエラーになります。入力検証エラーは `anyhow::Error::downcast_ref::<ValidationError>()` で識別できます。走査中のファイルシステムエラーは `ValidationError` ではありません。`.` と `./` は処理対象ディレクトリーを表し、その配下全体を保持します。

`RemovalPlan` と `PlannedItem` は getter 経由で読み取ります。getter は参照を返し、計画の内容は変更できません。公開フィールドを使っていたコードは getter 呼び出しへ移行してください。

```rust,no_run
use remove_except::build_plan;
use std::path::Path;

let patterns = ["Cargo.toml".to_owned(), "src/**".to_owned()];
let plan = build_plan(Path::new("."), &patterns).expect("valid plan");

for item in plan.delete_roots() {
    println!("{}", item.relative_path());
}
```

`RemovalPlan` の getter:

- `root()`: 正規化された処理対象ディレクトリー。
- `direct_match_count()`: パターンに直接一致した項目数。
- `keep_items()`: 保持項目と必要な祖先。相対パス順です。
- `delete_items()`: すべての削除対象。相対パス順です。
- `delete_roots()`: 削除関数に渡す最上位項目。削除対象ディレクトリーの子孫は含みません。

`PlannedItem` は `path()`、`relative_path()`、`item_type()` を公開します。相対パスは `/` 区切りです。`path()` は正規化された処理対象ディレクトリー配下の絶対パスを返し、Windows では標準ライブラリーの `\\?\` 拡張パス形式を含む場合があります。

走査時にシンボリックリンクはたどりません。CLI は計画作成後に `delete_roots` を削除します。削除中にエラーが起きた場合、それ以前に削除された項目は復元されません。

## リリースの準備と公開

0.x 系では、文書化された動作を戻す不具合修正は Patch、後方互換な機能追加・CLI または公開ライブラリー API の互換性を壊す変更・MSRV の引き上げは Minor とします。複数の変更を含む場合は最も大きい更新区分を使います。リリースに含める変更が確定してからバージョンを一度だけ更新し、`Cargo.toml` と `Cargo.lock` のルートパッケージを揃えます。文書のみの変更では通常バージョンを上げません。

GitHub のソース／バイナリー Release と crates.io は別々の配布先です。リリース計画で対象バージョン、含める変更、提供する配布先、各公開の承認者を明記します。一つの配布先への承認を、別の配布先の公開承認とみなしてはいけません。

### 順序付きリリース手順

手順1〜4は共通です。GitHub Release を提供する場合に限り5〜6を、crates.io を提供する場合に限り7を実施します。両方を提供する場合は5〜7の順に進めます。手順8では、選択した配布先だけを検証します。各段階の結果、commit／tag、workflow run、Release／crate URL、必要な未完了事項をリリース準備 PR に記録します。

1. **内容と配布先を確定する。** リリース差分を最後の正式公開版から確認し、0.x の規則で版数を決めます。Prepare Release workflow を使う GitHub Release（GitHub のソースアーカイブと Windows x64／Linux x64 バイナリー）と crates.io を個別に選択します。更新日時・削除結果・安全性・互換性に影響する変更がないか確認します。
2. **配布物に入る文書を先に完成させる。** `README.ja.md` を正本として `README.md` を同期し、リリースノート、タグ導入コマンド、アーカイブ名、crates.io 説明を確認します。**Prepare Release workflow はタグ上の両 README をバイナリーアーカイブへコピーし、Cargo は公開時の `README.md` を crate に含めます。main の公開後READMEを直しても既存アーカイブや公開済みcrate内の文書は更新されません。** そのため、配布物に必要な文書修正はタグ／crate公開より前にマージします。公開前後で意味が変わる文言を避け、版数非依存のインストール説明など、公開後も正確な表現を優先します。
3. **版数変更 PR をマージする。** `Cargo.toml` と `Cargo.lock` のルートパッケージだけを更新し、日英READMEと必要な開発文書を同じリリース変更に含めます。必要な CI とレビューを通してマージし、その後の `main` の Rust CI と CodeQL が成功したことを確認します。crate を提供する場合は、タグを作る前に package inventory と dry-run も確認します。
4. **main の確定 commit にタグを作成する。** version tag が未使用であること、`main` が更新済みでCI成功であること、タグ対象の `Cargo.toml` version と一致することを確認します。明示的な承認後に注釈付き tag を作成・push します。公開済み tag は移動・再利用しません。

   ```sh
   git tag -a vX.Y.Z -m "remove-except vX.Y.Z"
   git push origin vX.Y.Z
   ```

5. **GitHub Release を提供する場合：タグからバイナリーを作る。** 手動起動の **Prepare Release** workflow を正確なタグから実行します。GitHub Release を提供しない場合は、この手順と手順6を飛ばします。

   ```sh
   gh workflow run prepare-release.yml --ref vX.Y.Z
   ```

   workflow の全 job が成功し、draft ができるまで待ちます。失敗・skip・警告を成功扱いせず、原因が解決するまで公開へ進みません。workflow はタグが `main` 上にあることと package version を照合し、Windows x64／Linux x64 のテスト、lockfile と各 binary の `cargo audit`、固定版 `cargo-auditable` によるビルド、checksum、attestation、両READMEとライセンスを含むアーカイブ作成を実施します。Linux binary は Ubuntu 22.04 でビルドし、glibc 2.35 以降が必要です。埋め込み依存情報は脆弱性スキャンではありません。
6. **GitHub Release を提供する場合：draft を手動で公開前検査する。** 次の項目がすべて一致するまで **Publish release** を実行しません。GitHub Release を提供しない場合は、この手順を飛ばします。

   ```sh
   gh release view vX.Y.Z --repo tresuke/remove-except --json isDraft,tagName,targetCommitish,assets,body,url
   ```

   アセットはリポジトリー外の一時ディレクトリーへダウンロードします。次の例では、`vX.Y.Z` を対象versionに置き換えてください。

   ```powershell
   $releaseDir = Join-Path $env:TEMP ("remove-except-vX.Y.Z-" + [guid]::NewGuid())
   New-Item -ItemType Directory -Path $releaseDir
   gh release download vX.Y.Z --repo tresuke/remove-except --dir $releaseDir
   ```

   ```sh
   release_dir="$(mktemp -d)"
   gh release download vX.Y.Z --repo tresuke/remove-except --dir "$release_dir"
   ```

   タグが計画したcommitを指し、最新の `main` の祖先であることも確認します。`git rev-parse` の出力を計画したcommitと照合してください。

   ```sh
   git fetch origin main --tags
   git rev-parse "vX.Y.Z^{commit}"
   git merge-base --is-ancestor "vX.Y.Z^{commit}" origin/main
   ```

   アーカイブのchecksumを照合し、ファイル一覧と実行ファイルのversionを確認します。Windows と Linux の検証はそれぞれの対象OSで行います。

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

   Windows では計算した SHA-256 と checksum ファイルの値を大文字・小文字を区別せず照合します。一致しない場合は公開へ進みません。検査後に一時ディレクトリーを削除します。

   - draft のタグ、source commit、package version が計画と一致する。タグ commit が `main` の祖先である。
   - release notes が最後の正式公開版からの利用者向け差分を説明し、未公開版を公開済みと誤記していない。自動生成文面をそのまま信用しない。
   - Windows ZIP／Linux tar.gz と各 `.sha256` の4アセットが存在し、ダウンロードしたファイルのSHA-256がchecksumと一致する。
   - 両アーカイブに正しいversionの実行ファイル、両README、両licenseが含まれる。Windows／Linux binary の `--version` を確認する。
   - 両アーカイブの来歴証明を検証する。例：`gh attestation verify <archive-path> --repo tresuke/remove-except --signer-workflow tresuke/remove-except/.github/workflows/prepare-release.yml`
   - 実行結果と公開判断を記録し、GitHub Release の正式公開を明示承認する。tag push やworkflow成功だけでは公開しない。
7. **crates.io は独立した手動手順で公開する。** リリース計画で対象に含め、公開の明示承認がある場合のみ行います。package READMEが公開後も正確であること、同versionが未登録であること、次のpackage確認に成功することを確認します。maintainer のローカルCargo認証情報だけを使い、リポジトリーやActionsにtokenを保存しません。dry-runと実公開は別操作です。実公開後はregistry登録と隔離installを確認します。詳しくは「crate 配布の検証」を参照してください。
8. **公開後の照合を完了する。** GitHub Release を提供した場合は、その公開状態、4アセット、checksum、attestation を確認します。crates.io を提供した場合は、登録versionと導入結果を確認します。`main` のREADMEが現在利用できる配布先を案内していることを確認し、必要なら通常の文書PRで同期します。最終結果をリリース準備PRに記録します。

### 公開後の README と配布物

公開後チェックは、main の利用者向け案内の確認・同期です。公開済みバイナリー内のREADMEやcrateに同梱されたREADMEを変更する手順ではありません。これらの文書に誤りがあれば、公開物を同じversionのまま差し替えたりtagを移動したりせず、影響を記録し、次の適切なversionで修正します。文書のみの修正では通常、version更新や再リリースは不要です。

- [ ] `README.ja.md` と `README.md` のReleaseリンク、asset名、checksum、source tag、crates.io導入案内が実際の公開状態と一致する。
- [ ] 公開された配布先だけを利用可能として記載し、未公開のものは利用可能と誤認させない。
- [ ] 必要なREADME同期は通常のPRで行い、merge後のmainを確認する。すでに一致している場合は変更せず、確認結果を記録する。
- [ ] 配布先ごとの検証結果、手動操作、承認、関連URLをリリース準備PRに記録してから、リリース後作業を完了扱いにする。

### crate 配布の検証

crateを提供する場合は、公開に使うクリーンなcommitで次を実行し、READMEを含む収録ファイルを確認します。

```sh
cargo audit --file Cargo.lock
cargo package --list --locked
cargo package --locked
cargo publish --dry-run --locked
```

Cargo の `include` 許可リストには配布するソース、利用者向け文書、ライセンス、必要な build target のみを含め、ローカル指示、テスト、fixture、開発者専用ユーティリティは除外します。dry-run成功は認証や公開成功の証明ではありません。crate versionが未登録であることを確認します。`cargo owner --list remove-except` は maintainer access の確認に使えますが、公開権限の確認にはなりません。認証または権限エラーが出た場合は停止し、maintainer がローカル環境で認証を修復します。

crates.io の **Account Settings → API Tokens** から、既存の `remove-except` crate の新version公開に必要な最小権限のtokenを作ります。可能ならcrateに限定し、短い有効期間を指定します。maintainer が管理するローカル環境で Cargo のcredential providerを確認してください。Cargo のpromptを使う場合は `cargo login` を実行し、tokenはpromptにだけ入力します。`cargo:token` providerは既定で `$CARGO_HOME/credentials.toml` にtokenを平文保存するため、利用可能な環境ではOSのcredential storeなど、より安全なproviderを使います。tokenをコマンド引数、環境変数、チャット、ソース、ログ、リポジトリー、Actionsへ保存・転記してはいけません。token値は記録しません。

公開が個別に承認された後、maintainer のローカル環境でのみ実行します。

```sh
cargo publish --locked
```

公開後は crates.io のversion page/APIを確認し、隔離したrootへの `cargo install remove-except --version '=X.Y.Z' --locked --root <temporary-directory>` と導入binaryの `--version` で配布物を検証します。検証用ファイルは確認後に削除します。公開済みversionは上書きできません。crateのREADME、GitHub binary内README、mainのREADMEは別々の公開物であることを忘れないでください。

## 性能ベンチマーク

`plan_generation` ベンチマークは計測前に一時 fixture を作成します。fixture 作成と初回の warm-up は計測時間に含めません。ケースには、単一 glob・複数 glob・リテラルディレクトリー・全件保持を比較する広い2,080項目のツリー、疎な一致と全件保持を比較する8,208項目のツリー、深さのある48項目のツリーが含まれます。各ケースは中央値、サンプル範囲、走査項目数、パターン数、概算 items/秒を出力します。保持率や glob 数を変える場合は、関連するケースを重点的に確認します。

```sh
cargo bench --bench plan_generation
cargo bench --bench plan_generation -- --samples 25 --iterations 10
```

既定の9サンプル×3反復は短時間の確認用です。比較では同じ OS、マシン、引数で25サンプル×10反復を使います。Windows ネイティブと Ubuntu の両方で計測し、Ubuntu では Linux ホーム配下の同期済みコピーを使って Ubuntu のリリースを記録します。対象ケースと計測条件も記録してください。中央値だけでなくサンプル範囲も確認し、範囲が重なり差が重要な場合は50サンプル×20反復で再計測します。OS 間や異なるケース間で絶対時間や改善率を合算しません。範囲が重なる小さな差を改善と断定しないでください。

8,208項目を超える実ワークロードでは、まず両 OS で広い50,000項目のケースを追加して計測します。100,000項目は必要性が確認できてから検討し、fixture 作成時間は引き続き計測から除外します。

CLI 全体を計測する場合は release バイナリーと `hyperfine` を使います。次の dry-run 例は手動 fixture を読み取るだけで、削除しません。

```powershell
cargo build --release
hyperfine --warmup 3 --runs 10 'target/release/remove-except.exe --dry-run --summary both --root manual-test-workspace keep.txt keep'
```

CLI の計測にはプロセス起動、計画作成、描画が含まれます。`build_plan` 単体の計測と混同せず、比較間で fixture を同一にしてください。
