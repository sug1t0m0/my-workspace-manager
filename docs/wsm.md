# wsm (Workspace Manager)

GitHub の Project / Issue を起点に、リポジトリのワークスペース(git worktree +
ターミナルセッション + DevContainer)を開閉するツール。

このドキュメントは、将来の Rust / Go への書き換えに向けて、現在 zsh 実装に
暗黙に埋まっている概念と契約を明文化したもの。書き換え時はここに書かれた
概念語彙・JSON API・決定事項が仕様となる。

## ファイル構成

```
dot_local/bin/executable_wsm-core   # ロジック層 (JSON API)。ホストのみに配置
dot_local/bin/executable_wsm        # UI 層。ホスト・DevContainer 共通の単一実装
dot_config/wsm/config.toml.tmpl     # マシン設定 (TOML)。ホストのみに配置
```

レイヤーは 2 つ。

- **ロジック層 (wsm-core)**: 状態の照会と変更をすべて担う。入出力は JSON。
  端末やユーザー対話には関与しない。ホストのみに配置される。
- **UI 層 (wsm)**: fzf による対話的選択と表示整形、ターミナル
  (タブ・セッションアタッチ)連携のみを担う。状態には直接触れず、
  必ず core の JSON API を経由する。ホスト・DevContainer で同一の実装が動き、
  core への到達方法 (Transport) だけが実行環境で変わる。

## 概念モデル

### Workspace

中心となるエンティティ。`(ns_repo, id)` の組で一意に識別される。

- `ns_repo`: GitHub の `<namespace>/<repo>` 形式
- `id`: `main`(リポジトリ本体)または Issue 番号(worktree)

`id` によって実体と配置が決まる。

| id | 実体 | パス | ブランチ |
|---|---|---|---|
| `main` | ghq クローン本体 | `~/ghq/github.com/<ns_repo>` | (そのまま) |
| Issue 番号 | git worktree | `~/worktrees/github.com/<ns_repo>/<id>` | `feature/<id>` |

パス・ブランチ名の導出は core のみが行う。UI 層は導出規則を持たず、
core の応答から受け取る。

> 書き換え時の注意: `main` は ID 空間に混ざった番兵値。型のある言語では
> `Main | Issue(番号)` のような直和型で表現する。

### Target

UI 層でユーザーが Workspace を指定するための記法。

- `<ns>/<repo>#<id>`: 完全指定
- `<id>` のみ: カレントディレクトリの git remote (origin) から `ns_repo` を解決

### Session

Workspace に紐づくターミナルセッション。セッションマネージャーの実装は
交換可能で、`WSM_SESSION_MANAGER` 環境変数(既定: `tmux`)で選択する。
現在の実装は tmux と herdr。

セッションマネージャーが満たすべき操作(書き換え時の interface / trait 境界):

- 存在確認 (exists)
- 作成 (ensure: なければ作業ディレクトリを指定して作る)
- 破棄 (remove)
- 一覧 (names)

セッション名は `(ns_repo, id)` から導出する。

| id | セッション名 |
|---|---|
| `main` | `<ns>.<repo>` (スラッシュをドットに置換) |
| Issue 番号 | `<ns>.<repo>-<id>` |

> legacy 互換: 旧形式 `<ns>/<repo>(-<id>)` の tmux セッションは移行期間中のみ
> 検出・アタッチ・削除の対象とする。新規作成は常に新形式(決定事項を参照)。

### DevContainer

Workspace 上で起動するコンテナ。1 つの Workspace に複数の設定
(devcontainer.json)を同時に立てられる。コンテナの同一性は Docker ラベルの
3 つ組で管理する。

| ラベル | 値 |
|---|---|
| `wsm.ns-repo` | `ns_repo` |
| `wsm.issue-id` | `id` |
| `wsm.config` | 設定名 (下記) |

設定名は devcontainer.json の場所から導出する。

| 場所 | 設定名 |
|---|---|
| `<ws>/.devcontainer/devcontainer.json` または `<ws>/.devcontainer.json` | `repo` |
| `<ws>/.devcontainer/<name>/devcontainer.json` | `repo-<name>` |
| 上記以外 (フォールバック。`WSM_DEFAULT_DEVCONTAINER_CONFIG` で変更可) | `default` |

状態は `running` / `stopped` / `none` の 3 値。

起動 (up) は冪等で、実行前の状態に応じて結果が決まる:

| 実行前の状態 | 動作 | 結果 |
|---|---|---|
| none | ビルドして起動 | `created` |
| stopped | 再起動 | `started` |
| running | 何もしない | `reused` |

`--remove-existing-container` は渡さない。ボリュームや onCreate の副作用を
再入場時に保持するため。

worktree の Workspace では、worktree 本体と git common dir の両方を
`/workspaces/` 配下に $HOME 相対パスを保ってマウントし、コンテナ内パスを
`WSM_WORKTREE_PATH` / `WSM_WORKTREE_COMMON_DIR` として remote-env で渡す。

