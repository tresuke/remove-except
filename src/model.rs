use std::path::{Path, PathBuf};

/// 走査対象内で見つかった項目の種類です。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    /// 通常のファイルです。
    File,
    /// ディレクトリです。
    Directory,
    /// シンボリックリンク自体です。リンク先は走査しません。
    Symlink,
}

/// 保持または削除の計画に含まれる、走査時点のファイルシステム項目です。
///
/// 計画作成後にファイルシステムが変更された場合、この値は自動更新されません。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    /// 正規化されたルートを基準に解決した項目の絶対パスです。
    /// Windowsではcanonicalize表現により `\\?\` の拡張パスprefixが含まれる場合があります。
    pub(crate) path: PathBuf,
    /// ルートからの相対パスです。区切り文字には `/` を使います。
    pub(crate) relative_path: String,
    /// 項目の種類です。
    pub(crate) item_type: ItemType,
}

impl PlannedItem {
    /// 正規化されたルートを基準に解決した項目の絶対パスを返します。
    ///
    /// Windowsではcanonicalize表現により `\\?\` の拡張パスprefixが含まれる場合があります。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// ルートからの相対パスを `/` 区切りで返します。
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    /// 項目の種類を返します。
    pub fn item_type(&self) -> ItemType {
        self.item_type
    }
}

/// ルート配下の走査時点の情報と保持・削除対象をまとめた計画です。
///
/// この値の作成やgetterによる参照はファイルシステムを変更しません。
/// 計画作成後にファイルシステムが変更されても、内容は自動更新・再検証されません。
#[derive(Debug)]
pub struct RemovalPlan {
    /// 正規化された走査ルートです。このパス自体は計画に含まれません。
    pub(crate) root: PathBuf,
    /// 保持パターンに直接一致した項目数です。祖先のみの項目は含みません。
    pub(crate) direct_match_count: usize,
    /// 保持される項目とその祖先です。相対パス順に並びます。
    pub(crate) keep_items: Vec<PlannedItem>,
    /// 削除対象となる全項目です。相対パス順に並びます。
    pub(crate) delete_items: Vec<PlannedItem>,
    /// 実削除に使う最上位の項目です。削除対象ディレクトリ内の子孫は含みません。
    pub(crate) delete_roots: Vec<PlannedItem>,
}

impl RemovalPlan {
    /// 正規化された走査ルートを返します。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 保持パターンに直接一致した項目数を返します。祖先項目は数えません。
    pub fn direct_match_count(&self) -> usize {
        self.direct_match_count
    }

    /// 保持項目とその祖先を相対パス順で返します。
    pub fn keep_items(&self) -> &[PlannedItem] {
        &self.keep_items
    }

    /// 削除対象の全項目を相対パス順で返します。
    pub fn delete_items(&self) -> &[PlannedItem] {
        &self.delete_items
    }

    /// 実削除に使う最上位の項目を返します。
    ///
    /// 含まれる項目は削除計画上の候補であり、このgetterは削除を実行しません。
    pub fn delete_roots(&self) -> &[PlannedItem] {
        &self.delete_roots
    }
}
