//! DevContainer ロールの devcontainers/cli + docker 実装。
//! コンテナの同一性は Docker ラベル (wsm.ns-repo / wsm.issue-id / wsm.config)。

use crate::exec;
use std::path::{Path, PathBuf};

/// (ns_repo, id) に一致するコンテナの集約状態: running / stopped / none。
pub fn state(ns_repo: &str, id: &str) -> &'static str {
    let states = exec::stdout_if_ok(
        "docker",
        &[
            "ps",
            "-a",
            "--filter",
            &format!("label=wsm.ns-repo={ns_repo}"),
            "--filter",
            &format!("label=wsm.issue-id={id}"),
            "--format",
            "{{.State}}",
        ],
    );
    match states {
        Some(s) if s.lines().any(|l| l == "running") => "running",
        Some(s) if s.lines().any(|l| !l.is_empty()) => "stopped",
        _ => "none",
    }
}

/// (ns_repo, id) に一致するコンテナをすべて停止・削除する。冪等。
pub fn down(ns_repo: &str, id: &str) {
    let ids = exec::stdout_if_ok(
        "docker",
        &[
            "ps",
            "-a",
            "--filter",
            &format!("label=wsm.ns-repo={ns_repo}"),
            "--filter",
            &format!("label=wsm.issue-id={id}"),
            "--format",
            "{{.ID}}",
        ],
    );
    ids.iter()
        .flat_map(|out| out.lines())
        .filter(|cid| !cid.is_empty())
        .for_each(|cid| exec::run_ignoring_failure("docker", &["rm", "-f", cid]));
}

/// Workspace 内の devcontainer 設定 (設定名, パス)。
/// 順序: primary ("repo") → 名前付き ("repo-<name>"、名前順)。
pub fn repo_configs(workspace: &Path) -> Vec<(String, PathBuf)> {
    let primary = [
        workspace.join(".devcontainer/devcontainer.json"),
        workspace.join(".devcontainer.json"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .map(|p| ("repo".to_owned(), p));

    primary.into_iter().chain(named_configs(workspace)).collect()
}

fn named_configs(workspace: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(workspace.join(".devcontainer")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join("devcontainer.json").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let path = workspace.join(".devcontainer").join(&name).join("devcontainer.json");
            (format!("repo-{name}"), path)
        })
        .collect()
}
