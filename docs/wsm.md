# wsm (Workspace Manager)

GitHub の Project / Issue を起点に、リポジトリのワークスペース(git worktree +
ターミナルセッション + DevContainer)を開閉するツール。

このドキュメントは、将来の Rust / Go への書き換えに向けて、現在 zsh 実装に
暗黙に埋まっている概念と契約を明文化したもの。書き換え時はここに書かれた
概念語彙・JSON API・決定事項が仕様となる。

外部ツールとの連携はすべてツール非依存の「ロール」(契約)として定義する。
GitHub・ghq・git worktree・tmux・herdr・devcontainers/cli・Ghostty といった
具体的なツールは、それぞれのロールの 1 実装に過ぎない(ロールと実装 節)。

## ファイル構成

```
bin/wsm-core          # ロジック層 (JSON API)。zsh 版 (現行)。ホストのみに配置
bin/wsm               # UI 層。ホスト・DevContainer 共通の単一実装
crates/wsm-core       # ロジック層の Rust 版 (開発中)。zsh 版と JSON API 互換
config.toml.example   # マシン設定 (TOML) のサンプル。実体は ~/.config/wsm/config.toml
```

## レイヤー構成

配置単位(実行ファイル)は 2 つ。

- **ロジック層 (wsm-core)**: 状態の照会と変更をすべて担う。入出力は JSON。
  端末やユーザー対話には関与しない。ホストのみに配置される。
- **UI 層 (wsm)**: fzf による対話的選択と表示整形、ターミナル
  (タブ・セッションアタッチ)連携のみを担う。状態には直接触れず、
  必ず core の JSON API を経由する。ホスト・DevContainer で同一の実装が動き、
  core への到達方法 (Transport) だけが実行環境で変わる。

内部の設計レイヤーは 4 つに分かれる。

```
UI 層 (wsm)            fzf 選択 / Target 解決 / Terminal・Transport ロール
  │  JSON API (Transport 経由)
オーケストレーション層   ロールの合成。open / remove の手順、合成ビュー
ドメイン層              Workspace 識別子と導出規則 (パス・ブランチ・セッション名・ラベル)
ロール層                Tracker / RepoStore / Worktree / SessionManager / DevContainer の実装
```

- ロール実装は互いを知らない。合成(依存の順序)を知るのは
  オーケストレーション層だけ(open / remove の手順と合成ビュー 節)
- 導出規則はドメイン層に集約する。ロール実装は導出済みの名前・パスを
  受け取るだけ。実装を交換しても識別子が変わってはならないため、
  導出をロール実装側に持たせない

## ロールと実装

原則: **特定ツールでの実装は、ロールの 1 実装に過ぎない。**

| ロール | 責務 | 層 | 現在の実装 | 追加の例 |
|---|---|---|---|---|
| Tracker | Project / リポジトリ / Issue の照会 | core | GitHub (gh) | Jira, Linear, GitLab Issues |
| RepoStore | ローカルクローンの列挙とパス解決 | core | ghq | 任意の配置規約 |
| Worktree | 作業ツリーの state / ensure / remove | core | git worktree | — |
| SessionManager | セッションの state / ensure / remove / 一覧 | core | tmux, herdr | zellij |
| DevContainer | 実行環境の state / ensure / remove | core | devcontainers/cli + docker | — |
| Terminal | アタッチ用タブを開く | UI | Ghostty (osascript) | iTerm2, WezTerm, kitty |
| Transport | UI から core への到達 | UI | local, ssh | — |

実装の追加・削除が満たすべき条件(書き換え時の設計制約):

- 実装はロール契約(trait)だけに依存し、他ロール・オーケストレーション・UI を
  知らない
- 実装の追加は「契約を実装し、選択レジストリに登録する」ことだけで完結する。
  既存コードに必要な変更は選択肢の追加のみで、オーケストレーションや UI の
  ロジックには波及しない
- 契約は wsm 本体が定義する。実装は本体と別のクレート・別のリポジトリに
  置けてよい

