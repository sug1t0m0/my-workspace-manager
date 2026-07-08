// wsm-server の JSON API 契約テスト (docs/wsm.md が仕様)。
//
// 既定ではビルドしたバイナリを検証する。WSM_SERVER_BIN=<path> で別の
// ビルド (リリースバイナリ等) に差し替えて同じスイートを回せる。
//
// JSON の比較は意味比較 (パース後の等価判定)。整形の差は契約に含めない。

mod harness;

use harness::TestEnv;
use serde_json::json;

const USAGE_ERROR: &str = "Usage: wsm-server <list-projects|list-repos|list-issues|list-workspaces|list-devcontainer-configs|list-session-managers|open|remove>";

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
// SSH 経由で呼ばれるため、パラメータごとに形まで検証する (docs/wsm.md 共通仕様)。

#[test]
fn required_flags_are_enforced_per_subcommand() {
    // Arrange
    let env = TestEnv::new();
    let cases: &[(&[&str], &str)] = &[
        (&["list-issues"], "--repo required"),
        (&["list-devcontainer-configs"], "--repo required"),
        (&["list-devcontainer-configs", "--repo", "owner/repo"], "--issue required"),
        (&["open"], "--repo required"),
        (&["open", "--repo", "owner/repo"], "--issue required"),
        (&["remove"], "--repo required"),
        (&["remove", "--repo", "owner/repo"], "--issue required"),
    ];

    for (args, expected) in cases {
        // Act
        let out = env.run(args);

        // Assert
        assert_eq!(out.status, Some(1), "args: {args:?}");
        assert_eq!(out.stderr_json(), json!({ "error": expected }), "args: {args:?}");
    }
}

#[test]
fn empty_flag_values_count_as_missing() {
    // Arrange
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-issues", "--repo", ""]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stderr_json(), json!({ "error": "--repo required" }));
}

#[test]
fn parameter_shapes_are_validated() {
    // Arrange
    let env = TestEnv::new();
    let cases: &[(&[&str], &str)] = &[
        // repo: <ns>/<repo> の形。メタ文字・トラバーサル・オプション注入を弾く
        (&["list-issues", "--repo", "owner/repo;rm -rf"], "Invalid repo: owner/repo;rm -rf"),
        // ns は GitHub 規則 (英数と -)。ドット・アンダースコアは入らない
        // (セッション名の / → . 変換の単射性の根拠)
        (&["list-issues", "--repo", "my.org/repo"], "Invalid repo: my.org/repo"),
        (&["list-issues", "--repo", "my_org/repo"], "Invalid repo: my_org/repo"),
        (&["list-issues", "--repo", "repo-without-namespace"], "Invalid repo: repo-without-namespace"),
        (&["list-issues", "--repo", "a/b/c"], "Invalid repo: a/b/c"),
        (&["list-issues", "--repo", "../repo"], "Invalid repo: ../repo"),
        (&["list-issues", "--repo", "-owner/repo"], "Invalid repo: -owner/repo"),
        // issue: 英数と - のみ (先頭は英数)。メタ文字・トラバーサル・オプション注入を弾く
        (&["open", "--repo", "owner/repo", "--issue", "CHH_111"], "Invalid issue: CHH_111"),
        (&["remove", "--repo", "owner/repo", "--issue", "42;rm -rf"], "Invalid issue: 42;rm -rf"),
        (&["open", "--repo", "owner/repo", "--issue", "../42"], "Invalid issue: ../42"),
        (&["list-devcontainer-configs", "--repo", "owner/repo", "--issue", "-42"], "Invalid issue: -42"),
        // user: 英数と - のみ / project: 数字のみ
        (&["list-projects", "--user", "bad_user"], "Invalid user: bad_user"),
        (&["list-repos", "--project", "5x", "--user", "me"], "Invalid project: 5x"),
    ];

    for (args, expected) in cases {
        // Act
        let out = env.run(args);

        // Assert
        assert_eq!(out.status, Some(1), "args: {args:?}");
        assert_eq!(out.stderr_json(), json!({ "error": expected }), "args: {args:?}");
    }
}

#[test]
fn repeated_flags_use_the_last_value() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub("^gh issue list --repo owner/repo", "");

    // Act
    let out = env.run(&["list-issues", "--repo", "aaa/xxx", "--repo", "owner/repo"]);

    // Assert: 同名フラグの重複は後勝ち (zsh のループ上書きと同じ)
    assert_eq!(out.status, Some(0));
    let invocations = env.invocations();
    assert!(invocations.iter().any(|l| l.contains("--repo owner/repo")), "{invocations:?}");
    assert!(!invocations.iter().any(|l| l.contains("aaa/xxx")), "{invocations:?}");
}

