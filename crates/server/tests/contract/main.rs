// wsm-server の JSON API 契約テスト (docs/wsm.md が仕様)。
//
// 既定ではビルドしたバイナリを検証する。WSM_SERVER_BIN=<path> で別の
// ビルド (リリースバイナリ等) に差し替えて同じスイートを回せる。
//
// JSON の比較は意味比較 (パース後の等価判定)。整形の差は契約に含めない。

mod harness;

use harness::TestEnv;
use serde_json::json;

const USAGE_ERROR: &str = "Usage: wsm-server <list-repo-groups|list-group-issues|list-repos|list-issues|list-workspaces|list-devcontainer-configs|list-session-managers|list-trackers|open|remove>";

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

// --- list-repo-groups ---

#[test]
fn list_repo_groups_returns_open_groups_as_array() {
    // Arrange
    let env = TestEnv::new();
    env.stub(
        "^tracker list-repo-groups-v0$",
        r#"[{"id":"1","title":"Roadmap"},{"id":"3","title":"Backlog"}]"#,
    );

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert: プラグインの返した順のまま
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "1", "title": "Roadmap", "tracker": "fake" },
            { "id": "3", "title": "Backlog", "tracker": "fake" },
        ])
    );
}

#[test]
fn list_repo_groups_returns_empty_array_when_plugin_fails() {
    // Arrange: プラグインは未スタブ → 起動失敗相当 (認証切れ等)。照会は縮退する
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([]));
}

#[test]
fn list_repo_groups_fails_when_no_tracker_configured() {
    // Arrange: [[tracker]] のない設定。対話フローの入り口なので縮退せず表面化させる
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", &env.managers_config(&["tmux", "herdr"]));

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(
        out.stderr_json(),
        json!({ "error": "no tracker configured (add [[tracker]] to config.toml)" })
    );
}

#[test]
fn list_repo_groups_drops_items_with_invalid_ids() {
    // Arrange: プラグインの出力は信頼しない入力。id の形に違反する要素は捨てる
    let env = TestEnv::new();
    env.stub(
        "^tracker list-repo-groups-v0$",
        r#"[{"id":"1","title":"Ok"},{"id":"../evil","title":"Bad"},{"id":7,"title":"NotString"}]"#,
    );

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "id": "1", "title": "Ok", "tracker": "fake" }]));
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
        // parent: issue と同じ文法。番兵値 main は親になれない
        (&["list-issues", "--repo", "owner/repo", "--parent", "../4"], "Invalid parent: ../4"),
        (&["list-issues", "--repo", "owner/repo", "--parent", "main"], "Invalid parent: main"),
        // cursor: プラグイン発行の不透明な token (base64 系 + URL safe)
        (&["list-issues", "--repo", "owner/repo", "--cursor", "a;b"], "Invalid cursor: a;b"),
        // group: issue と同じ不透明な id の文法
        (&["list-repos", "--group", "5;x"], "Invalid group: 5;x"),
        (&["list-repos", "--group", "-5"], "Invalid group: -5"),
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
    env.stub("^docker ps -a", "").stub("^tracker list-issues-v0 --repo owner/repo$", "[]");

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
    env.stub("^ghq list$", "github.com/owner/.github\n")
        .stub("^docker ps -a", "")
        .stub("^tracker list-issues-v0 --repo owner/.github$", "[]");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/.github"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json()["issues"][0]["id"], "main");
}

#[test]
fn list_repo_groups_preserves_backslashes_in_titles() {
    // Arrange: タイトルにバックスラッシュを含む repo-group (JSON エスケープで保たれる)
    let env = TestEnv::new();
    env.stub("^tracker list-repo-groups-v0$", r#"[{"id":"1","title":"Group \\ A"}]"#);

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "id": "1", "title": "Group \\ A", "tracker": "fake" }]));
}

// --- list-repos ---

#[test]
fn list_repos_lists_all_ghq_repos_when_group_none() {
    // Arrange: host は github.com に限らない (サブグループ等の 4 セグメントは非対応)
    let env = TestEnv::new();
    env.stub(
        "^ghq list$",
        "github.com/owner/tool\ngithub.com/owner/repo\ngitlab.example.com/other/repo\ngitlab.com/group/sub/repo\n",
    );

    // Act
    let out = env.run(&["list-repos", "--group", "none"]);

    // Assert: 全 host をソート済みで列挙。Tracker (gh) には触れない
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "ns_repo": "other/repo", "active_count": 0 },
            { "ns_repo": "owner/repo", "active_count": 0 },
            { "ns_repo": "owner/tool", "active_count": 0 },
        ])
    );
    assert!(
        !env.invocations().iter().any(|l| l.starts_with("tracker ")),
        "group none must not touch the tracker: {:?}",
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
    let out = env.run(&["list-repos", "--group", "none"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "ns_repo": "owner/repo", "active_count": 2 }]));
}

