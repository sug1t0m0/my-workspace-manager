// wsm-tracker-github のプラグイン契約テスト (docs/wsm.md「Tracker プラグイン契約」)。
//
// gh は PATH 先頭のフェイク (パターン表 → stdout / exit code) に差し替える。
// 検証の観点は 2 つ: 契約どおりの JSON を返すこと (成功は 1 ドキュメント +
// exit 0、失敗は非ゼロで JSON を出さない) と、gh との会話 (引数列)。

use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const FAKE_GH: &str = r#"#!/bin/sh
line="gh $(printf '%s' "$*" | tr '\n' ' ')"
printf '%s\n' "$line" >> "$WSM_TEST_LOG"
for p in "$WSM_TEST_RESPONSES"/*.pattern; do
  [ -e "$p" ] || continue
  if printf '%s\n' "$line" | grep -Eq -- "$(cat "$p")"; then
    base="${p%.pattern}"
    [ -f "$base.stdout" ] && cat "$base.stdout"
    code=0
    [ -f "$base.exit" ] && code="$(cat "$base.exit")"
    exit "$code"
  fi
done
exit 1
"#;

struct TestEnv {
    root: tempfile::TempDir,
    stub_count: AtomicUsize,
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
    fn new() -> Self {
        let env = Self { root: tempfile::tempdir().expect("create tempdir"), stub_count: AtomicUsize::new(0) };
        fs::create_dir_all(env.dir("fakes")).unwrap();
        fs::create_dir_all(env.dir("responses")).unwrap();
        fs::write(env.dir("invocations.log"), "").unwrap();
        let gh = env.dir("fakes").join("gh");
        fs::write(&gh, FAKE_GH).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        env
    }

    fn stub(&self, pattern: &str, stdout: &str) -> &Self {
        let n = self.stub_count.fetch_add(1, Ordering::SeqCst);
        let base = self.dir("responses").join(format!("{n:03}"));
        fs::write(base.with_extension("pattern"), pattern).unwrap();
        fs::write(base.with_extension("stdout"), stdout).unwrap();
        self
    }

    fn run(&self, args: &[&str]) -> PluginOutput {
        self.run_env(args, &[])
    }

    fn run_env(&self, args: &[&str], envs: &[(&str, &str)]) -> PluginOutput {
        let path = format!(
            "{}:{}",
            self.dir("fakes").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let output = Command::new(env!("CARGO_BIN_EXE_wsm-tracker-github"))
            .args(args)
            .env("PATH", path)
            .env("WSM_TEST_LOG", self.dir("invocations.log"))
            .env("WSM_TEST_RESPONSES", self.dir("responses"))
            .env_remove("WSM_TRACKER_GITHUB_OWNER")
            .envs(envs.iter().copied())
            .output()
            .expect("run wsm-tracker-github");
        PluginOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn invocations(&self) -> Vec<String> {
        fs::read_to_string(self.dir("invocations.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
}

#[test]
fn list_projects_filters_closed_and_maps_ids_to_strings() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^gh api user -q .login$", "me\n").stub(
        "^gh project list --owner me --format json$",
        r#"{"projects":[{"number":1,"title":"Alpha","closed":false},{"number":2,"title":"Done","closed":true}],"totalCount":2}"#,
    );

    // Act
    let out = env.run(&["list-projects-v0"]);

    // Assert: closed は落ち、id は文字列
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "id": "1", "title": "Alpha" }]));
}

#[test]
fn owner_env_override_skips_self_resolution() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^gh project list --owner myorg --format json$", r#"{"projects":[]}"#);

    // Act
    let out = env.run_env(&["list-projects-v0"], &[("WSM_TRACKER_GITHUB_OWNER", "myorg")]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([]));
    assert!(
        !env.invocations().iter().any(|l| l.starts_with("gh api user")),
        "owner override must skip self resolution: {:?}",
        env.invocations()
    );
}

#[test]
fn project_repos_returns_ns_repo_array() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^gh api user -q .login$", "me\n")
        .stub("^gh api graphql .* -F num=5 .*$", "owner/repo\nowner/tool\n");

    // Act
    let out = env.run(&["project-repos-v0", "--project", "5"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!(["owner/repo", "owner/tool"]));
}

#[test]
fn project_repos_rejects_non_numeric_project() {
    // Arrange: GitHub Projects の id は番号。他トラッカーの id 形式は弾く
    let env = TestEnv::new();

    // Act
    let out = env.run(&["project-repos-v0", "--project", "CHH"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
}

#[test]
fn list_issues_maps_numbers_and_preserves_tricky_titles() {
    // Arrange: タブ・バックスラッシュ入りタイトルも JSON エスケープで保たれる
    let env = TestEnv::new();
    env.stub(
        "^gh issue list --repo owner/repo --limit 50 --json number,title$",
        r#"[{"number":42,"title":"Fix\tbug"},{"number":7,"title":"Path C:\\tmp"}]"#,
    );

    // Act
    let out = env.run(&["list-issues-v0", "--repo", "owner/repo"]);

    // Assert: gh の返却順を保つ
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "42", "title": "Fix\tbug" },
            { "id": "7", "title": "Path C:\\tmp" },
        ])
    );
}

#[test]
fn issue_maps_state_to_neutral_vocabulary() {
    // Arrange
    let env = TestEnv::new();
    env.stub(
        "^gh issue view 42 --repo owner/repo --json title,state$",
        r#"{"title":"Fix bug","state":"CLOSED"}"#,
    );

    // Act
    let out = env.run(&["issue-v0", "--repo", "owner/repo", "--id", "42"]);

    // Assert: 実装固有の CLOSED は中立語彙 closed に写す
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!({ "title": "Fix bug", "state": "closed" }));
}

#[test]
fn gh_failure_is_nonzero_without_partial_json() {
    // Arrange: gh は未スタブ → 起動失敗相当 (認証切れ等)
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-issues-v0", "--repo", "owner/repo"]);

    // Assert: 部分的な JSON を stdout に出さない
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert!(!out.stderr.is_empty(), "diagnosis must go to stderr");
}

#[test]
fn info_reports_ready_when_scopes_include_read_project() {
    // Arrange: gh api user -i はヘッダ + JSON 本文を返す
    let env = TestEnv::new();
    env.stub(
        "^gh api user -i$",
        "HTTP/2.0 200 OK\nX-Oauth-Scopes: gist, read:org, read:project, repo\n\n{\"login\":\"me\"}\n",
    );

    // Act
    let out = env.run(&["info-v0"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["name"], "github");
    assert_eq!(v["ready"], true);
    assert_eq!(v["diagnosis"], serde_json::Value::Null);
    assert!(
        v["protocol"].as_array().is_some_and(|p| p.iter().any(|s| s == "info-v0")),
        "protocol must enumerate supported verbs: {v}"
    );
}

#[test]
fn info_reports_missing_read_project_scope_with_fix() {
    // Arrange: ログイン済みだがトークンに read:project がない
    // (プロジェクト照会が黙って空になる、実際に起きた事故のケース)
    let env = TestEnv::new();
    env.stub(
        "^gh api user -i$",
        "HTTP/2.0 200 OK\nX-Oauth-Scopes: gist, repo\n\n{\"login\":\"me\"}\n",
    );

    // Act
    let out = env.run(&["info-v0"]);

    // Assert: ready:false のデータとして返す (info 自体は成功)
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["ready"], false);
    let diagnosis = v["diagnosis"].as_str().expect("diagnosis must explain the problem");
    assert!(diagnosis.contains("read:project"), "unexpected diagnosis: {diagnosis}");
    assert!(diagnosis.contains("gh auth refresh"), "diagnosis must include the fix: {diagnosis}");
}

#[test]
fn info_reports_unready_when_gh_fails() {
    // Arrange: gh は未スタブ → 起動失敗相当 (未ログイン等)
    let env = TestEnv::new();

    // Act
    let out = env.run(&["info-v0"]);

    // Assert: 使えない状態も ready:false のデータ (非ゼロ終了にしない)
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["ready"], false);
    assert!(v["diagnosis"].is_string(), "diagnosis must be present: {v}");
}

#[test]
fn unknown_verb_fails_with_usage() {
    // Arrange: 前方互換の逃げ道 (新しい wsm が新動詞を呼んだときの見え方)
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-issues-v1"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert!(out.stderr.starts_with("Usage:"), "unexpected stderr: {}", out.stderr);
}
