//! Settings: config.toml (`key = "value"` のサブセットのみ対応) と
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

/// 設定されたセッションマネージャーの列。config.toml の tmux_path /
/// herdr_path の出現順 = 選択順で、先頭が既定。path が設定されていない
/// マネージャーは存在しない扱い (選択不能・プローブや破棄の対象外)。
pub struct Managers {
    entries: Vec<(SessionManager, PathBuf)>,
}

impl Managers {
    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|(manager, _)| manager.name()).collect()
    }

    pub fn default_manager(&self) -> Option<SessionManager> {
        self.entries.first().map(|(manager, _)| *manager)
    }

    pub fn path(&self, manager: SessionManager) -> Option<&Path> {
        self.entries.iter().find(|(m, _)| *m == manager).map(|(_, path)| path.as_path())
    }
}

/// config.toml から tmux_path / herdr_path を出現順に読む。
pub fn session_managers(home: &Path) -> Managers {
    let entries = std::fs::read_to_string(config_file(home))
        .map(|content| {
            content
                .lines()
                .filter_map(|line| {
                    [("tmux_path", SessionManager::Tmux), ("herdr_path", SessionManager::Herdr)]
                        .into_iter()
                        .find_map(|(key, manager)| {
                            parse_config_line(line, key)
                                .filter(|value| !value.is_empty())
                                .map(|value| (manager, expand_tilde(home, value)))
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    Managers { entries }
}

fn manager_from_name(raw: &str) -> Result<SessionManager, String> {
    match raw {
        "tmux" => Ok(SessionManager::Tmux),
        "herdr" => Ok(SessionManager::Herdr),
        other => Err(format!("Invalid session manager: {other}")),
    }
}

fn require_configured(
    managers: &Managers,
    manager: SessionManager,
) -> Result<SessionManager, String> {
    managers
        .path(manager)
        .map(|_| manager)
        .ok_or_else(|| format!("session manager not configured: {}", manager.name()))
}

/// 使用するマネージャー:
///   WSM_SESSION_MANAGER > default_session_manager > 設定の先頭。
/// いずれも設定済み (パスあり) のもののみ有効。
pub fn session_manager(home: &Path, managers: &Managers) -> Result<SessionManager, String> {
    match env_override("WSM_SESSION_MANAGER").or_else(|| config_value(home, "default_session_manager")) {
        Some(raw) => require_configured(managers, manager_from_name(&raw)?),
        None => managers.default_manager().ok_or_else(|| {
            "no session manager configured (set tmux_path / herdr_path in config.toml)".to_owned()
        }),
    }
}

/// UI 表示用の既定マネージャー名 (default_session_manager > 先頭)。
pub fn default_manager_name(home: &Path, managers: &Managers) -> Option<&'static str> {
    config_value(home, "default_session_manager")
        .and_then(|raw| manager_from_name(&raw).ok())
        .filter(|manager| managers.path(*manager).is_some())
        .map(SessionManager::name)
        .or_else(|| managers.default_manager().map(SessionManager::name))
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
