# my-workspace-manager (wsm)

GitHub の Project / Issue を起点に、リポジトリのワークスペース (git worktree +
ターミナルセッション + DevContainer) を開閉するツール。

設計・仕様の詳細は [docs/wsm.md](docs/wsm.md) を参照。

## 構成

2 層構成で、状態の照会・変更はすべてロジック層に集約する。

```
bin/wsm-core     # ロジック層 (JSON API)。状態の照会・変更を担う。ホストのみに配置
bin/wsm          # UI 層。fzf 選択・表示整形・ターミナル連携。core の JSON API を経由する
crates/core      # ロジック層の Rust 版 (試運転中)。zsh 版と JSON API 互換で並存
crates/shared    # core と将来の client が共有する語彙
```

- **wsm-core**: 状態の照会と変更をすべて担う。入出力は JSON。端末やユーザー対話には
  関与しない。ホストのみに配置される。
- **wsm**: fzf による対話的選択と表示整形、ターミナル連携のみを担う。状態には直接
  触れず、必ず core の JSON API を経由する。ホスト・DevContainer で同一の実装が動き、
  core への到達方法 (Transport) だけが実行環境で変わる。

外部ツールとの連携 (GitHub, ghq, git worktree, tmux/herdr, devcontainers/cli,
Ghostty) はすべてツール非依存の「ロール」(契約) として定義し、各ツールは
ロールの 1 実装として交換・追加できる。詳細は docs/wsm.md の「ロールと実装」節。

## 設定

マシン設定は `~/.config/wsm/config.toml` (XDG_CONFIG_HOME 準拠) に置き、core が読む。
[config.toml.example](config.toml.example) を参照。同名の環境変数はその場の
オーバーライドとして優先される (詳細は docs/wsm.md の Settings / 環境変数 節)。

## 前提ツール

- **ホスト**: tmux または herdr, fzf, jq, gh (GitHub CLI), ghq, git, docker,
  devcontainer (devcontainers/cli)
- **DevContainer**: fzf, jq, ssh

## インストール

TBD。当面は dotfiles 側から取り込む (方式は検討中)。手動で使う場合は
`bin/wsm` / `bin/wsm-core` を PATH の通ったディレクトリ (例: `~/.local/bin`) へ配置する。
配布の出し分け (DevContainer には UI の `wsm` のみ、ロジック層 `wsm-core` と設定は
ホストのみ) は取り込み側で行う。

## ライセンス

MIT License。[LICENSE](LICENSE) を参照。
