//! Settings: config.toml + config.local.toml (overlay) と環境変数オーバーライド。
//! 優先順位: 環境変数 > config.local.toml > config.toml > 組み込み既定値。
//!
//! 設定ファイルは TOML として解析し (toml crate)、使うのはトップレベルの
//! 文字列値と `[[tracker]]` / `[[repo]]` テーブル。値はすべて文字列で、
//! それ以外の型は設定誤りとしてエラーにする (黙って捨てない)。
//!
//! 2 ファイルの合成規則 (どちらも無ければ空として扱う):
//! - トップレベル値は overlay が後勝ち
//! - `[[tracker]]` / `[[repo]]` は連結 (config.toml → config.local.toml の順)。
//!   同名の tracker は設定誤り (tracker 名は全体で一意)
//! - tmux_path / herdr_path の並び順は config.toml の出現順を骨格にし、
//!   同じキーは値だけ overlay に差し替え、overlay にだけあるキーは末尾
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

/// 設定されたセッションマネージャーの列。設定ファイルの tmux_path /
/// herdr_path の出現順 = 選択順で、先頭が既定 (2 ファイルの合成規則は
/// `Settings::session_managers` を参照)。path が設定されていない
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
    /// このインスタンスが書かれていた設定ファイル。
    source: PathBuf,
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

    /// このインスタンスが書かれていた設定ファイル。`Settings::load` が読んだ
    /// パス (`$XDG_CONFIG_HOME/wsm` または `~/.config/wsm` 配下。正規化は
    /// しない)。診断用。
    pub fn source(&self) -> &Path {
        &self.source
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

/// `key = "value"` の 1 対。
type Pair = (String, String);

/// 対の列 (テーブル 1 ブロック分、またはトップレベル全体)。
/// ファイルの記述順を保つ (tmux_path / herdr_path の出現順が選択 UI の並び順)。
type Pairs = Vec<Pair>;

/// 設定ファイル 1 つの解析結果。トップレベルの文字列値 (記述順) と、
/// `[[tracker]]` / `[[repo]]` の各ブロック (記述順)。それ以外のテーブルは
/// 読み飛ばす。
struct Document {
    source: PathBuf,
    scalars: Pairs,
    trackers: Vec<Pairs>,
    repos: Vec<Pairs>,
}

impl Document {
    /// 設定なし (ファイルが無い) を表す空の文書。
    fn empty(source: &Path) -> Self {
        Self {
            source: source.to_owned(),
            scalars: Vec::new(),
            trackers: Vec::new(),
            repos: Vec::new(),
        }
    }

    /// ファイルを読んで解析する。無ければ空 (設定なし) として扱い、
    /// 存在するのに読めない・TOML として壊れているのはエラー。
    fn read(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(content) => Self::parse(path, &content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty(path)),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// TOML として解析し、使う部分だけを取り出す。`tracker` / `repo` は配列の
    /// テーブル、それ以外のテーブルは読み飛ばし、残るトップレベルの値は
    /// すべて文字列であること。他の配列 (`[[Tracker]]` のような typo) と
    /// 数値・真偽値は設定誤り。
    fn parse(source: &Path, content: &str) -> Result<Self, String> {
        let mut table: toml::Table =
            content.parse().map_err(|e| parse_error(source, content, &e))?;
        let file = file_name_of(source);
        let trackers = table_blocks(table.remove("tracker"), "tracker", &file)?;
        let repos = table_blocks(table.remove("repo"), "repo", &file)?;
        let scalars = table
            .into_iter()
            .filter_map(|(key, value)| match value {
                toml::Value::String(s) => Some(Ok((key, s))),
                toml::Value::Table(_) => None,
                toml::Value::Array(_) => {
                    Some(Err(format!("[[{key}]] is not a known table in {file}")))
                }
                _ => Some(Err(format!("{key} must be a string in {file}"))),
            })
            .collect::<Result<Pairs, String>>()?;
        Ok(Self { source: source.to_owned(), scalars, trackers, repos })
    }

    /// トップレベルの文字列キー (TOML は同名キーを許さないので一意)。
    fn scalar(&self, key: &str) -> Option<&str> {
        self.scalars.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// `[[tracker]]` の各ブロックを、出どころのファイルと組にして記述順に返す。
    fn tracker_blocks(&self) -> impl Iterator<Item = (&Path, &Pairs)> {
        self.trackers.iter().map(|pairs| (self.source.as_path(), pairs))
    }

    /// `[[repo]]` の各ブロックを、出どころのファイルと組にして記述順に返す。
    fn repo_blocks(&self) -> impl Iterator<Item = (&Path, &Pairs)> {
        self.repos.iter().map(|pairs| (self.source.as_path(), pairs))
    }
}

/// TOML の解析エラーを 1 行にする:
/// `<path>: TOML parse error at line N, column M: <message>`。
/// toml crate の Display は該当行の抜き出しを含む複数行で、error JSON の値には
/// 向かないため組み直す。
fn parse_error(source: &Path, content: &str, error: &toml::de::Error) -> String {
    let path = source.display();
    let message = error.message();
    match error.span().map(|span| line_column(content, span.start)) {
        Some((line, column)) => {
            format!("{path}: TOML parse error at line {line}, column {column}: {message}")
        }
        None => format!("{path}: TOML parse error: {message}"),
    }
}

/// バイトオフセットから 1 始まりの (行, 桁) を求める。桁は文字数で数える
/// (toml crate 自身の表示と同じ数え方)。toml の span はトークン境界を指すので
/// 文字境界に乗らないことはないが、乗らない・範囲外のときは末尾として扱う。
fn line_column(content: &str, offset: usize) -> (usize, usize) {
    let before = content.get(..offset).unwrap_or(content);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |last| last.chars().count()) + 1;
    (line, column)
}

/// `[[<header>]]` の各ブロックを (key, value) の対の列に写す。配列のテーブル
/// 以外の形 (単一テーブルや文字列) は設定誤り。
fn table_blocks(
    value: Option<toml::Value>,
    header: &str,
    file: &str,
) -> Result<Vec<Pairs>, String> {
    let not_array_of_tables = || format!("[[{header}]] must be an array of tables in {file}");
    let blocks = match value {
        None => return Ok(Vec::new()),
        Some(toml::Value::Array(blocks)) => blocks,
        Some(_) => return Err(not_array_of_tables()),
    };
    blocks
        .into_iter()
        .map(|block| match block {
            toml::Value::Table(section) => section
                .into_iter()
                .map(|(key, value)| block_pair(header, file, key, value))
                .collect(),
            _ => Err(not_array_of_tables()),
        })
        .collect()
}

/// ブロック内の 1 項目。キーの形と値の型 (文字列のみ) を検証する。
fn block_pair(header: &str, file: &str, key: String, value: toml::Value) -> Result<Pair, String> {
    if !is_valid_key(&key) {
        return Err(format!("Invalid key in [[{header}]]: {key} in {file}"));
    }
    match value {
        toml::Value::String(s) => Ok((key, s)),
        _ => Err(format!("[[{header}]] {key} must be a string in {file}")),
    }
}

/// ブロック内で使えるキー: 英小文字始まりの英小文字・数字・`_` のみ。
/// tracker ではそのまま環境変数名 (`WSM_TRACKER_<KEY大文字>`) に写す。repo の
/// 予約キーも同じ字種なので両方に適用し、`Name` のような typo を表面化させる。
fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 読み込み済みのマシン設定。コマンドの入口で 1 回作り、各ユースケースに渡す。
pub struct Settings {
    home: PathBuf,
    /// `config.toml`。想定する使い分けは dotfiles が生成・管理する値 (マシン
    /// 導出値と全マシン共通の値。docs/wsm.md「dotfiles との境界」) だが、
    /// server は区別しない。
    base: Document,
    /// `config.local.toml`。想定する使い分けは手書きのマシン・所属固有の値。
    /// 無ければ空。
    overlay: Document,
}

impl Settings {
    /// `~/.config/wsm/` (`XDG_CONFIG_HOME` 準拠) の config.toml と
    /// config.local.toml を読む。どちらも無ければ空として扱う。
    pub fn load(home: &Path) -> Result<Self, String> {
        let dir = config_dir(home);
        Ok(Self {
            home: home.to_owned(),
            base: Document::read(&dir.join("config.toml"))?,
            overlay: Document::read(&dir.join("config.local.toml"))?,
        })
    }

    /// パス導出と RepoStore の基点になる HOME。
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// 設定ファイルのトップレベル値 (overlay が後勝ち)。
    fn scalar(&self, key: &str) -> Option<&str> {
        self.overlay.scalar(key).or_else(|| self.base.scalar(key))
    }

    /// 環境変数 > 設定ファイル、で文字列値を解決する。
    fn value(&self, env: &str, key: &str) -> Option<String> {
        env_override(env).or_else(|| self.scalar(key).map(str::to_owned))
    }

    /// tmux_path / herdr_path を出現順に読む。並びは config.toml の出現順を
    /// 骨格にし、overlay にだけあるキーを末尾に足す。値は overlay が後勝ち
    /// (パスを差し替えても並びは変わらない)。空文字のパスは未設定と同じ扱い。
    pub fn session_managers(&self) -> Managers {
        // マネージャーのキーの出現順 (base → overlay、初出のみ)
        let managers_in_order = self
            .base
            .scalars
            .iter()
            .chain(&self.overlay.scalars)
            .filter_map(|(key, _)| SessionManager::from_path_key(key).map(|m| (m, key.as_str())))
            .fold(Vec::new(), |mut managers, entry| {
                if !managers.contains(&entry) {
                    managers.push(entry);
                }
                managers
            });
        let entries = managers_in_order
            .into_iter()
            .filter_map(|(manager, key)| Some((manager, self.scalar(key)?)))
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
    ///
    /// 2 ファイルの `[[repo]]` は連結する。ns/name の重複は RepoStore が
    /// 検索時に検出する (ghq のリポジトリとの重複もそこでしか分からない)。
    pub fn custom_repos(&self) -> Result<Vec<RepoEntry>, String> {
        self.base
            .repo_blocks()
            .chain(self.overlay.repo_blocks())
            .map(|(source, section)| repo_entry(&self.home, source, section))
            .collect()
    }

    /// [[tracker]] と default_tracker を読む。2 ファイルの `[[tracker]]` は
    /// 連結し、tracker 名は全体で一意であること。
    pub fn trackers(&self) -> Result<Trackers, String> {
        let entries = self
            .base
            .tracker_blocks()
            .chain(self.overlay.tracker_blocks())
            .map(|(source, section)| tracker_entry(&self.home, source, section))
            .collect::<Result<Vec<Tracker>, String>>()?;

        for (i, a) in entries.iter().enumerate() {
            for b in &entries[i + 1..] {
                // tracker 名は全体で一意 (1 ファイル内でも 2 ファイル間でも)。
                // 同名は「どちらの設定か」を決められない (overlay 優先で黙って
                // 置き換えると、意図しない上書きに気づけない)
                if a.name == b.name {
                    return Err(format!(
                        "tracker name duplicated: {} ({}, {})",
                        a.name,
                        file_name_of(&a.source),
                        file_name_of(&b.source)
                    ));
                }
                // ns の割り当ては一意であること (同じ ns を 2 つのトラッカーが
                // 主張したらどちらの世界か決められない)
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

    // 予約キー以外はプラグインへの環境変数 (キーは一意なので衝突しない)
    let env = std::iter::once(("WSM_TRACKER_NAME".to_owned(), name.clone()))
        .chain(
            section
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "name" | "path" | "ns"))
                .map(|(key, value)| (format!("WSM_TRACKER_{}", key.to_uppercase()), value.clone())),
        )
        .collect();

    Ok(Tracker { name, path: expand_tilde(home, &path), ns, env, source: source.to_owned() })
}

/// テーブル内のキーの値 (TOML は同名キーを許さないので一意)。
fn table_value(section: &Pairs, key: &str) -> Option<String> {
    section.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn env_override(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// 設定ファイルの置き場 `$XDG_CONFIG_HOME/wsm` (既定 `~/.config/wsm`)。
fn config_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("wsm")
}