tmux 使用時は、起動したコンテナへ `docker exec` で入る専用ウィンドウ (🐳) を
セッションに追加する。重複防止はコンテナ ID を pane オプション `@wsm_cid` に
記録して行う。ウィンドウを閉じればオプションも消えるため、次回 open で
自動再作成される。

### Transport

UI 層から core への到達方法。`WSM_TRANSPORT` で明示指定がなければ、
`wsm-core` が PATH にあるかどうかで自動判別する。

- `local`: `wsm-core` を直接実行 (ホスト)
- `ssh`: SSH (`host.docker.internal`) 越しに `wsm-core` を実行 (DevContainer)

UI 層のロジックは transport に依存しない。分岐するのは core の呼び出し方と、
ホスト固有機能 (Terminal アダプタ、セッションマネージャー選択) の有効判定のみ。

制約: 環境変数は SSH を越えないため、`WSM_SESSION_MANAGER` の指定は
`ssh` transport では反映されない (ホスト側の既定が使われる)。

### Settings

マシン設定はホスト側の設定ファイル `~/.config/wsm/config.toml`
(`XDG_CONFIG_HOME` 準拠) に置き、core が読む。core は常にホストで動くため、
transport にかかわらず同じ設定が見える。フォーマットは TOML
(Rust の設定エコシステム標準。キーは snake_case でそのまま struct にマップできる)。

| キー | 内容 |
|---|---|
| `session_manager` | 既定のセッションマネージャー |
| `default_devcontainer_config` | フォールバック devcontainer 設定のパス |

優先順位: 環境変数 > 設定ファイル > 組み込み既定値。環境変数は
「その場のオーバーライド」(UI の `-m` フラグ等) にのみ使う。

設定ファイルに移せないものが 2 種ある。

- 接続情報 (`WSM_HOST` / `HOST_USER` / `HOST_SSH_KEY`): コンテナ内の UI が
  ホストへ到達する前に必要な値のため、DevContainer 側の環境変数で与える
- `WSM_TRANSPORT`: UI が core に到達する方法の指定であり、core の設定では
  ないため

### Terminal

ワークスペースを開いたあと、セッションにアタッチしたタブを端末エミュレーターに
開かせるアダプタ。セッションマネージャー・Transport と同格の交換可能なロールで、
UI 層に属する。現在の実装は Ghostty (osascript 経由、macOS のみ)。

ホスト以外(Transport が SSH のとき)ではアタッチできないため、Terminal は
何もしない。DevContainer からの open は「ホスト側にセッションを用意する」
ところまでが責務。

## JSON API 契約 (wsm-core)

すべてのサブコマンドは JSON を stdout に返す。エラー時は
`{"error": "<message>"}` を stderr に出して非ゼロで終了する。

### 照会系

`list-projects [--user <user>]`
→ open な GitHub Project の一覧。`--user` 省略時は core が gh で自己解決する。
```json
[{"number": 1, "title": "..."}]
```

`list-repos --project <number|none> [--user <user>]`
→ リポジトリ一覧。`none` は ghq 管理下の全リポジトリ、番号指定時は
その Project に属し ghq 管理下にもあるもの(`--user` は番号指定時のみ使用、
省略時は自己解決)。`active_count` はアクティブな Workspace 数。
```json
[{"ns_repo": "owner/repo", "active_count": 0}]
```

`list-issues --repo <ns_repo>`
→ `main` + open な Issue + 孤児 worktree(closed だがセッションが残っているもの)。
```json
[{"id": "main", "title": "...", "active": false, "closed": false, "devcontainer": "none"}]
```

`list-workspaces`
→ 全リポジトリ横断のアクティブ Workspace 一覧。スキーマは list-issues に
`ns_repo` を加えたもの。

`list-devcontainer-configs --repo <ns_repo> --issue <id>`
→ その Workspace で選択可能な devcontainer 設定の一覧。
```json
[{"name": "repo", "path": "/path/to/devcontainer.json", "source": "repo"}]
```

### 変更系

`open --repo <ns_repo> --issue <id> [--config <path>]...`
→ Workspace を開く。worktree・セッションを必要に応じて作成し、
`--config` があれば DevContainer も起動する。`session` / `path` / `manager` は
UI 層がアタッチに使う (UI 層は導出規則も設定も持たない)。
```json
{"status": "ok", "message": "...", "session": "owner.repo-123", "path": "/Users/me/worktrees/github.com/owner/repo/123", "manager": "tmux"}
```

`remove --repo <ns_repo> --target <id>`
→ セッション・DevContainer を破棄。worktree の場合は worktree も削除する。
```json
{"status": "ok", "message": "..."}
```

### 共通仕様

