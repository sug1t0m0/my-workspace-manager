// 契約テストのハーネス。
//
// テスト対象は既定でビルドした Rust 版バイナリ、環境変数 WSM_CORE_BIN で
// zsh 版 (bin/wsm-core) 等に差し替えられる。外部コマンド (gh, ghq, git,
// tmux, herdr, docker, devcontainer) は PATH 先頭のフェイクに差し替え、
// テストごとの一時 HOME と合わせて完全に隔離する。
//
// フェイクの応答はテストが stub() で登録するパターン表 (拡張正規表現 →
// stdout / exit code) で決まり、最初に一致したものが使われる。どのパターン
// にも一致しない呼び出しは「出力なし・exit 1」に倒す。すべての呼び出しは
// ログに記録され、invocations() で検証できる。

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const FAKE_COMMANDS: &[&str] = &["gh", "ghq", "git", "tmux", "herdr", "docker", "devcontainer"];

const FAKE_SCRIPT: &str = r#"#!/bin/sh
# 汎用フェイク: 呼び出しを 1 行でログに記録し、パターン表の最初の一致で応答する。
line="$(basename "$0") $(printf '%s' "$*" | tr '\n' ' ')"
printf '%s\n' "$line" >> "$WSM_TEST_LOG"
for p in "$WSM_TEST_RESPONSES"/*.pattern; do
  [ -e "$p" ] || continue
  if printf '%s\n' "$line" | grep -Eq -- "$(cat "$p")"; then
    base="${p%.pattern}"
    [ -f "$base.stdout" ] && cat "$base.stdout"
    [ -f "$base.exit" ] && exit "$(cat "$base.exit")"
    exit 0
  fi
done
exit 1
"#;

pub struct TestEnv {
    root: tempfile::TempDir,
    stub_count: AtomicUsize,
}

pub struct CoreOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CoreOutput {
    pub fn stdout_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n--- stdout ---\n{}", self.stdout))
    }

    pub fn stderr_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stderr)
            .unwrap_or_else(|e| panic!("stderr is not JSON: {e}\n--- stderr ---\n{}", self.stderr))
    }
}

fn core_bin() -> PathBuf {
    std::env::var_os("WSM_CORE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_wsm-core")))
}

impl TestEnv {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("create tempdir");
        let env = Self { root, stub_count: AtomicUsize::new(0) };

        fs::create_dir_all(env.home()).unwrap();
        fs::create_dir_all(env.responses_dir()).unwrap();
        fs::create_dir_all(env.fakes_dir()).unwrap();
        fs::write(env.log_file(), "").unwrap();
        FAKE_COMMANDS.iter().for_each(|cmd| env.install_fake(cmd));
        env
    }

    /// パターン (拡張正規表現) に一致する外部コマンド呼び出しに stdout を返す (exit 0)。
    pub fn stub(&self, pattern: &str, stdout: &str) -> &Self {
        self.add_stub(pattern, stdout, None)
    }

    /// stub() の exit code 指定版。異常系 (devcontainer up 失敗等) のテストで使う。
    #[allow(dead_code)]
    pub fn stub_exit(&self, pattern: &str, stdout: &str, code: i32) -> &Self {
        self.add_stub(pattern, stdout, Some(code))
    }

    /// 一時 HOME 配下にファイルを書く (親ディレクトリも作る)。
    pub fn write_home(&self, rel: &str, content: &str) -> &Self {
        let path = self.home().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        self
    }

    /// wsm-core を隔離環境で実行する。
    pub fn run(&self, args: &[&str]) -> CoreOutput {
        let path = format!(
            "{}:{}",
            self.fakes_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let output = Command::new(core_bin())
            .args(args)
            .env("HOME", self.home())
            .env("PATH", path)
            .env("WSM_TEST_LOG", self.log_file())
            .env("WSM_TEST_RESPONSES", self.responses_dir())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("WSM_SESSION_MANAGER")
            .env_remove("WSM_DEFAULT_DEVCONTAINER_CONFIG")
            .output()
            .expect("run wsm-core");
        CoreOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// 記録された外部コマンド呼び出し (1 呼び出し 1 行)。
    pub fn invocations(&self) -> Vec<String> {
        fs::read_to_string(self.log_file())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    pub fn home_str(&self) -> String {
        self.home().to_str().unwrap().to_owned()
    }

    fn fakes_dir(&self) -> PathBuf {
        self.root.path().join("fakes")
    }

    fn responses_dir(&self) -> PathBuf {
        self.root.path().join("responses")
    }

    fn log_file(&self) -> PathBuf {
        self.root.path().join("invocations.log")
    }

    fn install_fake(&self, cmd: &str) {
        let path = self.fakes_dir().join(cmd);
        fs::write(&path, FAKE_SCRIPT).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn add_stub(&self, pattern: &str, stdout: &str, code: Option<i32>) -> &Self {
        let n = self.stub_count.fetch_add(1, Ordering::SeqCst);
        let base = self.responses_dir().join(format!("{n:03}"));
        fs::write(base.with_extension("pattern"), pattern).unwrap();
        fs::write(base.with_extension("stdout"), stdout).unwrap();
        if let Some(code) = code {
            fs::write(base.with_extension("exit"), code.to_string()).unwrap();
        }
        self
    }
}