Tracker と RepoStore は独立に交換できる。例えば「Issue 管理は Jira、
リポジトリのホスティングは GitLab」という運用では、Tracker = Jira 実装が
Issue と RepoRef の対応を返し、RepoStore が RepoRef をローカルの実体に
解決する。両者をつなぐのは RepoRef だけで、同一サービスであることを
前提にしない。

## 概念モデル

### RepoRef と Workspace

RepoRef はリポジトリの識別子で、概念上は `<host>/<namespace>/<repo>`
(例: `github.com/owner/repo`)。

Workspace は中心となるエンティティ。`(RepoRef, id)` の組で一意に識別される。

- `id`: `main`(リポジトリ本体)または Issue 番号(worktree)

`id` によって実体と配置が決まる。

| id | 実体 | パス | ブランチ |
|---|---|---|---|
| `main` | クローン本体 (RepoStore) | `~/ghq/<host>/<ns_repo>` | (そのまま) |
| Issue 番号 | git worktree | `~/worktrees/<host>/<ns_repo>/<id>` | `feature/<id>` |

パス・ブランチ名の導出は core のみが行う。UI 層は導出規則を持たず、
core の応答から受け取る。

> 書き換え時の注意: `main` は ID 空間に混ざった番兵値。型のある言語では
> `Main | Issue(番号)` のような直和型で表現する。

> 拡張点 (host): 現行実装は host を `github.com` に固定しており、
> JSON API の `ns_repo`・セッション名・Docker ラベルはいずれも host を
> 含まない。複数ホスト対応時、パス規約は `~/ghq/<host>/` /
> `~/worktrees/<host>/` へ自然に拡張されるが、セッション名・ラベル・
> API の識別子には host を含める拡張(と衝突回避)が必要になる。
> 形式はその時点で決定する。

### Target

UI 層でユーザーが Workspace を指定するための記法。

- `<ns>/<repo>#<id>`: 完全指定
- `<id>` のみ: カレントディレクトリの git remote (origin) から `ns_repo` を解決

### 共通の動詞語彙

Worktree / SessionManager / DevContainer の 3 ロールは同じ動詞で操作する。

| 動詞 | 意味 |
|---|---|
| state | 現在状態の照会。状態型はロールごと(有無の 2 値、`running / stopped / none` の 3 値) |
| ensure | 冪等な作成。結果は Outcome (`created` / `started` / `reused`) |
| remove | 冪等な破棄。存在しなければ何もせず成功する |

多重度はロールごとに異なる。

| ロール | Workspace との対応 |
|---|---|
| Worktree | Issue Workspace につき 1(main は対象外) |
| SessionManager | 実装ごとのマッピング(tmux: Workspace につき 1 セッション、herdr: リポジトリにつき 1 セッション + Issue ごとの workspace。SessionManager 節を参照) |
| DevContainer | Workspace × 設定名ごとに 0..n |
| Terminal | 追跡しない(開いたタブは wsm の管理外) |

> 書き換え時の注意: 動詞の語彙は揃えるが、単一の trait に無理に統一しない。
> ensure に必要な情報 (Spec) と状態型 (State) がロールごとに異なるため、
> 関連型で分ける:
>
> ```rust
> trait Facet {
>     type Spec;   // ensure に必要な情報 (cwd / ブランチ / config パス等)
>     type State;  // 有無の 2 値や running/stopped/none の 3 値
>     fn state(&self, key: &FacetKey) -> Result<Self::State>;
>     fn ensure(&self, key: &FacetKey, spec: &Self::Spec) -> Result<Outcome>;
>     fn remove(&self, key: &FacetKey) -> Result<()>;
> }
> ```

Terminal は状態を追跡しないため、この語彙の対象外(open_tab のみ。
Terminal 節)。Tracker / RepoStore は読み取り専用で、動詞語彙を持たない。

### Tracker

Project / リポジトリ / Issue のメタデータ照会。読み取り専用。

契約(書き換え時の trait 境界):

