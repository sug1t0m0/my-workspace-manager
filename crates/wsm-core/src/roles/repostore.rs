//! RepoStore ロールの ghq 実装。ローカルクローンの列挙。読み取り専用。
//! github.com 固定は現行実装の制約 (docs/wsm.md の拡張点を参照)。
//! 出力は RepoRef にパースし、形の不正な行は捨てる。

use crate::domain::RepoRef;
use crate::exec;

/// ローカルにあるリポジトリの一覧 (`ghq list`、ns_repo の文字列順)。
pub fn list() -> Vec<RepoRef> {
    exec::stdout_if_ok("ghq", &["list"])
        .map(|out| {
            let mut repos: Vec<RepoRef> = out
                .lines()
                .filter_map(|line| line.strip_prefix("github.com/"))
                .filter_map(RepoRef::parse)
                .collect();
            repos.sort_by_key(RepoRef::ns_repo);
            repos
        })
        .unwrap_or_default()
}

/// ローカルにあるリポジトリの一覧 (`ghq list -p`、ghq の出力順)。
pub fn list_in_ghq_order() -> Vec<RepoRef> {
    exec::stdout_if_ok("ghq", &["list", "-p"])
        .map(|out| {
            out.lines()
                .filter_map(|line| line.split_once("/github.com/").map(|(_, ns_repo)| ns_repo))
                .filter_map(RepoRef::parse)
                .collect()
        })
        .unwrap_or_default()
}
