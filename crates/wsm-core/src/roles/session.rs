//! SessionManager ロールの tmux / herdr 実装。
//! セッション名の導出はドメイン層の責務で、ここは受け取った名前を使うだけ。

use crate::exec;
use crate::settings::SessionManager;
use std::path::Path;

fn tmux_exists(session: &str) -> bool {
    exec::succeeds("tmux", &["has-session", "-t", &format!("={session}")])
}

fn herdr_exists(session: &str) -> bool {
    exec::stdout_if_ok("herdr", &["session", "list", "--json"])
        .and_then(|out| serde_json::from_str::<serde_json::Value>(&out).ok())
        .and_then(|v| v["sessions"].as_array().cloned())
        .is_some_and(|sessions| {
            sessions.iter().any(|s| {
                s["name"].as_str() == Some(session) && s["running"].as_bool() == Some(true)
            })
        })
}

/// マネージャー実装を横断した存在確認 (zsh 版と同じく tmux / herdr の両方を見る)。
pub fn workspace_session_exists(session: &str) -> bool {
    tmux_exists(session) || herdr_exists(session)
}

/// 冪等な作成。tmux はなければ作る。herdr はアタッチ時に作られるため
/// インストール確認のみ。
pub fn ensure(manager: SessionManager, session: &str, cwd: &Path) -> Result<(), String> {
    match manager {
        SessionManager::Tmux if tmux_exists(session) => Ok(()),
        SessionManager::Tmux => {
            let cwd = cwd.to_string_lossy();
            exec::succeeds("tmux", &["new-session", "-d", "-s", session, "-c", &cwd])
                .then_some(())
                .ok_or_else(|| format!("Failed to create session: {session}"))
        }
        SessionManager::Herdr => exec::which("herdr")
            .map(|_| ())
            .ok_or_else(|| "herdr not installed".to_owned()),
    }
}

/// マネージャー実装を横断した冪等な破棄。
pub fn remove_workspace_sessions(session: &str) {
    exec::run_ignoring_failure("tmux", &["kill-session", "-t", &format!("={session}")]);
    if exec::which("herdr").is_some() {
        exec::run_ignoring_failure("herdr", &["session", "delete", session, "--json"]);
    }
}

/// UI が Terminal にそのまま渡すアタッチ用コマンド (open 応答の attach_command)。
pub fn attach_command(manager: SessionManager, session: &str, workspace: &Path, home: &Path) -> String {
    match manager {
        SessionManager::Tmux => {
            let tmux = exec::which("tmux")
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/opt/homebrew/bin/tmux".to_owned());
            format!("{tmux} attach-session -t '{session}'")
        }
        SessionManager::Herdr => {
            let herdr = exec::which("herdr")
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| home.join(".local/bin/herdr").to_string_lossy().into_owned());
            let script =
                format!("cd '{}' && exec '{herdr}' --session '{session}'", workspace.display());
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