- projects(): open な Project の一覧 (id, title)
- repos_in_project(project): Project に属する RepoRef の一覧
- issues(repo): open な Issue の一覧 (id, title, state)
- issue(repo, id): 単一 Issue の照会(孤児 worktree の title / closed 解決に使う)

Project は「リポジトリ・Issue の任意のグルーピング」程度に弱く定義し、
GitHub Projects の仕様に寄せない。Project 概念を持たないトラッカーの実装は
projects() が空を返してよい(UI には「Project: none」で全リポジトリを出す
経路が既にある)。

現在の実装: GitHub (gh CLI。`gh project list` / GraphQL projectV2 /
`gh issue list` / `gh issue view`)。Project のリポジトリ解決は GraphQL の
`repositoryOwner` + inline fragment を使い、`ns` が個人 (User) でも
organization でも同じように動く。

### RepoStore

ローカルにクローン済みのリポジトリの列挙と、クローン本体のパス解決。
読み取り専用。

- list(): ローカルにある RepoRef の一覧
- path(repo): クローン本体のパス

現在の実装: ghq (`~/ghq/<host>/<ns_repo>`)。クローンの作成 (`ghq get`) は
現状 wsm のスコープ外。

`list-repos` の「Project に属し、かつローカルにもある」という絞り込みは、
Tracker と RepoStore の結果の交差であり、オーケストレーション層の責務。
どちらのロールも相手を知らない。

### Worktree

Issue Workspace の作業ツリー。

- state: 有 / 無
- ensure: ブランチ `feature/<id>` の worktree を
  `~/worktrees/<host>/<ns_repo>/<id>` に作る。ブランチが既存ならそれを
  チェックアウトし、なければ作る。`--relative-paths` を付ける
  (DevContainer マウント時にホスト絶対パスへの参照を避けるため)
- remove: worktree を削除する

Main Workspace はクローン本体(RepoStore の管轄)をそのまま使うため、
Worktree ロールは関与しない。

現在の実装: git worktree。

### SessionManager

Workspace に紐づくターミナルセッションの管理。実装は交換可能で、
`WSM_SESSION_MANAGER` 環境変数(既定: config.toml の `session_manager`)で
選択する。現在の実装は tmux と herdr。

契約は `(ns_repo, id)` をキーに取る(書き換え時の trait 境界):

- state: 存在確認
- ensure: 冪等に用意し、アタッチ対象のセッション名を返す
- remove: 冪等な破棄
- add_window(session, name, command, dedup_key): セッションに、指定コマンドを
  実行するウィンドウ(タブ)を追加する。冪等で、同じ dedup_key を持つ
  ウィンドウが生きていれば何もしない(tmux 実装は pane オプション `@wsm_cid`
  にキーを記録する。ウィンドウを閉じればキーも消えるため、次回は再作成される)。
  ウィンドウ概念を持たない実装は noop でよい(herdr は noop)。
  DevContainer の 🐳 ウィンドウ(DevContainer 節)はオーケストレーション層が
  この操作で配線し、DevContainer ロールと SessionManager ロールは互いを知らない

Workspace とセッションの対応は実装ごとに異なる。名前・ラベルの導出は
ドメイン層が行い、実装は導出済みの値を使うだけ。

| 実装 | main | Issue |
|---|---|---|
| tmux | セッション `<ns>_<repo>` | セッション `<ns>_<repo>_<id>` |
| herdr | セッション `<ns>.<repo>` + workspace (ラベル = リポジトリ名) | 同セッション内の workspace (ラベル = `<id>`) |

herdr のセッション名は `<ns>.<repo>`(`/` をドットに置換)。GitHub の
namespace(user / organization とも)にはドットが使えないため、最初のドットが
常に区切りとなり、この変換は単射になる(repo の入力検証が ns にドットを
許さないのはこの保証のため)。herdr はセッション名がそのままディレクトリ名に
なるためスラッシュが使えない。

