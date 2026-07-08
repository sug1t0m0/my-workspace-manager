//! DevContainer ロールの devcontainers/cli + docker 実装。
//! コンテナの同一性は Docker ラベル (wsm.ns-repo / wsm.issue-id / wsm.config)。

use wsm_shared::domains::{self as domain, RepoRef, WorkspaceId};
use crate::infra::exec;
use std::path::{Path, PathBuf};

/// up の冪等契約: 実行前の状態に応じて結果が決まる。
#[derive(Clone, Copy)]
pub enum Outcome {
    Created,
    Started,
    Reused,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Started => "started",
            Self::Reused => "reused",
        }
    }
}

/// (repo, id) に一致するコンテナの集約状態: running / stopped / none。
pub fn state(repo: &RepoRef, id: &str) -> &'static str {
    let ns_repo = repo.ns_repo();
    let states =
        exec::stdout_if_ok("docker", &labels_args(&ns_repo, id, None, "ps", "-a", "{{.State}}"));
    match states {
        Some(s) if s.lines().any(|l| l == "running") => "running",
        Some(s) if s.lines().any(|l| !l.is_empty()) => "stopped",
        _ => "none",
    }
}

/// (repo, id) に一致するコンテナをすべて停止・削除する。冪等。
pub fn down(repo: &RepoRef, id: &str) {
    let ns_repo = repo.ns_repo();
    let ids = exec::stdout_if_ok("docker", &labels_args(&ns_repo, id, None, "ps", "-a", "{{.ID}}"));
    ids.iter()
        .flat_map(|out| out.lines())
        .filter(|cid| !cid.is_empty())
        .for_each(|cid| exec::run_ignoring_failure("docker", &["rm", "-f", cid]));
}

/// devcontainer.json の場所から wsm.config ラベル値を導出する純粋関数。
pub fn config_name(workspace: &Path, config_path: &Path) -> String {
    let ws = workspace.to_string_lossy();
    let cfg = config_path.to_string_lossy();
    if cfg == format!("{ws}/.devcontainer/devcontainer.json") || cfg == format!("{ws}/.devcontainer.json") {
        "repo".to_owned()
    } else {
        cfg.strip_prefix(&format!("{ws}/.devcontainer/"))
            .and_then(|rest| rest.strip_suffix("/devcontainer.json"))
            .filter(|name| !name.is_empty())
            .map(|name| format!("repo-{name}"))
            .unwrap_or_else(|| "default".to_owned())
    }
}

/// (ns_repo, id, config 名) に一致するコンテナの状態。up の outcome 判定に使う。
fn state_for_config(ns_repo: &str, id: &str, cname: &str) -> Option<&'static str> {
    exec::stdout_if_ok("docker", &labels_args(ns_repo, id, Some(cname), "ps", "-a", "{{.State}}"))
        .and_then(|out| match out.lines().next() {
            Some("running") => Some("running"),
            Some(s) if !s.is_empty() => Some("stopped"),
            _ => None,
        })
}

/// 1 つの devcontainer を冪等に起動する。worktree Workspace では git common dir
/// も /workspaces/ 配下にマウントし、コンテナ内パスを WSM_* remote-env で渡す。
/// `--remove-existing-container` は渡さない (再入場時に副作用を保持するため)。
pub fn up(
    paths: &domain::Paths,
    repo: &RepoRef,
    id: &WorkspaceId,
    workspace: &Path,
    config_path: &Path,
    cname: &str,
) -> Result<Outcome, String> {
    exec::which("devcontainer").ok_or("devcontainer CLI not installed")?;
    if !config_path.is_file() {
        return Err(format!("Config not found: {}", config_path.display()));
    }

    let before = state_for_config(&repo.ns_repo(), id.as_str(), cname);
    let args = up_args(paths, repo, id, workspace, config_path, cname);
    if !exec::succeeds("devcontainer", &args) {
        return Err("devcontainer up failed".to_owned());
    }
    Ok(match before {
        Some("running") => Outcome::Reused,
        Some(_) => Outcome::Started,
        None => Outcome::Created,
    })
}