- `active`: セッションが存在するか
- `closed`: GitHub Issue が closed か
- `devcontainer`: `running` / `stopped` / `none`
- 引数値は `[a-zA-Z0-9/_.-]+` のみ許可(SSH 経由で呼ばれるため入力検証必須)

## 決定事項

### JSON API を書き換えの境界とする

Rust / Go 版はまず wsm-core を置き換える。上記 JSON API を仕様として実装し、
UI 層は変更なしで差し替えられること。UI まで取り込むかは core 置き換え後に
判断する。

### legacy tmux セッション名は移行期間を経て退役する

旧形式 `<ns>/<repo>(-<id>)` のセッション名は次の段階で退役する。

1. 移行期間(現在): 新規作成は常に新形式。既存の旧形式セッションは
   検出・アタッチ・削除の対象として使い続けられる
2. 旧形式セッションがすべて閉じられた時点で、互換コードを現行 zsh 版から削除する

書き換えは互換コード削除後に行い、Rust / Go 版には旧形式を持ち込まない。

### UI は単一の実行ファイルにする

wsm.tmpl (zshrc 関数) と wsm-client の二重実装を、単一の実行ファイル `wsm` に
統合する。`wsm` はシェル状態に触れないため、shell 関数である必要がない。

- transport は自動判別する: `wsm-core` が PATH にあればローカル実行、
  なければ SSH。`WSM_TRANSPORT` で明示指定も可能
- ホスト固有の処理 (Terminal アダプタ) のみ実行環境で分岐する
- zshrc 側には環境変数の設定だけを残す

### 導出ロジックは core に集約する

パス・セッション名・ブランチ名の導出は core のみが持つ。UI 層に残っている
重複導出(セッション名、Workspace パス)は core の応答参照に置き換える。
Target 解決だけはカレントディレクトリに依存するため UI 層の責務とする。

### gh の呼び出しは core に閉じる

UI 層は gh を直接呼ばない。GitHub ユーザーの解決は core が行う
(`--user` 省略時に core が自己解決)。これにより DevContainer からの
SSH ホワイトリストは `wsm-core` エントリだけで足りる。

### 個人・マシン依存の値はツールに焼き込まない

フォールバック devcontainer 設定のパスなど、個人・マシン依存の値は
環境変数として dotfiles 側で設定する。ツール側の既定値は「なし」。

### パス配置とブランチ規則は規約として固定する

以下は設定にせず、ツールの規約とする(個人ツールであり、可変にする
メンテナンスコストに見合わないため)。

- リポジトリは `~/ghq/github.com/` 配下 (github.com のみ対応)
- worktree は `~/worktrees/github.com/` 配下
- worktree のブランチは `feature/<id>`

## dotfiles との境界

wsm を専用リポジトリへ移した後も dotfiles に残るもの。この一覧が
wsm リポジトリと dotfiles の契約になる。

| dotfiles に残るもの | 内容 |
|---|---|
| インストール導線 | wsm の実行ファイルを `~/.local/bin` に配置する (現在は chezmoi、移設後はインストールスクリプト等) |
| 設定ファイルの配置 | `~/.config/wsm/config.toml` (マシン・個人依存の値) |
| SSH ホワイトリスト | `allowed-commands.sh` の `wsm-core` エントリ。wsm が必要とするのはこれのみ |
| SSH 鍵の配置 | DevContainer → ホストの鍵 (`~/.ssh/devcontainer`) |
| 配布の出し分け | DevContainer には UI (`wsm`) のみ配り、ロジック層 (`wsm-core`) と設定ファイルは配らない |

## 書き換え前の残タスク

- legacy セッション名互換コードの削除 (旧形式セッションの移行完了後)

その他の整理 (gh 直呼びの解消、個人依存既定値の除去、UI 層の統合、
導出ロジックの core への集約) は適用済み。

## 環境変数

マシン設定は config.toml が正 (Settings 節を参照)。環境変数の役割は
その場のオーバーライドと、設定ファイルに移せない値に限る。

| 変数 | 既定値 | 用途 |
|---|---|---|
| `WSM_SESSION_MANAGER` | (config.toml) | セッションマネージャーのオーバーライド。UI の `-m` / fzf 選択が export する |
| `WSM_DEFAULT_DEVCONTAINER_CONFIG` | (config.toml) | フォールバック devcontainer 設定のオーバーライド |
| `WSM_TRANSPORT` | 自動判別 | core への到達方法の明示指定 (`local` / `ssh`) |
| `WSM_HOST` | `host.docker.internal` | (ssh transport) SSH 接続先 |
| `HOST_USER` | なし (ssh transport では必須) | (ssh transport) SSH ユーザー |
| `HOST_SSH_KEY` | `devcontainer` | (ssh transport) `~/.ssh/` 配下の鍵名 |

## 前提ツール

**ホスト**: tmux または herdr, fzf, jq, gh (GitHub CLI), ghq, git,
docker, devcontainer (devcontainers/cli)

**DevContainer**: fzf, jq, ssh