#[test]
fn list_repos_filters_group_repos_by_local_clones() {
    // Arrange: repo-group には 2 リポジトリ、ローカルにあるのは片方だけ
    let env = TestEnv::new();
    env.stub("^tracker repo-group-repos-v0 --group 5$", r#"["owner/repo","owner/other"]"#)
        .stub("^ghq list$", "github.com/owner/repo\ngithub.com/mine/tool\n");

    // Act
    let out = env.run(&["list-repos", "--group", "5"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "ns_repo": "owner/repo", "active_count": 0 }]));
}

// --- RepoStore (host とカスタムリポジトリ) ---
// 識別子は <ns>/<repo> のまま。host は RepoStore が解決するメタ情報で、
// worktree のパス導出 (<worktree_root>/<host>/<ns>/<repo>/<id>) にだけ現れる。

#[test]
fn ghq_repo_on_another_host_uses_host_scoped_paths() {
    // Arrange: ghq に gitlab のクローンだけがある
    let env = TestEnv::new();
    env.stub("^ghq list$", "gitlab.example.com/team/svc\n")
        .stub("worktree add --relative-paths -b feature/7 ", "")
        .stub("^tmux new-session -d -s team_svc_7 -c ", "");
    let home = env.home_str();
    let worktree_path = format!("{home}/worktrees/gitlab.example.com/team/svc/7");

    // Act
    let out = env.run(&["open", "--repo", "team/svc", "--issue", "7"]);

    // Assert: クローンも worktree も host セグメント配下。セッション名に host は含めない
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["session"], "team_svc_7");
    assert_eq!(v["path"], worktree_path.as_str());
    assert!(
        env.invocations().contains(&format!(
            "git -C {home}/ghq/gitlab.example.com/team/svc worktree add --relative-paths -b feature/7 {worktree_path}"
        )),
        "worktree must derive from the entry's host and clone path: {:?}",
        env.invocations()
    );
}

#[test]
fn custom_repo_from_config_opens_with_common_worktree_logic() {
    // Arrange: ghq 管理外のクローン (~/work/aaa) を [[repo]] で登録する。
    // name は省略 (path の basename)。ghq には何もない
    let env = TestEnv::new();
    let config = format!(
        "{}[[repo]]\npath = \"~/work/aaa\"\nhost = \"gitlab.example.com\"\nns = \"myteam\"\n",
        env.managers_config(&["tmux", "herdr"])
    );
    env.write_home(".config/wsm/config.toml", &config)
        .stub("^ghq list$", "")
        .stub("worktree add --relative-paths -b feature/CHH-111 ", "")
        .stub("^tmux new-session -d -s myteam_aaa_CHH-111 -c ", "");
    let home = env.home_str();
    let worktree_path = format!("{home}/worktrees/gitlab.example.com/myteam/aaa/CHH-111");

    // Act
    let out = env.run(&["open", "--repo", "myteam/aaa", "--issue", "CHH-111"]);

    // Assert: クローンは設定の path、worktree は host メタ情報から共通の導出
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v["session"], "myteam_aaa_CHH-111");
    assert_eq!(v["path"], worktree_path.as_str());
    assert!(
        env.invocations().contains(&format!(
            "git -C {home}/work/aaa worktree add --relative-paths -b feature/CHH-111 {worktree_path}"
        )),
        "worktree must be created from the configured clone path: {:?}",
        env.invocations()
    );
}

#[test]
fn custom_repo_appears_in_list_repos() {
    // Arrange: ghq の owner/repo (既定スタブ) + 設定登録の myteam/aaa
    let env = TestEnv::new();
    let config = format!(
        "{}[[repo]]\npath = \"~/work/aaa\"\nhost = \"gitlab.example.com\"\nns = \"myteam\"\n",
        env.managers_config(&["tmux", "herdr"])
    );
    env.write_home(".config/wsm/config.toml", &config);

    // Act
    let out = env.run(&["list-repos", "--group", "none"]);

    // Assert: ソース (ghq / 設定) を問わず同じ一覧に出る
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "ns_repo": "myteam/aaa", "active_count": 0 },
            { "ns_repo": "owner/repo", "active_count": 0 },
        ])
    );
}