/// devcontainer up の引数列を組み立てる純粋関数 (zsh 版と同一の並び)。
fn up_args(
    paths: &domain::Paths,
    repo: &RepoRef,
    id: &WorkspaceId,
    workspace: &Path,
    config_path: &Path,
    cname: &str,
) -> Vec<String> {
    let base = [
        "up",
        "--workspace-folder",
        &workspace.to_string_lossy(),
        "--config",
        &config_path.to_string_lossy(),
        "--id-label",
        &format!("wsm.ns-repo={}", repo.ns_repo()),
        "--id-label",
        &format!("wsm.issue-id={}", id.as_str()),
        "--id-label",
        &format!("wsm.config={cname}"),
    ]
    .map(str::to_owned);

    let worktree_extras = matches!(id, WorkspaceId::Issue(_))
        .then(|| {
            let container_worktree = format!(
                "/workspaces/{}",
                workspace.strip_prefix(&paths.home).unwrap_or(workspace).display()
            );
            let common_dir = domain::ghq_path(paths, repo).join(".git");
            let container_common = format!(
                "/workspaces/{}",
                common_dir.strip_prefix(&paths.home).unwrap_or(&common_dir).display()
            );
            [
                "--mount-git-worktree-common-dir".to_owned(),
                "--remote-env".to_owned(),
                format!("WSM_WORKTREE_PATH={container_worktree}"),
                "--remote-env".to_owned(),
                format!("WSM_WORKTREE_COMMON_DIR={container_common}"),
            ]
        })
        .into_iter()
        .flatten();

    base.into_iter().chain(worktree_extras).collect()
}

/// 起動済みコンテナに入る exec コマンドを組み立てる (docker・ラベル・remoteUser
/// の知識はここに閉じる)。返り値は (コンテナ ID, コマンド)。コンテナがなければ None。
pub fn exec_command(
    repo: &RepoRef,
    id: &WorkspaceId,
    cname: &str,
    paths: &domain::Paths,
) -> Option<(String, String)> {
    let ns_repo = repo.ns_repo();
    let cid = exec::stdout_if_ok("docker", &labels_args(&ns_repo, id.as_str(), Some(cname), "ps", "-q", ""))?
        .lines()
        .next()
        .filter(|l| !l.is_empty())?
        .to_owned();

    // devcontainer.metadata の remoteUser を尊重する (ファイル所有権をホスト UID に合わせる)
    let remote_user = exec::stdout_if_ok(
        "docker",
        &["inspect", "--format", r#"{{index .Config.Labels "devcontainer.metadata"}}"#, &cid],
    )
    .and_then(|out| serde_json::from_str::<serde_json::Value>(out.trim()).ok())
    .and_then(|metadata| {
        metadata.as_array().and_then(|entries| {
            entries.iter().find_map(|e| e["remoteUser"].as_str().map(str::to_owned))
        })
    });

    // コンテナ内 workdir はホストパスの $HOME 相対 (devcontainers/cli のマウント規則)
    let ws = domain::workspace_path(paths, repo, id);
    let workdir = format!("/workspaces/{}", ws.strip_prefix(&paths.home).unwrap_or(&ws).display());
    let user_part = remote_user.map(|u| format!(" --user '{u}'")).unwrap_or_default();
    let command = format!("docker exec -it{user_part} -w '{workdir}' '{cid}' zsh");
    Some((cid, command))
}

/// docker ps 系の引数列 (ラベルフィルタ) を組み立てる純粋関数。
fn labels_args(
    ns_repo: &str,
    id: &str,
    cname: Option<&str>,
    subcmd: &str,
    mode: &str,
    format: &str,
) -> Vec<String> {
    let filters = [
        Some(format!("label=wsm.ns-repo={ns_repo}")),
        Some(format!("label=wsm.issue-id={id}")),
        cname.map(|c| format!("label=wsm.config={c}")),
    ]
    .into_iter()
    .flatten()
    .flat_map(|f| ["--filter".to_owned(), f]);

    let format_args =
        (!format.is_empty()).then(|| ["--format".to_owned(), format.to_owned()]).into_iter().flatten();

    [subcmd.to_owned(), mode.to_owned()].into_iter().chain(filters).chain(format_args).collect()
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
