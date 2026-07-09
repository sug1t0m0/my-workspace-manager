// wsm-tracker-github-api のプラグイン契約テスト (docs/wsm.md「Tracker プラグイン契約」)。
//
// GitHub API はローカルの使い捨て HTTP サーバに差し替え
// (WSM_TRACKER_GITHUB_API_URL)、gh は PATH 先頭のフェイクに差し替える
// (認証はトークンを返すだけの役)。検証の観点は、契約どおりの JSON を
// 返すことと、API への問い合わせ (リクエスト本文) の形。

use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// 使い捨て API サーバ。応答キューを順に返し、リクエストを記録する。
struct ApiServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl ApiServer {
    /// (追加ヘッダ行, 応答本文) のキューで起動する。
    fn serve(responses: Vec<(&'static str, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        std::thread::spawn(move || {
            for (extra_headers, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept");
                recorded.lock().unwrap().push(read_request(&mut stream));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                    body.len(),
                    extra_headers,
                    body
                );
                stream.write_all(response.as_bytes()).expect("write response");
            }
        });
        Self { url, requests }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// リクエスト行 + 本文を "METHOD PATH\n<body>" の形で読む。
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut chunk).expect("read request");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("content-length").then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0usize);
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk).expect("read body");
        buf.extend_from_slice(&chunk[..n]);
    }
    let request_line = headers.lines().next().unwrap_or_default();
    let (method_path, _) = request_line.rsplit_once(' ').unwrap_or((request_line, ""));
    let body = String::from_utf8_lossy(&buf[header_end..header_end + content_length]);
    format!("{method_path}\n{body}")
}

struct TestEnv {
    root: tempfile::TempDir,
}

struct PluginOutput {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

impl PluginOutput {
    fn stdout_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n--- stdout ---\n{}", self.stdout))
    }
}

impl TestEnv {
    /// トークンを返すフェイク gh を持つ環境。
    fn new() -> Self {
        let env = Self { root: tempfile::tempdir().expect("create tempdir") };
        env.install_gh("#!/bin/sh\necho ghs_test_token\n");
        env
    }

    /// gh が失敗する環境 (未ログイン等)。
    fn with_broken_gh() -> Self {
        let env = Self { root: tempfile::tempdir().expect("create tempdir") };
        env.install_gh("#!/bin/sh\necho 'not logged in' >&2\nexit 1\n");
        env
    }

    fn install_gh(&self, script: &str) {
        let dir = self.root.path().join("fakes");
        fs::create_dir_all(&dir).unwrap();
        let gh = dir.join("gh");
        fs::write(&gh, script).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, api_url: &str, args: &[&str]) -> PluginOutput {
        self.run_env(api_url, args, &[])
    }