#[test]
fn dotted_repo_names_stay_valid() {
    // Arrange: .github のような先頭ドットのリポジトリ名は正当 (締めすぎ防止)
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub("^gh issue list --repo owner/.github", "");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/.github"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json()[0]["id"], "main");
}

#[test]
fn list_projects_preserves_backslashes_in_titles() {
    // Arrange: タイトルにバックスラッシュを含む Project (zsh の echo は \\ を解釈して JSON を壊す)
    let env = TestEnv::new();
    env.stub("^gh api user -q .login", "me\n").stub(
        "^gh project list --owner me --format json",
        "{\"number\":1,\"title\":\"Group \\\\ A\"}\n",
    );

    // Act
    let out = env.run(&["list-projects"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "number": 1, "title": "Group \\ A" }]));
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
        .stub("^tmux has-session -t =owner_repo$", "")
        .stub("^tmux has-session -t =owner_repo_42$", "");

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
        .stub("^tmux has-session -t =owner_repo$", "")
        .stub("^tmux has-session -t =owner_repo_42$", "")
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

#[test]
fn list_issues_shows_orphaned_worktrees_as_closed_in_worktree_order() {
    // Arrange: Issue 41, 42 は open 一覧に出てこない (closed) が、
    // worktree とセッションが残っている。43 だけが open
    let env = TestEnv::new();
    let home = env.home_str();
    env.write_home("ghq/github.com/owner/repo/.gitkeep", "")
        .write_home("worktrees/github.com/owner/repo/41/.gitkeep", "")
        .write_home("worktrees/github.com/owner/repo/42/.gitkeep", "")
        .stub(
            "worktree list --porcelain",
            &format!(
                "worktree {home}/worktrees/github.com/owner/repo/41\nHEAD aaa\nbranch refs/heads/feature/41\n\nworktree {home}/worktrees/github.com/owner/repo/42\nHEAD bbb\nbranch refs/heads/feature/42\n\n"
            ),
        )
        .stub("^tmux has-session -t =owner_repo_41$", "")
        .stub("^tmux has-session -t =owner_repo_42$", "")
        .stub("^docker ps -a", "")
        .stub("^gh issue list --repo owner/repo", "43\tOther work\n")
        .stub("^gh issue view 41 --repo owner/repo --json title -q", "Old bug\n")
        .stub("^gh issue view 42 --repo owner/repo --json title -q", "Stale spike\n");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: 孤児は closed:true & active:true で、worktree 一覧順に並ぶ
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "main", "title": "main", "active": false, "closed": false, "devcontainer": "none" },
            { "id": "43", "title": "Other work", "active": false, "closed": false, "devcontainer": "none" },
            { "id": "41", "title": "Old bug", "active": true, "closed": true, "devcontainer": "none" },
            { "id": "42", "title": "Stale spike", "active": true, "closed": true, "devcontainer": "none" },
        ])
    );
}

#[test]
fn list_issues_aggregates_devcontainer_states() {
    // Arrange: Issue 42 のコンテナは停止のみ、43 は停止+稼働の混在
    let env = TestEnv::new();
    env.stub("^docker ps -a --filter label=wsm.ns-repo=owner/repo --filter label=wsm.issue-id=main ", "")
        .stub("^docker ps -a --filter label=wsm.ns-repo=owner/repo --filter label=wsm.issue-id=42 ", "exited\n")
        .stub(
            "^docker ps -a --filter label=wsm.ns-repo=owner/repo --filter label=wsm.issue-id=43 ",
            "exited\nrunning\n",
        )
        .stub("^gh issue list --repo owner/repo", "42\tFix bug\n43\tAdd feature\n");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: 1 つでも running があれば running、行はあるが running がなければ stopped
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "main", "title": "main", "active": false, "closed": false, "devcontainer": "none" },
            { "id": "42", "title": "Fix bug", "active": false, "closed": false, "devcontainer": "stopped" },
            { "id": "43", "title": "Add feature", "active": false, "closed": false, "devcontainer": "running" },
        ])
    );
}

