//! Settings: config.toml (トップレベルの `key = "value"` と `[[tracker]]` /
//! `[[repo]]` テーブルのサブセットのみ対応) と環境変数オーバーライド。
//! 優先順位: 環境変数 > 設定ファイル > 組み込み既定値。
//!
//! 読み込みはコマンドの入口で 1 回 (`Settings::load`)。ファイルの内容は
//! 解析済みの `Document` として持ち、以降の問い合わせはファイルに触らない。

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

    /// 設定キー (`tmux_path` / `herdr_path`) からマネージャーを引く。
    fn from_path_key(key: &str) -> Option<Self> {
        match key {
            "tmux_path" => Some(Self::Tmux),
            "herdr_path" => Some(Self::Herdr),
            _ => None,
        }
    }

    /// 名前 (`tmux` / `herdr`) からマネージャーを引く
    /// (WSM_SESSION_MANAGER / default_session_manager の値)。
    fn from_name(raw: &str) -> Result<Self, String> {
        match raw {
            "tmux" => Ok(Self::Tmux),
            "herdr" => Ok(Self::Herdr),
            other => Err(format!("Invalid session manager: {other}")),
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

/// 設定された Tracker プラグインの 1 インスタンス。同じバイナリ (path) を
/// 別名・別設定で複数登録できる (例: 個人用と organization 用)。
///
/// - 予約キー (`name` / `path` / `ns`) 以外の `key = "value"` は、起動時に
///   環境変数 `WSM_TRACKER_<KEY大文字>` としてプラグインに渡る。wsm は値の
///   意味を知らない (トラッカー固有の設定はプラグインの責務、の具体化)。
///   `WSM_TRACKER_NAME` (インスタンス名) は常に渡る
/// - `ns` (カンマ区切り) は、その namespace のリポジトリの既定トラッカーに
///   なる (GitHub では ns = owner)
pub struct Tracker {
    name: String,
    path: PathBuf,
    ns: Vec<String>,
    env: Vec<(String, String)>,
}

impl Tracker {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// プラグイン起動時に渡す環境変数 (WSM_TRACKER_NAME + 拡張キー)。
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }
}

/// 設定された Tracker プラグインの列 (`[[tracker]]` テーブル)。
/// マネージャーと同じ規則: 列挙したものだけが存在し、フォールバックはない。
/// 既定は `default_tracker` で明示し、未指定なら列挙の先頭。
pub struct Trackers {
    entries: Vec<Tracker>,
    default: Option<String>,
}

impl Trackers {
    /// 既定を先頭にした全インスタンス (グループ集約・list-trackers 用)。
    pub fn all(&self) -> Vec<&Tracker> {
        let default = self.default_name();
        let mut ordered: Vec<&Tracker> =
            self.entries.iter().filter(|t| Some(t.name.as_str()) == default).collect();
        ordered.extend(self.entries.iter().filter(|t| Some(t.name.as_str()) != default));
        ordered
    }

    /// 既定トラッカー名 (default_tracker > 列挙の先頭)。
    pub fn default_name(&self) -> Option<&str> {
        self.default.as_deref().or(self.entries.first().map(|t| t.name.as_str()))
    }

    /// 既定トラッカー。未設定は設定誤りとして表面化させる。
    pub fn default_tracker(&self) -> Result<&Tracker, String> {
        self.named(
            self.default_name().ok_or("no tracker configured (add [[tracker]] to config.toml)")?,
        )
    }

    /// 名前からインスタンスを解決する。列挙にないのは設定誤りでエラー。
    pub fn named(&self, name: &str) -> Result<&Tracker, String> {
        self.entries
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| format!("tracker not configured: {name}"))
    }

    /// 名前指定 (任意) からインスタンスを解決する。無指定は既定。
    pub fn named_or_default(&self, name: Option<&str>) -> Result<&Tracker, String> {
        match name {
            Some(name) => self.named(name),
            None => self.default_tracker(),
        }
    }

    /// リポジトリのトラッカー解決。優先順位:
    /// 明示 ([[repo]].tracker) > ns マッピング > 既定。
    /// トラッカーが全く設定されていなければ None (照会は縮退する)。
    /// RepoStore に未登録のリポジトリ (アンブレラ等) も照会できるよう、
    /// RepoEntry ではなく識別子と明示指定を受ける。
    pub fn for_repo(
        &self,
        repo: &RepoRef,
        explicit: Option<&str>,
    ) -> Result<Option<&Tracker>, String> {
        if let Some(name) = explicit {
            return self.named(name).map(Some);
        }
        if let Some(tracker) = self.entries.iter().find(|t| t.ns.iter().any(|ns| ns == repo.ns()))
        {
            return Ok(Some(tracker));
        }
        if self.entries.is_empty() {
            return Ok(None);
        }
        self.default_tracker().map(Some)
    }
}

