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

/// 引数値の許可文字 (SSH 経由で呼ばれるため入力検証必須)。
pub fn is_valid_arg(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
}