#[test]
fn list_issues_preserves_backslash_sequences_in_titles() {
    // Arrange: タイトルに literal な \t (バックスラッシュ + t) を含む Issue。
    // zsh の echo は \t をタブに解釈してタイトルを壊す
    let env = TestEnv::new();
    env.stub("^docker ps -a", "")
        .stub("^gh issue list --repo owner/repo", "42\tKeep \\t literal\n");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json()[1],
        json!({ "id": "42", "title": "Keep \\t literal", "active": false, "closed": false, "devcontainer": "none" })
    );
}

// --- パス導出の基点 ---

#[test]
fn respects_custom_ghq_root() {
    // Arrange: ghq root が既定 (~/ghq) 以外の場所を返す
    let env = TestEnv::new();
    let home = env.home_str();
    env.stub("^ghq root$", &format!("{home}/src\n"))
        .write_home("src/github.com/owner/repo/.devcontainer/devcontainer.json", "{}");

    // Act
    let out = env.run(&["list-devcontainer-configs", "--repo", "owner/repo", "--issue", "main"]);

    // Assert: パス導出が ghq root に追随する
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json()[0]["path"],
        format!("{home}/src/github.com/owner/repo/.devcontainer/devcontainer.json")
    );
}

#[test]
fn respects_configured_worktree_root() {
    // Arrange: config.toml で worktree の置き場を変更 (チルダ展開も検証)
    let env = TestEnv::new();
    let home = env.home_str();
    env.write_home(
            ".config/wsm/config.toml",
            &format!("{}worktree_root = \"~/wt\"\n", env.managers_config(&["tmux", "herdr"])),
        )
        .stub("worktree add --relative-paths -b feature/42 ", "")
        .stub("^tmux new-session -d -s owner_repo_42 -c ", "");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json()["path"], format!("{home}/wt/github.com/owner/repo/42"));
    assert!(
        env.invocations()
            .iter()
            .any(|l| l.contains(&format!("-b feature/42 {home}/wt/github.com/owner/repo/42"))),
        "worktree must be created under the configured root: {:?}",
        env.invocations()
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
    env.stub("^tmux new-session -d -s owner_repo -c ", "");
    let ghq_path = format!("{}/ghq/github.com/owner/repo", env.home_str());

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["session"], "owner_repo");
    assert_eq!(v["path"], ghq_path.as_str());
    let attach = v["attach_command"].as_str().expect("attach_command is a string");
    assert!(
        attach.ends_with("tmux attach-session -t 'owner_repo'"),
        "unexpected attach_command: {attach}"
    );
    assert!(
        env.invocations()
            .contains(&format!("tmux new-session -d -s owner_repo -c {ghq_path}")),
        "session must be created at the ghq path: {:?}",
        env.invocations()
    );
}

#[test]
fn open_issue_creates_worktree_branch_and_session() {
    // Arrange: worktree もブランチも存在しない (show-ref は未スタブ → 失敗)
    let env = TestEnv::new();
    env.stub("worktree add --relative-paths -b feature/42 ", "")
        .stub("^tmux new-session -d -s owner_repo_42 -c ", "");
    let home = env.home_str();
    let worktree_path = format!("{home}/worktrees/github.com/owner/repo/42");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["session"], "owner_repo_42");
    assert_eq!(v["path"], worktree_path.as_str());
    assert!(
        env.invocations().contains(&format!(
            "git -C {home}/ghq/github.com/owner/repo worktree add --relative-paths -b feature/42 {worktree_path}"
        )),
        "worktree must be added with a new feature branch: {:?}",
        env.invocations()
    );
}

#[test]
fn open_issue_accepts_tracker_style_ids() {
    // Arrange: Jira 形式の Issue id (CHH-111)。worktree もブランチも存在しない
    let env = TestEnv::new();
    env.stub("worktree add --relative-paths -b feature/CHH-111 ", "")
        .stub("^tmux new-session -d -s owner_repo_CHH-111 -c ", "");
    let home = env.home_str();
    let worktree_path = format!("{home}/worktrees/github.com/owner/repo/CHH-111");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "CHH-111"]);

    // Assert: 数字の id と同じ導出規則 (ブランチ・セッション名・パス) が適用される
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["session"], "owner_repo_CHH-111");
    assert_eq!(v["path"], worktree_path.as_str());
    assert!(
        env.invocations().contains(&format!(
            "git -C {home}/ghq/github.com/owner/repo worktree add --relative-paths -b feature/CHH-111 {worktree_path}"
        )),
        "worktree must be added with a new feature branch: {:?}",
        env.invocations()
    );
}

// --- セッションマネージャーの設定 ---

