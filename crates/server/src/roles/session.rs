//! SessionManager ロールの tmux / herdr 実装。
//!
//! Workspace とセッションの対応は実装ごとに異なる:
//! - tmux: Workspace ごとに 1 セッション (`<ns>_<repo>(_<id>)`)
//! - herdr: リポジトリ単位のセッション (`<ns>.<repo>`) に、Issue ごとの
//!   workspace (ラベル = Issue 番号) を追加する。セッション外からの workspace
//!   操作は HERDR_SOCKET_PATH で対象セッションの socket を指定して行う
//!
//! バイナリは設定 (tmux_path / herdr_path) のパスで起動する (PATH 非依存)。
//! パスが設定されていないマネージャーは存在しない扱いで、プローブや破棄の
//! 対象にもならない。名前・ラベルの導出はドメイン層の関数を使う。

use crate::infra::exec;
use crate::infra::settings::{Managers, SessionManager};
use std::path::Path;
use wsm_shared::domains::{self as domain, RepoRef, WorkspaceId};

fn tmux_exists(bin: &Path, session: &str) -> bool {
    exec::succeeds(bin, &["has-session", "-t", &format!("={session}")])
}

// --- herdr ---

fn herdr_sessions(bin: &Path) -> Option<serde_json::Value> {
    exec::stdout_if_ok(bin, &["session", "list", "--json"])
        .and_then(|out| serde_json::from_str(&out).ok())
}

fn herdr_session_running(bin: &Path, session: &str) -> bool {
    herdr_sessions(bin)
        .and_then(|v| v["sessions"].as_array().cloned())
        .is_some_and(|sessions| {
            sessions.iter().any(|s| {
                s["name"].as_str() == Some(session) && s["running"].as_bool() == Some(true)
            })
        })
}

fn herdr_socket_path(bin: &Path, session: &str) -> Option<String> {
    herdr_sessions(bin)?["sessions"].as_array()?.iter().find_map(|s| {
        (s["name"].as_str() == Some(session))
            .then(|| s["socket_path"].as_str().map(str::to_owned))
            .flatten()
    })
}

