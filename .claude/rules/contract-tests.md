---
paths:
  - "crates/**/tests/**"
---

# 契約テストの書き方

一般規約 (AAA、外部コマンドはフェイク、挙動変更は docs と契約テストを同時更新) は
CLAUDE.md にある。ここはハーネス固有の情報だけ。

## wsm-server (`crates/server/tests/contract/`)

ハーネスは `harness.rs`。`TestEnv::new()` が一時 HOME と PATH 先頭のフェイク
(ghq, git, tmux, herdr, docker, devcontainer, tracker, tracker2, ps) を用意する。

- 検証の道具: `out.stdout_json()` / `out.stderr_json()` (パースして `json!` と意味比較。
  整形の差は契約に含めない)、`env.invocations()` (外部コマンドの呼び出しログ。
  引数列も契約の一部)
- 失敗の契約は「stderr にちょうど 1 つの `{"error": ...}` + 非ゼロ終了」。`stderr_json()` は
  stderr 全体を 1 つの JSON としてパースするので、無言の失敗も 2 行出力も落ちる
- 既定の設定: `tmux_path` / `herdr_path` はフェイクを指し (tmux が先頭 = 既定)、
  `[[tracker]]` はフェイク `tracker` (名前 `fake`)。`ghq list` は
  `github.com/owner/repo` を返す既定 stub。構成を変えるテストは
  `env.write_home(".config/wsm/config.toml", ...)` と `env.stub("^ghq list$", ...)` で上書きする
- stub の種類: `stub(pattern, stdout)` (exit 0)、`stub_exit(pattern, stdout, code)` (異常系)、
  `stub_once(pattern, stdout)` (一度一致したら消える。ポーリング待ちの再現用)、
  `stub_default` (zzz- 接頭辞で常に最後に評価される既定)
- パターンは拡張正規表現で、ログ行 `<コマンド名> <引数...>` に対して辞書順で最初の一致が
  使われる。herdr のセッションターゲット (`HERDR_SOCKET_PATH=...`) とトラッカーの拡張キー
  (`WSM_TRACKER_<KEY>=...`。常に付く `WSM_TRACKER_NAME` は除く) は行頭に付くので、
  必要ならパターンに含める
- どのパターンにも一致しない呼び出しは「出力なし・exit 1」(外部コマンドの起動失敗相当)。
  照会系はそれでも縮退して成功する契約、変更系は error JSON で失敗する契約
- `WSM_SERVER_BIN=<path>` で別ビルド (リリースバイナリ等) に差し替えて同じスイートを回せる

## Tracker プラグイン (`crates/tracker-*/tests/contract.rs`)

- JSON の検証は `out.stdout_json()` のみ (`stderr_json()` は無い)。失敗の契約は server と違い「非ゼロ + stdout に部分 JSON を
  出さない」で、stderr は平文の診断 (未知の動詞なら `Usage:` で始まる)
- gh 版: gh を PATH 先頭のフェイクに差し替え、パターン表で応答する (server と同じ方式。
  `env.invocations()` で gh との会話を検証できる)
- api 版: `ApiServer::serve(vec![(追加ヘッダ, 本文)])` で使い捨て HTTP サーバを
  `127.0.0.1:0` に立て、`WSM_TRACKER_GITHUB_API_URL` で向ける。応答はキューの順に返り、
  `server.requests()` でリクエスト本文 (GraphQL のクエリ・変数) を検証する。
  gh はトークンを返すだけのフェイク (`TestEnv::new()`) か失敗するフェイク (`with_broken_gh()`)。
  ローカルポートを使うため、Claude Code のサンドボックスでは `.claude/settings.json` の
  `allowLocalBinding` が必要 (設定済み)

## 実行

- `-p <crate>` で 1 クレートに絞る。1 件だけなら末尾に `-- <テスト名>`
- server の契約テストは全件で 1 分前後かかる
