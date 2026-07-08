//! Settings: config.toml (zsh 版と同じ `key = "value"` サブセット) と
//! 環境変数オーバーライド。優先順位: 環境変数 > 設定ファイル > 組み込み既定値。

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SessionManager {
    Tmux,
    Herdr,
}

impl SessionManager {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
            Self::Herdr => "herdr",
        }
    }
}

pub fn session_manager(home: &Path) -> Result<SessionManager, String> {
    let raw = env_override("WSM_SESSION_MANAGER")
        .or_else(|| config_value(home, "session_manager"))
        .unwrap_or_else(|| "tmux".to_owned());
    match raw.as_str() {
        "tmux" => Ok(SessionManager::Tmux),
        "herdr" => Ok(SessionManager::Herdr),
        other => Err(format!("Invalid session manager: {other}")),
    }
}

/// 🐳 ウィンドウで docker exec するシェル (既定 `zsh`)。
pub fn devcontainer_shell(home: &Path) -> String {
    env_override("WSM_DEVCONTAINER_SHELL")
        .or_else(|| config_value(home, "devcontainer_shell"))
        .unwrap_or_else(|| "zsh".to_owned())
}

/// worktree の置き場 (既定 `~/worktrees`)。
pub fn worktree_root(home: &Path) -> PathBuf {
    env_override("WSM_WORKTREE_ROOT")
        .or_else(|| config_value(home, "worktree_root"))
        .map(|raw| expand_tilde(home, raw))
        .unwrap_or_else(|| home.join("worktrees"))
}

fn expand_tilde(home: &Path, raw: String) -> PathBuf {
    raw.strip_prefix("~/").map(|rest| home.join(rest)).unwrap_or_else(|| PathBuf::from(raw))
}

/// フォールバック devcontainer 設定。実在するファイルのときだけ返す。
pub fn default_devcontainer_config(home: &Path) -> Option<PathBuf> {
    let raw = env_override("WSM_DEFAULT_DEVCONTAINER_CONFIG")
        .or_else(|| config_value(home, "default_devcontainer_config"))?;
    let expanded = expand_tilde(home, raw);
    expanded.is_file().then_some(expanded)
}

fn env_override(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn config_file(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("wsm/config.toml")
}

/// config.toml からトップレベルの文字列キーを読む。最初の一致のみ。
fn config_value(home: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(config_file(home))
        .ok()?
        .lines()
        .find_map(|line| parse_config_line(line, key))
}

/// `key = "value"` (前後の空白と行末コメントを許容) を値に分解する。
fn parse_config_line(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    rest.split_once('"').map(|(value, _)| value.to_owned())
}