#[test]
fn list_session_managers_returns_configured_order() {
    // Arrange: 設定ファイルの並び順 (herdr が先頭 = 既定)
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", &env.managers_config(&["herdr", "tmux"]));

    // Act
    let out = env.run(&["list-session-managers"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([{ "name": "herdr", "default": true }, { "name": "tmux", "default": false }])
    );
}

#[test]
fn explicit_default_session_manager_overrides_order() {
    // Arrange: 並び順は tmux 先頭だが、default_session_manager で herdr を既定に
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.write_home(
        ".config/wsm/config.toml",
        &format!("{}default_session_manager = \"herdr\"\n", env.managers_config(&["tmux", "herdr"])),
    )
    .stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
    .stub(
        &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
        &herdr_workspaces_json(&[("w1", "repo")]),
    )
    .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace focus "), "");

    // Act
    let list = env.run(&["list-session-managers"]);
    let opened = env.run(&["open", "--repo", "owner/repo", "--issue", "main"]);

    // Assert: default フラグも選択も herdr
    assert_eq!(
        list.stdout_json(),
        json!([{ "name": "tmux", "default": false }, { "name": "herdr", "default": true }])
    );
    assert_eq!(opened.stdout_json()["message"], "Opened owner/repo (main) [herdr]");
}

#[test]
fn open_fails_when_no_session_manager_is_configured() {
    // Arrange: マネージャーのパスが 1 つも設定されていない (フォールバックなし)
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", "");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(
        out.stderr_json(),
        json!({ "error": "no session manager configured (set tmux_path / herdr_path in config.toml)" })
    );
}

// --- herdr (SessionManager 実装) ---
// herdr はリポジトリ単位のセッション (<ns>.<repo>) に Issue ごとの workspace
// (ラベル = Issue 番号) を追加するモデル。

/// herdr session list --json のフィクスチャ (owner/repo のセッション)。
fn herdr_sessions_json(home: &str, running: bool) -> String {
    format!(
        "{{\"sessions\":[{{\"default\":false,\"name\":\"owner.repo\",\"running\":{running},\"session_dir\":\"{home}/.config/herdr/sessions/owner.repo\",\"socket_path\":\"{}\"}}]}}\n",
        herdr_sock(home)
    )
}

fn herdr_sock(home: &str) -> String {
    format!("{home}/.config/herdr/sessions/owner.repo/herdr.sock")
}

/// herdr workspace list のフィクスチャ。(workspace_id, label) の列。
fn herdr_workspaces_json(entries: &[(&str, &str)]) -> String {
    let workspaces: Vec<String> = entries
        .iter()
        .map(|(id, label)| format!("{{\"workspace_id\":\"{id}\",\"label\":\"{label}\",\"focused\":false}}"))
        .collect();
    format!(
        "{{\"id\":\"cli:workspace:list\",\"result\":{{\"type\":\"workspace_list\",\"workspaces\":[{}]}}}}\n",
        workspaces.join(",")
    )
}

/// zsh の printf %q 相当 (attach_command の herdr 形式の期待値組み立て用)。
fn zsh_quoted(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            let escaped = !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'));
            escaped.then_some('\\').into_iter().chain(std::iter::once(c))
        })
        .collect()
}

/// herdr を先頭 (= 既定) にしたマネージャー設定。
fn herdr_first_config(env: &TestEnv) -> String {
    env.managers_config(&["herdr", "tmux"])
}

#[test]
fn open_issue_with_herdr_creates_workspace_in_repo_session() {
    // Arrange: リポジトリセッションは起動済みで main workspace もある。
    // Issue 42 の workspace はまだない
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.write_home(".config/wsm/config.toml", &herdr_first_config(&env))
        .stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(
            &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
            &herdr_workspaces_json(&[("w1", "repo")]),
        )
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace create "), "")
        .stub("worktree add --relative-paths -b feature/42 ", "");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42"]);

    // Assert: セッションはリポジトリ単位、workspace がラベル 42 で作られる。
    // main workspace は既存なので作り直さない
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["session"], "owner.repo");
    assert_eq!(v["message"], "Opened owner/repo 42 [herdr]");
    let invocations = env.invocations();
    assert!(
        invocations.contains(&format!(
            "HERDR_SOCKET_PATH={sock} herdr workspace create --cwd {home}/worktrees/github.com/owner/repo/42 --label 42 --focus"
        )),
        "issue workspace must be created in the repo session: {invocations:?}"
    );
    assert!(
        !invocations.iter().any(|l| l.contains("--label repo ")),
        "existing main workspace must not be recreated: {invocations:?}"
    );
    assert!(
        !invocations.iter().any(|l| l.starts_with("tmux new-session")),
        "herdr must not create tmux sessions: {invocations:?}"
    );
}