    fn run_env(&self, api_url: &str, args: &[&str], envs: &[(&str, &str)]) -> PluginOutput {
        let path = format!(
            "{}:{}",
            self.fakes_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let output = Command::new(env!("CARGO_BIN_EXE_wsm-tracker-github-api"))
            .args(args)
            .env("PATH", path)
            .env("WSM_TRACKER_GITHUB_API_URL", api_url)
            .env_remove("WSM_TRACKER_OWNER")
            .env_remove("WSM_TRACKER_GITHUB_OWNER")
            .envs(envs.iter().copied())
            .output()
            .expect("run wsm-tracker-github-api");
        PluginOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn fakes_dir(&self) -> PathBuf {
        self.root.path().join("fakes")
    }
}

#[test]
fn list_issues_v1_top_level_filters_out_children() {
    // Arrange: 110 は 100 の子 (parent 非 null)。トップレベルには出ない
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repository": { "issues": { "nodes": [
            { "number": 100, "title": "Epic", "parent": null, "subIssues": { "nodes": [{ "state": "OPEN" }, { "state": "CLOSED" }] } },
            { "number": 110, "title": "Child", "parent": { "number": 100 }, "subIssues": { "nodes": [] } },
            { "number": 101, "title": "Solo", "parent": null, "subIssues": { "nodes": [] } },
        ] } } } })
        .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["list-issues-v1", "--repo", "owner/repo"]);

    // Assert: has_children は subIssues の件数から導出される
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "100", "title": "Epic", "has_children": true },
            { "id": "101", "title": "Solo", "has_children": false },
        ])
    );
    let requests = server.requests();
    assert!(requests[0].starts_with("POST /graphql"), "unexpected request: {}", requests[0]);
    assert!(requests[0].contains("owner/repo") || requests[0].contains(r#""owner":"owner""#));
    assert!(
        requests[0].contains("CREATED_AT") && requests[0].contains("DESC"),
        "issues must be requested newest first: {}",
        requests[0]
    );
}

#[test]
fn list_issues_v1_with_parent_lists_open_children_only() {
    // Arrange: closed な子は open 一覧に出さない。子は所属リポジトリを名乗る
    // (sub-issues はリポジトリ横断で張れるため)
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repository": { "issue": { "subIssues": { "nodes": [
            { "number": 110, "title": "Open child", "state": "OPEN", "repository": { "nameWithOwner": "owner/repo" }, "subIssues": { "nodes": [{ "state": "OPEN" }] } },
            { "number": 9, "title": "Cross-repo child", "state": "OPEN", "repository": { "nameWithOwner": "owner/lib" }, "subIssues": { "nodes": [] } },
            { "number": 111, "title": "Done child", "state": "CLOSED", "repository": { "nameWithOwner": "owner/repo" }, "subIssues": { "nodes": [] } },
        ] } } } } })
        .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["list-issues-v1", "--repo", "owner/repo", "--parent", "100"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "110", "title": "Open child", "repo": "owner/repo", "has_children": true },
            { "id": "9", "title": "Cross-repo child", "repo": "owner/lib", "has_children": false },
        ])
    );
    assert!(server.requests()[0].contains(r#""number":100"#), "parent must be passed as a number");
}

#[test]
fn list_group_issues_maps_project_items_across_repos() {
    // Arrange: Projects V2 の items はリポジトリ横断。closed の項目は落とす
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repositoryOwner": { "projectV2": { "items": {
            "pageInfo": { "endCursor": "pi==", "hasNextPage": true },
            "nodes": [
                { "content": { "number": 42, "title": "App task", "state": "OPEN", "repository": { "nameWithOwner": "owner/repo" }, "subIssues": { "nodes": [{ "state": "OPEN" }] } } },
                { "content": { "number": 9, "title": "Lib task", "state": "OPEN", "repository": { "nameWithOwner": "owner/lib" }, "subIssues": { "nodes": [] } } },
                { "content": { "number": 1, "title": "Done", "state": "CLOSED", "repository": { "nameWithOwner": "owner/repo" }, "subIssues": { "nodes": [] } } },
                { "content": {} },
            ],
        } } } } })
        .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run_env(
        &server.url,
        &["list-group-issues-v0", "--group", "5"],
        &[("WSM_TRACKER_GITHUB_OWNER", "me")],
    );

    // Assert: draft や PR の項目 (Issue でない content) は落ちる
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({
            "issues": [
                { "id": "42", "title": "App task", "repo": "owner/repo", "has_children": true },
                { "id": "9", "title": "Lib task", "repo": "owner/lib", "has_children": false },
            ],
            "next_cursor": "pi==",
        })
    );
}

#[test]
fn list_issues_v0_returns_flat_list() {
    // Arrange: v0 は階層を平坦化した一覧 (子も含む・has_children なし相当)
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repository": { "issues": { "nodes": [
            { "number": 100, "title": "Epic", "parent": null, "subIssues": { "nodes": [{ "state": "OPEN" }, { "state": "CLOSED" }] } },
            { "number": 110, "title": "Child", "parent": { "number": 100 }, "subIssues": { "nodes": [] } },
        ] } } } })
        .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["list-issues-v0", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "100", "title": "Epic", "has_children": false },
            { "id": "110", "title": "Child", "has_children": false },
        ])
    );
}

