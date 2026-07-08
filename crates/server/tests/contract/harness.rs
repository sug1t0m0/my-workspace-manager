// 契約テストのハーネス。
//
// テスト対象は既定でビルドしたバイナリ、環境変数 WSM_SERVER_BIN で別の
// ビルド (リリースバイナリ等) に差し替えられる。外部コマンド (ghq, git,
// tmux, herdr, docker, devcontainer) と Tracker プラグイン (tracker) は
// PATH 先頭 / 設定のフェイクに差し替え、テストごとの一時 HOME と合わせて
// 完全に隔離する。
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

const FAKE_COMMANDS: &[&str] = &["ghq", "git", "tmux", "herdr", "docker", "devcontainer", "tracker"];

const FAKE_SCRIPT: &str = r#"#!/bin/sh
# 汎用フェイク: 呼び出しを 1 行でログに記録し、パターン表の最初の一致で応答する。
# HERDR_SOCKET_PATH はセッションのターゲット指定に使われる契約の一部なので、
# 設定されていればログ行の先頭に含める。.once マーカー付きのスタブは一度
# 一致したら消える (呼び出しごとに応答が変わる状況の再現用)。
line="$(basename "$0") $(printf '%s' "$*" | tr '\n' ' ')"
[ -n "${HERDR_SOCKET_PATH:-}" ] && line="HERDR_SOCKET_PATH=$HERDR_SOCKET_PATH $line"
printf '%s\n' "$line" >> "$WSM_TEST_LOG"
for p in "$WSM_TEST_RESPONSES"/*.pattern; do
  [ -e "$p" ] || continue
  if printf '%s\n' "$line" | grep -Eq -- "$(cat "$p")"; then
    base="${p%.pattern}"
    out=""
    [ -f "$base.stdout" ] && out="$base.stdout"
    code=0
    [ -f "$base.exit" ] && code="$(cat "$base.exit")"
    if [ -f "$base.once" ]; then
      [ -n "$out" ] && cat "$out"
      rm -f "$base.pattern" "$base.stdout" "$base.exit" "$base.once"
      exit "$code"
    fi
    [ -n "$out" ] && cat "$out"
    exit "$code"
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

fn server_bin() -> PathBuf {
    std::env::var_os("WSM_SERVER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_wsm-server")))
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
        // 既定のマネージャー設定 (tmux 先頭 = 既定) と Tracker プラグイン
        // (フェイクの tracker)。どちらもフォールバックを持たないため、設定が
        // 無いとそれぞれ open / プロジェクト照会ができない
        env.write_home(
            ".config/wsm/config.toml",
            &format!("{}{}", env.managers_config(&["tmux", "herdr"]), env.tracker_config()),
        );
        // 既定のリポジトリストア: owner/repo が ghq (github.com) にある。
        // 構成を変えるテストは ^ghq list$ を自前の stub() で上書きする
        env.stub_default("^ghq list$", "github.com/owner/repo\n");
        env
    }

    /// パターン (拡張正規表現) に一致する外部コマンド呼び出しに stdout を返す (exit 0)。
    pub fn stub(&self, pattern: &str, stdout: &str) -> &Self {
        self.add_stub(pattern, stdout, None, false)
    }

    /// stub() の exit code 指定版。異常系 (devcontainer up 失敗等) のテストで使う。
    pub fn stub_exit(&self, pattern: &str, stdout: &str, code: i32) -> &Self {
        self.add_stub(pattern, stdout, Some(code), false)
    }

    /// 一度一致したら消えるスタブ。同じ呼び出しへの応答が変化する状況
    /// (セッション起動待ちのポーリング等) の再現に使う。
    pub fn stub_once(&self, pattern: &str, stdout: &str) -> &Self {
        self.add_stub(pattern, stdout, None, true)
    }

    /// 既定スタブ。パターン表は名前の辞書順で最初の一致が使われるため、
    /// zzz- 接頭辞で常に最後に回し、テストの stub() が同じ呼び出しに
    /// 一致するときはそちらを勝たせる。
    pub fn stub_default(&self, pattern: &str, stdout: &str) -> &Self {
        let n = self.stub_count.fetch_add(1, Ordering::SeqCst);
        let base = self.responses_dir().join(format!("zzz-{n:03}"));
        fs::write(base.with_extension("pattern"), pattern).unwrap();
        fs::write(base.with_extension("stdout"), stdout).unwrap();
        self
    }

    /// 一時 HOME 配下にファイルを書く (親ディレクトリも作る)。
    pub fn write_home(&self, rel: &str, content: &str) -> &Self {
        let path = self.home().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        self
    }

    /// wsm-server を隔離環境で実行する。
    pub fn run(&self, args: &[&str]) -> CoreOutput {
        let path = format!(
            "{}:{}",
            self.fakes_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let output = Command::new(server_bin())
            .args(args)
            .env("HOME", self.home())
            .env("PATH", path)
            .env("WSM_TEST_LOG", self.log_file())
            .env("WSM_TEST_RESPONSES", self.responses_dir())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("WSM_SESSION_MANAGER")
            .env_remove("WSM_DEFAULT_DEVCONTAINER_CONFIG")
            .env_remove("WSM_WORKTREE_ROOT")
            .env_remove("WSM_DEVCONTAINER_SHELL")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_ENV")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_TAB_ID")
            .env_remove("HERDR_WORKSPACE_ID")
            .output()
            .expect("run wsm-server");
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

    /// マネージャー設定 (tmux_path / herdr_path) を指定の並び順で生成する。
    /// 並び順が選択順で、先頭が既定。パスはフェイクを指す。
    pub fn managers_config(&self, order: &[&str]) -> String {
        order
            .iter()
            .map(|name| format!("{name}_path = \"{}/{name}\"\n", self.fakes_dir_str()))
            .collect()
    }

    /// フェイクの Tracker プラグイン (tracker) を指す [[tracker]] 設定。
    pub fn tracker_config(&self) -> String {
        format!("[[tracker]]\nname = \"fake\"\npath = \"{}/tracker\"\n", self.fakes_dir_str())
    }

    /// フェイクの置き場 (PATH 先頭)。attach_command のバイナリパス検証に使う。
    pub fn fakes_dir_str(&self) -> String {
        self.fakes_dir().to_str().unwrap().to_owned()
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

    fn add_stub(&self, pattern: &str, stdout: &str, code: Option<i32>, once: bool) -> &Self {
        let n = self.stub_count.fetch_add(1, Ordering::SeqCst);
        let base = self.responses_dir().join(format!("{n:03}"));
        fs::write(base.with_extension("pattern"), pattern).unwrap();
        fs::write(base.with_extension("stdout"), stdout).unwrap();
        if let Some(code) = code {
            fs::write(base.with_extension("exit"), code.to_string()).unwrap();
        }
        if once {
            fs::write(base.with_extension("once"), "").unwrap();
        }
        self
    }
}
