//! オーケストレーション層: ロールを合成してサブコマンドを実装する。
//! ロール間の依存の順序 (worktree → session → devcontainer) と
//! 合成ビュー (active / 孤児 worktree) はここだけが知っている。

use crate::domain::{self, WorkspaceId};
use crate::roles::{devcontainer, session, tracker, worktree};
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
    let user = validated("user", user)?;
    Ok(Value::Array(tracker::open_projects(&user)))
}

pub fn list_issues(home: &Path, args: &[String]) -> CmdResult {
    let ns_repo = required(args, "--repo", "repo")?;

    let main_entry = issue_entry(
        "main",
        "main",
        session::workspace_session_exists(&domain::session_name(&ns_repo, &WorkspaceId::Main)),
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
    let ns_repo = required(args, "--repo", "repo")?;
    let issue = required(args, "--issue", "issue")?;
    let workspace = domain::workspace_path(home, &ns_repo, &WorkspaceId::parse(&issue));

    let repo_entries = devcontainer::repo_configs(&workspace).into_iter().map(|(name, path)| {
        json!({ "name": name, "path": path.to_string_lossy(), "source": "repo" })
    });
    let fallback_entry = settings::default_devcontainer_config(home)
        .map(|path| json!({ "name": "default", "path": path.to_string_lossy(), "source": "default" }));

    Ok(Value::Array(repo_entries.chain(fallback_entry).collect()))
}

pub fn open(home: &Path, args: &[String]) -> CmdResult {
    let ns_repo = required(args, "--repo", "repo")?;
    let issue = required(args, "--issue", "issue")?;
    if !flag_values(args, "--config").is_empty() {
        return Err("--config is not yet implemented in the Rust port".to_owned());
    }

    let id = WorkspaceId::parse(&issue);
    let manager = settings::session_manager(home)?;
    let workspace = domain::workspace_path(home, &ns_repo, &id);

    // 依存の順序: worktree (Issue のみ) → session → (devcontainer: 未実装)
    if let WorkspaceId::Issue(n) = &id {
        if !workspace.is_dir() {
            worktree::add(&domain::ghq_path(home, &ns_repo), &domain::branch_name(n), &workspace)?;
        }
    }
    let session = domain::session_name(&ns_repo, &id);
    session::ensure(manager, &session, &workspace)?;

    let message = match &id {
        WorkspaceId::Main => format!("Opened {ns_repo} (main) [{}]", manager.name()),
        WorkspaceId::Issue(n) => format!("Opened {ns_repo} #{n} [{}]", manager.name()),
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
    let ns_repo = required(args, "--repo", "repo")?;
    let target = required(args, "--target", "target")?;
    let id = WorkspaceId::parse(&target);
    let session = domain::session_name(&ns_repo, &id);

    // 破棄は open の逆順: session → devcontainer → worktree
    session::remove_workspace_sessions(&session);
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
                && session::workspace_session_exists(&domain::session_name(ns_repo, &id))
        })
        .map(|(_, issue)| issue)
        .collect()
}

fn issue_entry(id: &str, title: &str, active: bool, closed: bool, dc: &str) -> Value {
    json!({ "id": id, "title": title, "active": active, "closed": closed, "devcontainer": dc })
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == flag)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

fn required(args: &[String], flag: &str, name: &str) -> Result<String, String> {
    flag_value(args, flag)
        .ok_or_else(|| format!("{flag} required"))
        .and_then(|value| validated(name, value))
}

fn validated(name: &str, value: String) -> Result<String, String> {
    domain::is_valid_arg(&value)
        .then_some(value.clone())
        .ok_or_else(|| format!("Invalid {name}: {value}"))
}