#[test]
fn repo_groups_filter_closed_and_respect_owner_env() {
    // Arrange: owner を環境変数で与えると viewer 解決の呼び出しが消える
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repositoryOwner": { "projectsV2": { "nodes": [
            { "number": 1, "title": "Alpha", "closed": false },
            { "number": 2, "title": "Done", "closed": true },
        ] } } } })
        .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run_env(
        &server.url,
        &["list-repo-groups-v0"],
        &[("WSM_TRACKER_GITHUB_OWNER", "myorg")],
    );

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "id": "1", "title": "Alpha" }]));
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "owner override must skip viewer resolution: {requests:?}");
    assert!(requests[0].contains(r#""owner":"myorg""#));
}

#[test]
fn list_issues_v2_pages_with_cursor() {
    // Arrange: 続きがあるページ。cursor は pageInfo.endCursor から
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repository": { "issues": {
            "pageInfo": { "endCursor": "abc123==", "hasNextPage": true },
            "nodes": [
                { "number": 100, "title": "Newest", "parent": null, "subIssues": { "nodes": [] } },
            ],
        } } } })
        .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["list-issues-v2", "--repo", "owner/repo", "--cursor", "prev=="]);

    // Assert: {issues, next_cursor} で返し、受けた cursor は after 変数で渡す
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({
            "issues": [{ "id": "100", "title": "Newest", "has_children": false }],
            "next_cursor": "abc123==",
        })
    );
    assert!(
        server.requests()[0].contains(r#""cursor":"prev==""#),
        "cursor must be forwarded: {}",
        server.requests()[0]
    );
}

#[test]
fn list_issues_v2_last_page_has_no_cursor() {
    // Arrange: hasNextPage が false なら endCursor があっても続きなし
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repository": { "issues": {
            "pageInfo": { "endCursor": "zzz", "hasNextPage": false },
            "nodes": [],
        } } } })
        .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["list-issues-v2", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!({ "issues": [], "next_cursor": null }));
}

#[test]
fn issue_maps_state_to_neutral_vocabulary() {
    // Arrange
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": { "repository": { "issue": { "title": "Fix bug", "state": "CLOSED" } } } })
            .to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["issue-v0", "--repo", "owner/repo", "--id", "42"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!({ "title": "Fix bug", "state": "closed" }));
}

#[test]
fn graphql_errors_fail_without_partial_json() {
    // Arrange: HTTP 200 でも errors があれば失敗 (GraphQL の流儀)
    let server = ApiServer::serve(vec![(
        "",
        json!({ "data": null, "errors": [{ "message": "boom" }] }).to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["list-issues-v1", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("boom"), "GraphQL errors must surface: {}", out.stderr);
}

#[test]
fn token_failure_fails_queries_and_reports_unready_info() {
    // Arrange: gh 未ログイン相当。API サーバは呼ばれない
    let env = TestEnv::with_broken_gh();

    // Act
    let query = env.run("http://127.0.0.1:1", &["list-issues-v1", "--repo", "owner/repo"]);
    let info = env.run("http://127.0.0.1:1", &["info-v0"]);

    // Assert: 照会は非ゼロ、info は ready:false のデータとして成功
    assert_eq!(query.status, Some(1));
    assert_eq!(query.stdout, "");
    assert_eq!(info.status, Some(0));
    let v = info.stdout_json();
    assert_eq!(v["name"], "github-api");
    assert_eq!(v["ready"], false);
    assert!(
        v["diagnosis"].as_str().is_some_and(|d| d.contains("gh auth login")),
        "diagnosis must include the fix: {v}"
    );
}

#[test]
fn info_reports_missing_read_project_scope_with_fix() {
    // Arrange: /user は成功するがスコープに read:project がない
    let server = ApiServer::serve(vec![(
        "X-Oauth-Scopes: gist, repo\r\n",
        json!({ "login": "me" }).to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["info-v0"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["ready"], false);
    let diagnosis = v["diagnosis"].as_str().expect("diagnosis must explain the problem");
    assert!(diagnosis.contains("read:project"), "unexpected diagnosis: {diagnosis}");
}

#[test]
fn info_reports_ready_and_hierarchical_protocol() {
    // Arrange: スコープが揃っている
    let server = ApiServer::serve(vec![(
        "X-Oauth-Scopes: read:project, repo\r\n",
        json!({ "login": "me" }).to_string(),
    )]);
    let env = TestEnv::new();

    // Act
    let out = env.run(&server.url, &["info-v0"]);

    // Assert: 階層対応 (list-issues-v1) を protocol で宣言する
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["ready"], true);
    assert!(
        v["protocol"].as_array().is_some_and(|p| p.iter().any(|s| s == "list-issues-v1")),
        "protocol must include list-issues-v1: {v}"
    );
}

#[test]
fn unknown_verb_fails_with_usage() {
    // Arrange: 前方互換の逃げ道
    let env = TestEnv::new();

    // Act
    let out = env.run("http://127.0.0.1:1", &["list-issues-v3"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert!(out.stderr.starts_with("Usage:"), "unexpected stderr: {}", out.stderr);
}
