//! Usecase 層: ロールを合成して JSON API のユースケースを実装する。
//! ロール間の依存の順序 (worktree → session → devcontainer) と
//! 合成ビュー (active / 孤児 worktree) はここだけが知っている。
//! 入力はドメインの型で受け取る (引数解釈は presentations の責務)。

use crate::infra::settings;
use crate::roles::{devcontainer, repostore, session, tracker, worktree};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use wsm_shared::domains::{self as domain, RepoRef, WorkspaceId};

pub type CmdResult = Result<Value, String>;

/// パス導出の基点を解決する (コマンド呼び出しごとに 1 回)。
/// ghq のルートは RepoStore ロールに聞く (`ghq root` 尊重)。
fn paths(home: &Path) -> domain::Paths {
    domain::Paths {
        home: home.to_owned(),
        ghq_root: repostore::root(home),
        worktree_root: settings::worktree_root(home),
    }
}

/// user は未指定なら Tracker で自己解決する (解決した値も検証する)。
pub fn list_projects(user: Option<String>) -> CmdResult {
    let user = resolved_user(user)?;
    Ok(Value::Array(tracker::open_projects(&user)))
}

pub fn list_session_managers(home: &Path) -> CmdResult {
    let managers = settings::session_managers(home);
    Ok(Value::Array(managers.names().into_iter().map(|name| json!({ "name": name })).collect()))
}

pub fn list_repos(home: &Path, project: Option<String>, user: Option<String>) -> CmdResult {
    let project = project.unwrap_or_default();

    let repos: Vec<RepoRef> = if project.is_empty() || project == "none" {
        repostore::list()
    } else {
        // Tracker (Project 所属) と RepoStore (ローカルにある) の交差
        let user = resolved_user(user)?;
        let project = validated("project", project, domain::is_valid_project)?;
        let local: HashSet<RepoRef> = repostore::list().into_iter().collect();
        tracker::project_repos(&user, &project)
            .iter()
            .filter_map(|name| RepoRef::parse(name))
            .filter(|repo| local.contains(repo))
            .collect()
    };

    let paths = paths(home);
    let managers = settings::session_managers(home);
    Ok(Value::Array(
        repos
            .iter()
            .map(|repo| {
                json!({ "ns_repo": repo.ns_repo(), "active_count": active_count(&paths, repo, &managers) })
            })
            .collect(),
    ))
}

pub fn list_workspaces(home: &Path) -> CmdResult {
    let paths = paths(home);
    let managers = settings::session_managers(home);
    let entries = repostore::list_in_ghq_order()
        .into_iter()
        .flat_map(|repo| {
            let main_entry = session::workspace_session_exists(&repo, &WorkspaceId::Main, &managers)
                .then(|| {
                json!({
                    "ns_repo": repo.ns_repo(), "id": "main", "title": "main",
                    "active": true, "closed": false,
                    "devcontainer": devcontainer::state(&repo, "main"),
                })
            });

            let worktree_entries: Vec<Value> = active_issue_ids(&paths, &repo, &managers)
                .into_iter()
                .map(|id| {
                    let active = session::workspace_session_exists(
                        &repo,
                        &WorkspaceId::Issue(id.clone()),
                        &managers,
                    );
                    let (title, closed) = tracker::issue_title_and_state(&repo, &id)
                        .map(|(title, state)| (title, state == "CLOSED"))
                        .unwrap_or_else(|| ("unknown".to_owned(), false));
                    json!({
                        "ns_repo": repo.ns_repo(), "id": id, "title": title,
                        "active": active, "closed": closed,
                        "devcontainer": devcontainer::state(&repo, &id),
                    })
                })
                .collect();

            main_entry.into_iter().chain(worktree_entries).collect::<Vec<_>>()
        })
        .collect();
    Ok(Value::Array(entries))
}

pub fn list_issues(home: &Path, repo: &RepoRef) -> CmdResult {
    let paths = paths(home);
    let managers = settings::session_managers(home);
    let main_entry = issue_entry(
        "main",
        "main",
        session::workspace_session_exists(repo, &WorkspaceId::Main, &managers),
        false,
        devcontainer::state(repo, "main"),
    );

    let active_ids = active_issue_ids(&paths, repo, &managers);
    let open_issues = tracker::open_issues(repo);
    let open_ids: HashSet<&str> = open_issues.iter().map(|(id, _)| id.as_str()).collect();

    let issue_entries = open_issues.iter().map(|(id, title)| {
        issue_entry(
            id,
            title,
            active_ids.iter().any(|active| active == id),
            false,
            devcontainer::state(repo, id),
        )
    });

    // 孤児 worktree: Tracker 上は open でないがセッションが残っている Issue
    let orphan_entries = active_ids
        .iter()
        .filter(|id| !open_ids.contains(id.as_str()))
        .map(|id| {
            let title = tracker::issue_title(repo, id).unwrap_or_else(|| "unknown".to_owned());
            issue_entry(id, &title, true, true, devcontainer::state(repo, id))
        });

    Ok(Value::Array(
        std::iter::once(main_entry).chain(issue_entries).chain(orphan_entries).collect(),
    ))
}

pub fn list_devcontainer_configs(home: &Path, repo: &RepoRef, id: &WorkspaceId) -> CmdResult {
    let paths = paths(home);
    let workspace = domain::workspace_path(&paths, repo, id);

    let repo_entries = devcontainer::repo_configs(&workspace).into_iter().map(|(name, path)| {
        json!({ "name": name, "path": path.to_string_lossy(), "source": "repo" })
    });
    let fallback_entry = settings::default_devcontainer_config(home)
        .map(|path| json!({ "name": "default", "path": path.to_string_lossy(), "source": "default" }));

    Ok(Value::Array(repo_entries.chain(fallback_entry).collect()))
}