#[test]
fn ambiguous_repo_across_hosts_is_an_error() {
    // Arrange: 同じ ns/repo が 2 つの host にある (識別子の一意性規約に違反)
    let env = TestEnv::new();
    env.stub("^ghq list$", "github.com/owner/repo\ngitlab.com/owner/repo\n");

    // Act
    let out = env.run(&["open", "--repo", "owner/repo", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(
        out.stderr_json(),
        json!({ "error": "ambiguous repository: owner/repo (github.com, gitlab.com)" })
    );
}

#[test]
fn open_unknown_repo_fails_with_error_json() {
    // Arrange: ストア (ghq / 設定) のどこにもないリポジトリ
    let env = TestEnv::new();
    env.stub("^ghq list$", "");

    // Act
    let out = env.run(&["open", "--repo", "owner/nope", "--issue", "main"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr_json(), json!({ "error": "repository not found: owner/nope" }));
}

#[test]
fn invalid_repo_entry_in_config_fails_loudly() {
    // Arrange: 必須キー ns のない [[repo]] (設定ミスは黙って捨てない)
    let env = TestEnv::new();
    let config = format!(
        "{}[[repo]]\npath = \"~/work/aaa\"\nhost = \"gitlab.example.com\"\n",
        env.managers_config(&["tmux", "herdr"])
    );
    env.write_home(".config/wsm/config.toml", &config);

    // Act
    let out = env.run(&["list-repos", "--group", "none"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stderr_json(), json!({ "error": "[[repo]] requires ns in config.toml" }));
}

// --- Tracker のインスタンス分割 (拡張キー / ns マッピング / グループ集約) ---

/// 2 インスタンス構成の設定 (fake = 既定、other = tracker2 のフェイク)。
fn two_trackers_config(env: &TestEnv, other_extra: &str) -> String {
    format!(
        "{}{}[[tracker]]\nname = \"other\"\npath = \"{}/tracker2\"\n{}",
        env.managers_config(&["tmux", "herdr"]),
        env.tracker_config(),
        env.fakes_dir_str(),
        other_extra
    )
}

#[test]
fn tracker_extension_keys_are_passed_as_env() {
    // Arrange: 予約キー以外の owner はプラグインに WSM_TRACKER_OWNER として渡る
    let env = TestEnv::new();
    let config = format!(
        "{}{}owner = \"acme-corp\"\n",
        env.managers_config(&["tmux", "herdr"]),
        env.tracker_config()
    );
    env.write_home(".config/wsm/config.toml", &config).stub(
        "^WSM_TRACKER_OWNER=acme-corp tracker list-repo-groups-v0$",
        r#"[{"id":"1","title":"Org Board"}]"#,
    );

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert: env 付きのパターンにだけ一致している = 渡っている
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "id": "1", "title": "Org Board", "tracker": "fake" }]));
}

#[test]
fn ns_mapping_routes_repos_to_their_tracker() {
    // Arrange: owner ns は other トラッカーの世界。明示 (repo.tracker) なしでも
    // ns マッピングで tracker2 に照会が行く
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", &two_trackers_config(&env, "ns = \"owner\"\n"))
        .stub("^docker ps -a", "")
        .stub(
            "^tracker2 list-issues-v2 --repo owner/repo$",
            r#"{"issues":[{"id":"42","title":"Org task"}],"next_cursor":null}"#,
        );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json()["issues"][1]["title"], "Org task");
    assert!(
        !env.invocations().iter().any(|l| l.starts_with("tracker ")),
        "default tracker must not be asked: {:?}",
        env.invocations()
    );
}

#[test]
fn duplicate_ns_mapping_is_a_config_error() {
    // Arrange: 同じ ns を 2 つのトラッカーが主張 (どちらの世界か決められない)
    let env = TestEnv::new();
    let config = format!(
        "{}{}ns = \"acme\"\n[[tracker]]\nname = \"other\"\npath = \"{}/tracker2\"\nns = \"acme\"\n",
        env.managers_config(&["tmux", "herdr"]),
        env.tracker_config(),
        env.fakes_dir_str()
    );
    env.write_home(".config/wsm/config.toml", &config);

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(
        out.stderr_json(),
        json!({ "error": "ns mapped to multiple trackers: acme (fake, other)" })
    );
}

