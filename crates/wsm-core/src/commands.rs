//! オーケストレーション層: ロールを合成してサブコマンドを実装する。
//! ロール間の依存の順序 (worktree → session → devcontainer) と
//! 合成ビュー (active / 孤児 worktree) はここだけが知っている。

use crate::domain::{self, WorkspaceId};
use crate::roles::{devcontainer, repostore, session, tracker, worktree};
use crate::settings;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub type CmdResult = Result<Value, String>;

pub fn list_projects(args: &[String]) -> CmdResult {
    let user = match flag_value(args, "--user") {
        Some(user) => user,
        None => tracker::resolve_user().ok_or("failed to resolve GitHub user")?,
    };
    let user = validated("user", user, domain::is_valid_user)?;
    Ok(Value::Array(tracker::open_projects(&user)))
}

pub fn list_repos(home: &Path, args: &[String]) -> CmdResult {
    let project = flag_value(args, "--project").unwrap_or_default();

    let repos: Vec<String> = if project.is_empty() || project == "none" {
        repostore::list_ns_repos()
    } else {
        // Tracker (Project 所属) と RepoStore (ローカルにある) の交差
        let user = match flag_value(args, "--user") {
            Some(user) => user,
            None => tracker::resolve_user().ok_or("failed to resolve GitHub user")?,
        };
        let user = validated("user", user, domain::is_valid_user)?;
        let project = validated("project", project, domain::is_valid_project)?;
        let local: HashSet<String> = repostore::list_ns_repos().into_iter().collect();
        tracker::project_repos(&user, &project)
            .into_iter()
            .filter(|repo| local.contains(repo))
            .collect()
    };

    Ok(Value::Array(
        repos
            .iter()
            .map(|ns_repo| json!({ "ns_repo": ns_repo, "active_count": active_count(home, ns_repo) }))
            .collect(),
    ))
}

pub fn list_workspaces(home: &Path) -> CmdResult {
    let entries = repostore::list_ns_repos_in_ghq_order()
        .into_iter()
        .flat_map(|ns_repo| {
            let main_entry = session::workspace_session_exists(&ns_repo, &WorkspaceId::Main)
                .then(|| {
                    json!({
                        "ns_repo": ns_repo, "id": "main", "title": "main",
                        "active": true, "closed": false,
                        "devcontainer": devcontainer::state(&ns_repo, "main"),
                    })
                });

            let worktree_entries: Vec<Value> = active_issue_ids(home, &ns_repo)
                .into_iter()
                .map(|id| {
                    let active =
                        session::workspace_session_exists(&ns_repo, &WorkspaceId::Issue(id.clone()));
                    let (title, closed) = tracker::issue_title_and_state(&ns_repo, &id)
                        .map(|(title, state)| (title, state == "CLOSED"))
                        .unwrap_or_else(|| ("unknown".to_owned(), false));
                    json!({
                        "ns_repo": ns_repo, "id": id, "title": title,
                        "active": active, "closed": closed,
                        "devcontainer": devcontainer::state(&ns_repo, &id),
                    })
                })
                .collect();

            main_entry.into_iter().chain(worktree_entries).collect::<Vec<_>>()
        })
        .collect();
    Ok(Value::Array(entries))
}

pub fn list_issues(home: &Path, args: &[String]) -> CmdResult {
    let ns_repo = required(args, "--repo", "repo", domain::is_valid_repo)?;

    let main_entry = issue_entry(
        "main",
        "main",
        session::workspace_session_exists(&ns_repo, &WorkspaceId::Main),
        false,
        devcontainer::state(&ns_repo, "main"),
    );

    let active_ids = active_issue_ids(home, &ns_repo);
    let open_issues = tracker::open_issues(&ns_repo);
    let open_ids: HashSet<&str> = open_issues.iter().map(|(id, _)| id.as_str()).collect();

    let issue_entries = open_issues.iter().map(|(id, title)| {
        issue_entry(
            id,
            title,
            active_ids.iter().any(|active| active == id),
            false,
            devcontainer::state(&ns_repo, id),
        )
    });

    // 孤児 worktree: Tracker 上は open でないがセッションが残っている Issue
    let orphan_entries = active_ids
        .iter()
        .filter(|id| !open_ids.contains(id.as_str()))
        .map(|id| {
            let title = tracker::issue_title(&ns_repo, id).unwrap_or_else(|| "unknown".to_owned());
            issue_entry(id, &title, true, true, devcontainer::state(&ns_repo, id))
        });

    Ok(Value::Array(
        std::iter::once(main_entry).chain(issue_entries).chain(orphan_entries).collect(),
    ))
}

pub fn list_devcontainer_configs(home: &Path, args: &[String]) -> CmdResult {
    let ns_repo = required(args, "--repo", "repo", domain::is_valid_repo)?;
    let issue = required(args, "--issue", "issue", domain::is_valid_issue)?;
    let workspace = domain::workspace_path(home, &ns_repo, &WorkspaceId::parse(&issue));

    let repo_entries = devcontainer::repo_configs(&workspace).into_iter().map(|(name, path)| {
        json!({ "name": name, "path": path.to_string_lossy(), "source": "repo" })
    });
    let fallback_entry = settings::default_devcontainer_config(home)
        .map(|path| json!({ "name": "default", "path": path.to_string_lossy(), "source": "default" }));

    Ok(Value::Array(repo_entries.chain(fallback_entry).collect()))
}

