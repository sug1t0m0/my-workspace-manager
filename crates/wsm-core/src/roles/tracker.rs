//! Tracker ロールの GitHub 実装 (gh CLI)。読み取り専用。
//! gh との会話 (引数列) は zsh 版と同一に保つ。契約テストのフェイクが
//! この会話を前提にしているため。

use crate::domain::RepoRef;
use crate::exec;
use serde_json::Value;

const OPEN_PROJECTS_FILTER: &str = ".projects[] | select(.closed == false) | {number, title}";
const ISSUE_LINES_FILTER: &str = r#".[] | "\(.number)\t\(.title)""#;

/// gh の認証ユーザーを解決する (--user 省略時の自己解決)。
pub fn resolve_user() -> Option<String> {
    exec::stdout_if_ok("gh", &["api", "user", "-q", ".login"])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// open な Project の一覧 ({number, title} の列)。取得できなければ空。
pub fn open_projects(user: &str) -> Vec<Value> {
    exec::stdout_if_ok(
        "gh",
        &["project", "list", "--owner", user, "--format", "json", "-q", OPEN_PROJECTS_FILTER],
    )
    .map(|out| out.lines().filter_map(|line| serde_json::from_str(line).ok()).collect())
    .unwrap_or_default()
}

/// open な Issue の (番号, タイトル) の列。取得できなければ空。
pub fn open_issues(repo: &RepoRef) -> Vec<(String, String)> {
    let ns_repo = repo.ns_repo();
    exec::stdout_if_ok(
        "gh",
        &["issue", "list", "--repo", &ns_repo, "--limit", "50", "--json", "number,title", "-q", ISSUE_LINES_FILTER],
    )
    .map(|out| {
        out.lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(number, title)| (number.to_owned(), title.to_owned()))
            .collect()
    })
    .unwrap_or_default()
}

/// Project に属するリポジトリの ns_repo 一覧。取得できなければ空。
/// GraphQL クエリは zsh 版と同一 (会話の互換性のため)。
/// repositoryOwner はユーザー・organization の両方を解決できる。
pub fn project_repos(user: &str, project: &str) -> Vec<String> {
    const QUERY: &str = "\n      query($owner: String!, $num: Int!) {\n        repositoryOwner(login: $owner) {\n          ... on User {\n            projectV2(number: $num) {\n              repositories(first: 100) {\n                nodes { nameWithOwner }\n              }\n            }\n          }\n          ... on Organization {\n            projectV2(number: $num) {\n              repositories(first: 100) {\n                nodes { nameWithOwner }\n              }\n            }\n          }\n        }\n      }";
    exec::stdout_if_ok(
        "gh",
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={QUERY}"),
            "-f",
            &format!("owner={user}"),
            "-F",
            &format!("num={project}"),
            "-q",
            ".data.repositoryOwner.projectV2.repositories.nodes[].nameWithOwner",
        ],
    )
    .map(|out| out.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
    .unwrap_or_default()
}

/// 単一 Issue のタイトルと状態 (list-workspaces 用)。
pub fn issue_title_and_state(repo: &RepoRef, issue: &str) -> Option<(String, String)> {
    let ns_repo = repo.ns_repo();
    exec::stdout_if_ok(
        "gh",
        &["issue", "view", issue, "--repo", &ns_repo, "--json", "title,state", "-q", r#""\(.title)\t\(.state)""#],
    )
    .and_then(|out| {
        out.trim_end_matches('\n')
            .split_once('\t')
            .map(|(title, state)| (title.to_owned(), state.to_owned()))
    })
}

/// 単一 Issue のタイトル (孤児 worktree の解決用)。
pub fn issue_title(repo: &RepoRef, issue: &str) -> Option<String> {
    let ns_repo = repo.ns_repo();
    exec::stdout_if_ok(
        "gh",
        &["issue", "view", issue, "--repo", &ns_repo, "--json", "title", "-q", ".title"],
    )
    .map(|s| s.trim_end_matches('\n').to_owned())
    .filter(|s| !s.is_empty())
}
