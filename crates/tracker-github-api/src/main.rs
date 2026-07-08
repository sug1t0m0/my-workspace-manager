// wsm-tracker-github-api: 公式 GitHub Tracker プラグイン (GraphQL API 直叩き)
//
// wsm の Tracker プラグイン契約 (docs/wsm.md) を実装する。gh 版
// (wsm-tracker-github) との違い:
// - Issue の親子関係 (sub-issues) に対応し、list-issues-v1 を実装する
// - GitHub とは GraphQL API で直接会話し、gh には認証だけを借りる
//   (`gh auth token`。トークンの保管・発行・スコープ変更のライフサイクルは
//   gh に委ね、PAT の手動管理を持ち込まない)
//
// 接続先は WSM_TRACKER_GITHUB_API_URL で差し替えられる (既定
// https://api.github.com。契約テストがローカルの HTTP サーバを指すのに使う)。

use serde_json::{json, Value};
use std::process::{Command, ExitCode};
use std::time::Duration;

const USAGE: &str = "Usage: wsm-tracker-github-api <list-repo-groups-v0|repo-group-repos-v0|list-issues-v0|list-issues-v1|issue-v0|info-v0>";

const PROTOCOL: &[&str] = &[
    "list-repo-groups-v0",
    "repo-group-repos-v0",
    "list-issues-v0",
    "list-issues-v1",
    "issue-v0",
    "info-v0",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<Value, String> {
    let (verb, rest) = args.split_first().ok_or(USAGE)?;
    match verb.as_str() {
        "list-repo-groups-v0" => list_repo_groups(),
        "repo-group-repos-v0" => repo_group_repos(&flag(rest, "--group")?),
        "list-issues-v0" => list_issues_flat(&flag(rest, "--repo")?),
        "list-issues-v1" => list_issues(&flag(rest, "--repo")?, optional_flag(rest, "--parent")),
        "issue-v0" => issue(&flag(rest, "--repo")?, &flag(rest, "--id")?),
        "info-v0" => Ok(info()),
        _ => Err(USAGE.to_owned()),
    }
}

// --- 動詞 ---

/// open な repo-group の {id, title} の列 (GitHub での実体は Projects V2)。
fn list_repo_groups() -> Result<Value, String> {
    const QUERY: &str = "query($owner: String!) {
      repositoryOwner(login: $owner) {
        ... on User { projectsV2(first: 50) { nodes { number title closed } } }
        ... on Organization { projectsV2(first: 50) { nodes { number title closed } } }
      }
    }";
    let owner = owner()?;
    let data = graphql(QUERY, json!({ "owner": owner }))?;
    let groups = data["repositoryOwner"]["projectsV2"]["nodes"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .filter(|p| p["closed"].as_bool() != Some(true))
                .filter_map(|p| {
                    let number = p["number"].as_u64()?;
                    Some(json!({ "id": number.to_string(), "title": p["title"].as_str()? }))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Value::Array(groups))
}

/// repo-group (= GitHub Project) に属するリポジトリの ns_repo の列。
fn repo_group_repos(group: &str) -> Result<Value, String> {
    const QUERY: &str = "query($owner: String!, $num: Int!) {
      repositoryOwner(login: $owner) {
        ... on User { projectV2(number: $num) { repositories(first: 100) { nodes { nameWithOwner } } } }
        ... on Organization { projectV2(number: $num) { repositories(first: 100) { nodes { nameWithOwner } } } }
      }
    }";
    let number = numeric(group, "group")?;
    let owner = owner()?;
    let data = graphql(QUERY, json!({ "owner": owner, "num": number }))?;
    let repos = data["repositoryOwner"]["projectV2"]["repositories"]["nodes"]
        .as_array()
        .map(|nodes| {
            nodes.iter().filter_map(|n| n["nameWithOwner"].as_str()).map(Value::from).collect()
        })
        .unwrap_or_default();
    Ok(Value::Array(repos))
}

/// open な Issue の平坦な一覧 (v0。階層を知らない古い wsm 向け)。
fn list_issues_flat(repo: &str) -> Result<Value, String> {
    let nodes = open_issue_nodes(repo)?;
    Ok(Value::Array(nodes.iter().filter_map(|n| issue_item(n, false)).collect()))
}

/// open な Issue (v1)。--parent 省略時は親を持たないもの、指定時はその子。
fn list_issues(repo: &str, parent: Option<String>) -> Result<Value, String> {
    match parent {
        None => {
            let nodes = open_issue_nodes(repo)?;
            Ok(Value::Array(
                nodes
                    .iter()
                    .filter(|n| n["parent"].is_null())
                    .filter_map(|n| issue_item(n, true))
                    .collect(),
            ))
        }
        Some(parent) => {
            const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
              repository(owner: $owner, name: $name) {
                issue(number: $number) {
                  subIssues(first: 50) { nodes { number title state subIssues(first: 50) { nodes { state } } } }
                }
              }
            }";
            let (owner, name) = split_repo(repo)?;
            let number = numeric(&parent, "parent")?;
            let data =
                graphql(QUERY, json!({ "owner": owner, "name": name, "number": number }))?;
            let children = data["repository"]["issue"]["subIssues"]["nodes"]
                .as_array()
                .map(|nodes| {
                    nodes
                        .iter()
                        .filter(|n| n["state"].as_str() == Some("OPEN"))
                        .filter_map(|n| issue_item(n, true))
                        .collect()
                })
                .unwrap_or_default();
            Ok(Value::Array(children))
        }
    }
}

