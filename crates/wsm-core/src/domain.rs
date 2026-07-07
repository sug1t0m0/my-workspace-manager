//! ドメイン層: Workspace の識別子と導出規則。純粋関数のみで、副作用を持たない。

use std::path::{Path, PathBuf};

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

pub fn ghq_path(home: &Path, ns_repo: &str) -> PathBuf {
    home.join("ghq/github.com").join(ns_repo)
}

pub fn workspace_path(home: &Path, ns_repo: &str, id: &WorkspaceId) -> PathBuf {
    match id {
        WorkspaceId::Main => ghq_path(home, ns_repo),
        WorkspaceId::Issue(issue) => home.join("worktrees/github.com").join(ns_repo).join(issue),
    }
}

pub fn branch_name(issue: &str) -> String {
    format!("feature/{issue}")
}

pub fn session_name(ns_repo: &str, id: &WorkspaceId) -> String {
    let repo_key = ns_repo.replace('/', ".");
    match id {
        WorkspaceId::Main => repo_key,
        WorkspaceId::Issue(issue) => format!("{repo_key}-{issue}"),
    }
}

/// tmux はセッション名の `.` と `:` をターゲット構文予約のため黙って `_` に
/// 置換する。tmux 実装ではドットを `_` にした名前を使う (herdr は canonical)。
pub fn tmux_session_name(ns_repo: &str, id: &WorkspaceId) -> String {
    session_name(ns_repo, id).replace('.', "_")
}

// --- 引数検証 (SSH 経由で呼ばれるため必須) ---
// シェルメタ文字だけでなく、パストラバーサル (..) やオプション注入 (先頭 -) も弾く。

/// repo: `<ns>/<repo>`。ns は GitHub 規則 (英数と `-`。user も org も同じ)、
/// repo は英数・`._-` (先頭 `-` とドットのみは不可)。ns にドットが入らないことで
/// セッション名の `/` → `.` 変換の単射性が保証される。
pub fn is_valid_repo(value: &str) -> bool {
    let segments: Vec<&str> = value.split('/').collect();
    matches!(segments.as_slice(), [ns, repo] if is_valid_user(ns) && is_valid_repo_segment(repo))
}

fn is_valid_repo_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('-')
        && !segment.chars().all(|c| c == '.')
        && segment.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// issue: `main` または数字のみ。
pub fn is_valid_issue(value: &str) -> bool {
    value == "main" || (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
}

/// user: 英数と `-` (先頭は英数)。
pub fn is_valid_user(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// project: 数字のみ (`none` は list-repos が検証の前に処理する)。
pub fn is_valid_project(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}