tmux はセッション名の `.` と `:` をターゲット構文予約のため**黙って `_` に
置換する**ので、tmux 実装だけ区切りをすべて `_` に統一した名前
(`<ns>_<repo>(_<id>)`)を使う。代償として repo 名の `.` / `_` / 末尾の
`_<数字>` の区別が tmux 内でだけ潰れる(該当する対を同時に開くと
セッションを共有してしまうが、データの同一性は `(ns_repo, id)` のまま保持
されるため壊れるのはセッションの共有だけ。個人ツールの制約として許容する)。

#### herdr 実装の詳細

herdr はリポジトリ単位のセッションに Issue ごとの workspace を追加するモデル。

- セッション外からの workspace 操作は、`herdr session list --json` の
  `socket_path` を `HERDR_SOCKET_PATH` に指定して行う
- ensure: セッションが running でなければ `herdr --session <name> server` を
  バックグラウンド起動し、running をポーリングで待つ (ヘッドレス起動)。
  ラベルは main = リポジトリ名、Issue = Issue 番号。
  **main の workspace は Issue open 時も常に保証する**: ヘッドレス起動直後の
  セッションは workspace ゼロで、アタッチ時の自動作成に任せると cwd が
  リポジトリにならない (ホームが開く) ため。作成/フォーカスの規則:
  - open 対象の workspace は `--focus` で作成、既存なら `workspace focus`
    (open = そこに向かう、の意図)
  - Issue open 時に main workspace が無ければ `--no-focus` で作る
    (フォーカスは Issue に向ける)
- 既知の制約: 数字のみのリポジトリ名 (例: `owner/2048`) は main の workspace
  ラベルが Issue workspace と区別できず、rm main の残存判定が誤爆する
- remove (Issue): `workspace close`。それがセッション内の最後の workspace
  だった場合はセッションも stop + delete で畳む
- remove (main): セッションの stop + delete。ただし wsm 管理の Issue workspace
  (ラベルが数字のみ) が残っている間は拒否する (器ごと道連れにしない)。
  この判定はオーケストレーション層が remove の冒頭で行う
- 存在確認 (実装横断): main はセッションが running か、Issue はさらに
  該当ラベルの workspace があるか。main の active はセッションの running と
  等価なので、Issue だけを開いた場合も main は active 扱いになる (既知の性質)
- attach_command はセッション単位 (`herdr --session <name>`)。main / Issue で
  同一で、Issue のフォーカスは ensure 時の focus が担う

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

起動後、オーケストレーション層はコンテナへ `docker exec` で入るウィンドウ
(🐳) をセッションに追加する。責務の分割:

- DevContainer ロール: 起動済みコンテナの exec コマンドとコンテナ ID を
  組み立てて返す(docker・ラベル・remoteUser の知識はここに閉じる)
- SessionManager ロール: add_window(session, `🐳`, command, dedup_key=コンテナ ID)
  でウィンドウを追加する(冪等。herdr は noop)
- 両者をつなぐのはオーケストレーション層のみ

現在の実装: devcontainers/cli + docker。

### Terminal

ワークスペースを開いたあと、セッションにアタッチしたタブを端末エミュレーターに
開かせるアダプタ。UI 層に属する。現在の実装は Ghostty (osascript 経由、
macOS のみ)。

契約は open_tab(attach_command) のみ。wsm は開いたタブを追跡しないため、
存在確認・削除は概念的に存在しない(他ロールと違い noop ですらない)。
attach_command は core の open 応答が返し、UI はそれをそのまま渡す
(決定事項を参照)。

ホスト以外(Transport が SSH のとき)ではアタッチできないため、Terminal は
何もしない。DevContainer からの open は「ホスト側にセッションを用意する」
ところまでが責務。

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

### open / remove の手順と合成ビュー

ロール間の依存の順序を知るのはオーケストレーション層だけ。open は
次の順で進む。

1. Worktree.ensure (Issue のみ) — セッションの cwd と DevContainer の
   マウント元になる