pub fn open(home: &Path, repo: &RepoRef, id: &WorkspaceId, configs: &[String]) -> CmdResult {
    let managers = settings::session_managers(home);
    let manager = settings::session_manager(&managers)?;
    let paths = paths(home);
    let workspace = domain::workspace_path(&paths, repo, id);

    // 依存の順序: worktree (Issue のみ) → session → devcontainer
    if let WorkspaceId::Issue(n) = id {
        if !workspace.is_dir() {
            worktree::add(&domain::ghq_path(&paths, repo), &domain::branch_name(n), &workspace)?;
        }
    }
    let session = session::ensure(manager, repo, id, &workspace, &paths, &managers)?;

    let shell = settings::devcontainer_shell(home);
    let outcomes = configs
        .iter()
        .map(|cfg| {
            let config_path = Path::new(cfg);
            let cname = devcontainer::config_name(&workspace, config_path);
            let outcome = devcontainer::up(&paths, repo, id, &workspace, config_path, &cname)
                .map_err(|_| format!("devcontainer up failed for {cfg}"))?;
            // 配線: DevContainer が exec コマンドを組み立て、SessionManager が
            // 🐳 ウィンドウを追加する (dedup キーはコンテナ ID)
            if let Some((cid, command)) = devcontainer::exec_command(repo, id, &cname, &paths, &shell) {
                session::add_window(manager, &session, "🐳", &command, &cid, &managers);
            }
            Ok(format!("{}: {cname}", outcome.label()))
        })
        .collect::<Result<Vec<String>, String>>()?;

    // トラッカー固有の記法 (# 等) は使わない: <ns_repo> <id> のスペース区切り
    let ns_repo = repo.ns_repo();
    let base_message = match id {
        WorkspaceId::Main => format!("Opened {ns_repo} (main) [{}]", manager.name()),
        WorkspaceId::Issue(n) => format!("Opened {ns_repo} {n} [{}]", manager.name()),
    };
    let message = match outcomes.is_empty() {
        true => base_message,
        false => format!("{base_message} + devcontainer(s) [{}]", outcomes.join(", ")),
    };
    Ok(json!({
        "status": "ok",
        "message": message,
        "session": session,
        "path": workspace.to_string_lossy(),
        "attach_command": session::attach_command(manager, &session, &workspace, &managers),
    }))
}

pub fn remove(home: &Path, repo: &RepoRef, id: &WorkspaceId) -> CmdResult {
    let paths = paths(home);
    let managers = settings::session_managers(home);

    // herdr のセッションは Issue workspace の器なので、残存中は main を消せない
    if *id == WorkspaceId::Main && session::herdr_blocks_main_removal(repo, &managers) {
        return Err(format!(
            "herdr session has open issue workspaces: {}",
            domain::herdr_session_name(repo)
        ));
    }

    // 破棄は open の逆順: session → devcontainer → worktree
    session::remove_workspace_sessions(repo, id, &managers);
    devcontainer::down(repo, id.as_str());

    match id {
        WorkspaceId::Main => Ok(json!({
            "status": "ok",
            "message": format!("Removed session: {}", repo.ns_repo()),
        })),
        WorkspaceId::Issue(issue) => {
            worktree::remove(&domain::ghq_path(&paths, repo), &domain::workspace_path(&paths, repo, id));
            // トラッカー固有の記法 (# 等) は使わない: <ns_repo> <id> のスペース区切り
            Ok(json!({
                "status": "ok",
                "message": format!("Removed worktree and session: {} {issue}", repo.ns_repo()),
            }))
        }
    }
}

/// 合成ビュー: アクティブな (= 規約パスにあり、セッションが生きている)
/// worktree の Issue 番号。Worktree ロールと SessionManager ロールの合成。
fn active_issue_ids(
    paths: &domain::Paths,
    repo: &RepoRef,
    managers: &settings::Managers,
) -> Vec<String> {
    let ghq = domain::ghq_path(paths, repo);
    if !ghq.is_dir() {
        return Vec::new();
    }
    let Some(porcelain) = worktree::list_porcelain(&ghq) else {
        return Vec::new();
    };
    worktree::parse_feature_worktrees(&porcelain)
        .into_iter()
        .filter(|(path, issue)| {
            let id = WorkspaceId::Issue(issue.clone());
            let expected = domain::workspace_path(paths, repo, &id);
            Path::new(path) == expected
                && expected.is_dir()
                && session::workspace_session_exists(repo, &id, managers)
        })
        .map(|(_, issue)| issue)
        .collect()
}

/// 合成ビュー: リポジトリ内のアクティブ Workspace 数
/// (アクティブな worktree + main セッションの有無)。
fn active_count(paths: &domain::Paths, repo: &RepoRef, managers: &settings::Managers) -> usize {
    active_issue_ids(paths, repo, managers).len()
        + usize::from(session::workspace_session_exists(repo, &WorkspaceId::Main, managers))
}

fn issue_entry(id: &str, title: &str, active: bool, closed: bool, dc: &str) -> Value {
    json!({ "id": id, "title": title, "active": active, "closed": closed, "devcontainer": dc })
}

/// --user 未指定時の自己解決。解決した値にも同じ検証をかける。
fn resolved_user(user: Option<String>) -> Result<String, String> {
    let user = match user {
        Some(user) => user,
        None => tracker::resolve_user().ok_or("failed to resolve GitHub user")?,
    };
    validated("user", user, domain::is_valid_user)
}

fn validated(name: &str, value: String, valid: fn(&str) -> bool) -> Result<String, String> {
    valid(&value).then_some(value.clone()).ok_or_else(|| format!("Invalid {name}: {value}"))
}
