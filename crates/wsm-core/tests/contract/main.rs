// wsm-core の JSON API 契約テスト (docs/wsm.md が仕様)。
//
// 既定ではビルドした Rust 版を検証する。zsh 版 (リファレンス実装) に対して
// 実行するには: WSM_CORE_BIN=$PWD/bin/wsm-core cargo test --test contract
//
// JSON の比較は意味比較 (パース後の等価判定)。整形の差は契約に含めない。

mod harness;

use harness::TestEnv;
use serde_json::json;

const USAGE_ERROR: &str = "Usage: wsm-core <list-projects|list-repos|list-issues|list-workspaces|list-devcontainer-configs|open|remove>";

// --- ディスパッチ ---

#[test]
fn usage_error_without_subcommand() {
    // Arrange
    let env = TestEnv::new();

    // Act
    let out = env.run(&[]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr_json(), json!({ "error": USAGE_ERROR }));
}

#[test]
fn usage_error_for_unknown_subcommand() {
    // Arrange
    let env = TestEnv::new();

    // Act
    let out = env.run(&["frobnicate"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stderr_json(), json!({ "error": USAGE_ERROR }));
}

// --- list-projects ---

#[test]
fn list_projects_returns_open_projects_as_array() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^gh api user -q .login", "me\n").stub(
        "^gh project list --owner me --format json",
        "{\"number\":1,\"title\":\"Roadmap\"}\n{\"number\":3,\"title\":\"Backlog\"}\n",
    );

    // Act
    let out = env.run(&["list-projects"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "number": 1, "title": "Roadmap" },
            { "number": 3, "title": "Backlog" },
        ])
    );
}

#[test]
fn list_projects_returns_empty_array_when_no_projects() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^gh api user -q .login", "me\n")
        .stub("^gh project list --owner me", "");

    // Act
    let out = env.run(&["list-projects"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([]));
}

#[test]
fn list_projects_with_explicit_user_skips_self_resolution() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^gh project list --owner someone", "{\"number\":7,\"title\":\"Ops\"}\n");

    // Act
    let out = env.run(&["list-projects", "--user", "someone"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "number": 7, "title": "Ops" }]));
    assert!(
        !env.invocations().iter().any(|l| l.starts_with("gh api user")),
        "must not resolve the user when --user is given"
    );
}

#[test]
fn list_projects_fails_when_user_cannot_be_resolved() {
    // Arrange: gh は成功するが login が空
    let env = TestEnv::new();
    env.stub("^gh api user -q .login", "");

    // Act
    let out = env.run(&["list-projects"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stderr_json(), json!({ "error": "failed to resolve GitHub user" }));
}

// --- 入力検証 ---

#[test]
fn list_issues_requires_repo_flag() {
    // Arrange
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-issues"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stderr_json(), json!({ "error": "--repo required" }));
}

#[test]
fn list_issues_rejects_repo_with_invalid_characters() {
    // Arrange
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo;rm -rf"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stderr_json(), json!({ "error": "Invalid repo: owner/repo;rm -rf" }));
}

// --- list-issues ---

#[test]
fn list_issues_combines_main_and_open_issues() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^docker ps -a", "")
        .stub("^gh issue list --repo owner/repo", "42\tFix bug\n43\tAdd feature\n");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "main", "title": "main", "active": false, "closed": false, "devcontainer": "none" },
            { "id": "42", "title": "Fix bug", "active": false, "closed": false, "devcontainer": "none" },
            { "id": "43", "title": "Add feature", "active": false, "closed": false, "devcontainer": "none" },
        ])
    );
}

// --- list-devcontainer-configs ---