/// `key = "value"` の対の列 (テーブル 1 ブロック分、またはトップレベル全体)。
type Pairs = Vec<(String, String)>;

/// 設定ファイル 1 つの解析結果。トップレベルの `key = "value"` (出現順) と、
/// `[[tracker]]` / `[[repo]]` の各ブロック (記述順)。それ以外のテーブルは
/// 読み飛ばす。
struct Document {
    source: PathBuf,
    scalars: Pairs,
    trackers: Vec<Pairs>,
    repos: Vec<Pairs>,
}

/// 解析中に「今どのテーブルの中か」を表す。
enum Table {
    Tracker,
    Repo,
    Other,
}

impl Table {
    fn from_header(line: &str) -> Self {
        match line {
            "[[tracker]]" => Self::Tracker,
            "[[repo]]" => Self::Repo,
            _ => Self::Other,
        }
    }
}

impl Document {
    /// ファイルを読んで解析する。無ければ空 (設定なし) として扱い、
    /// 存在するのに読めないのはエラー。
    fn read(path: &Path) -> Result<Self, String> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(Self::parse(path, &content))
    }

    /// 行単位の解析。`[` で始まる行はテーブルの区切りで、`[[tracker]]` /
    /// `[[repo]]` のブロックだけを集める。テーブルの外の `key = "value"` が
    /// トップレベル値。
    fn parse(source: &Path, content: &str) -> Self {
        let mut doc = Self {
            source: source.to_owned(),
            scalars: Vec::new(),
            trackers: Vec::new(),
            repos: Vec::new(),
        };
        let mut current: Option<(Table, Pairs)> = None;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                doc.close(current.take());
                current = Some((Table::from_header(trimmed), Vec::new()));
            } else if let Some(pair) = parse_table_line(trimmed) {
                match current.as_mut() {
                    Some((_, pairs)) => pairs.push(pair),
                    None => doc.scalars.push(pair),
                }
            }
        }
        doc.close(current.take());
        doc
    }

    /// 読み終えたテーブルを種類ごとの列に収める。
    fn close(&mut self, section: Option<(Table, Pairs)>) {
        match section {
            Some((Table::Tracker, pairs)) => self.trackers.push(pairs),
            Some((Table::Repo, pairs)) => self.repos.push(pairs),
            Some((Table::Other, _)) | None => {}
        }
    }

    /// トップレベルの文字列キー。最初の一致のみ。
    fn scalar(&self, key: &str) -> Option<&str> {
        self.scalars.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// 読み込み済みのマシン設定。コマンドの入口で 1 回作り、各ユースケースに渡す。
pub struct Settings {
    home: PathBuf,
    base: Document,
}

impl Settings {
    /// `~/.config/wsm/config.toml` (`XDG_CONFIG_HOME` 準拠) を読む。
    pub fn load(home: &Path) -> Result<Self, String> {
        let base = Document::read(&config_file(home))?;
        Ok(Self { home: home.to_owned(), base })
    }

    /// パス導出と RepoStore の基点になる HOME。
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// 設定ファイルのトップレベル値。
    fn scalar(&self, key: &str) -> Option<&str> {
        self.base.scalar(key)
    }

    /// 環境変数 > 設定ファイル、で文字列値を解決する。
    fn value(&self, env: &str, key: &str) -> Option<String> {
        env_override(env).or_else(|| self.scalar(key).map(str::to_owned))
    }

    /// tmux_path / herdr_path を出現順に読む。空文字のパスは未設定と同じ扱い。
    pub fn session_managers(&self) -> Managers {
        let entries = self
            .base
            .scalars
            .iter()
            .filter_map(|(key, value)| {
                SessionManager::from_path_key(key).map(|manager| (manager, value.as_str()))
            })
            .filter(|(_, path)| !path.is_empty())
            .map(|(manager, path)| (manager, expand_tilde(&self.home, path)))
            .collect();
        Managers { entries }
    }

    /// 使用するマネージャー:
    ///   WSM_SESSION_MANAGER > default_session_manager > 設定の先頭。
    /// いずれも設定済み (パスあり) のもののみ有効。
    pub fn session_manager(&self, managers: &Managers) -> Result<SessionManager, String> {
        match self.value("WSM_SESSION_MANAGER", "default_session_manager") {
            Some(raw) => require_configured(managers, SessionManager::from_name(&raw)?),
            None => managers.default_manager().ok_or_else(|| {
                "no session manager configured (set tmux_path / herdr_path in config.toml)"
                    .to_owned()
            }),
        }
    }

    /// UI 表示用の既定マネージャー名 (default_session_manager > 先頭)。
    pub fn default_manager_name(&self, managers: &Managers) -> Option<&'static str> {
        self.scalar("default_session_manager")
            .and_then(|raw| SessionManager::from_name(raw).ok())
            .filter(|manager| managers.path(*manager).is_some())
            .map(SessionManager::name)
            .or_else(|| managers.default_manager().map(SessionManager::name))
    }

    /// 🐳 ウィンドウで docker exec するシェル (既定 `zsh`)。
    pub fn devcontainer_shell(&self) -> String {
        self.value("WSM_DEVCONTAINER_SHELL", "devcontainer_shell").unwrap_or_else(|| "zsh".to_owned())
    }

    /// worktree の置き場 (既定 `~/worktrees`)。
    pub fn worktree_root(&self) -> PathBuf {
        self.value("WSM_WORKTREE_ROOT", "worktree_root")
            .map(|raw| expand_tilde(&self.home, &raw))
            .unwrap_or_else(|| self.home.join("worktrees"))
    }

    /// フォールバック devcontainer 設定。実在するファイルのときだけ返す。
    pub fn default_devcontainer_config(&self) -> Option<PathBuf> {
        let raw = self.value("WSM_DEFAULT_DEVCONTAINER_CONFIG", "default_devcontainer_config")?;
        let expanded = expand_tilde(&self.home, &raw);
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
    pub fn custom_repos(&self) -> Result<Vec<RepoEntry>, String> {
        let source = self.base.source.as_path();
        self.base.repos.iter().map(|section| repo_entry(&self.home, source, section)).collect()
    }

    /// [[tracker]] と default_tracker を読む。
    pub fn trackers(&self) -> Result<Trackers, String> {
        let source = self.base.source.as_path();
        let entries = self
            .base
            .trackers
            .iter()
            .map(|section| tracker_entry(&self.home, source, section))
            .collect::<Result<Vec<Tracker>, String>>()?;

        // ns の割り当ては一意であること (同じ ns を 2 つのトラッカーが主張したら
        // どちらの世界か決められない)
        for (i, a) in entries.iter().enumerate() {
            for b in &entries[i + 1..] {
                if let Some(ns) = a.ns.iter().find(|ns| b.ns.contains(ns)) {
                    return Err(format!(
                        "ns mapped to multiple trackers: {ns} ({}, {})",
                        a.name, b.name
                    ));
                }
            }
        }

        let default = self.scalar("default_tracker").map(str::to_owned);
        if let Some(name) = &default
            && !entries.iter().any(|t| &t.name == name)
        {
            return Err(format!("tracker not configured: {name}"));
        }
        Ok(Trackers { entries, default })
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

fn expand_tilde(home: &Path, raw: &str) -> PathBuf {
    raw.strip_prefix("~/").map(|rest| home.join(rest)).unwrap_or_else(|| PathBuf::from(raw))
}

/// エラー文に出す設定ファイル名 (同じディレクトリ内で区別できる basename)。
fn file_name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// 必須キーの検証と RepoEntry への変換。設定の誤りは黙って捨てず
/// error JSON として表面化させる (フォールバックなしの方針)。
/// source はブロックが書かれていた設定ファイル (エラー文に出す)。
fn repo_entry(home: &Path, source: &Path, section: &Pairs) -> Result<RepoEntry, String> {
    let required = |key: &str| {
        table_value(section, key)
            .ok_or_else(|| format!("[[repo]] requires {key} in {}", file_name_of(source)))
    };
    let path = required("path")?;
    let host = required("host")?;
    let ns = required("ns")?;
    let clone_path = expand_tilde(home, &path);
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
    if let Some(tracker) = &tracker
        && !domain::is_valid_user(tracker)
    {
        return Err(format!("Invalid tracker name: {tracker}"));
    }
    Ok(RepoEntry { repo, host, clone_path, tracker })
}

/// `[[tracker]]` 1 ブロックの検証と Tracker への変換。
/// source はブロックが書かれていた設定ファイル (エラー文に出す)。
fn tracker_entry(home: &Path, source: &Path, section: &Pairs) -> Result<Tracker, String> {
    let required = |key: &str| {
        table_value(section, key)
            .ok_or_else(|| format!("[[tracker]] requires {key} in {}", file_name_of(source)))
    };
    let name = required("name")?;
    let path = required("path")?;
    if !domain::is_valid_user(&name) {
        return Err(format!("Invalid tracker name: {name}"));
    }
    let ns = match table_value(section, "ns") {
        None => Vec::new(),
        Some(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|ns| !ns.is_empty())
            .map(|ns| {
                domain::is_valid_user(ns)
                    .then(|| ns.to_owned())
                    .ok_or_else(|| format!("Invalid tracker ns: {ns}"))
            })
            .collect::<Result<Vec<_>, String>>()?,
    };

    // 予約キー以外はプラグインへの環境変数 (同名キーの重複は後勝ち)
    let mut env: Vec<(String, String)> = vec![("WSM_TRACKER_NAME".to_owned(), name.clone())];
    for (key, value) in section {
        if matches!(key.as_str(), "name" | "path" | "ns") {
            continue;
        }
        let var = format!("WSM_TRACKER_{}", key.to_uppercase());
        match env.iter_mut().find(|(existing, _)| existing == &var) {
            Some((_, existing)) => *existing = value.clone(),
            None => env.push((var, value.clone())),
        }
    }

    Ok(Tracker { name, path: expand_tilde(home, &path), ns, env })
}

/// `key = "value"` (前後の空白と行末コメントを許容) を対に分解する。
/// キーは英小文字始まりの英小文字・数字・`_` のみ (環境変数名に安全に
/// 写せる形)。それ以外の行 (コメント・空行・形の違う行) は None。
fn parse_table_line(line: &str) -> Option<(String, String)> {
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    let mut chars = key.chars();
    let valid_key = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid_key {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('"')?;
    rest.split_once('"').map(|(value, _)| (key.to_owned(), value.to_owned()))
}

/// テーブル内のキーの値 (同名キーの重複は後勝ち)。
fn table_value(section: &Pairs, key: &str) -> Option<String> {
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
