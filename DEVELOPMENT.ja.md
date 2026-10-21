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

必要に応じて release バイナリーをビルドします。

```sh
cargo build --release
```

依存更新やコード変更でより新しいコンパイラーが必要になった場合は、`rust-version`、CI の toolchain matrix、両 README の記載を合わせて見直します。Ubuntu または WSL で検証する場合は、`/mnt/...` ではなく Linux ホームディレクトリー配下のリポジトリーコピーを使い、Cargo が `PATH` に含まれることを確認してください。

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

公開ライブラリー API の docs.rs での公開は将来の予定であり、現時点では公開していません。ローカルで API ドキュメントを生成する方法は「実行と検証」を参照してください。crate の `build_plan(root, patterns)` 関数は、処理対象ディレクトリーと保持パターンから `RemovalPlan` を作成します。

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

## リリースの準備

0.x 系では、文書化された動作を戻す不具合修正は Patch、後方互換な機能追加・CLI または公開ライブラリー API の互換性を壊す変更・MSRV の引き上げは Minor とします。複数の変更を含む場合は最も大きい更新区分を使います。リリースに含める変更が確定してからバージョンを一度だけ更新し、`Cargo.toml` と `Cargo.lock` のルートパッケージを揃えます。文書のみの変更では通常バージョンを上げません。

公開済みの `v0.2.4` はソースコードのみのリリースで、変更できません。今後のバージョンでは、`Cargo.toml` と `Cargo.lock` のルートパッケージのバージョンを PR で更新し、必須チェック通過後にマージし、更新後の `main` の CI を確認します。リリース内容の承認後、新しいバージョンタグを作成して push します。

```sh
git tag -a vX.Y.Z -m "remove-except vX.Y.Z"
git push origin vX.Y.Z
```

手動起動の **Prepare Release** workflow はそのバージョンタグから実行します。たとえば GitHub CLI では次のとおりです。

```sh
gh workflow run prepare-release.yml --ref vX.Y.Z
```

タグが `main` 上のコミットを指し、パッケージのバージョンと一致することを確認します。workflow は Windows x64 と Linux x64 のバイナリーを検証し、lockfile と各バイナリーを `cargo audit` で監査します。固定バージョンの `cargo-auditable` でビルドし、バイナリーと両 README、ライセンスをアーカイブにまとめ、SHA-256 checksum と来歴証明を作成して、GitHub Release のドラフトを作成または更新します。埋め込み依存情報は crate 名とバージョンを示すもので、脆弱性スキャンやサプライチェーン攻撃への防御ではありません。Linux バイナリーは Ubuntu 22.04 コンテナーでビルドされ、glibc 2.35 以降が必要です。

タグの push だけでは Release は公開されません。ドラフトの対象コミット、生成されたリリースノート、アーカイブ、checksum、来歴証明を確認してから手動で公開します。既に公開したバージョンの内容を置き換えてはいけません。

crate 配布の準備では、Cargo package の内容を確認し検証します。

```sh
cargo package --list --locked
cargo package --locked
cargo publish --dry-run --locked
```

Cargo の `include` 許可リストには配布するソース、利用者向け文書、ライセンス、必要な build target のみを含め、ローカル指示、テスト、fixture、開発者専用ユーティリティは除外します。dry-run が成功しても crate は公開されません。crates.io への公開は maintainer のローカル認証情報を使う別途承認済みの手動操作です。crates.io token をリポジトリーや Release workflow に保存してはいけません。

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
