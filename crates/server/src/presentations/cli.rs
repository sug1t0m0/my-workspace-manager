//! JSON API (CLI) の presentation。
//!
//! 責務は「引数 → ドメイン型への変換」と「結果 → stdout / stderr / exit code」
//! だけで、状態には触れない。パースが検証を兼ねる (RepoRef::parse /
//! is_valid_issue) ため、usecase には正しい形の値しか渡らない。

use crate::usecases::{self, CmdResult};
use std::path::PathBuf;
use std::process::ExitCode;
use wsm_shared::domains::{self as domain, RepoRef, WorkspaceId};

const USAGE: &str = "Usage: wsm-server <list-projects|list-repos|list-issues|list-workspaces|list-devcontainer-configs|list-session-managers|open|remove>";

pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{}", serde_json::json!({ "error": message }));
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> CmdResult {
    let home = std::env::var("HOME").map(PathBuf::from).map_err(|_| "HOME is not set")?;
    let (subcmd, rest) = args.split_first().ok_or(USAGE)?;
    match subcmd.as_str() {
        "list-projects" => usecases::list_projects(flag_value(rest, "--user")),
        "list-repos" => {
            usecases::list_repos(&home, flag_value(rest, "--project"), flag_value(rest, "--user"))
        }
        "list-issues" => usecases::list_issues(&home, &required_repo(rest)?),
        "list-workspaces" => usecases::list_workspaces(&home),
        "list-session-managers" => usecases::list_session_managers(&home),
        "list-devcontainer-configs" => {
            let repo = required_repo(rest)?;
            let id = required_issue(rest)?;
            usecases::list_devcontainer_configs(&home, &repo, &id)
        }
        "open" => {
            let repo = required_repo(rest)?;
            let id = required_issue(rest)?;
            usecases::open(&home, &repo, &id, &flag_values(rest, "--config"))
        }
        "remove" => {
            let repo = required_repo(rest)?;
            let id = required_issue(rest)?;
            usecases::remove(&home, &repo, &id)
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// フラグの値を返す。同名フラグの重複は後勝ち (zsh 版のループ上書きと同じ契約)。
/// 空文字の値は未指定と同じ扱い (zsh 版の [[ -z ]] と同じ契約)。
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .rposition(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .filter(|v| !v.is_empty())
        .cloned()
}

fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == flag)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

/// --repo をパースして RepoRef を得る (パース = 検証)。
fn required_repo(args: &[String]) -> Result<RepoRef, String> {
    let value = flag_value(args, "--repo").ok_or("--repo required")?;
    RepoRef::parse(&value).ok_or_else(|| format!("Invalid repo: {value}"))
}

/// --issue をパースして WorkspaceId を得る (パース = 検証)。
fn required_issue(args: &[String]) -> Result<WorkspaceId, String> {
    let value = flag_value(args, "--issue").ok_or("--issue required")?;
    domain::is_valid_issue(&value)
        .then(|| WorkspaceId::parse(&value))
        .ok_or_else(|| format!("Invalid issue: {value}"))
}
