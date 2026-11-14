# wsm (Workspace Manager)

トラッカーの repo-group / Issue を起点に、リポジトリのワークスペース
(git worktree + ターミナルセッション + DevContainer) を開閉する個人ツール。
サーバー (Rust, `crates/server`) が状態を持ち、クライアント (zsh, `bin/wsm`) は
fzf 選択と表示だけを担う。Tracker は外部コマンドのプラグイン (`crates/tracker-*`)。

## 仕様の正は docs/wsm.md

挙動・JSON API・プラグイン契約・決定事項はすべて docs/wsm.md にある。このファイルは
地図であり、仕様を書き写さない。挙動を変えるときは docs/wsm.md と契約テストの両方を
同じ変更で更新する (テストにない挙動は契約ではない)。

| 知りたいこと | docs/wsm.md で読む節 |
|---|---|
| クレート・ファイルの配置、依存の向き | ファイル構成、レイヤー構成 |
| ロール (Tracker / RepoStore / Worktree / SessionManager / DevContainer / Terminal / Transport) の責務と実装 | ロールと実装、概念モデル |
| wsm-server のサブコマンドと JSON の形、引数検証、既知の制約 | JSON API 契約 (wsm-server) |
| Tracker プラグインの動詞・バージョニング・信頼境界 | Tracker プラグイン契約 |
| config.toml のキー、環境変数との優先順位 | Settings、環境変数 |
| なぜそう決めたか (書き換えの境界、Issue 起点の選択、識別子の形など) | 決定事項 |
| 配布と dotfiles の境界、前提ツール | 配布は GitHub Releases で行う、dotfiles との境界、前提ツール |

## コマンド

- 契約テスト (受け入れ条件): `cargo test --test contract` (3 クレート分が走る)。
  1 クレートだけなら `cargo test -p wsm-server --test contract` のように `-p` で絞る
- UI の回帰テスト: `sh tests/run.sh` (1 本だけなら `sh tests/<file>.sh`)。構文確認: `zsh -n bin/wsm`
- 開発中のサーバーを UI に掴ませる: `PATH="$PWD/target/debug:$PATH" wsm ...`
- 配布はタグ `v*` の push → GitHub Actions が Release に添付 → dotfiles 側が配置する。
  手順は `/release` スキル (ユーザーが起動する)。`~/.local/bin` のバイナリを手で上書きしない

## 規約

- Rust は関数型寄り: 不変データ・式指向・小さな純関数の合成。副作用 (プロセス起動・IO) は
  `infra` と `roles` の境界に寄せる
- テストは Arrange-Act-Assert。`// Arrange` / `// Act` / `// Assert` のコメントで区切る
- 外部コマンド (gh, ghq, git, tmux, herdr, docker, devcontainer) はテストではフェイクに
  差し替える。実コマンドをテストから呼ばない
- ドメイン概念は型で持つ (`ns/repo` は `RepoRef`)。文字列表現は境界でだけ組み立てる。
  引数の解釈と検証は presentations、usecases はドメイン型を受け取る
- 個人・マシン依存の値 (パス、owner 名、ラベル名) をコードとこのファイルに書かない。
  設定は `~/.config/wsm/config.toml` (例: config.toml.example)
- `cargo fmt` / `cargo clippy` は規約・CI に未採用 (採用は別 Issue で決める)。整形だけの
  差分を機能変更に混ぜない
- コミットメッセージは `feat:` / `fix:` / `docs:` / `test:` / `perf:` / `refactor:` /
  `ci:` / `chore:` + 日本語の要約 1 行。本文に背景と理由を書く
- `gh api` での書き込み (mutation、`-X POST` など) は実行前にユーザーの確認を取る。
  他の gh / git の書き込み操作は `.claude/settings.json` で確認必須にしている

## 触る場所ごとの手順

契約テストの書き方、`bin/wsm` のピッカー規約、Tracker プラグインの追加手順は
`.claude/rules/` にあり、該当ファイルを読む・編集するときに自動で読み込まれる。
