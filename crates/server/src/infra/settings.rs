//! Settings: config.toml (トップレベルの `key = "value"` と `[[repo]]`
//! テーブルのサブセットのみ対応) と環境変数オーバーライド。
//! 優先順位: 環境変数 > 設定ファイル > 組み込み既定値。

use std::path::{Path, PathBuf};
use wsm_shared::domains::{self as domain, RepoEntry, RepoRef};

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

/// ghq 管理外のリポジトリの登録 (`[[repo]]` テーブル)。識別子のメタ情報
/// (host / ns / name) を設定で与え、worktree はドメイン共通の導出を使う。
///
/// ```toml
/// [[repo]]
/// path = "~/work/aaa"          # クローン本体の場所 (必須)
/// host = "gitlab.example.com"  # worktree パス導出に使う host (必須)
/// ns   = "myteam"              # 識別子の namespace (必須)
/// name = "aaa"                 # 識別子のリポジトリ名 (省略時 path の basename)
/// tracker = "jira-team"        # 使うトラッカー名 (省略時 default_tracker)
/// ```
pub fn custom_repos(home: &Path) -> Result<Vec<RepoEntry>, String> {
    let content = std::fs::read_to_string(config_file(home)).unwrap_or_default();
    table_sections(&content, "repo", &["path", "host", "ns", "name", "tracker"])
        .iter()
        .map(|section| repo_entry(home, section))
        .collect()
}

/// 必須キーの検証と RepoEntry への変換。設定の誤りは黙って捨てず
/// error JSON として表面化させる (フォールバックなしの方針)。
fn repo_entry(home: &Path, section: &[(String, String)]) -> Result<RepoEntry, String> {
    let path = table_value(section, "path").ok_or("[[repo]] requires path in config.toml")?;
    let host = table_value(section, "host").ok_or("[[repo]] requires host in config.toml")?;
    let ns = table_value(section, "ns").ok_or("[[repo]] requires ns in config.toml")?;
    let clone_path = expand_tilde(home, path);
    let name = match table_value(section, "name") {
        Some(name) => name,
        None => clone_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or("[[repo]] path has no basename for name")?,
    };
    if !domain::is_valid_host(&host) {
        return Err(format!("Invalid repo host: {host}"));
    }
    let repo = RepoRef::parse(&format!("{ns}/{name}"))
        .ok_or_else(|| format!("Invalid repo entry: {ns}/{name}"))?;
    let tracker = table_value(section, "tracker");
    if let Some(tracker) = &tracker {
        if !domain::is_valid_user(tracker) {
            return Err(format!("Invalid tracker name: {tracker}"));
        }
    }
    Ok(RepoEntry { repo, host, clone_path, tracker })
}

/// 設定された Tracker プラグインの列 (`[[tracker]]` テーブル)。
/// マネージャーと同じ規則: 列挙したものだけが存在し、フォールバックはない。
/// 既定は `default_tracker` で明示し、未指定なら列挙の先頭。
pub struct Trackers {
    entries: Vec<(String, PathBuf)>,
    default: Option<String>,
}

impl Trackers {
    /// 既定トラッカーのプラグイン (プロジェクト照会などリポジトリ非依存の
    /// 呼び出しに使う)。未設定は設定誤りとして表面化させる。
    pub fn default_plugin(&self) -> Result<&Path, String> {
        self.plugin_of(self.default.as_deref().or(self.entries.first().map(|(n, _)| n.as_str()))
            .ok_or("no tracker configured (add [[tracker]] to config.toml)")?)
    }

    /// リポジトリの選択 (RepoEntry.tracker) からプラグインを解決する。
    /// 名前指定が列挙にないのは設定誤りでエラー。無指定はトラッカーが
    /// 全く設定されていなければ None (照会は縮退する)。
    pub fn plugin_for(&self, tracker: Option<&str>) -> Result<Option<&Path>, String> {
        match tracker {
            Some(name) => self.plugin_of(name).map(Some),
            None if self.entries.is_empty() => Ok(None),
            None => self.default_plugin().map(Some),
        }
    }

    fn plugin_of(&self, name: &str) -> Result<&Path, String> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, path)| path.as_path())
            .ok_or_else(|| format!("tracker not configured: {name}"))
    }
}

/// config.toml から [[tracker]] と default_tracker を読む。
pub fn trackers(home: &Path) -> Result<Trackers, String> {
    let content = std::fs::read_to_string(config_file(home)).unwrap_or_default();
    let entries = table_sections(&content, "tracker", &["name", "path"])
        .iter()
        .map(|section| {
            let name =
                table_value(section, "name").ok_or("[[tracker]] requires name in config.toml")?;
            let path =
                table_value(section, "path").ok_or("[[tracker]] requires path in config.toml")?;
            if !domain::is_valid_user(&name) {
                return Err(format!("Invalid tracker name: {name}"));
            }
            Ok((name, expand_tilde(home, path)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let default = config_value(home, "default_tracker");
    if let Some(name) = &default {
        if !entries.iter().any(|(n, _)| n == name) {
            return Err(format!("tracker not configured: {name}"));
        }
    }
    Ok(Trackers { entries, default })
}

/// `[[<header>]]` テーブルの列を、テーブルごとの (key, value) の対に読む。
/// 対象キーのみ拾い、別のセクションヘッダでテーブルは終わる。
fn table_sections(content: &str, header: &str, keys: &[&str]) -> Vec<Vec<(String, String)>> {
    let marker = format!("[[{header}]]");
    let mut sections: Vec<Vec<(String, String)>> = Vec::new();
    let mut current: Option<Vec<(String, String)>> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == marker {
            sections.extend(current.take());
            current = Some(Vec::new());
        } else if trimmed.starts_with('[') {
            sections.extend(current.take());
        } else if let Some(section) = current.as_mut() {
            for key in keys {
                if let Some(value) = parse_config_line(trimmed, key) {
                    section.push(((*key).to_owned(), value));
                }
            }
        }
    }
    sections.extend(current.take());
    sections
}

/// テーブル内のキーの値 (同名キーの重複は後勝ち)。
fn table_value(section: &[(String, String)], key: &str) -> Option<String> {
    section.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone())
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