#[test]
fn repo_groups_aggregate_all_trackers_with_default_first() {
    // Arrange: 2 インスタンス。default_tracker で other を既定にする
    // (トップレベルキーはテーブルより前に置く — 後ろに置くとテーブルの
    // 拡張キーとして解釈される)
    let env = TestEnv::new();
    let config = format!(
        "{}default_tracker = \"other\"\n{}[[tracker]]\nname = \"other\"\npath = \"{}/tracker2\"\n",
        env.managers_config(&["tmux", "herdr"]),
        env.tracker_config(),
        env.fakes_dir_str()
    );
    env.write_home(".config/wsm/config.toml", &config)
        .stub("^tracker list-repo-groups-v0$", r#"[{"id":"1","title":"Personal"}]"#)
        .stub("^tracker2 list-repo-groups-v0$", r#"[{"id":"7","title":"Org Board"}]"#);

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert: 既定 (other) のグループが先。各グループが tracker を名乗る
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([
            { "id": "7", "title": "Org Board", "tracker": "other" },
            { "id": "1", "title": "Personal", "tracker": "fake" },
        ])
    );
}

#[test]
fn repo_groups_survive_one_broken_tracker() {
    // Arrange: fake は応答するが other は壊れている (未スタブ → 非ゼロ)。
    // 縮退の単位はトラッカーごと
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", &two_trackers_config(&env, ""))
        .stub("^tracker list-repo-groups-v0$", r#"[{"id":"1","title":"Personal"}]"#);

    // Act
    let out = env.run(&["list-repo-groups"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([{ "id": "1", "title": "Personal", "tracker": "fake" }]));
}

#[test]
fn group_issues_route_to_the_named_tracker() {
    // Arrange: --tracker でグループを持つインスタンスを指定する
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", &two_trackers_config(&env, ""))
        .stub("^docker ps -a", "")
        .stub(
            "^tracker2 list-group-issues-v0 --group 7$",
            r#"{"issues":[{"id":"9","title":"Org task","repo":"acme/api"}],"next_cursor":null}"#,
        );

    // Act
    let out = env.run(&["list-group-issues", "--group", "7", "--tracker", "other"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json()["issues"][0]["repo"], "acme/api");
    assert!(
        !env.invocations().iter().any(|l| l.starts_with("tracker ")),
        "only the named tracker must be asked: {:?}",
        env.invocations()
    );
}

// --- Tracker プラグインの選択と検証 ---

#[test]
fn list_issues_degrades_to_main_when_no_tracker_configured() {
    // Arrange: [[tracker]] のない設定。リポジトリ単位の照会は縮退する
    // (プロジェクト照会と違い、Tracker なしでも main の開閉はできるべき)
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", &env.managers_config(&["tmux", "herdr"]))
        .stub("^docker ps -a", "");

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
    );
}

#[test]
fn repo_with_unknown_tracker_name_is_a_config_error() {
    // Arrange: [[repo]] が列挙にないトラッカー名を指す (設定誤りは表面化させる)
    let env = TestEnv::new();
    let config = format!(
        "{}{}[[repo]]\npath = \"~/work/aaa\"\nhost = \"gitlab.example.com\"\nns = \"myteam\"\ntracker = \"jira\"\n",
        env.managers_config(&["tmux", "herdr"]),
        env.tracker_config()
    );
    env.write_home(".config/wsm/config.toml", &config).stub("^ghq list$", "");

    // Act
    let out = env.run(&["list-issues", "--repo", "myteam/aaa"]);

    // Assert
    assert_eq!(out.status, Some(1));
    assert_eq!(out.stderr_json(), json!({ "error": "tracker not configured: jira" }));
}

#[test]
fn list_trackers_reports_installed_and_ready_state() {
    // Arrange: フェイクの tracker は実在する実行ファイル。info-v0 が自己診断を返す
    let env = TestEnv::new();
    env.stub(
        "^tracker info-v0$",
        r#"{"name":"fake","ready":true,"protocol":["list-issues-v0","info-v0"]}"#,
    );

    // Act
    let out = env.run(&["list-trackers"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!([{
            "name": "fake",
            "path": format!("{}/tracker", env.fakes_dir_str()),
            "default": true,
            "installed": true,
            "ready": true,
            "diagnosis": null,
            "protocol": ["list-issues-v0", "info-v0"],
        }])
    );
}

#[test]
fn list_trackers_reports_unready_with_diagnosis() {
    // Arrange: インストール済みだが使えない (認証切れ等)。診断が透過する
    let env = TestEnv::new();
    env.stub(
        "^tracker info-v0$",
        r#"{"ready":false,"diagnosis":"gh: missing scope read:project"}"#,
    );

    // Act
    let out = env.run(&["list-trackers"]);

    // Assert
    assert_eq!(out.status, Some(0));
    let v = &out.stdout_json()[0];
    assert_eq!(v["installed"], true);
    assert_eq!(v["ready"], false);
    assert_eq!(v["diagnosis"], "gh: missing scope read:project");
}

#[test]
fn list_trackers_reports_missing_binary_and_info_unsupported() {
    // Arrange: 実在しないパスのトラッカーと、info-v0 非対応 (未スタブ → 非ゼロ) の
    // トラッカーを列挙する
    let env = TestEnv::new();
    let config = format!(
        "{}{}[[tracker]]\nname = \"jira\"\npath = \"~/no/such/plugin\"\n",
        env.managers_config(&["tmux", "herdr"]),
        env.tracker_config()
    );
    env.write_home(".config/wsm/config.toml", &config);

    // Act
    let out = env.run(&["list-trackers"]);

    // Assert: info 非対応は ready 不明 (null)、実在しないものは installed:false
    assert_eq!(out.status, Some(0));
    let v = out.stdout_json();
    assert_eq!(v[0]["name"], "fake");
    assert_eq!(v[0]["installed"], true);
    assert_eq!(v[0]["ready"], serde_json::Value::Null);
    assert_eq!(v[1]["name"], "jira");
    assert_eq!(v[1]["installed"], false);
    assert_eq!(v[1]["ready"], serde_json::Value::Null);
}

#[test]
fn list_trackers_returns_empty_array_when_none_configured() {
    // Arrange: [[tracker]] のない設定 (診断コマンドなのでエラーにしない)
    let env = TestEnv::new();
    env.write_home(".config/wsm/config.toml", &env.managers_config(&["tmux", "herdr"]));

    // Act
    let out = env.run(&["list-trackers"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([]));
}

#[test]
fn invalid_issue_ids_from_plugin_are_dropped() {
    // Arrange: プラグインの出力は信頼しない入力。id の文法違反と番兵値 main は捨てる
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub(
        "^tracker list-issues-v1 --repo owner/repo$",
        r#"[{"id":"42","title":"Ok"},{"id":"../evil","title":"Bad"},{"id":"main","title":"Sentinel"}]"#,
    );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "42", "title": "Ok", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
    );
}

// --- list-workspaces ---

#[test]
fn list_workspaces_returns_empty_array_when_none_active() {
    // Arrange
    let env = TestEnv::new();
    env.stub("^ghq list$", "github.com/owner/repo\n");

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
        .stub("^ghq list$", "github.com/owner/repo\n")
        .stub(
            "worktree list --porcelain",
            &format!(
                "worktree {home}/worktrees/github.com/owner/repo/42\nHEAD bbb\nbranch refs/heads/feature/42\n\n"
            ),
        )
        .stub("^tmux has-session -t =owner_repo$", "")
        .stub("^tmux has-session -t =owner_repo_42$", "")
        .stub("^docker ps -a", "")
        .stub(
            "^tracker issue-v0 --repo owner/repo --id 42$",
            r#"{"title":"Fix bug","state":"closed"}"#,
        );

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
    // Arrange: 階層対応プラグイン (list-issues-v1)。42 は子 Issue を持つ
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub(
        "^tracker list-issues-v1 --repo owner/repo$",
        r#"[{"id":"42","title":"Fix bug","has_children":true},{"id":"43","title":"Add feature"}]"#,
    );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: has_children はそのまま透過し、省略は false に補われる
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "42", "title": "Fix bug", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": true },
            { "id": "43", "title": "Add feature", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
    );
}

#[test]
fn list_issues_falls_back_to_v0_for_legacy_plugins() {
    // Arrange: v1 を知らないプラグイン (未知の動詞 → 非ゼロ)。v0 に落ちて
    // has_children は false に補われる
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub(
        "^tracker list-issues-v0 --repo owner/repo$",
        r#"[{"id":"42","title":"Fix bug"}]"#,
    );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "42", "title": "Fix bug", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
    );
    let invocations = env.invocations();
    assert!(
        invocations.contains(&"tracker list-issues-v1 --repo owner/repo".to_owned()),
        "v1 must be tried first: {invocations:?}"
    );
}

#[test]
fn cross_repo_children_carry_their_home_repo() {
    // Arrange: sub-issues はリポジトリ横断で張れる。よその子は repo 付きで返り、
    // セッション・コンテナはその repo の文脈で見る
    let env = TestEnv::new();
    env.stub("^docker ps -a --filter label=wsm.ns-repo=owner/repo ", "")
        .stub("^docker ps -a --filter label=wsm.ns-repo=owner/lib ", "running\n")
        .stub("^tmux has-session -t =owner_lib_9$", "")
        .stub(
            "^tracker list-issues-v2 --repo owner/repo --parent 42$",
            r#"{"issues":[{"id":"421","title":"Same repo"},{"id":"9","title":"In lib","repo":"owner/lib"},{"id":"7","title":"Bad repo","repo":"owner//"}],"next_cursor":null}"#,
        );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo", "--parent", "42"]);

    // Assert: repo 省略は照会リポジトリに補われ、形の不正な repo は要素ごと捨てる
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "421", "title": "Same repo", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "9", "title": "In lib", "repo": "owner/lib", "active": true, "closed": false, "devcontainer": "running", "has_children": false },
        ], "next_cursor": null })
    );
}