#[test]
fn open_issue_with_herdr_focuses_existing_workspace() {
    // Arrange: Issue 42 の workspace が既にある
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.write_home(".config/wsm/config.toml", &herdr_first_config(&env))
        .write_home("worktrees/github.com/owner/repo/42/.gitkeep", "")
        .stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(
            &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
            &herdr_workspaces_json(&[("w1", "repo"), ("w7", "42")]),
        )
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace focus "), "");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42"]);

    // Assert: 再作成せず、既存 workspace にフォーカスする
    assert_eq!(out.status, Some(0));
    let invocations = env.invocations();
    assert!(
        invocations.contains(&format!("HERDR_SOCKET_PATH={sock} herdr workspace focus w7")),
        "existing workspace must be focused: {invocations:?}"
    );
    assert!(
        !invocations.iter().any(|l| l.contains("workspace create")),
        "must not create a duplicate workspace: {invocations:?}"
    );
}

#[test]
fn open_issue_with_herdr_starts_session_headlessly_when_not_running() {
    // Arrange: セッション未起動 → 1 回目の照会は not running、以降 running
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.write_home(".config/wsm/config.toml", &herdr_first_config(&env))
        .stub_once("^herdr session list --json$", &herdr_sessions_json(&home, false))
        .stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"), &herdr_workspaces_json(&[]))
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace create "), "")
        .stub("worktree add --relative-paths -b feature/42 ", "");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42"]);

    // Assert: ヘッドレス起動 → main workspace (フォーカスなし) → Issue workspace
    assert_eq!(out.status, Some(0));
    let invocations = env.invocations();
    assert!(
        invocations.contains(&"herdr --session owner.repo server".to_owned()),
        "repo session must be started headlessly: {invocations:?}"
    );
    assert!(
        invocations.contains(&format!(
            "HERDR_SOCKET_PATH={sock} herdr workspace create --cwd {home}/ghq/github.com/owner/repo --label repo --no-focus"
        )),
        "main workspace must be created without stealing focus: {invocations:?}"
    );
    assert!(
        invocations.contains(&format!(
            "HERDR_SOCKET_PATH={sock} herdr workspace create --cwd {home}/worktrees/github.com/owner/repo/42 --label 42 --focus"
        )),
        "issue workspace must be created and focused: {invocations:?}"
    );
}

#[test]
fn open_main_with_herdr_attaches_to_repo_session() {
    // Arrange: セッション未起動 → ヘッドレス起動 + main の workspace を作る
    // (ヘッドレス起動直後は workspace ゼロで、アタッチ時の自動作成に任せると
    //  cwd がリポジトリにならないため)
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.write_home(".config/wsm/config.toml", &herdr_first_config(&env))
        .stub_once("^herdr session list --json$", &herdr_sessions_json(&home, false))
        .stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"), &herdr_workspaces_json(&[]))
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace create "), "");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["session"], "owner.repo");
    assert_eq!(v["message"], "Opened owner/repo (main) [herdr]");
    let script = format!(
        "cd '{home}/ghq/github.com/owner/repo' && exec '{}/herdr' --session 'owner.repo'",
        env.fakes_dir_str()
    );
    assert_eq!(v["attach_command"], format!("/bin/bash -lc {}", zsh_quoted(&script)));
    let invocations = env.invocations();
    assert!(invocations.contains(&"herdr --session owner.repo server".to_owned()));
    assert!(
        invocations.contains(&format!(
            "HERDR_SOCKET_PATH={sock} herdr workspace create --cwd {home}/ghq/github.com/owner/repo --label repo --focus"
        )),
        "main workspace must be created at the ghq path: {invocations:?}"
    );
}

#[test]
fn open_main_with_herdr_focuses_existing_main_workspace() {
    // Arrange: main の workspace (ラベル = リポジトリ名) が既にある
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.write_home(".config/wsm/config.toml", &herdr_first_config(&env))
        .stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(
            &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
            &herdr_workspaces_json(&[("w1", "repo"), ("w7", "42")]),
        )
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace focus "), "");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let invocations = env.invocations();
    assert!(
        invocations.contains(&format!("HERDR_SOCKET_PATH={sock} herdr workspace focus w1")),
        "existing main workspace must be focused: {invocations:?}"
    );
    assert!(
        !invocations.iter().any(|l| l.contains("workspace create")),
        "must not create a duplicate main workspace: {invocations:?}"
    );
}