2. SessionManager.ensure (cwd = Workspace パス)
3. DevContainer.ensure (`--config` ごと) + SessionManager.add_window で
   🐳 ウィンドウを配線
4. UI: Terminal.open_tab(attach_command) (ホストのみ)

remove は逆順で破棄する(Terminal は管理外なので対象外):
セッション破棄 → DevContainer 破棄 → worktree 削除。

複数ロールの結果から算出する派生値(合成ビュー)もオーケストレーション層が
持ち、ロールには入れない。

| 派生値 | 定義 | 参照するロール |
|---|---|---|
| `active` | セッションが存在するか | SessionManager |
| `active_count` | リポジトリ内のアクティブ Workspace 数 | Worktree × SessionManager |
| 孤児 worktree | Tracker 上は closed だがセッションが残っている Issue | Tracker × Worktree × SessionManager |
| `devcontainer` | ラベル一致コンテナの集約状態 | DevContainer |

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
→ `main` + open な Issue + 孤児 worktree(closed だがセッションが残っているもの。
並びは worktree 一覧順)。
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
`--config` があれば DevContainer も起動する。
```json
{"status": "ok", "message": "...", "session": "owner_repo_123", "path": "/Users/me/worktrees/github.com/owner/repo/123", "attach_command": "/opt/homebrew/bin/tmux attach-session -t 'owner_repo_123'"}
```

- `attach_command`: UI がそのまま Terminal に渡すアタッチ用コマンド。
  core (ホスト側) が SessionManager 実装に応じて組み立てる
- `session` / `path`: 参考情報。UI のアタッチには使わない

`remove --repo <ns_repo> --issue <id>`
→ セッション・DevContainer を破棄。worktree の場合は worktree も削除する。
(open と同じ id を受けるため、フラグ名も `--issue` で対称にしている)
```json
{"status": "ok", "message": "..."}
```

### 共通仕様

- `message` 内の Workspace 表記は `<ns_repo> <id>` のスペース区切り
  (`#` などトラッカー固有の記法は使わない。セッション名も表示しない —
  実装ごとに名前が違うため)
- `active`: セッションが存在するか
- `closed`: GitHub Issue が closed か
- `devcontainer`: `running` / `stopped` / `none`
- 引数はパラメータごとに形まで検証する(SSH 経由で呼ばれるため必須。
  シェルメタ文字に加え、パストラバーサル `..` とオプション注入 `-...` も弾く)
  - `repo`: `<ns>/<repo>`。ns は GitHub 規則(英数と `-`。user / org とも
    同じ)、repo は英数・`._-`(先頭 `-` とドットのみは不可。`.github` の
    ような先頭ドットは可)。ns にドットを許さないことがセッション名導出の
    単射性の根拠(SessionManager 節)
  - `issue`: `main` または数字のみ
  - `user`: 英数と `-`(先頭は英数)
  - `project`: 数字のみ(`none` は list-repos の特別値)
  - `--config`: 検証しない(ローカルパスを許容する)
  - 空文字の値は未指定と同じ扱い(`--repo ""` は `--repo required`)
  - 同名フラグの重複は後勝ち
  - 違反時は `{"error":"--<flag> required"}` または
    `{"error":"Invalid <name>: <value>"}` を stderr に出して非ゼロ終了
- 既知の制約
  - open な Issue の取得は 50 件まで(`gh issue list --limit 50`)
  - タブを含む Issue タイトルは非対応(gh の `-q` 出力をタブ区切りで
    パースするため、タブ以降が欠落する)

## 決定事項

### エラーは必ずちょうど 1 つの error JSON で返す

失敗時の契約は「stderr にちょうど 1 つの `{"error": ...}` を出して
非ゼロ終了する」。Rust 版はこれを満たす。現行 zsh 版には満たせない
既知の欠陥があるが、書き換えで解消されるため修正しない:

- 外部コマンド自体の起動失敗(gh 未ログイン、docker daemon 停止、
  `ghq list` の結果が空など)のとき、`set -e` / `pipefail` により
  **JSON を出さず無言で exit 1** することがある
