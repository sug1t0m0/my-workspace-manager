//! SessionManager ロールの tmux / herdr 実装。
//!
//! Workspace とセッションの対応は実装ごとに異なる:
//! - tmux: Workspace ごとに 1 セッション (`<ns>_<repo>(_<id>)`)
//! - herdr: リポジトリ単位のセッション (`<ns>.<repo>`) に、Issue ごとの
//!   workspace (ラベル = Issue 番号) を追加する。セッション外からの workspace
//!   操作は HERDR_SOCKET_PATH で対象セッションの socket を指定して行う
//!
//! 名前・ラベルの導出はドメイン層の関数を使う (実装側に導出規則を持たせない)。

use wsm_shared::domains::{self as domain, RepoRef, WorkspaceId};
use crate::infra::exec;
use crate::infra::settings::SessionManager;
use std::path::Path;

fn tmux_exists(session: &str) -> bool {
    exec::succeeds("tmux", &["has-session", "-t", &format!("={session}")])
}

// --- herdr ---

fn herdr_sessions() -> Option<serde_json::Value> {
    exec::stdout_if_ok("herdr", &["session", "list", "--json"])
        .and_then(|out| serde_json::from_str(&out).ok())
}

fn herdr_session_running(session: &str) -> bool {
    herdr_sessions()
        .and_then(|v| v["sessions"].as_array().cloned())
        .is_some_and(|sessions| {
            sessions.iter().any(|s| {
                s["name"].as_str() == Some(session) && s["running"].as_bool() == Some(true)
            })
        })
}

fn herdr_socket_path(session: &str) -> Option<String> {
    herdr_sessions()?["sessions"].as_array()?.iter().find_map(|s| {
        (s["name"].as_str() == Some(session))
            .then(|| s["socket_path"].as_str().map(str::to_owned))
            .flatten()
    })
}