#[test]
fn remove_issue_with_herdr_closes_workspace_only() {
    // Arrange: Issue 42 の workspace の他にも workspace が残っている
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(
            &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
            &herdr_workspaces_json(&[("w1", "my-workspace-manager"), ("w7", "42")]),
        )
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace close "), "")
        .stub("^docker ps -a", "");

    // Act
    let out = env.run(&["remove", "--repo", "owner/repo", "--issue", "42"]);

    // Assert: workspace close のみで、セッションは残す
    assert_eq!(out.status, Some(0));
    let invocations = env.invocations();
    assert!(
        invocations.contains(&format!("HERDR_SOCKET_PATH={sock} herdr workspace close w7")),
        "issue workspace must be closed: {invocations:?}"
    );
    assert!(
        !invocations.iter().any(|l| l.starts_with("herdr session stop")),
        "session must survive while other workspaces remain: {invocations:?}"
    );
}

#[test]
fn remove_last_herdr_issue_workspace_stops_the_session() {
    // Arrange: Issue 42 の workspace がセッション内の最後の workspace
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(
            &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
            &herdr_workspaces_json(&[("w7", "42")]),
        )
        .stub(&format!("^HERDR_SOCKET_PATH={sock} herdr workspace close "), "")
        .stub("^herdr session stop owner\\.repo --json$", "")
        .stub("^herdr session delete owner\\.repo --json$", "")
        .stub("^docker ps -a", "");

    // Act
    let out = env.run(&["remove", "--repo", "owner/repo", "--issue", "42"]);

    // Assert: 空になったセッションは stop + delete で畳む
    assert_eq!(out.status, Some(0));
    let invocations = env.invocations();
    assert!(invocations.contains(&format!("HERDR_SOCKET_PATH={sock} herdr workspace close w7")));
    assert!(invocations.contains(&"herdr session stop owner.repo --json".to_owned()));
    assert!(invocations.contains(&"herdr session delete owner.repo --json".to_owned()));
}

#[test]
fn remove_main_with_herdr_refuses_while_issue_workspaces_remain() {
    // Arrange: セッションに main 以外の workspace (Jira 形式の Issue id) が残っている
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.stub("^herdr session list --json$", &herdr_sessions_json(&home, true)).stub(
        &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
        &herdr_workspaces_json(&[("w1", "my-workspace-manager"), ("w7", "CHH-111")]),
    );

    // Act
    let out = env.run(&["remove", "--repo", "owner/repo", "--issue", "main"]);

    // Assert: エラーにして何も壊さない
    assert_eq!(out.status, Some(1));
    assert_eq!(
        out.stderr_json(),
        json!({ "error": "herdr session has open issue workspaces: owner.repo" })
    );
    let invocations = env.invocations();
    assert!(
        !invocations.iter().any(|l| l.starts_with("tmux kill-session") || l.starts_with("herdr session stop")),
        "refusal must not tear anything down: {invocations:?}"
    );
}

#[test]
fn remove_main_with_herdr_stops_session_when_no_issue_workspaces() {
    // Arrange: main workspace (ラベル = リポジトリ名) 以外は残っていない
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(
            &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
            &herdr_workspaces_json(&[("w1", "repo")]),
        )
        .stub("^herdr session stop owner\\.repo --json$", "")
        .stub("^herdr session delete owner\\.repo --json$", "")
        .stub("^docker ps -a", "");

    // Act
    let out = env.run(&["remove", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "status": "ok", "message": "Removed session: owner/repo" })
    );
    let invocations = env.invocations();
    assert!(invocations.contains(&"herdr session stop owner.repo --json".to_owned()));
    assert!(invocations.contains(&"herdr session delete owner.repo --json".to_owned()));
}