pub fn open(home: &Path, args: &[String]) -> CmdResult {
    let ns_repo = required(args, "--repo", "repo", domain::is_valid_repo)?;
    let issue = required(args, "--issue", "issue", domain::is_valid_issue)?;
    let configs = flag_values(args, "--config");

    let id = WorkspaceId::parse(&issue);
    let manager = settings::session_manager(home)?;
    let workspace = domain::workspace_path(home, &ns_repo, &id);

    // 依存の順序: worktree (Issue のみ) → session → devcontainer
    if let WorkspaceId::Issue(n) = &id {
        if !workspace.is_dir() {
            worktree::add(&domain::ghq_path(home, &ns_repo), &domain::branch_name(n), &workspace)?;
        }
    }
    let session = session::ensure(manager, &ns_repo, &id, &workspace)?;

    let outcomes = configs
        .iter()
        .map(|cfg| {
            let config_path = Path::new(cfg);
            let cname = devcontainer::config_name(&workspace, config_path);
            let outcome = devcontainer::up(home, &ns_repo, &id, &workspace, config_path, &cname)
                .map_err(|_| format!("devcontainer up failed for {cfg}"))?;
            // 配線: DevContainer が exec コマンドを組み立て、SessionManager が
            // 🐳 ウィンドウを追加する (dedup キーはコンテナ ID)
            if let Some((cid, command)) = devcontainer::exec_command(&ns_repo, &id, &cname) {
                session::add_window(manager, &session, "🐳", &command, &cid);
            }
            Ok(format!("{}: {cname}", outcome.label()))
        })
        .collect::<Result<Vec<String>, String>>()?;

    let base_message = match &id {
        WorkspaceId::Main => format!("Opened {ns_repo} (main) [{}]", manager.name()),
        WorkspaceId::Issue(n) => format!("Opened {ns_repo} #{n} [{}]", manager.name()),
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
        "attach_command": session::attach_command(manager, &session, &workspace, home),
    }))
}

pub fn remove(home: &Path, args: &[String]) -> CmdResult {
    let ns_repo = required(args, "--repo", "repo", domain::is_valid_repo)?;
    let issue = required(args, "--issue", "issue", domain::is_valid_issue)?;
    let id = WorkspaceId::parse(&issue);
    let session = domain::session_name(&ns_repo, &id);

    // herdr のセッションは Issue workspace の器なので、残存中は main を消せない
    if id == WorkspaceId::Main && session::herdr_blocks_main_removal(&ns_repo) {
        return Err(format!(
            "herdr session has open issue workspaces: {}",
            domain::session_name(&ns_repo, &WorkspaceId::Main)
        ));
    }

    // 破棄は open の逆順: session → devcontainer → worktree
    session::remove_workspace_sessions(&ns_repo, &id);
    devcontainer::down(&ns_repo, id.as_str());

    match &id {
        WorkspaceId::Main => Ok(json!({
            "status": "ok",
            "message": format!("Removed session: {ns_repo}"),
        })),
        WorkspaceId::Issue(_) => {
            worktree::remove(
                &domain::ghq_path(home, &ns_repo),
                &domain::workspace_path(home, &ns_repo, &id),
            );
            Ok(json!({
                "status": "ok",
                "message": format!("Removed worktree and session: {session}"),
            }))
        }
    }
}

/// 合成ビュー: アクティブな (= 規約パスにあり、セッションが生きている)
/// worktree の Issue 番号。Worktree ロールと SessionManager ロールの合成。
fn active_issue_ids(home: &Path, ns_repo: &str) -> Vec<String> {
    let ghq = domain::ghq_path(home, ns_repo);
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
            let expected = domain::workspace_path(home, ns_repo, &id);
            Path::new(path) == expected
                && expected.is_dir()
                && session::workspace_session_exists(ns_repo, &id)
        })
        .map(|(_, issue)| issue)
        .collect()
}

/// 合成ビュー: リポジトリ内のアクティブ Workspace 数
/// (アクティブな worktree + main セッションの有無)。
fn active_count(home: &Path, ns_repo: &str) -> usize {
    active_issue_ids(home, ns_repo).len()
        + usize::from(session::workspace_session_exists(ns_repo, &WorkspaceId::Main))
}

fn issue_entry(id: &str, title: &str, active: bool, closed: bool, dc: &str) -> Value {
    json!({ "id": id, "title": title, "active": active, "closed": closed, "devcontainer": dc })
}

/// フラグの値を返す。空文字の値は未指定と同じ扱い (zsh 版の [[ -z ]] と同じ契約)。
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .filter(|v| !v.is_empty())
        .cloned()
}

fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == flag)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

fn required(
    args: &[String],
    flag: &str,
    name: &str,
    valid: fn(&str) -> bool,
) -> Result<String, String> {
    flag_value(args, flag)
        .ok_or_else(|| format!("{flag} required"))
        .and_then(|value| validated(name, value, valid))
}

fn validated(name: &str, value: String, valid: fn(&str) -> bool) -> Result<String, String> {
    valid(&value).then_some(value.clone()).ok_or_else(|| format!("Invalid {name}: {value}"))
}