- devcontainer の設定不在・CLI 未インストール時に error JSON を
  **2 行** stderr に出す(内側のエラーと外側の `devcontainer up failed`)

契約テストは「両実装が通る」範囲を対象とするため、この差は docs で
管理する(zsh 版に合わせたテストは書かない)。

### JSON API を書き換えの境界とする

Rust / Go 版はまず wsm-core を置き換える。上記 JSON API を仕様として実装し、
UI 層は変更なしで差し替えられること。UI まで取り込むかは core 置き換え後に
判断する。

Rust 版は `crates/wsm-core` に置き、完成まで zsh 版 (`bin/wsm-core`) と
並存させる(zsh 版は削除せず、配布・日常利用の実体であり続ける)。
開発中の比較は JSON API 境界で行う: 同じサブコマンドを両実装に投げて
出力を比べる。UI ごと試すときは `PATH="$PWD/target/debug:$PATH"` を前置して
UI に Rust 版を掴ませる(UI は PATH 上の `wsm-core` を呼ぶだけなので無変更)。
配布物の切り替えは Rust 版の完成後にまとめて行い、そのときまで zsh 版には
手を入れない。

### 契約テストを両実装の受け入れ条件とする

JSON API の契約テストを `crates/wsm-core/tests/contract/` に置く。
外部コマンド (gh, ghq, git, tmux, herdr, docker, devcontainer) は PATH 先頭の
フェイクに差し替え、隔離した一時 HOME の下で「応答」と「呼び出しログ」の
両面から挙動を検証する。

- テスト対象は既定でビルドした Rust 版。
  `WSM_CORE_BIN=$PWD/bin/wsm-core cargo test --test contract` で
  zsh 版 (リファレンス実装) に切り替えられる
- zsh 版で緑にしたテストが Rust 版の受け入れ条件になる。両実装が
  常に同じスイートを通ること
- JSON の比較は意味比較 (パース後の等価判定)。整形の差は契約に含めない
- 外部コマンドとの会話 (引数列) も契約の一部。Rust 版は zsh 版と同じ
  引数で外部コマンドを呼ぶ (フェイクの応答がそれを前提にするため)
- テストは Arrange-Act-Assert パターンで構造化する

### ロール契約を実装追加の境界とする

外部ツール連携は必ずロール契約(trait)越しに行い、特定ツールの実装は
契約の 1 実装として追加・削除できること。実装の追加が満たすべき条件は
「ロールと実装」節の設計制約の通り。想定する追加の例:

- Tracker: Jira(リポジトリは GitLab、という組み合わせも含む)
- Terminal: 別の端末エミュレーターで試しに開いてみる

いずれも、契約の実装と選択レジストリへの登録だけで済み、
オーケストレーション・UI・他ロールの変更を要しないこと。実装を wsm 本体と
別のリポジトリ・クレートに置くことを妨げないこと。

### DevContainer と SessionManager を直接結合しない

DevContainer 側のコードが特定のセッションマネージャー (tmux) を知ると、
唯一のロール間結合になってしまう。🐳 ウィンドウは SessionManager 契約の
add_window(汎用の「コマンドを実行するウィンドウの追加」。非対応実装は
noop)に一般化し、オーケストレーション層が配線する。(適用済み)

### open 応答の attach_command で UI のマネージャー分岐をなくす

SessionManager 実装の知識が UI に漏れると、実装を追加するたびに UI の
修正が必要になる。core が `attach_command` を返し、UI はそれを Terminal に
渡すだけにする。実装を追加したときに変わるのは core 側だけになる。
(適用済み。かつて UI が分岐に使っていた `manager` フィールドはこのとき廃止した)

### herdr は「リポジトリセッション + Issue workspace」にマップする

tmux のように Workspace ごとにフラットなセッションを作るのではなく、
herdr 本来のモデル (セッションの中に workspace) に合わせる。
挙動の決定 (2026-07-07):