#[test]
fn list_group_issues_spans_repositories() {
    // Arrange: repo-group 横断の Issue 一覧 (Issue 起点フローのトップレベル)。
    // 各要素の repo は必須で、欠落した要素は捨てる
    let env = TestEnv::new();
    env.stub("^docker ps -a", "")
        .stub("^tmux has-session -t =owner_lib_9$", "")
        .stub(
            "^tracker list-group-issues-v0 --group 5$",
            r#"{"issues":[{"id":"42","title":"App task","repo":"owner/repo","has_children":true},{"id":"9","title":"Lib task","repo":"owner/lib"},{"id":"1","title":"No repo"}],"next_cursor":"page2=="}"#,
        );

    // Act
    let out = env.run(&["list-group-issues", "--group", "5"]);

    // Assert: main・孤児はリポジトリ単位の概念なので出ない
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "42", "title": "App task", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": true },
            { "id": "9", "title": "Lib task", "repo": "owner/lib", "active": true, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": "page2==" })
    );
}

#[test]
fn list_group_issues_degrades_to_empty_for_legacy_plugins() {
    // Arrange: list-group-issues-v0 を知らないプラグイン (未知の動詞 → 非ゼロ)。
    // UI はこれを見て従来のリポジトリ起点フローに落ちる
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-group-issues", "--group", "5"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!({ "issues": [], "next_cursor": null }));
}

