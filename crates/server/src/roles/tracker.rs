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

/// プラグインが返す open な Issue の 1 要素。repo は Issue の所属リポジトリ
/// (省略時は照会したリポジトリ。sub-issues はリポジトリ横断で張れるため、
/// 子がよそのリポジトリのこともある)。
pub struct IssueItem {
    pub id: String,
    pub title: String,
    pub has_children: bool,
    pub repo: Option<String>,
}

/// open な Issue の 1 ページ。(一覧, 続きの cursor) を返す。取得できなければ空。
/// 並びと 1 ページの件数はプラグインの責務 (UI はそのまま表示する)。
///
/// ページング対応の list-issues-v2 → 階層対応の list-issues-v1 → 平坦な
/// list-issues-v0 の順に試す (未知の動詞 → 非ゼロ、で非対応を検知)。
/// 下位動詞で表現できない照会 (--parent は v0、--cursor は v1 以下) の
/// フォールバックは空。cursor はプラグインの出力から引数へ還流するため、
/// 形の不正なものは「続きなし」に落とす。
pub fn open_issues(
    bin: &Path,
    repo: &RepoRef,
    parent: Option<&str>,
    cursor: Option<&str>,
) -> (Vec<IssueItem>, Option<String>) {
    let ns_repo = repo.ns_repo();
    let mut args = vec!["list-issues-v2", "--repo", ns_repo.as_str()];
    if let Some(parent) = parent {
        args.extend(["--parent", parent]);
    }
    if let Some(cursor) = cursor {
        args.extend(["--cursor", cursor]);
    }
    if let Some(page) = call(bin, &args) {
        if let Some(items) = page["issues"].as_array() {
            return (issue_items(items, true), next_cursor_of(&page));
        }
    }
    if cursor.is_some() {
        return (Vec::new(), None);
    }

    let mut args = vec!["list-issues-v1", "--repo", ns_repo.as_str()];
    if let Some(parent) = parent {
        args.extend(["--parent", parent]);
    }
    if let Some(items) = call(bin, &args).and_then(|v| v.as_array().cloned()) {
        return (issue_items(&items, true), None);
    }
    if parent.is_some() {
        return (Vec::new(), None);
    }

    let items = call(bin, &["list-issues-v0", "--repo", &ns_repo])
        .and_then(|v| v.as_array().cloned())
        .map(|items| issue_items(&items, false))
        .unwrap_or_default();
    (items, None)
}

/// repo-group に属する open な Issue の 1 ページ (リポジトリ横断)。
/// 各要素の repo は必須で、欠落・形の不正な要素は捨てる。
/// 非対応プラグイン (未知の動詞 → 非ゼロ) は空 (UI は従来のリポジトリ起点
/// フローに落ちる)。
pub fn group_issues(
    bin: &Path,
    group: &str,
    cursor: Option<&str>,
) -> (Vec<IssueItem>, Option<String>) {
    let mut args = vec!["list-group-issues-v0", "--group", group];
    if let Some(cursor) = cursor {
        args.extend(["--cursor", cursor]);
    }
    let Some(page) = call(bin, &args) else { return (Vec::new(), None) };
    let Some(items) = page["issues"].as_array() else { return (Vec::new(), None) };
    let issues = issue_items(items, true).into_iter().filter(|i| i.repo.is_some()).collect();
    (issues, next_cursor_of(&page))
}

fn next_cursor_of(page: &Value) -> Option<String> {
    page["next_cursor"].as_str().filter(|c| domain::is_valid_cursor(c)).map(str::to_owned)
}

fn issue_items(items: &[Value], hierarchical: bool) -> Vec<IssueItem> {
    items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?;
            if !valid_issue_id(id) {
                return None;
            }
            let title = item["title"].as_str()?;
            // repo は任意だが、形の不正な値は要素ごと捨てる (open の対象に
            // 流れ込むため、黙って「同じリポジトリ」に読み替えない)
            let repo = match &item["repo"] {
                Value::Null => None,
                Value::String(repo) => Some(RepoRef::parse(repo)?.ns_repo()),
                _ => return None,
            };
            Some(IssueItem {
                id: id.to_owned(),
                title: title.to_owned(),
                has_children: hierarchical && item["has_children"].as_bool().unwrap_or(false),
                repo,
            })
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