#[test]
fn list_issues_marks_herdr_workspace_as_active() {
    // Arrange: tmux セッションはないが、herdr の repo セッションに
    // Issue 42 の workspace が生きている (マネージャー設定に依らず検出される)
    let env = TestEnv::new();
    let home = env.home_str();
    let sock = herdr_sock(&home);
    env.write_home("ghq/github.com/owner/repo/.gitkeep", "")
        .write_home("worktrees/github.com/owner/repo/42/.gitkeep", "")
        .stub("^herdr session list --json$", &herdr_sessions_json(&home, true))
        .stub(
            &format!("^HERDR_SOCKET_PATH={sock} herdr workspace list$"),
            &herdr_workspaces_json(&[("w7", "42")]),
        )
        .stub(
            "worktree list --porcelain",
            &format!(
                "worktree {home}/worktrees/github.com/owner/repo/42\nHEAD bbb\nbranch refs/heads/feature/42\n\n"
            ),
        )
        .stub("^docker ps -a", "")
        .stub("^gh issue list --repo owner/repo", "42\tFix bug\n");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: main はセッション running で active、42 は workspace 存在で active
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "main", "title": "main", "active": true, "closed": false, "devcontainer": "none" },
            { "id": "42", "title": "Fix bug", "active": true, "closed": false, "devcontainer": "none" },
        ])
    );
}

// --- open --config (DevContainer) ---

#[test]
fn respects_configured_devcontainer_shell() {
    // Arrange: 🐳 ウィンドウで exec するシェルを bash に設定
    let env = TestEnv::new();
    env.write_home(
            ".config/wsm/config.toml",
            &format!("{}devcontainer_shell = \"bash\"\n", env.managers_config(&["tmux", "herdr"])),
        )
        .write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner_repo -c ", "")
        .stub("^docker ps -a", "")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("^docker ps -q ", "abc123\n")
        .stub("^docker inspect --format ", "[]\n")
        .stub("^tmux new-window -d -P -F ", "%5\n");
    let cfg = format!("{}/ghq/github.com/owner/repo/.devcontainer/devcontainer.json", env.home_str());

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main", "--config", &cfg]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert!(
        env.invocations().iter().any(|l| l.starts_with("tmux new-window") && l.ends_with("'abc123' bash")),
        "exec shell must follow devcontainer_shell: {:?}",
        env.invocations()
    );
}

#[test]
fn open_with_config_starts_devcontainer_and_adds_window() {
    // Arrange: コンテナは存在しない (before = none) → created
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner_repo -c ", "")
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
            "tmux new-window -d -P -F #{{pane_id}} -t owner_repo: -n 🐳 docker exec -it --user 'dev' -w '/workspaces/ghq/github.com/owner/repo' 'abc123' zsh"
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
        .stub("^tmux new-session -d -s owner_repo_42 -c ", "")
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
    assert_eq!(v["message"], "Opened owner/repo 42 [tmux] + devcontainer(s) [created: default]");
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
        .stub("^tmux has-session -t =owner_repo$", "")
        .stub("^docker ps -a", "running\n")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("^docker ps -q ", "abc123\n")
        .stub("^docker inspect --format ", "[{\"remoteUser\":\"dev\"}]\n")
        .stub("^tmux list-panes -s -t owner_repo -F ", "abc123\n");
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
fn open_with_config_restarts_stopped_container() {
    // Arrange: コンテナは stopped (exited) → started
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner_repo -c ", "")
        .stub("^docker ps -a", "exited\n")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("^docker ps -q ", "abc123\n")
        .stub("^docker inspect --format ", "[{\"remoteUser\":\"dev\"}]\n")
        .stub("^tmux new-window -d -P -F ", "%5\n");
    let cfg = format!("{}/ghq/github.com/owner/repo/.devcontainer/devcontainer.json", env.home_str());

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main", "--config", &cfg]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json()["message"],
        "Opened owner/repo (main) [tmux] + devcontainer(s) [started: repo]"
    );
}

