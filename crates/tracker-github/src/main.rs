// wsm-tracker-github: 公式 GitHub Tracker プラグイン (gh CLI)
//
// wsm の Tracker プラグイン契約 (docs/wsm.md) の v0 動詞を実装する。
// 成功時は stdout に JSON 1 ドキュメント + exit 0、失敗時は stderr に診断を
// 出して非ゼロ終了 (部分的な JSON は出さない)。非対話で動き、プロンプトは
// 出さない (gh には GH_PROMPT_DISABLED を渡す)。
//
// 認証は gh のログインに委ねる。プロジェクトの owner は gh の認証ユーザーを
// 自己解決し、WSM_TRACKER_GITHUB_OWNER でオーバーライドできる
// (organization の Project を使う場合など)。

use serde_json::{json, Value};
use std::process::{Command, ExitCode};

const USAGE: &str =
    "Usage: wsm-tracker-github <list-repo-groups-v0|repo-group-repos-v0|list-issues-v0|issue-v0|info-v0>";

const PROTOCOL: &[&str] =
    &["list-repo-groups-v0", "repo-group-repos-v0", "list-issues-v0", "issue-v0", "info-v0"];

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
        "list-issues-v0" => list_issues(&flag(rest, "--repo")?),
        "issue-v0" => issue(&flag(rest, "--repo")?, &flag(rest, "--id")?),
        "info-v0" => Ok(info()),
        // 未知の動詞は Usage + 非ゼロ (前方互換: 新しい wsm が新動詞を
        // 呼んだとき、古いプラグインはここで見えて失敗する)
        _ => Err(USAGE.to_owned()),
    }
}

/// open な repo-group の {id, title} の列。GitHub での repo-group の実体は
/// Projects (V2) で、id は Project 番号の文字列表現。
fn list_repo_groups() -> Result<Value, String> {
    let owner = owner()?;
    let out = gh(&["project", "list", "--owner", &owner, "--format", "json"])?;
    let v: Value = serde_json::from_str(&out).map_err(|e| format!("unexpected gh output: {e}"))?;
    let projects = v["projects"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|p| p["closed"].as_bool() != Some(true))
                .filter_map(|p| {
                    let id = p["number"].as_u64()?;
                    Some(json!({ "id": id.to_string(), "title": p["title"].as_str()? }))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Value::Array(projects))
}

/// repo-group (= GitHub Project) に属するリポジトリの ns_repo の列。
/// repositoryOwner はユーザー・organization の両方を解決できる。
fn repo_group_repos(group: &str) -> Result<Value, String> {
    let number: u64 =
        group.parse().map_err(|_| format!("Invalid group: {group} (GitHub Projects の id は番号)"))?;
    const QUERY: &str = "\n      query($owner: String!, $num: Int!) {\n        repositoryOwner(login: $owner) {\n          ... on User {\n            projectV2(number: $num) {\n              repositories(first: 100) {\n                nodes { nameWithOwner }\n              }\n            }\n          }\n          ... on Organization {\n            projectV2(number: $num) {\n              repositories(first: 100) {\n                nodes { nameWithOwner }\n              }\n            }\n          }\n        }\n      }";
    let owner = owner()?;
    let out = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={QUERY}"),
        "-f",
        &format!("owner={owner}"),
        "-F",
        &format!("num={number}"),
        "-q",
        ".data.repositoryOwner.projectV2.repositories.nodes[].nameWithOwner",
    ])?;
    Ok(Value::Array(out.lines().filter(|l| !l.is_empty()).map(Value::from).collect()))
}

/// open な Issue の {id, title} の列 (gh の返却順)。
fn list_issues(repo: &str) -> Result<Value, String> {
    let out = gh(&["issue", "list", "--repo", repo, "--limit", "50", "--json", "number,title"])?;
    let v: Value = serde_json::from_str(&out).map_err(|e| format!("unexpected gh output: {e}"))?;
    let issues = v
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|issue| {
                    let number = issue["number"].as_u64()?;
                    Some(json!({ "id": number.to_string(), "title": issue["title"].as_str()? }))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Value::Array(issues))
}

/// 単一 Issue の {title, state}。state は契約の中立語彙 open / closed。
fn issue(repo: &str, id: &str) -> Result<Value, String> {
    let out = gh(&["issue", "view", id, "--repo", repo, "--json", "title,state"])?;
    let v: Value = serde_json::from_str(&out).map_err(|e| format!("unexpected gh output: {e}"))?;
    let title = v["title"].as_str().ok_or("unexpected gh output: missing title")?;
    let state = match v["state"].as_str() {
        Some("CLOSED") => "closed",
        Some(_) => "open",
        None => return Err("unexpected gh output: missing state".to_owned()),
    };
    Ok(json!({ "title": title, "state": state }))
}

/// 自己診断 (info-v0)。トラッカーが使えない状態は ready:false のデータとして
/// 返すため、info 自体は常に成功する (exit 0)。
/// gh api user -i の 1 呼び出しで、ログイン状態とトークンのスコープ
/// (X-Oauth-Scopes ヘッダ) を確認する。プロジェクト照会には read:project が
/// 必要で、欠けていると照会が黙って空になるため、ここで修復手順ごと表面化させる。
fn info() -> Value {
    let (ready, diagnosis) = match gh(&["api", "user", "-i"]) {
        Err(message) => (false, Some(message)),
        Ok(response) => match scopes_header(&response) {
            Some(scopes) if !scopes.split(',').any(|s| s.trim() == "read:project") => (
                false,
                Some(
                    "gh token is missing scope read:project \
                     (run: gh auth refresh -h github.com -s read:project)"
                        .to_owned(),
                ),
            ),
            // ヘッダなし (fine-grained token 等) はスコープを判定できないため ready 扱い
            _ => (true, None),
        },
    };
    json!({ "name": "github", "protocol": PROTOCOL, "ready": ready, "diagnosis": diagnosis })
}

/// `gh api -i` の応答ヘッダから X-Oauth-Scopes の値を取り出す。
fn scopes_header(response: &str) -> Option<&str> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("x-oauth-scopes").then(|| value.trim())
    })
}

/// プロジェクトの owner。環境変数が優先、なければ gh の認証ユーザー。
fn owner() -> Result<String, String> {
    if let Ok(owner) = std::env::var("WSM_TRACKER_GITHUB_OWNER") {
        if !owner.is_empty() {
            return Ok(owner);
        }
    }
    let login = gh(&["api", "user", "-q", ".login"])?.trim().to_owned();
    if login.is_empty() {
        return Err("failed to resolve GitHub user".to_owned());
    }
    Ok(login)
}

/// gh を非対話で起動し、成功時の stdout を返す。失敗時は gh の stderr を
/// そのまま診断として返す (wsm は解釈しないが、手動実行時に原因が見える)。
fn gh(args: &[&str]) -> Result<String, String> {
    let output = Command::new("gh")
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .map_err(|e| format!("failed to run gh: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh {} failed: {}", args.first().unwrap_or(&""), stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// フラグの値 (同名フラグの重複は後勝ち。空文字は未指定と同じ)。
fn flag(args: &[String], name: &str) -> Result<String, String> {
    args.iter()
        .rposition(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or_else(|| format!("{name} required"))
}
