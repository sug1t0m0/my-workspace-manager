//! 外部コマンド実行の薄いラッパ。プロセス起動の副作用はここに閉じ、
//! 呼び出し側は「出力を得る / 成否を見る / 失敗を無視する」の 3 語彙だけを使う。

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Output};

fn output(cmd: impl AsRef<OsStr>, args: &[impl AsRef<OsStr>]) -> Option<Output> {
    Command::new(cmd).args(args).output().ok()
}

fn output_env(
    cmd: impl AsRef<OsStr>,
    args: &[impl AsRef<OsStr>],
    envs: &[(&str, &str)],
) -> Option<Output> {
    Command::new(cmd).args(args).envs(envs.iter().copied()).output().ok()
}

/// 環境変数付きの stdout_if_ok (herdr のセッションターゲット指定などに使う)。
pub fn stdout_if_ok_env(
    cmd: impl AsRef<OsStr>,
    args: &[impl AsRef<OsStr>],
    envs: &[(&str, &str)],
) -> Option<String> {
    output_env(cmd, args, envs)
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// 環境変数付きの run_ignoring_failure。
pub fn run_ignoring_failure_env(
    cmd: impl AsRef<OsStr>,
    args: &[impl AsRef<OsStr>],
    envs: &[(&str, &str)],
) {
    let _ = output_env(cmd, args, envs);
}

/// コマンドが成功したときだけ stdout を返す (失敗・起動不能は None)。
pub fn stdout_if_ok(cmd: impl AsRef<OsStr>, args: &[impl AsRef<OsStr>]) -> Option<String> {
    output(cmd, args)
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

pub fn succeeds(cmd: impl AsRef<OsStr>, args: &[impl AsRef<OsStr>]) -> bool {
    output(cmd, args).is_some_and(|o| o.status.success())
}

/// 失敗してもよい呼び出し (ベストエフォートの後始末などに使う)。
pub fn run_ignoring_failure(cmd: impl AsRef<OsStr>, args: &[impl AsRef<OsStr>]) {
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

/// パスが実行ファイルとして存在するか (list-trackers の installed 判定に使う)。
pub fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
