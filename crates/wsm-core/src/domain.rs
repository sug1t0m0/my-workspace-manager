//! ドメイン層: Workspace の識別子と導出規則。純粋関数のみで、副作用を持たない。

use std::path::{Path, PathBuf};

/// リポジトリの識別子。ns (user / organization) と repo は別の概念なので
/// 分けて保持し、`<ns>/<repo>` の文字列表現は境界 (JSON・Docker ラベル・
/// gh への引数) でだけ組み立てる。パースが入力検証を兼ねるため、
/// 不正な形の RepoRef はそもそも作れない。
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RepoRef {
    ns: String,
    repo: String,
}

impl RepoRef {
    /// `<ns>/<repo>` をパースする。ns は GitHub 規則 (英数と `-`。user も
    /// org も同じ)、repo は英数・`._-` (先頭 `-` とドットのみは不可)。
    /// ns にドットが入らないことで、セッション名の導出 (`<ns>.<repo>`) が
    /// 単射になる。
    pub fn parse(value: &str) -> Option<Self> {
        match value.split('/').collect::<Vec<_>>().as_slice() {
            [ns, repo] if is_valid_user(ns) && is_valid_repo_name(repo) => {
                Some(Self { ns: (*ns).to_owned(), repo: (*repo).to_owned() })
            }
            _ => None,
        }
    }

    pub fn ns(&self) -> &str {
        &self.ns
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// 外部表現 `<ns>/<repo>`。
    pub fn ns_repo(&self) -> String {
        format!("{}/{}", self.ns, self.repo)
    }
}

fn is_valid_repo_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.chars().all(|c| c == '.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Workspace の id。`main` はリポジトリ本体、それ以外は Issue 番号の worktree。
#[derive(Clone, PartialEq, Eq)]
pub enum WorkspaceId {
    Main,
    Issue(String),
}

impl WorkspaceId {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "main" => Self::Main,
            issue => Self::Issue(issue.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Main => "main",
            Self::Issue(issue) => issue,
        }
    }
}

pub fn ghq_path(home: &Path, repo: &RepoRef) -> PathBuf {
    home.join("ghq/github.com").join(repo.ns()).join(repo.repo())
}

pub fn workspace_path(home: &Path, repo: &RepoRef, id: &WorkspaceId) -> PathBuf {
    match id {
        WorkspaceId::Main => ghq_path(home, repo),
        WorkspaceId::Issue(issue) => {
            home.join("worktrees/github.com").join(repo.ns()).join(repo.repo()).join(issue)
        }
    }
}

pub fn branch_name(issue: &str) -> String {
    format!("feature/{issue}")
}

/// herdr のセッション名 `<ns>.<repo>`。GitHub の namespace にドットが使えない
/// ため、最初のドットが常に区切りとなり単射になる。
pub fn herdr_session_name(repo: &RepoRef) -> String {
    format!("{}.{}", repo.ns(), repo.repo())
}

/// herdr の workspace ラベル。main はリポジトリ名 (アタッチ時に見える名前)、
/// Issue は Issue 番号。
pub fn herdr_workspace_label(repo: &RepoRef, id: &WorkspaceId) -> String {
    match id {
        WorkspaceId::Main => repo.repo().to_owned(),
        WorkspaceId::Issue(issue) => issue.clone(),
    }
}

/// tmux はセッション名の `.` と `:` をターゲット構文予約のため黙って `_` に
/// 置換する。tmux 実装では区切りをすべて `_` に統一した名前を使う。
/// `<ns>_<repo>` / `<ns>_<repo>_<id>`
pub fn tmux_session_name(repo: &RepoRef, id: &WorkspaceId) -> String {
    let repo_key = format!("{}_{}", repo.ns(), repo.repo().replace('.', "_"));
    match id {
        WorkspaceId::Main => repo_key,
        WorkspaceId::Issue(issue) => format!("{repo_key}_{issue}"),
    }
}

// --- 引数検証 (SSH 経由で呼ばれるため必須) ---
// repo の検証は RepoRef::parse が兼ねる。

/// issue: `main` または数字のみ。
pub fn is_valid_issue(value: &str) -> bool {
    value == "main" || (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
}

/// user: 英数と `-` (先頭は英数)。GitHub の user / org 名の規則。
pub fn is_valid_user(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// project: 数字のみ (`none` は list-repos が検証の前に処理する)。
pub fn is_valid_project(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}
