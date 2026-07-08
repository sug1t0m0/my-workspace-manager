# my-workspace-manager (wsm)

GitHub の Project / Issue を起点に、リポジトリのワークスペース (git worktree +
ターミナルセッション + DevContainer) を開閉するツール。

設計・仕様の詳細は [docs/wsm.md](docs/wsm.md) を参照。

## 構成

2 層構成で、状態の照会・変更はすべてサーバーに集約する。

```
bin/wsm-server     # サーバー (JSON API)。状態の照会・変更を担う。ホストのみに配置
bin/wsm          # クライアント。fzf 選択・表示整形・ターミナル連携。server の JSON API を経由する
crates/server      # サーバーの Rust 版 (試運転中)。zsh 版と JSON API 互換で並存
crates/shared    # server と将来の client が共有する語彙
```

- **wsm-server**: 状態の照会と変更をすべて担う。入出力は JSON。端末やユーザー対話には
  関与しない。ホストのみに配置される。
- **wsm**: fzf による対話的選択と表示整形、ターミナル連携のみを担う。状態には直接
  触れず、必ず server の JSON API を経由する。ホスト・DevContainer で同一の実装が動き、
  server への到達方法 (Transport) だけが実行環境で変わる。

外部ツールとの連携 (GitHub, ghq, git worktree, tmux/herdr, devcontainers/cli,
Ghostty) はすべてツール非依存の「ロール」(契約) として定義し、各ツールは
ロールの 1 実装として交換・追加できる。詳細は docs/wsm.md の「ロールと実装」節。

## 設定

マシン設定は `~/.config/wsm/config.toml` (XDG_CONFIG_HOME 準拠) に置き、server が読む。
[config.toml.example](config.toml.example) を参照。同名の環境変数はその場の
オーバーライドとして優先される (詳細は docs/wsm.md の Settings / 環境変数 節)。

## 前提ツール

- **ホスト**: tmux または herdr, fzf, jq, gh (GitHub CLI), ghq, git, docker,
  devcontainer (devcontainers/cli)
- **DevContainer**: fzf, jq, ssh

## インストール

[GitHub Releases](https://github.com/sug1t0m0/my-workspace-manager/releases) から
取得する (タグ push で CI がビルド・添付する)。

- `wsm-server-aarch64-apple-darwin` → `~/.local/bin/wsm-server` (ホストのみ)
- `wsm` → `~/.local/bin/wsm` (ホスト・DevContainer 共通)

dotfiles からは chezmoi external で `releases/latest/download/<asset>` を
指定して取得する。配布の出し分け (DevContainer にはクライアント `wsm` のみ、
サーバー `wsm-server` と設定はホストのみ) は取り込み側で行う。

## ライセンス

MIT License。[LICENSE](LICENSE) を参照。
