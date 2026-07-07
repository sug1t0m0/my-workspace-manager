//! RepoStore ロールの ghq 実装。ローカルクローンの列挙。読み取り専用。
//! github.com 固定は現行実装の制約 (docs/wsm.md の拡張点を参照)。

use crate::exec;

/// ローカルにある ns_repo の一覧 (`ghq list`、ソート済み)。
pub fn list_ns_repos() -> Vec<String> {
    exec::stdout_if_ok("ghq", &["list"])
        .map(|out| {
            let mut repos: Vec<String> = out
                .lines()
                .filter_map(|line| line.strip_prefix("github.com/"))
                .map(str::to_owned)
                .collect();
            repos.sort();
            repos
        })
        .unwrap_or_default()
}

/// ローカルにある ns_repo の一覧 (`ghq list -p`、ghq の出力順)。
pub fn list_ns_repos_in_ghq_order() -> Vec<String> {
    exec::stdout_if_ok("ghq", &["list", "-p"])
        .map(|out| {
            out.lines()
                .filter_map(|line| line.split_once("/github.com/"))
                .map(|(_, ns_repo)| ns_repo.to_owned())
                .collect()
        })
        .unwrap_or_default()
}
