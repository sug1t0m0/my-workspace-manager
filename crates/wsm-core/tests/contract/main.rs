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

// --- list-repos ---

#[test]
fn list_repos_lists_all_ghq_repos_when_project_none() {
    // Arrange
    let env = TestEnv::new();
    env.stub(
        "^ghq list$",
        "github.com/owner/tool\ngithub.com/owner/repo\nexample.com/other/repo\n",
    );

    // Act
    let out = env.run(&["list-repos", "--project", "none"]);

    // Assert: github.com のみ・ソート済み。Tracker (gh) には触れない
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "ns_repo": "owner/repo", "active_count": 0 },
            { "ns_repo": "owner/tool", "active_count": 0 },
        ])
    );
    assert!(
        !env.invocations().iter().any(|l| l.starts_with("gh ")),
        "project none must not touch the tracker: {:?}",
        env.invocations()
    );
}

#[test]
fn list_repos_counts_active_workspaces() {
    // Arrange: main セッションと Issue 42 の worktree + セッションが生きている
    let env = TestEnv::new();
    let home = env.home_str();
    env.write_home("ghq/github.com/owner/repo/.gitkeep", "")
        .write_home("worktrees/github.com/owner/repo/42/.gitkeep", "")
        .stub("^ghq list$", "github.com/owner/repo\n")
        .stub(
            "worktree list --porcelain",
            &format!(
                "worktree {home}/ghq/github.com/owner/repo\nHEAD aaa\nbranch refs/heads/main\n\nworktree {home}/worktrees/github.com/owner/repo/42\nHEAD bbb\nbranch refs/heads/feature/42\n\n"
            ),
        )
        .stub("^tmux has-session -t =owner\\.repo$", "")
        .stub("^tmux has-session -t =owner\\.repo-42$", "");

    // Act
    let out = env.run(&["list-repos", "--project", "none"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "ns_repo": "owner/repo", "active_count": 2 }]));
}

#[test]
fn list_repos_filters_project_repos_by_local_clones() {
    // Arrange: Project には 2 リポジトリ、ローカルにあるのは片方だけ
    let env = TestEnv::new();
    env.stub("^gh api graphql -f query=", "owner/repo\nowner/other\n")
        .stub("^ghq list$", "github.com/owner/repo\ngithub.com/mine/tool\n");

    // Act
    let out = env.run(&["list-repos", "--project", "5", "--user", "me"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "ns_repo": "owner/repo", "active_count": 0 }]));
}

// --- list-workspaces ---

#[test]
fn list_workspaces_returns_empty_array_when_none_active() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^ghq list -p$", &format!("{}/ghq/github.com/owner/repo\n", env.home_str()));

    // Act
    let out = env.run(&["list-workspaces"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([]));
}

#[test]
fn list_workspaces_lists_main_and_worktree_entries() {
    // Arrange: main セッションと、closed な Issue 42 の worktree セッションが生きている
    let env = TestEnv::new();
    let home = env.home_str();
    env.write_home("ghq/github.com/owner/repo/.gitkeep", "")
        .write_home("worktrees/github.com/owner/repo/42/.gitkeep", "")
        .stub("^ghq list -p$", &format!("{home}/ghq/github.com/owner/repo\n"))
        .stub(
            "worktree list --porcelain",
            &format!(
                "worktree {home}/worktrees/github.com/owner/repo/42\nHEAD bbb\nbranch refs/heads/feature/42\n\n"
            ),
        )
        .stub("^tmux has-session -t =owner\\.repo$", "")
        .stub("^tmux has-session -t =owner\\.repo-42$", "")
        .stub("^docker ps -a", "")
        .stub("^gh issue view 42 --repo owner/repo --json title,state", "Fix bug\tCLOSED\n");

    // Act
    let out = env.run(&["list-workspaces"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "ns_repo": "owner/repo", "id": "main", "title": "main", "active": true, "closed": false, "devcontainer": "none" },
            { "ns_repo": "owner/repo", "id": "42", "title": "Fix bug", "active": true, "closed": true, "devcontainer": "none" },
        ])
    );
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

// --- open --config (DevContainer) ---