#[test]
fn list_devcontainer_configs_lists_repo_configs() {
    // Arrange
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .write_home("ghq/github.com/owner/repo/.devcontainer/alt/devcontainer.json", "{}");
    let ws = format!("{}/ghq/github.com/owner/repo", env.home_str());

    // Act
    let out = env.run(&["list-devcontainer-configs", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "name": "repo", "path": format!("{ws}/.devcontainer/devcontainer.json"), "source": "repo" },
            { "name": "repo-alt", "path": format!("{ws}/.devcontainer/alt/devcontainer.json"), "source": "repo" },
        ])
    );
}

#[test]
fn list_devcontainer_configs_includes_configured_fallback() {
    // Arrange: リポジトリ側に設定はなく、config.toml のフォールバックだけがある
    let env = TestEnv::new();
    let fallback = format!("{}/fallback/devcontainer.json", env.home_str());
    env.write_home("fallback/devcontainer.json", "{}").write_home(
        ".config/wsm/config.toml",
        &format!("default_devcontainer_config = \"{fallback}\"\n"),
    );

    // Act
    let out = env.run(&["list-devcontainer-configs", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([{ "name": "default", "path": fallback, "source": "default" }])
    );
}

// --- open ---

#[test]
fn open_main_creates_session_and_returns_attach_command() {
    // Arrange: セッションは存在しない (has-session は未スタブ → 失敗)
    let env = TestEnv::new();
    env.stub("^tmux new-session -d -s owner\\.repo -c ", "");
    let ghq_path = format!("{}/ghq/github.com/owner/repo", env.home_str());

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["session"], "owner.repo");
    assert_eq!(v["path"], ghq_path.as_str());
    let attach = v["attach_command"].as_str().expect("attach_command is a string");
    assert!(
        attach.ends_with("tmux attach-session -t 'owner.repo'"),
        "unexpected attach_command: {attach}"
    );
    assert!(
        env.invocations()
            .contains(&format!("tmux new-session -d -s owner.repo -c {ghq_path}")),
        "session must be created at the ghq path: {:?}",
        env.invocations()
    );
}

#[test]
fn open_issue_creates_worktree_branch_and_session() {
    // Arrange: worktree もブランチも存在しない (show-ref は未スタブ → 失敗)
    let env = TestEnv::new();
    env.stub("worktree add --relative-paths -b feature/42 ", "")
        .stub("^tmux new-session -d -s owner\\.repo-42 -c ", "");
    let home = env.home_str();
    let worktree_path = format!("{home}/worktrees/github.com/owner/repo/42");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["session"], "owner.repo-42");
    assert_eq!(v["path"], worktree_path.as_str());
    assert!(
        env.invocations().contains(&format!(
            "git -C {home}/ghq/github.com/owner/repo worktree add --relative-paths -b feature/42 {worktree_path}"
        )),
        "worktree must be added with a new feature branch: {:?}",
        env.invocations()
    );
}

// --- remove ---

#[test]
fn remove_issue_tears_down_session_and_worktree() {
    // Arrange: 該当コンテナなし
    let env = TestEnv::new();
    env.stub("^docker ps -a", "");
    let home = env.home_str();

    // Act
    let out = env.run(&["remove", "--repo", "owner/repo", "--target", "42"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "status": "ok", "message": "Removed worktree and session: owner.repo-42" })
    );
    let invocations = env.invocations();
    assert!(invocations.contains(&"tmux kill-session -t =owner.repo-42".to_owned()));
    assert!(invocations.contains(&format!(
        "git -C {home}/ghq/github.com/owner/repo worktree remove {home}/worktrees/github.com/owner/repo/42"
    )));
}

#[test]
fn remove_main_kills_session_but_keeps_the_clone() {
    // Arrange: 該当コンテナなし
    let env = TestEnv::new();
    env.stub("^docker ps -a", "");

    // Act
    let out = env.run(&["remove", "--repo", "owner/repo", "--target", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "status": "ok", "message": "Removed session: owner/repo" })
    );
    assert!(
        !env.invocations().iter().any(|l| l.contains("worktree remove")),
        "main must never remove a worktree: {:?}",
        env.invocations()
    );
}
