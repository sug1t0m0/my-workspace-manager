// wsm-core: Workspace Manager core (JSON API)
// 仕様は docs/wsm.md。zsh 版 (bin/wsm-core) と同一の JSON API を実装し、
// UI 層 (bin/wsm) を無変更で差し替えられることをゴールとする。
//
// レイヤー: main (入出力の副作用) → commands (オーケストレーション)
//           → roles (外部ツール連携) / domain (純粋な導出規則)

mod commands;
mod domain;
mod exec;
mod roles;
mod settings;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage: wsm-core <list-projects|list-repos|list-issues|list-workspaces|list-devcontainer-configs|open|remove>";

fn main() -> ExitCode {
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

fn run(args: &[String]) -> commands::CmdResult {
    let home = std::env::var("HOME").map(PathBuf::from).map_err(|_| "HOME is not set")?;
    let (subcmd, rest) = args.split_first().ok_or(USAGE)?;
    match subcmd.as_str() {
        "list-projects" => commands::list_projects(rest),
        "list-issues" => commands::list_issues(&home, rest),
        "list-devcontainer-configs" => commands::list_devcontainer_configs(&home, rest),
        "open" => commands::open(&home, rest),
        "remove" => commands::remove(&home, rest),
        "list-repos" | "list-workspaces" => {
            Err(format!("{subcmd} is not yet implemented in the Rust port"))
        }
        _ => Err(USAGE.to_owned()),
    }
}
