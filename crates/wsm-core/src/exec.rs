//! 外部コマンド実行の薄いラッパ。プロセス起動の副作用はここに閉じ、
//! 呼び出し側は「出力を得る / 成否を見る / 失敗を無視する」の 3 語彙だけを使う。

use std::path::PathBuf;
use std::process::{Command, Output};

fn output(cmd: &str, args: &[&str]) -> Option<Output> {
    Command::new(cmd).args(args).output().ok()
}

/// コマンドが成功したときだけ stdout を返す (失敗・起動不能は None)。
pub fn stdout_if_ok(cmd: &str, args: &[&str]) -> Option<String> {
    output(cmd, args)
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

pub fn succeeds(cmd: &str, args: &[&str]) -> bool {
    output(cmd, args).is_some_and(|o| o.status.success())
}

/// 失敗してもよい呼び出し (zsh 版の `|| true` に相当)。
pub fn run_ignoring_failure(cmd: &str, args: &[&str]) {
    let _ = output(cmd, args);
}

/// PATH から実行ファイルを探す (`command -v` 相当)。
pub fn which(cmd: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(cmd))
            .find(|candidate| is_executable(candidate))
    })
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
