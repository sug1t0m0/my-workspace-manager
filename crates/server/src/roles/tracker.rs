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

/// open なプロジェクトの {id, title} の列。取得できなければ空。
pub fn open_projects(bin: &Path) -> Vec<Value> {
    call(bin, &["list-projects-v0"])
        .and_then(|v| v.as_array().cloned())
        .map(|items| items.into_iter().filter(valid_project).collect())
        .unwrap_or_default()
}

fn valid_project(item: &Value) -> bool {
    item["id"].as_str().is_some_and(domain::is_valid_project) && item["title"].is_string()
}

/// プロジェクトに属するリポジトリの ns_repo 一覧。取得できなければ空。
pub fn project_repos(bin: &Path, project: &str) -> Vec<String> {
    call(bin, &["project-repos-v0", "--project", project])
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

/// open な Issue の (id, タイトル) の列。取得できなければ空。
/// 並びはプラグインの返した順 (UI の表示順)。
pub fn open_issues(bin: &Path, repo: &RepoRef) -> Vec<(String, String)> {
    call(bin, &["list-issues-v0", "--repo", &repo.ns_repo()])
        .and_then(|v| v.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item["id"].as_str()?;
                    let title = item["title"].as_str()?;
                    valid_issue_id(id).then(|| (id.to_owned(), title.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
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