#[test]
fn open_with_config_starts_devcontainer_and_adds_window() {
    // Arrange: コンテナは存在しない (before = none) → created
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner\\.repo -c ", "")
        .stub("^docker ps -a", "")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("^docker ps -q ", "abc123\n")
        .stub("^docker inspect --format ", "[{\"remoteUser\":\"dev\"}]\n")
        .stub("^tmux new-window -d -P -F ", "%5\n");
    let ws = format!("{}/ghq/github.com/owner/repo", env.home_str());
    let cfg = format!("{ws}/.devcontainer/devcontainer.json");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main", "--config", &cfg]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["message"], "Opened owner/repo (main) [tmux] + devcontainer(s) [created: repo]");
    let invocations = env.invocations();
    assert!(
        invocations.contains(&format!(
            "devcontainer up --workspace-folder {ws} --config {cfg} --id-label wsm.ns-repo=owner/repo --id-label wsm.issue-id=main --id-label wsm.config=repo"
        )),
        "devcontainer up must carry the identity labels: {invocations:?}"
    );
    assert!(
        invocations.contains(&format!(
            "tmux new-window -d -P -F #{{pane_id}} -t owner.repo: -n 🐳 docker exec -it --user 'dev' -w '/workspaces/ghq/github.com/owner/repo' 'abc123' zsh"
        )),
        "🐳 window must exec into the container as remoteUser: {invocations:?}"
    );
    assert!(
        invocations.contains(&"tmux set-option -p -t %5 @wsm_cid abc123".to_owned()),
        "dedup key must be recorded on the new pane: {invocations:?}"
    );
}

#[test]
fn open_issue_with_config_mounts_worktree_and_common_dir() {
    // Arrange: worktree Workspace ではフォールバック設定 (config 名 default) を使う
    let env = TestEnv::new();
    env.write_home("fallback/devcontainer.json", "{}")
        .stub("worktree add --relative-paths -b feature/42 ", "")
        .stub("^tmux new-session -d -s owner\\.repo-42 -c ", "")
        .stub("^docker ps -a", "")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("^docker ps -q ", "");
    let home = env.home_str();
    let cfg = format!("{home}/fallback/devcontainer.json");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42", "--config", &cfg]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["message"], "Opened owner/repo #42 [tmux] + devcontainer(s) [created: default]");
    let invocations = env.invocations();
    assert!(
        invocations.contains(&format!(
            "devcontainer up --workspace-folder {home}/worktrees/github.com/owner/repo/42 --config {cfg} --id-label wsm.ns-repo=owner/repo --id-label wsm.issue-id=42 --id-label wsm.config=default --mount-git-worktree-common-dir --remote-env WSM_WORKTREE_PATH=/workspaces/worktrees/github.com/owner/repo/42 --remote-env WSM_WORKTREE_COMMON_DIR=/workspaces/ghq/github.com/owner/repo/.git"
        )),
        "worktree open must mount the git common dir and pass WSM_* remote-env: {invocations:?}"
    );
    assert!(
        !invocations.iter().any(|l| l.starts_with("tmux new-window")),
        "no window when no container is running: {invocations:?}"
    );
}

#[test]
fn open_with_config_reuses_running_container_and_dedups_window() {
    // Arrange: コンテナは running (before = running) → reused、
    // 既存ペインが dedup キーを持っている → ウィンドウは追加しない
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux has-session -t =owner\\.repo$", "")
        .stub("^docker ps -a", "running\n")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("^docker ps -q ", "abc123\n")
        .stub("^docker inspect --format ", "[{\"remoteUser\":\"dev\"}]\n")
        .stub("^tmux list-panes -s -t owner\\.repo -F ", "abc123\n");
    let cfg = format!("{}/ghq/github.com/owner/repo/.devcontainer/devcontainer.json", env.home_str());

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main", "--config", &cfg]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json()["message"],
        "Opened owner/repo (main) [tmux] + devcontainer(s) [reused: repo]"
    );
    assert!(
        !env.invocations().iter().any(|l| l.starts_with("tmux new-window")),
        "window must be deduped by container id: {:?}",
        env.invocations()
    );
}

#[test]
fn open_fails_when_devcontainer_up_fails() {
    // Arrange
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner\\.repo -c ", "")
        .stub("^docker ps -a", "")
        .stub_exit("^devcontainer up --workspace-folder ", "", 1);
    let cfg = format!("{}/ghq/github.com/owner/repo/.devcontainer/devcontainer.json", env.home_str());

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main", "--config", &cfg]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr_json(),
        json!({ "error": format!("devcontainer up failed for {cfg}") })
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