#[test]
fn open_with_multiple_configs_starts_each_container() {
    // Arrange: repo と repo-alt の 2 設定を同時に立てる
    let env = TestEnv::new();
    let home = env.home_str();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .write_home("ghq/github.com/owner/repo/.devcontainer/alt/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner_repo -c ", "")
        .stub("^docker ps -a", "")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("wsm\\.config=repo$", "cid-repo\n")
        .stub("wsm\\.config=repo-alt$", "cid-alt\n")
        .stub("^docker inspect --format ", "[{\"remoteUser\":\"dev\"}]\n")
        .stub("^tmux new-window -d -P -F ", "%5\n");
    let ws = format!("{home}/ghq/github.com/owner/repo");
    let cfg_repo = format!("{ws}/.devcontainer/devcontainer.json");
    let cfg_alt = format!("{ws}/.devcontainer/alt/devcontainer.json");

    // Act
    let out = env.run(&[
        "open", "--repo", "owner/repo", "--issue", "main", "--config", &cfg_repo, "--config", &cfg_alt,
    ]);

    // Assert: 設定ごとにラベル付きで up され、結果が join される
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json()["message"],
        "Opened owner/repo (main) [tmux] + devcontainer(s) [created: repo, created: repo-alt]"
    );
    let invocations = env.invocations();
    assert!(
        invocations.iter().any(|l| {
            l.starts_with("devcontainer up") && l.contains("--config ") && l.ends_with("--id-label wsm.config=repo")
        }),
        "repo config must be brought up with its label: {invocations:?}"
    );
    assert!(
        invocations.iter().any(|l| l.starts_with("devcontainer up") && l.ends_with("--id-label wsm.config=repo-alt")),
        "repo-alt config must be brought up with its label: {invocations:?}"
    );
    assert!(
        invocations.iter().any(|l| l.starts_with("tmux new-window") && l.contains("'cid-repo'")),
        "🐳 window for the repo container: {invocations:?}"
    );
    assert!(
        invocations.iter().any(|l| l.starts_with("tmux new-window") && l.contains("'cid-alt'")),
        "🐳 window for the alt container: {invocations:?}"
    );
}

#[test]
fn devcontainer_window_without_remote_user_omits_user_flag() {
    // Arrange: devcontainer.metadata に remoteUser がない
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner_repo -c ", "")
        .stub("^docker ps -a", "")
        .stub("^devcontainer up --workspace-folder ", "")
        .stub("^docker ps -q ", "abc123\n")
        .stub("^docker inspect --format ", "[]\n")
        .stub("^tmux new-window -d -P -F ", "%5\n");
    let cfg = format!("{}/ghq/github.com/owner/repo/.devcontainer/devcontainer.json", env.home_str());

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main", "--config", &cfg]);

    // Assert: --user なしの正規形 (単一スペース) で exec する
    assert_eq!(out.status, Some(0));
    assert!(
        env.invocations().contains(&format!(
            "tmux new-window -d -P -F #{{pane_id}} -t owner_repo: -n 🐳 docker exec -it -w '/workspaces/ghq/github.com/owner/repo' 'abc123' zsh"
        )),
        "window command must omit --user cleanly: {:?}",
        env.invocations()
    );
}

#[test]
fn open_fails_when_devcontainer_up_fails() {
    // Arrange
    let env = TestEnv::new();
    env.write_home("ghq/github.com/owner/repo/.devcontainer/devcontainer.json", "{}")
        .stub("^tmux new-session -d -s owner_repo -c ", "")
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
    let out = env.run(&["remove", "--repo", "owner/repo", "--issue", "42"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "status": "ok", "message": "Removed worktree and session: owner/repo 42" })
    );
    let invocations = env.invocations();
    assert!(invocations.contains(&"tmux kill-session -t =owner_repo_42".to_owned()));
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
    let out = env.run(&["remove", "--repo", "owner/repo", "--issue", "main"]);

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

// --- エラー契約 (外部コマンド起動失敗) ---
// docs/wsm.md 決定事項「エラーは必ずちょうど 1 つの error JSON で返す」。
// 外部コマンドが起動失敗しても「JSON なしの無言の失敗」はしない: 照会系は
// 取得できなかった部分を除いた結果で成功し、変更系は 1 つの error JSON で失敗する。

#[test]
fn list_repos_returns_empty_array_when_ghq_fails() {
    // Arrange: ghq は未スタブ → 起動失敗相当 (出力なし・exit 1)
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-repos", "--project", "none"]);

    // Assert: 無言の exit 1 ではなく、空配列で成功する
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([]));
    assert_eq!(out.stderr, "");
}

#[test]
fn list_issues_degrades_to_main_when_tracker_fails() {
    // Arrange: gh は未スタブ → 起動失敗相当 (gh 未ログイン等)
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: 取得できなかった Issue を除き、main だけの一覧で成功する
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "main", "title": "main", "active": false, "closed": false, "devcontainer": "none" },
        ])
    );
    assert_eq!(out.stderr, "");
}

#[test]
fn open_fails_with_single_error_json_when_worktree_add_fails() {
    // Arrange: git は未スタブ → show-ref も worktree add も失敗する
    let env = TestEnv::new();

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "42"]);

    // Assert: stderr はちょうど 1 つの error JSON、stdout は空
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr_json(), json!({ "error": "Failed to create worktree" }));
}
