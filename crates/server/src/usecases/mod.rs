//! Usecase 層: ロールを合成して JSON API のユースケースを実装する。
//! ロール間の依存の順序 (worktree → session → devcontainer) と
//! 合成ビュー (active / 孤児 worktree) はここだけが知っている。
//! 入力はドメインの型で受け取る (引数解釈は presentations の責務)。

use crate::infra::settings;
use crate::roles::{devcontainer, repostore, session, tracker, worktree};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use wsm_shared::domains::{self as domain, RepoEntry, RepoRef, WorkspaceId};

pub type CmdResult = Result<Value, String>;

/// パス導出の基点を解決する (コマンド呼び出しごとに 1 回)。
/// クローン本体のパスは RepoStore が解決する (RepoEntry.clone_path)。
fn paths(home: &Path) -> domain::Paths {
    domain::Paths { home: home.to_owned(), worktree_root: settings::worktree_root(home) }
}

/// open な repo-group の一覧 (既定トラッカー)。トラッカー未設定は
/// 対話フローの入り口で設定誤りを表面化させるため、縮退せずエラーにする。
pub fn list_repo_groups(home: &Path) -> CmdResult {
    let trackers = settings::trackers(home)?;
    Ok(Value::Array(tracker::repo_groups(trackers.default_plugin()?)))
}

/// 設定されたトラッカーの一覧と診断 (wsm doctor 用)。設定順で、
/// installed はプラグイン実行ファイルの存在、ready / diagnosis / protocol は
/// info-v0 の自己診断 (非対応なら null)。
pub fn list_trackers(home: &Path) -> CmdResult {
    let trackers = settings::trackers(home)?;
    let default = trackers.default_name().map(str::to_owned);
    Ok(Value::Array(
        trackers
            .entries()
            .map(|(name, path)| {
                let installed = crate::infra::exec::is_executable(path);
                let (ready, diagnosis, protocol) = installed
                    .then(|| tracker::info(path))
                    .flatten()
                    .map(|(ready, diagnosis, protocol)| (Some(ready), diagnosis, protocol))
                    .unwrap_or((None, None, None));
                json!({
                    "name": name,
                    "path": path.to_string_lossy(),
                    "default": Some(name) == default.as_deref(),
                    "installed": installed,
                    "ready": ready,
                    "diagnosis": diagnosis,
                    "protocol": protocol,
                })
            })
            .collect(),
    ))
}

pub fn list_session_managers(home: &Path) -> CmdResult {
    let managers = settings::session_managers(home);
    let default = settings::default_manager_name(home, &managers);
    Ok(Value::Array(
        managers
            .names()
            .into_iter()
            .map(|name| json!({ "name": name, "default": Some(name) == default }))
            .collect(),
    ))
}

pub fn list_repos(home: &Path, group: Option<String>) -> CmdResult {
    let group = group.unwrap_or_default();

    let repos: Vec<RepoEntry> = if group.is_empty() || group == "none" {
        let mut entries = repostore::entries(home)?;
        entries.sort_by_key(|entry| entry.repo.ns_repo());
        entries
    } else {
        // Tracker (repo-group 所属) と RepoStore (ローカルにある) の交差
        let group = validated("group", group, domain::is_valid_group)?;
        let plugin = settings::trackers(home)?.default_plugin()?.to_owned();
        let entries = repostore::entries(home)?;
        tracker::repo_group_repos(&plugin, &group)
            .iter()
            .filter_map(|name| RepoRef::parse(name))
            .filter_map(|repo| entries.iter().find(|entry| entry.repo == repo))
            .cloned()
            .collect()
    };

    let paths = paths(home);
    let managers = settings::session_managers(home);
    Ok(Value::Array(
        repos
            .iter()
            .map(|entry| {
                json!({ "ns_repo": entry.repo.ns_repo(), "active_count": active_count(&paths, entry, &managers) })
            })
            .collect(),
    ))
}

pub fn list_workspaces(home: &Path) -> CmdResult {
    let paths = paths(home);
    let managers = settings::session_managers(home);
    let trackers = settings::trackers(home)?;
    let mut rows = Vec::new();
    for entry in repostore::entries(home)? {
        let repo = &entry.repo;
        let plugin = trackers.plugin_for(entry.tracker.as_deref())?;
        let main_entry = session::workspace_session_exists(repo, &WorkspaceId::Main, &managers)
            .then(|| {
                json!({
                    "ns_repo": repo.ns_repo(), "id": "main", "title": "main",
                    "active": true, "closed": false,
                    "devcontainer": devcontainer::state(repo, "main"),
                })
            });

        let worktree_entries: Vec<Value> = active_issue_ids(&paths, &entry, &managers)
            .into_iter()
            .map(|id| {
                let active = session::workspace_session_exists(
                    repo,
                    &WorkspaceId::Issue(id.clone()),
                    &managers,
                );
                let (title, closed) = plugin
                    .and_then(|bin| tracker::issue(bin, repo, &id))
                    .unwrap_or_else(|| ("unknown".to_owned(), false));
                json!({
                    "ns_repo": repo.ns_repo(), "id": id, "title": title,
                    "active": active, "closed": closed,
                    "devcontainer": devcontainer::state(repo, &id),
                })
            })
            .collect();

        rows.extend(main_entry.into_iter().chain(worktree_entries));
    }
    Ok(Value::Array(rows))
}