/// 対象セッションの workspace 一覧: (workspace_id, label) の列。
fn herdr_workspaces(socket: &str) -> Vec<(String, String)> {
    exec::stdout_if_ok_env("herdr", &["workspace", "list"], &[("HERDR_SOCKET_PATH", socket)])
        .and_then(|out| serde_json::from_str::<serde_json::Value>(&out).ok())
        .and_then(|v| v["result"]["workspaces"].as_array().cloned())
        .map(|workspaces| {
            workspaces
                .iter()
                .filter_map(|w| {
                    Some((w["workspace_id"].as_str()?.to_owned(), w["label"].as_str()?.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn herdr_workspace_id(socket: &str, label: &str) -> Option<String> {
    herdr_workspaces(socket).into_iter().find_map(|(id, l)| (l == label).then_some(id))
}

/// セッションをヘッドレスで起動し、running になるまで待つ (冪等)。
fn herdr_ensure_running(session: &str) -> Result<(), String> {
    if herdr_session_running(session) {
        return Ok(());
    }
    std::process::Command::new("herdr")
        .args(["--session", session, "server"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to start herdr session: {session} ({e})"))?;
    for _ in 0..50 {
        if herdr_session_running(session) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(format!("failed to start herdr session: {session}"))
}

fn herdr_stop_and_delete(session: &str) {
    exec::run_ignoring_failure("herdr", &["session", "stop", session, "--json"]);
    exec::run_ignoring_failure("herdr", &["session", "delete", session, "--json"]);
}

fn is_issue_label(label: &str) -> bool {
    !label.is_empty() && label.chars().all(|c| c.is_ascii_digit())
}

/// herdr のセッションに wsm 管理の Issue workspace が残っているか。
/// main の remove を拒否する判定に使う (合成はオーケストレーション層)。
pub fn herdr_blocks_main_removal(repo: &RepoRef) -> bool {
    let session = domain::herdr_session_name(repo);
    herdr_session_running(&session)
        && herdr_socket_path(&session).is_some_and(|sock| {
            herdr_workspaces(&sock).iter().any(|(_, label)| is_issue_label(label))
        })
}

// --- Workspace とセッションの対応 (マネージャー実装を横断) ---

/// マネージャー実装を横断した存在確認。tmux はセッション名で、herdr は
/// リポジトリセッション (+ Issue なら workspace ラベル) で判定する。
pub fn workspace_session_exists(repo: &RepoRef, id: &WorkspaceId) -> bool {
    if tmux_exists(&domain::tmux_session_name(repo, id)) {
        return true;
    }
    let repo_session = domain::herdr_session_name(repo);
    if !herdr_session_running(&repo_session) {
        return false;
    }
    match id {
        WorkspaceId::Main => true,
        WorkspaceId::Issue(issue) => herdr_socket_path(&repo_session)
            .is_some_and(|sock| herdr_workspace_id(&sock, issue).is_some()),
    }
}

/// Workspace のセッションを冪等に用意し、アタッチ対象のセッション名を返す。
/// herdr はリポジトリセッションを (なければヘッドレス起動して) 用意し、
/// Issue なら workspace を作成/フォーカスする。
pub fn ensure(
    manager: SessionManager,
    repo: &RepoRef,
    id: &WorkspaceId,
    cwd: &Path,
    home: &Path,
) -> Result<String, String> {
    match manager {
        SessionManager::Tmux => {
            let session = domain::tmux_session_name(repo, id);
            if !tmux_exists(&session) {
                let cwd = cwd.to_string_lossy();
                if !exec::succeeds("tmux", &["new-session", "-d", "-s", &session, "-c", &cwd]) {
                    return Err(format!("Failed to create session: {session}"));
                }
            }
            Ok(session)
        }
        SessionManager::Herdr => {
            exec::which("herdr").ok_or("herdr not installed")?;
            let session = domain::herdr_session_name(repo);
            herdr_ensure_running(&session)?;
            let sock = herdr_socket_path(&session)
                .ok_or_else(|| format!("failed to resolve herdr socket: {session}"))?;
            let env = [("HERDR_SOCKET_PATH", sock.as_str())];

            // main の workspace (ラベル = リポジトリ名) は Issue open 時も常に
            // 保証する。ヘッドレス起動直後のセッションは workspace ゼロで、
            // アタッチ時の自動作成に任せると cwd がリポジトリにならないため。
            // Issue open 時はフォーカスを奪わない
            let opening_main = matches!(id, WorkspaceId::Main);
            let main_label = domain::herdr_workspace_label(repo, &WorkspaceId::Main);
            match (herdr_workspace_id(&sock, &main_label), opening_main) {
                (None, _) => {
                    let ghq = domain::ghq_path(home, repo);
                    let ghq = ghq.to_string_lossy();
                    let focus_flag = if opening_main { "--focus" } else { "--no-focus" };
                    exec::stdout_if_ok_env(
                        "herdr",
                        &["workspace", "create", "--cwd", &ghq, "--label", &main_label, focus_flag],
                        &env,
                    )
                    .ok_or("failed to create herdr workspace")?;
                }
                (Some(wid), true) => {
                    exec::run_ignoring_failure_env("herdr", &["workspace", "focus", &wid], &env)
                }
                (Some(_), false) => {}
            }

            if let WorkspaceId::Issue(issue) = id {
                match herdr_workspace_id(&sock, issue) {
                    Some(wid) => {
                        exec::run_ignoring_failure_env("herdr", &["workspace", "focus", &wid], &env)
                    }
                    None => {
                        let cwd = cwd.to_string_lossy();
                        exec::stdout_if_ok_env(
                            "herdr",
                            &["workspace", "create", "--cwd", &cwd, "--label", issue, "--focus"],
                            &env,
                        )
                        .ok_or("failed to create herdr workspace")?;
                    }
                }
            }
            Ok(session)
        }
    }
}

/// セッション/workspace の冪等な破棄 (マネージャー実装を横断)。
/// herdr: Issue は workspace close (最後の 1 つならセッションも畳む)、
/// main はセッションの stop + delete (Issue 残存チェックはオーケストレーション層)。
pub fn remove_workspace_sessions(repo: &RepoRef, id: &WorkspaceId) {
    exec::run_ignoring_failure(
        "tmux",
        &["kill-session", "-t", &format!("={}", domain::tmux_session_name(repo, id))],
    );

    let session = domain::herdr_session_name(repo);
    if !herdr_session_running(&session) {
        return;
    }
    match id {
        WorkspaceId::Main => herdr_stop_and_delete(&session),
        WorkspaceId::Issue(issue) => {
            let Some(sock) = herdr_socket_path(&session) else { return };
            let workspaces = herdr_workspaces(&sock);
            let Some((wid, _)) = workspaces.iter().find(|(_, label)| label == issue) else {
                return;
            };
            exec::run_ignoring_failure_env(
                "herdr",
                &["workspace", "close", wid],
                &[("HERDR_SOCKET_PATH", sock.as_str())],
            );
            if workspaces.len() <= 1 {
                herdr_stop_and_delete(&session);
            }
        }
    }
}

/// セッションに、指定コマンドを実行するウィンドウを追加する。冪等で、
/// 同じ dedup_key を持つペインが生きていれば何もしない (tmux はキーを
/// pane オプション @wsm_cid に記録する)。ウィンドウ概念を持たない herdr は noop。
pub fn add_window(manager: SessionManager, session: &str, name: &str, command: &str, dedup_key: &str) {
    match manager {
        SessionManager::Tmux => {
            let already = exec::stdout_if_ok("tmux", &["list-panes", "-s", "-t", session, "-F", "#{@wsm_cid}"])
                .is_some_and(|out| out.lines().any(|line| line == dedup_key));
            if already {
                return;
            }
            let pane = exec::stdout_if_ok(
                "tmux",
                &["new-window", "-d", "-P", "-F", "#{pane_id}", "-t", &format!("{session}:"), "-n", name, command],
            )
            .map(|out| out.trim().to_owned())
            .filter(|pane| !pane.is_empty());
            if let Some(pane) = pane {
                exec::run_ignoring_failure("tmux", &["set-option", "-p", "-t", &pane, "@wsm_cid", dedup_key]);
            }
        }
        SessionManager::Herdr => {}
    }
}

/// UI が Terminal にそのまま渡すアタッチ用コマンド (open 応答の attach_command)。
/// バイナリは PATH から絶対パスに解決する (Ghostty はログインシェルを介さず
/// コマンドを起動するため)。見つからないときは素の名前に倒す。
pub fn attach_command(manager: SessionManager, session: &str, workspace: &Path) -> String {
    match manager {
        SessionManager::Tmux => {
            let tmux = resolved_bin("tmux");
            format!("{tmux} attach-session -t '{session}'")
        }
        SessionManager::Herdr => {
            let herdr = resolved_bin("herdr");
            let script =
                format!("cd '{}' && exec '{herdr}' --session '{session}'", workspace.display());
            format!("/bin/bash -lc {}", quote_word(&script))
        }
    }
}

fn resolved_bin(name: &str) -> String {
    exec::which(name).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_owned())
}

/// zsh の printf %q 相当: 英数と `_./-` 以外をバックスラッシュでエスケープする。
fn quote_word(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            let escaped = !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'));
            escaped.then_some('\\').into_iter().chain(std::iter::once(c))
        })
        .collect()
}
