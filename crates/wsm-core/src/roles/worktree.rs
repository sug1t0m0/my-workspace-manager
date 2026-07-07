//! Worktree ロールの git worktree 実装。Issue Workspace の作業ツリーを扱う
//! (main はクローン本体をそのまま使うため、このロールは関与しない)。

use crate::exec;
use std::path::Path;

/// worktree を作る。ブランチが既存ならそれを使い、なければ作る。
/// `--relative-paths` は DevContainer マウント時にホスト絶対パスへの参照を
/// 避けるため。
pub fn add(ghq_path: &Path, branch: &str, worktree_path: &Path) -> Result<(), String> {
    let ghq = ghq_path.to_string_lossy();
    let worktree = worktree_path.to_string_lossy();
    let branch_ref = format!("refs/heads/{branch}");
    let branch_exists =
        exec::succeeds("git", &["-C", &ghq, "show-ref", "--verify", "--quiet", &branch_ref]);

    let (created, error) = if branch_exists {
        (
            exec::succeeds("git", &["-C", &ghq, "worktree", "add", "--relative-paths", &worktree, branch]),
            "Failed to add worktree",
        )
    } else {
        (
            exec::succeeds("git", &["-C", &ghq, "worktree", "add", "--relative-paths", "-b", branch, &worktree]),
            "Failed to create worktree",
        )
    };
    created.then_some(()).ok_or_else(|| error.to_owned())
}

/// 冪等な削除 (存在しなければ何もしない)。
pub fn remove(ghq_path: &Path, worktree_path: &Path) {
    let ghq = ghq_path.to_string_lossy();
    let worktree = worktree_path.to_string_lossy();
    exec::run_ignoring_failure("git", &["-C", &ghq, "worktree", "remove", &worktree]);
}

/// `git worktree list --porcelain` の出力から (worktree パス, Issue 番号) の
/// 対を取り出す純粋関数。feature/<id> ブランチの worktree のみ対象。
pub fn parse_feature_worktrees(porcelain: &str) -> Vec<(String, String)> {
    porcelain
        .lines()
        .fold((None, Vec::new()), |(current, mut found), line| {
            if let Some(path) = line.strip_prefix("worktree ") {
                (Some(path.to_owned()), found)
            } else if let Some(issue) = line.strip_prefix("branch refs/heads/feature/") {
                if let Some(path) = current {
                    found.push((path, issue.to_owned()));
                }
                (None, found)
            } else if line.is_empty() {
                (None, found)
            } else {
                (current, found)
            }
        })
        .1
}

/// リポジトリの worktree 一覧 (porcelain) を取得する。
pub fn list_porcelain(ghq_path: &Path) -> Option<String> {
    let ghq = ghq_path.to_string_lossy();
    exec::stdout_if_ok("git", &["-C", &ghq, "worktree", "list", "--porcelain"])
}