/// 単一 Issue の {title, state}。state は契約の中立語彙 open / closed。
fn issue(repo: &str, id: &str) -> Result<Value, String> {
    const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) { issue(number: $number) { title state } }
    }";
    let (owner, name) = split_repo(repo)?;
    let number = numeric(id, "id")?;
    let data = graphql(QUERY, json!({ "owner": owner, "name": name, "number": number }))?;
    let issue = &data["repository"]["issue"];
    let title = issue["title"].as_str().ok_or("unexpected API response: missing title")?;
    let state = match issue["state"].as_str() {
        Some("CLOSED") => "closed",
        Some(_) => "open",
        None => return Err("unexpected API response: missing state".to_owned()),
    };
    Ok(json!({ "title": title, "state": state }))
}

/// 自己診断。トラッカーが使えない状態は ready:false のデータとして返す
/// (info 自体は常に exit 0)。/user の応答ヘッダでトークンのスコープを確認する。
fn info() -> Value {
    let (ready, diagnosis) = match probe() {
        Ok(()) => (true, None),
        Err(message) => (false, Some(message)),
    };
    json!({ "name": "github-api", "protocol": PROTOCOL, "ready": ready, "diagnosis": diagnosis })
}

fn probe() -> Result<(), String> {
    let token = token()?;
    let response = ureq::get(&format!("{}/user", api_url()))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("bearer {token}"))
        .set("User-Agent", "wsm-tracker-github-api")
        .call()
        .map_err(|e| format!("GitHub API failed: {e}"))?;
    // ヘッダなし (fine-grained token 等) はスコープを判定できないため ready 扱い
    if let Some(scopes) = response.header("x-oauth-scopes") {
        if !scopes.split(',').any(|s| s.trim() == "read:project") {
            return Err("gh token is missing scope read:project \
                 (run: gh auth refresh -h github.com -s read:project)"
                .to_owned());
        }
    }
    Ok(())
}

// --- GitHub との会話 ---

/// トップレベル判定に必要な parent と、子 Issue の state 付きの
/// open Issue ノード。
fn open_issue_nodes(repo: &str) -> Result<Vec<Value>, String> {
    const QUERY: &str = "query($owner: String!, $name: String!) {
      repository(owner: $owner, name: $name) {
        issues(states: OPEN, first: 50) {
          nodes { number title parent { number } subIssues(first: 50) { nodes { state } } }
        }
      }
    }";
    let (owner, name) = split_repo(repo)?;
    let data = graphql(QUERY, json!({ "owner": owner, "name": name }))?;
    Ok(data["repository"]["issues"]["nodes"].as_array().cloned().unwrap_or_default())
}

fn issue_item(node: &Value, hierarchical: bool) -> Option<Value> {
    let number = node["number"].as_u64()?;
    let title = node["title"].as_str()?;
    // has_children は「open な子がいるか」。closed の子しかない Issue に
    // ▸ を付けても、掘った先が空になるだけのため (契約の一覧も open のみ)
    let has_children = hierarchical
        && node["subIssues"]["nodes"].as_array().is_some_and(|children| {
            children.iter().any(|child| child["state"].as_str() == Some("OPEN"))
        });
    Some(json!({ "id": number.to_string(), "title": title, "has_children": has_children }))
}

fn graphql(query: &str, variables: Value) -> Result<Value, String> {
    let token = token()?;
    let response = ureq::post(&format!("{}/graphql", api_url()))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("bearer {token}"))
        .set("User-Agent", "wsm-tracker-github-api")
        .send_json(json!({ "query": query, "variables": variables }))
        .map_err(|e| format!("GitHub API failed: {e}"))?;
    let body: Value = response.into_json().map_err(|e| format!("unexpected API response: {e}"))?;
    if let Some(errors) = body["errors"].as_array().filter(|errors| !errors.is_empty()) {
        let messages: Vec<&str> =
            errors.iter().filter_map(|e| e["message"].as_str()).collect();
        return Err(format!("GitHub API errors: {}", messages.join(" / ")));
    }
    Ok(body["data"].clone())
}

/// 認証は gh から借りる。トークンの保管・更新は gh のログインに委ねる。
fn token() -> Result<String, String> {
    let output = Command::new("gh")
        .args(["auth", "token", "-h", "github.com"])
        .output()
        .map_err(|e| format!("failed to run gh: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh auth token failed: {} (run: gh auth login)", stderr.trim()));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if token.is_empty() {
        return Err("gh auth token returned an empty token (run: gh auth login)".to_owned());
    }
    Ok(token)
}

fn api_url() -> String {
    std::env::var("WSM_TRACKER_GITHUB_API_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| "https://api.github.com".to_owned())
}

/// repo-group の owner。環境変数が優先、なければ認証ユーザー (viewer)。
fn owner() -> Result<String, String> {
    if let Ok(owner) = std::env::var("WSM_TRACKER_GITHUB_OWNER") {
        if !owner.is_empty() {
            return Ok(owner);
        }
    }
    let data = graphql("query { viewer { login } }", json!({}))?;
    data["viewer"]["login"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "failed to resolve GitHub user".to_owned())
}

// --- 引数の解釈 ---

fn numeric(value: &str, name: &str) -> Result<u64, String> {
    value.parse().map_err(|_| format!("Invalid {name}: {value} (GitHub の id は番号)"))
}

fn split_repo(repo: &str) -> Result<(&str, &str), String> {
    repo.split_once('/').ok_or_else(|| format!("Invalid repo: {repo}"))
}

/// フラグの値 (同名フラグの重複は後勝ち。空文字は未指定と同じ)。
fn flag(args: &[String], name: &str) -> Result<String, String> {
    optional_flag(args, name).ok_or_else(|| format!("{name} required"))
}

fn optional_flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .rposition(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .filter(|v| !v.is_empty())
        .cloned()
}