- main の remove は、wsm 管理の Issue workspace が残っている間は拒否する
- 最後の Issue workspace を閉じてセッションが空になったら stop + delete で畳む
- open 時は該当 workspace にフォーカスを移す
- Issue workspace のラベルは Issue 番号のみ (`42`)。「wsm 管理の workspace」の
  判定はラベルが数字のみであることを使う

### 旧形式 tmux セッション名は廃止済み

旧形式 `<ns>/<repo>(-<id>)` のセッション名と、その検出・アタッチ・削除の
互換コードは削除した。セッション名は常に新形式 `<ns>.<repo>(-<id>)`。
Rust / Go 版にも旧形式は持ち込まない。

### UI は単一の実行ファイルにする

wsm.tmpl (zshrc 関数) と wsm-client の二重実装を、単一の実行ファイル `wsm` に
統合する。`wsm` はシェル状態に触れないため、shell 関数である必要がない。

- transport は自動判別する: `wsm-core` が PATH にあればローカル実行、
  なければ SSH。`WSM_TRANSPORT` で明示指定も可能
- ホスト固有の処理 (Terminal アダプタ) のみ実行環境で分岐する
- zshrc 側には環境変数の設定だけを残す

### 導出ロジックは core に集約する

パス・セッション名・ブランチ名の導出は core (ドメイン層) のみが持つ。
UI 層にもロール実装にも導出規則を置かない。Target 解決だけは
カレントディレクトリに依存するため UI 層の責務とする。

### gh の呼び出しは core に閉じる

UI 層は Tracker (現在は gh) を直接呼ばない。GitHub ユーザーの解決は core が
行う(`--user` 省略時に core が自己解決)。これにより DevContainer からの
SSH ホワイトリストは `wsm-core` エントリだけで足りる。

### 個人・マシン依存の値はツールに焼き込まない

フォールバック devcontainer 設定のパスなど、個人・マシン依存の値は
環境変数として dotfiles 側で設定する。ツール側の既定値は「なし」。

### パス配置とブランチ規則は規約として固定する

以下は設定にせず、ツールの規約とする(個人ツールであり、可変にする
メンテナンスコストに見合わないため)。

- リポジトリは `~/ghq/<host>/` 配下(現行実装が対応する host は
  github.com のみ)
- worktree は `~/worktrees/<host>/` 配下
- worktree のブランチは `feature/<id>`

## dotfiles との境界

wsm 本体は本リポジトリ、利用環境の構成は dotfiles が持つ。この一覧が
wsm リポジトリと dotfiles の契約になる。

| dotfiles に残るもの | 内容 |
|---|---|
| インストール導線 | wsm の実行ファイルを `~/.local/bin` に配置する (dotfiles が chezmoi external で本リポジトリの `bin/` から取得) |
| 設定ファイルの配置 | `~/.config/wsm/config.toml` (マシン・個人依存の値) |
| SSH ホワイトリスト | `allowed-commands.sh` の `wsm-core` エントリ。wsm が必要とするのはこれのみ |
| SSH 鍵の配置 | DevContainer → ホストの鍵 (`~/.ssh/devcontainer`) |
| 配布の出し分け | DevContainer には UI (`wsm`) のみ配り、ロジック層 (`wsm-core`) と設定ファイルは配らない |

## 書き換え前の残タスク

なし。JSON API は現行 zsh 版の出力がそのまま Rust / Go 版の仕様となる
最終形になっている。

適用済みの整理: legacy セッション名互換コードの削除、open 応答への
`attach_command` 追加と UI の `manager` 分岐の除去、🐳 ウィンドウの
add_window 化、gh 直呼びの解消、個人依存既定値の除去、UI 層の統合、
導出ロジックの core への集約。

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

この一覧は現在のロール実装セットの前提であり、実装を差し替えれば変わる。

**ホスト**: tmux または herdr, fzf, jq, gh (GitHub CLI), ghq, git,
docker, devcontainer (devcontainers/cli)

**DevContainer**: fzf, jq, ssh