/// 対象セッションの workspace 一覧: (workspace_id, label) の列。
fn herdr_workspaces(bin: &Path, socket: &str) -> Vec<(String, String)> {
    exec::stdout_if_ok_env(bin, &["workspace", "list"], &[("HERDR_SOCKET_PATH", socket)])
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

fn herdr_workspace_id(bin: &Path, socket: &str, label: &str) -> Option<String> {
    herdr_workspaces(bin, socket).into_iter().find_map(|(id, l)| (l == label).then_some(id))
}

/// セッションをヘッドレスで起動し、running になるまで待つ (冪等)。
fn herdr_ensure_running(bin: &Path, session: &str) -> Result<(), String> {
    if herdr_session_running(bin, session) {
        return Ok(());
    }
    std::process::Command::new(bin)
        .args(["--session", session, "server"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to start herdr session: {session} ({e})"))?;
    for _ in 0..50 {
        if herdr_session_running(bin, session) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(format!("failed to start herdr session: {session}"))
}

fn herdr_stop_and_delete(bin: &Path, session: &str) {
    exec::run_ignoring_failure(bin, &["session", "stop", session, "--json"]);
    exec::run_ignoring_failure(bin, &["session", "delete", session, "--json"]);
}

fn is_issue_label(label: &str) -> bool {
    !label.is_empty() && label.chars().all(|c| c.is_ascii_digit())
}

/// herdr のセッションに wsm 管理の Issue workspace が残っているか。
/// main の remove を拒否する判定に使う (合成はオーケストレーション層)。
pub fn herdr_blocks_main_removal(repo: &RepoRef, managers: &Managers) -> bool {
    let Some(bin) = managers.path(SessionManager::Herdr) else { return false };
    let session = domain::herdr_session_name(repo);
    herdr_session_running(bin, &session)
        && herdr_socket_path(bin, &session).is_some_and(|sock| {
            herdr_workspaces(bin, &sock).iter().any(|(_, label)| is_issue_label(label))
        })
}

// --- Workspace とセッションの対応 (設定されたマネージャーを横断) ---

/// 設定されたマネージャーを横断した存在確認。tmux はセッション名で、herdr は
/// リポジトリセッション (+ Issue なら workspace ラベル) で判定する。
pub fn workspace_session_exists(repo: &RepoRef, id: &WorkspaceId, managers: &Managers) -> bool {
    if let Some(bin) = managers.path(SessionManager::Tmux) {
        if tmux_exists(bin, &domain::tmux_session_name(repo, id)) {
            return true;
        }
    }
    let Some(bin) = managers.path(SessionManager::Herdr) else { return false };
    let repo_session = domain::herdr_session_name(repo);
    if !herdr_session_running(bin, &repo_session) {
        return false;
    }
    match id {
        WorkspaceId::Main => true,
        WorkspaceId::Issue(issue) => herdr_socket_path(bin, &repo_session)
            .is_some_and(|sock| herdr_workspace_id(bin, &sock, issue).is_some()),
    }
}

/// Workspace のセッションを冪等に用意し、アタッチ対象のセッション名を返す。
/// herdr はリポジトリセッションを (なければヘッドレス起動して) 用意し、
/// main / Issue の workspace を作成・フォーカスする。
pub fn ensure(
    manager: SessionManager,
    repo: &RepoRef,
    id: &WorkspaceId,
    cwd: &Path,
    paths: &domain::Paths,
    managers: &Managers,
) -> Result<String, String> {
    let bin = managers
        .path(manager)
        .ok_or_else(|| format!("session manager not configured: {}", manager.name()))?;
    match manager {
        SessionManager::Tmux => {
            let session = domain::tmux_session_name(repo, id);
            if !tmux_exists(bin, &session) {
                let cwd = cwd.to_string_lossy();
                if !exec::succeeds(bin, &["new-session", "-d", "-s", &session, "-c", &cwd]) {
                    return Err(format!("Failed to create session: {session}"));
                }
            }
            Ok(session)
        }
        SessionManager::Herdr => {
            let session = domain::herdr_session_name(repo);
            herdr_ensure_running(bin, &session)?;
            let sock = herdr_socket_path(bin, &session)
                .ok_or_else(|| format!("failed to resolve herdr socket: {session}"))?;
            let env = [("HERDR_SOCKET_PATH", sock.as_str())];

            // main の workspace (ラベル = リポジトリ名) は Issue open 時も常に
            // 保証する。ヘッドレス起動直後のセッションは workspace ゼロで、
            // アタッチ時の自動作成に任せると cwd がリポジトリにならないため。
            // Issue open 時はフォーカスを奪わない
            let opening_main = matches!(id, WorkspaceId::Main);
            let main_label = domain::herdr_workspace_label(repo, &WorkspaceId::Main);
            match (herdr_workspace_id(bin, &sock, &main_label), opening_main) {
                (None, _) => {
                    let ghq = domain::ghq_path(paths, repo);
                    let ghq = ghq.to_string_lossy();
                    let focus_flag = if opening_main { "--focus" } else { "--no-focus" };
                    exec::stdout_if_ok_env(
                        bin,
                        &["workspace", "create", "--cwd", &ghq, "--label", &main_label, focus_flag],
                        &env,
                    )
                    .ok_or("failed to create herdr workspace")?;
                }
                (Some(wid), true) => {
                    exec::run_ignoring_failure_env(bin, &["workspace", "focus", &wid], &env)
                }
                (Some(_), false) => {}
            }

            if let WorkspaceId::Issue(issue) = id {
                match herdr_workspace_id(bin, &sock, issue) {
                    Some(wid) => {
                        exec::run_ignoring_failure_env(bin, &["workspace", "focus", &wid], &env)
                    }
                    None => {
                        let cwd = cwd.to_string_lossy();
                        exec::stdout_if_ok_env(
                            bin,
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

/// セッション/workspace の冪等な破棄 (設定されたマネージャーを横断)。
/// herdr: Issue は workspace close (最後の 1 つならセッションも畳む)、
/// main はセッションの stop + delete (Issue 残存チェックはオーケストレーション層)。
pub fn remove_workspace_sessions(repo: &RepoRef, id: &WorkspaceId, managers: &Managers) {
    if let Some(bin) = managers.path(SessionManager::Tmux) {
        exec::run_ignoring_failure(
            bin,
            &["kill-session", "-t", &format!("={}", domain::tmux_session_name(repo, id))],
        );
    }

    let Some(bin) = managers.path(SessionManager::Herdr) else { return };
    let session = domain::herdr_session_name(repo);
    if !herdr_session_running(bin, &session) {
        return;
    }
    match id {
        WorkspaceId::Main => herdr_stop_and_delete(bin, &session),
        WorkspaceId::Issue(issue) => {
            let Some(sock) = herdr_socket_path(bin, &session) else { return };
            let workspaces = herdr_workspaces(bin, &sock);
            let Some((wid, _)) = workspaces.iter().find(|(_, label)| label == issue) else {
                return;
            };
            exec::run_ignoring_failure_env(
                bin,
                &["workspace", "close", wid],
                &[("HERDR_SOCKET_PATH", sock.as_str())],
            );
            if workspaces.len() <= 1 {
                herdr_stop_and_delete(bin, &session);
            }
        }
    }
}

/// セッションに、指定コマンドを実行するウィンドウを追加する。冪等で、
/// 同じ dedup_key を持つペインが生きていれば何もしない (tmux はキーを
/// pane オプション @wsm_cid に記録する)。ウィンドウ概念を持たない herdr は noop。
pub fn add_window(
    manager: SessionManager,
    session: &str,
    name: &str,
    command: &str,
    dedup_key: &str,
    managers: &Managers,
) {
    match manager {
        SessionManager::Tmux => {
            let Some(bin) = managers.path(SessionManager::Tmux) else { return };
            let already =
                exec::stdout_if_ok(bin, &["list-panes", "-s", "-t", session, "-F", "#{@wsm_cid}"])
                    .is_some_and(|out| out.lines().any(|line| line == dedup_key));
            if already {
                return;
            }
            let pane = exec::stdout_if_ok(
                bin,
                &["new-window", "-d", "-P", "-F", "#{pane_id}", "-t", &format!("{session}:"), "-n", name, command],
            )
            .map(|out| out.trim().to_owned())
            .filter(|pane| !pane.is_empty());
            if let Some(pane) = pane {
                exec::run_ignoring_failure(bin, &["set-option", "-p", "-t", &pane, "@wsm_cid", dedup_key]);
            }
        }
        SessionManager::Herdr => {}
    }
}

/// UI が Terminal にそのまま渡すアタッチ用コマンド (open 応答の attach_command)。
/// バイナリは設定されたパスを使う (Ghostty はログインシェルを介さず
/// コマンドを起動するため、PATH に依存しない)。
pub fn attach_command(
    manager: SessionManager,
    session: &str,
    workspace: &Path,
    managers: &Managers,
) -> String {
    let bin = managers
        .path(manager)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| manager.name().to_owned());
    match manager {
        SessionManager::Tmux => format!("{bin} attach-session -t '{session}'"),
        SessionManager::Herdr => {
            let script =
                format!("cd '{}' && exec '{bin}' --session '{session}'", workspace.display());
            format!("/bin/bash -lc {}", quote_word(&script))
        }
    }
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