#[test]
fn list_issues_pages_through_v2_plugins() {
    // Arrange: v2 対応プラグイン。1 ページ目に続きの cursor がある
    let env = TestEnv::new();
    env.stub("^docker ps -a", "")
        .stub(
            "^tracker list-issues-v2 --repo owner/repo$",
            r#"{"issues":[{"id":"42","title":"Newest"}],"next_cursor":"abc=="}"#,
        )
        .stub(
            "^tracker list-issues-v2 --repo owner/repo --cursor abc==$",
            r#"{"issues":[{"id":"41","title":"Older"}],"next_cursor":null}"#,
        );

    // Act
    let first = env.run(&["list-issues", "--repo", "owner/repo"]);
    let second = env.run(&["list-issues", "--repo", "owner/repo", "--cursor", "abc=="]);

    // Assert: 続きのページには main も孤児も出ない
    assert_eq!(first.status, Some(0));
    assert_eq!(
        first.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "42", "title": "Newest", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": "abc==" })
    );
    assert_eq!(second.status, Some(0));
    assert_eq!(
        second.stdout_json(),
        json!({ "issues": [
            { "id": "41", "title": "Older", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
    );
}

#[test]
fn invalid_cursor_from_plugin_is_dropped() {
    // Arrange: プラグインの出力は信頼しない入力。cursor も形を検証し、
    // 不正なら「続きなし」に落とす (引数へ還流するため)
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub(
        "^tracker list-issues-v2 --repo owner/repo$",
        r#"{"issues":[],"next_cursor":"bad cursor; rm"}"#,
    );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json()["next_cursor"], serde_json::Value::Null);
}

#[test]
fn list_issues_with_parent_lists_children_only() {
    // Arrange: 階層のドリルダウン。main と孤児はトップレベル専用
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub(
        "^tracker list-issues-v1 --repo owner/repo --parent 42$",
        r#"[{"id":"421","title":"Child A","has_children":true},{"id":"422","title":"Child B"}]"#,
    );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo", "--parent", "42"]);

    // Assert: main 行はなく、子 Issue だけが返る
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "421", "title": "Child A", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": true },
            { "id": "422", "title": "Child B", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
    );
}

#[test]
fn list_issues_shows_orphaned_worktrees_with_real_state_in_worktree_order() {
    // Arrange: Issue 41, 42 はトップレベルの open 一覧に出てこないが、
    // worktree とセッションが残っている。41 は closed、42 は open な子 Issue
    // (階層化により「一覧にない = closed」とは限らなくなった)
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
        .stub("^tracker list-issues-v1 --repo owner/repo$", r#"[{"id":"43","title":"Other work"}]"#)
        .stub("^tracker issue-v0 --repo owner/repo --id 41$", r#"{"title":"Old bug","state":"closed"}"#)
        .stub("^tracker issue-v0 --repo owner/repo --id 42$", r#"{"title":"Deep child","state":"open"}"#);

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: 孤児は active:true・closed は Tracker の実際の state で、
    // worktree 一覧順に並ぶ
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "43", "title": "Other work", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "41", "title": "Old bug", "repo": "owner/repo", "active": true, "closed": true, "devcontainer": "none", "has_children": false },
            { "id": "42", "title": "Deep child", "repo": "owner/repo", "active": true, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
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
        .stub(
            "^tracker list-issues-v1 --repo owner/repo$",
            r#"[{"id":"42","title":"Fix bug"},{"id":"43","title":"Add feature"}]"#,
        );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: 1 つでも running があれば running、行はあるが running がなければ stopped
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "42", "title": "Fix bug", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "stopped", "has_children": false },
            { "id": "43", "title": "Add feature", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "running", "has_children": false },
        ], "next_cursor": null })
    );
}

#[test]
fn list_issues_preserves_backslash_sequences_in_titles() {
    // Arrange: タイトルに literal な \t (バックスラッシュ + t) を含む Issue
    let env = TestEnv::new();
    env.stub("^docker ps -a", "").stub(
        "^tracker list-issues-v1 --repo owner/repo$",
        r#"[{"id":"42","title":"Keep \\t literal"}]"#,
    );

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json()["issues"][1],
        json!({ "id": "42", "title": "Keep \\t literal", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false })
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
        .stub("^tracker list-issues-v0 --repo owner/repo$", r#"[{"id":"42","title":"Fix bug"}]"#);

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: main はセッション running で active、42 は workspace 存在で active
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": true, "closed": false, "devcontainer": "none", "has_children": false },
            { "id": "42", "title": "Fix bug", "repo": "owner/repo", "active": true, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
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
    // Arrange: ghq list が起動失敗相当 (出力なし・exit 1)
    let env = TestEnv::new();
    env.stub_exit("^ghq list$", "", 1);

    // Act
    let out = env.run(&["list-repos", "--group", "none"]);

    // Assert: 無言の exit 1 ではなく、空配列で成功する
    assert_eq!(out.status, Some(0));
    assert_eq!(out.stdout_json(), json!([]));
    assert_eq!(out.stderr, "");
}

#[test]
fn list_issues_degrades_to_main_when_tracker_fails() {
    // Arrange: プラグインは未スタブ → 起動失敗相当 (認証切れ等)
    let env = TestEnv::new();

    // Act
    let out = env.run(&["list-issues", "--repo", "owner/repo"]);

    // Assert: 取得できなかった Issue を除き、main だけの一覧で成功する
    assert_eq!(out.status, Some(0));
    assert_eq!(
        out.stdout_json(),
        json!({ "issues": [
            { "id": "main", "title": "main", "repo": "owner/repo", "active": false, "closed": false, "devcontainer": "none", "has_children": false },
        ], "next_cursor": null })
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