pub fn list_issues(home: &Path, repo: &RepoRef) -> CmdResult {
    let entry = repostore::lookup(home, repo)?;
    let paths = paths(home);
    let managers = settings::session_managers(home);
    let plugin = settings::trackers(home)?.plugin_for(entry.tracker.as_deref())?.map(Path::to_owned);
    let main_entry = issue_entry(
        "main",
        "main",
        session::workspace_session_exists(repo, &WorkspaceId::Main, &managers),
        false,
        devcontainer::state(repo, "main"),
    );

    let active_ids = active_issue_ids(&paths, &entry, &managers);
    let open_issues = plugin
        .as_deref()
        .map(|bin| tracker::open_issues(bin, repo))
        .unwrap_or_default();
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
            let title = plugin
                .as_deref()
                .and_then(|bin| tracker::issue(bin, repo, id))
                .map(|(title, _)| title)
                .unwrap_or_else(|| "unknown".to_owned());
            issue_entry(id, &title, true, true, devcontainer::state(repo, id))
        });

    Ok(Value::Array(
        std::iter::once(main_entry).chain(issue_entries).chain(orphan_entries).collect(),
    ))
}

pub fn list_devcontainer_configs(home: &Path, repo: &RepoRef, id: &WorkspaceId) -> CmdResult {
    let entry = repostore::lookup(home, repo)?;
    let paths = paths(home);
    let workspace = domain::workspace_path(&paths, &entry, id);

    let repo_entries = devcontainer::repo_configs(&workspace).into_iter().map(|(name, path)| {
        json!({ "name": name, "path": path.to_string_lossy(), "source": "repo" })
    });
    let fallback_entry = settings::default_devcontainer_config(home)
        .map(|path| json!({ "name": "default", "path": path.to_string_lossy(), "source": "default" }));

    Ok(Value::Array(repo_entries.chain(fallback_entry).collect()))
}

pub fn open(home: &Path, repo: &RepoRef, id: &WorkspaceId, configs: &[String]) -> CmdResult {
    let entry = repostore::lookup(home, repo)?;
    let managers = settings::session_managers(home);
    let manager = settings::session_manager(home, &managers)?;
    let paths = paths(home);
    let workspace = domain::workspace_path(&paths, &entry, id);

    // 依存の順序: worktree (Issue のみ) → session → devcontainer
    if let WorkspaceId::Issue(n) = id {
        if !workspace.is_dir() {
            worktree::add(&entry.clone_path, &domain::branch_name(n), &workspace)?;
        }
    }
    let session = session::ensure(manager, repo, id, &workspace, &entry.clone_path, &managers)?;

    let shell = settings::devcontainer_shell(home);
    let outcomes = configs
        .iter()
        .map(|cfg| {
            let config_path = Path::new(cfg);
            let cname = devcontainer::config_name(&workspace, config_path);
            let outcome =
                devcontainer::up(home, &entry.clone_path, repo, id, &workspace, config_path, &cname)
                    .map_err(|_| format!("devcontainer up failed for {cfg}"))?;
            // 配線: DevContainer が exec コマンドを組み立て、SessionManager が
            // 🐳 ウィンドウを追加する (dedup キーはコンテナ ID)
            if let Some((cid, command)) =
                devcontainer::exec_command(repo, id, &cname, home, &workspace, &shell)
            {
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

    // 破棄は open の逆順: session → devcontainer → worktree。
    // セッションとコンテナの破棄はパスに依存しないため、ストアで解決できない
    // リポジトリ (クローン消失後など) でも掃除できる
    session::remove_workspace_sessions(repo, id, &managers);
    devcontainer::down(repo, id.as_str());

    match id {
        WorkspaceId::Main => Ok(json!({
            "status": "ok",
            "message": format!("Removed session: {}", repo.ns_repo()),
        })),
        WorkspaceId::Issue(issue) => {
            if let Ok(entry) = repostore::lookup(home, repo) {
                worktree::remove(&entry.clone_path, &domain::workspace_path(&paths, &entry, id));
            }
            // トラッカー固有の記法 (# 等) は使わない: <ns_repo> <id> のスペース区切り
            Ok(json!({
                "status": "ok",
                "message": format!("Removed worktree and session: {} {issue}", repo.ns_repo()),
            }))
        }
    }
}

/// 合成ビュー: アクティブな (= 規約パスにあり、セッションが生きている)
/// worktree の Issue id。Worktree ロールと SessionManager ロールの合成。
fn active_issue_ids(
    paths: &domain::Paths,
    entry: &RepoEntry,
    managers: &settings::Managers,
) -> Vec<String> {
    if !entry.clone_path.is_dir() {
        return Vec::new();
    }
    let Some(porcelain) = worktree::list_porcelain(&entry.clone_path) else {
        return Vec::new();
    };
    worktree::parse_feature_worktrees(&porcelain)
        .into_iter()
        .filter(|(path, issue)| {
            let id = WorkspaceId::Issue(issue.clone());
            let expected = domain::workspace_path(paths, entry, &id);
            Path::new(path) == expected
                && expected.is_dir()
                && session::workspace_session_exists(&entry.repo, &id, managers)
        })
        .map(|(_, issue)| issue)
        .collect()
}

/// 合成ビュー: リポジトリ内のアクティブ Workspace 数
/// (アクティブな worktree + main セッションの有無)。
fn active_count(paths: &domain::Paths, entry: &RepoEntry, managers: &settings::Managers) -> usize {
    active_issue_ids(paths, entry, managers).len()
        + usize::from(session::workspace_session_exists(&entry.repo, &WorkspaceId::Main, managers))
}

fn issue_entry(id: &str, title: &str, active: bool, closed: bool, dc: &str) -> Value {
    json!({ "id": id, "title": title, "active": active, "closed": closed, "devcontainer": dc })
}

fn validated(name: &str, value: String, valid: fn(&str) -> bool) -> Result<String, String> {
    valid(&value).then_some(value.clone()).ok_or_else(|| format!("Invalid {name}: {value}"))
}
