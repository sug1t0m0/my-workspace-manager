//! Tracker ロール: プロジェクト・Issue 照会のプラグインディスパッチ。読み取り専用。
//!
//! プラグインは v0 動詞を受けて JSON 1 ドキュメントを stdout に返す実行
//! ファイル (docs/wsm.md「Tracker プラグイン契約」)。どのプラグインを使うかは
//! 設定 ([[tracker]] / default_tracker / [[repo]].tracker) が決め、解決は
//! 呼び出し側 (usecases) が行う。
//!
//! プラグインの出力は信頼しない入力として扱う: id はブランチ名・セッション名・
//! Docker ラベルに流れ込むため形を検証し、違反する要素は捨てる。

use crate::infra::exec;
use serde_json::Value;
use std::path::Path;
use wsm_shared::domains::{self as domain, RepoRef};

fn call(bin: &Path, args: &[&str]) -> Option<Value> {
    exec::stdout_if_ok(bin, args).and_then(|out| serde_json::from_str(&out).ok())
}

/// open な repo-group (リポジトリのグルーピング) の {id, title} の列。
/// 取得できなければ空。
pub fn repo_groups(bin: &Path) -> Vec<Value> {
    call(bin, &["list-repo-groups-v0"])
        .and_then(|v| v.as_array().cloned())
        .map(|items| items.into_iter().filter(valid_group).collect())
        .unwrap_or_default()
}

fn valid_group(item: &Value) -> bool {
    item["id"].as_str().is_some_and(domain::is_valid_group) && item["title"].is_string()
}

/// repo-group に属するリポジトリの ns_repo 一覧。取得できなければ空。
pub fn repo_group_repos(bin: &Path, group: &str) -> Vec<String> {
    call(bin, &["repo-group-repos-v0", "--group", group])
        .and_then(|v| v.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|s| RepoRef::parse(s).is_some())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// open な Issue。1 要素は (id, タイトル, 子 Issue の有無)。取得できなければ空。
/// 並びはプラグインの返した順 (UI の表示順)。
///
/// まず階層対応の list-issues-v1 を試し、非対応のプラグイン (未知の動詞 →
/// 非ゼロ) には list-issues-v0 に落ちて has_children を false で補う。
/// v0 は階層を表現できないため、`--parent` 指定時のフォールバックは空。
pub fn open_issues(bin: &Path, repo: &RepoRef, parent: Option<&str>) -> Vec<(String, String, bool)> {
    let ns_repo = repo.ns_repo();
    let mut args = vec!["list-issues-v1", "--repo", ns_repo.as_str()];
    if let Some(parent) = parent {
        args.extend(["--parent", parent]);
    }
    if let Some(items) = call(bin, &args).and_then(|v| v.as_array().cloned()) {
        return issue_items(&items, true);
    }
    if parent.is_some() {
        return Vec::new();
    }
    call(bin, &["list-issues-v0", "--repo", &ns_repo])
        .and_then(|v| v.as_array().cloned())
        .map(|items| issue_items(&items, false))
        .unwrap_or_default()
}

fn issue_items(items: &[Value], hierarchical: bool) -> Vec<(String, String, bool)> {
    items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?;
            let title = item["title"].as_str()?;
            let has_children = hierarchical && item["has_children"].as_bool().unwrap_or(false);
            valid_issue_id(id).then(|| (id.to_owned(), title.to_owned(), has_children))
        })
        .collect()
}

/// 単一 Issue の (タイトル, closed か)。取得できなければ None。
pub fn issue(bin: &Path, repo: &RepoRef, id: &str) -> Option<(String, bool)> {
    let v = call(bin, &["issue-v0", "--repo", &repo.ns_repo(), "--id", id])?;
    let title = v["title"].as_str()?.to_owned();
    let closed = match v["state"].as_str()? {
        "closed" => true,
        "open" => false,
        _ => return None,
    };
    Some((title, closed))
}

/// プラグインが返す Issue id の検証。id の文法に加え、Workspace id 空間の
/// 番兵値 `main` と衝突するものも捨てる。
fn valid_issue_id(id: &str) -> bool {
    id != "main" && domain::is_valid_issue(id)
}

/// プラグインの自己診断 (info-v0)。形を検証したフィールドだけを通す:
/// (ready, diagnosis, protocol)。info-v0 非対応・出力不正は None。
pub fn info(bin: &Path) -> Option<(bool, Option<String>, Option<Vec<String>>)> {
    let v = call(bin, &["info-v0"])?;
    let ready = v["ready"].as_bool()?;
    let diagnosis = v["diagnosis"].as_str().map(str::to_owned);
    let protocol = v["protocol"].as_array().map(|verbs| {
        verbs.iter().filter_map(|verb| verb.as_str()).map(str::to_owned).collect()
    });
    Some((ready, diagnosis, protocol))
}
