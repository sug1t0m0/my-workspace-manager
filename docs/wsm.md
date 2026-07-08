# wsm (Workspace Manager)

トラッカーの repo-group / Issue を起点に、リポジトリのワークスペース
(git worktree + ターミナルセッション + DevContainer)を開閉するツール。

このドキュメントは、概念語彙・JSON API・決定事項を明文化した仕様。
サーバーはこの仕様を境界に zsh 版から Rust 版へ書き換え済みで、挙動は
契約テスト (crates/server/tests/contract/) が固定する。

外部ツールとの連携はすべてツール非依存の「ロール」(契約)として定義する。
GitHub・ghq・git worktree・tmux・herdr・devcontainers/cli・Ghostty といった
具体的なツールは、それぞれのロールの 1 実装に過ぎない(ロールと実装 節)。

## ファイル構成

```
crates/server              # サーバー (JSON API, Rust)。ホストのみに配置
bin/wsm                    # クライアント (zsh)。ホスト・DevContainer 共通の単一実装
crates/shared              # server と (将来の) client が共有する語彙 (domains)
crates/tracker-github-api  # 公式 GitHub プラグイン (API 直叩き・sub-issues 対応。推奨)
crates/tracker-github      # 公式 GitHub プラグイン (gh CLI 版・階層なし)
config.toml.example        # マシン設定 (TOML) のサンプル。実体は ~/.config/wsm/config.toml
```

Rust 側のクレート/レイヤー構成 (レイヤー構成 節と対応):

```
crates/shared/src/domains/      # RepoRef, WorkspaceId, 導出規則, 検証 (純粋)
crates/server/src/presentations/  # CLI: 引数 → ドメイン型、結果 → JSON / exit code
crates/server/src/usecases/       # オーケストレーション (依存の順序と合成ビュー)
crates/server/src/roles/          # Tracker / RepoStore / SessionManager / Worktree / DevContainer
crates/server/src/infra/          # exec (プロセス起動), settings (config.toml)
crates/client/                  # UI を Rust 化するときにここへ (現在は bin/wsm の zsh)
```

- 依存の向きは presentations → usecases → roles / infra → (すべてから) domains
- usecases はドメインの型 (`RepoRef`, `WorkspaceId`) を受け取る。引数文字列の
  解釈と検証エラーの組み立ては presentations の責務 (パース = 検証)
- レイヤー名について: repositories 相当の層は `roles` と呼ぶ。このドメインは
  エンティティ自体が「リポジトリ」なので、レイヤー名に repositories を使うと
  紛らわしいため (概念モデルの「ロール」と同じ語彙)

## レイヤー構成

配置単位(実行ファイル)は 2 つ。

- **サーバー (wsm-server)**: 状態の照会と変更をすべて担う。入出力は JSON。
  端末やユーザー対話には関与しない。ホストのみに配置される。
- **クライアント (wsm)**: fzf による対話的選択と表示整形、ターミナル
  (タブ・セッションアタッチ)連携のみを担う。状態には直接触れず、
  必ず server の JSON API を経由する。ホスト・DevContainer で同一の実装が動き、
  server への到達方法 (Transport) だけが実行環境で変わる。

内部の設計レイヤーは 4 つに分かれる。

```
クライアント (wsm)            fzf 選択 / Target 解決 / Terminal・Transport ロール
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
| Tracker | repo-group / リポジトリ / Issue の照会 | server | プラグイン (公式: wsm-tracker-github) | Jira, Linear, GitLab Issues (自作プラグイン) |
| RepoStore | ローカルクローンの列挙とパス解決 | server | ghq + 設定 `[[repo]]` | 任意の配置規約 |
| Worktree | 作業ツリーの state / ensure / remove | server | git worktree | — |
| SessionManager | セッションの state / ensure / remove / 一覧 | server | tmux, herdr | zellij |
| DevContainer | 実行環境の state / ensure / remove | server | devcontainers/cli + docker | — |
| Terminal | アタッチ用タブを開く | client | Ghostty (osascript) | iTerm2, WezTerm, kitty |
| Transport | UI から server への到達 | client | local, ssh | — |

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

RepoRef はリポジトリの識別子で、`<ns>/<repo>`(例: `owner/repo`)。
host(`github.com` 等)は識別子には含めず、RepoStore が解決するメタ情報
(RepoEntry = RepoRef + host + クローンのパス)とする。その代わり
「ns/repo は全 host・全ソースを横断して一意」を個人ツールの規約とし、
重複はエラーにする(RepoStore 節)。セッション名・Docker ラベル・JSON API
の `ns_repo` も host を含まない。

Workspace は中心となるエンティティ。`(RepoRef, id)` の組で一意に識別される。

- `id`: `main`(リポジトリ本体)または Issue id(worktree)

`id` によって実体と配置が決まる。

| id | 実体 | パス | ブランチ |
|---|---|---|---|
| `main` | クローン本体 (RepoStore) | RepoEntry.clone_path | (そのまま) |
| Issue id | git worktree | `<worktree root>/<host>/<ns_repo>/<id>` | `feature/<id>` |

worktree の導出はクローンの置き場(ghq / 設定登録)によらず共通で、
host は RepoEntry のメタ情報を使う。パス・ブランチ名の導出は server のみが
行う。クライアントは導出規則を持たず、server の応答から受け取る。

> 書き換え時の注意: `main` は ID 空間に混ざった番兵値。型のある言語では
> `Main | Issue(id)` のような直和型で表現する。

### Target

クライアントでユーザーが Workspace を指定するための記法。

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

repo-group / リポジトリ / Issue のメタデータ照会。読み取り専用。

実装は**外部コマンドのプラグイン**で、wsm 本体はトラッカーの知識を持たない
(「Tracker プラグイン契約」節)。server 側の責務はプラグインの解決
(設定 `[[tracker]]` / `default_tracker` / `[[repo]].tracker`)、起動、
出力の形検証だけ。

- repo-group 照会 (list-repo-groups / list-repos の group フィルタ) は
  既定トラッカーを使う。トラッカー未設定は縮退させずエラーにする
  (対話フローの入り口で設定誤りを表面化させる)
- リポジトリ単位の照会 (list-issues 等) は `[[repo]].tracker` で選んだ
  トラッカー (無指定は既定)。トラッカーが全く未設定なら空に縮退する
  (Tracker なしでも main の開閉はできるべき)

repo-group は「リポジトリの任意のグルーピング」であり、特定トラッカーの
仕様に寄せない (GitHub での実体は Projects、Jira ならプロジェクト等、
具体はプラグインに閉じる)。グルーピング概念を持たないトラッカーは
list-repo-groups が空を返してよい(UI には「none」で全リポジトリを
出す経路が既にある)。

公式プラグインは 2 つ (どちらも repo-group = GitHub Projects (V2)。owner は
認証ユーザーを自己解決し、`WSM_TRACKER_GITHUB_OWNER` で上書きできる):

- `wsm-tracker-github-api` (crates/tracker-github-api、**推奨**): GraphQL API
  を直接叩き、認証だけ gh に借りる (`gh auth token`。トークンの保管・更新・
  スコープ変更のライフサイクルは gh に委ね、PAT の手動管理を持ち込まない)。
  sub-issues (Issue の親子関係) に対応し、`list-issues-v1` を実装する。
  接続先は `WSM_TRACKER_GITHUB_API_URL` で差し替え可能 (契約テスト用)
- `wsm-tracker-github` (crates/tracker-github): gh CLI に API 呼び出しごと
  委ねる版。階層は非対応 (`list-issues-v0` のみ。wsm 側のフォールバックで
  平坦な一覧として動き続ける)

### RepoStore

ローカルにクローン済みのリポジトリの列挙と、クローン本体のパス解決。
読み取り専用。ソースは 2 つで、どちらも同じ RepoEntry
(RepoRef + host + クローンのパス) に正規化される。

- ghq: `ghq list` の `<host>/<ns>/<repo>` を任意の host で受け入れる
  (サブグループ等の 4 セグメント以上は非対応で、形の検証で落ちる)。
  ルートは `ghq root` コマンドで解決する (既定 `~/ghq`。ghq.root を
  変えている環境にも追随する)
- 設定 `[[repo]]`: ghq 管理外のクローンの登録 (Settings 節)。host / ns を
  メタ情報として与え、worktree はドメイン共通の導出を使う

エントリの操作は 2 つ。

- entries(): 全エントリ (ghq の出力順 → 設定の記述順)
- lookup(ns/repo): 識別子からエントリを解決する。見つからなければ
  `repository not found`、複数 host にまたがる重複は `ambiguous repository`
  のエラー (識別子の一意性が規約)

クローンの作成 (`ghq get`) は現状 wsm のスコープ外。

`list-repos` の「repo-group に属し、かつローカルにもある」という絞り込みは、
Tracker と RepoStore の結果の交差であり、オーケストレーション層の責務。
どちらのロールも相手を知らない。

### Worktree

Issue Workspace の作業ツリー。

- state: 有 / 無
- ensure: ブランチ `feature/<id>` の worktree を
  `<worktree root>/<host>/<ns_repo>/<id>` に作る (worktree root は設定
  `worktree_root`、既定 `~/worktrees`)。ブランチが既存ならそれを
  チェックアウトし、なければ作る。`--relative-paths` を付ける
  (DevContainer マウント時にホスト絶対パスへの参照を避けるため)
- remove: worktree を削除する

Main Workspace はクローン本体(RepoStore の管轄)をそのまま使うため、
Worktree ロールは関与しない。

現在の実装: git worktree。

### SessionManager

Workspace に紐づくターミナルセッションの管理。現在の実装は tmux と herdr。

使えるマネージャーは config.toml に **パス付きで列挙したものだけ**
(`tmux_path` / `herdr_path`。Settings 節)。ファイルでの出現順が選択 UI の
並び順。既定は `default_session_manager` で明示し、未指定なら列挙の先頭。
パスが設定されていないマネージャーは存在しない扱いになり、
選択・プローブ・破棄のすべてから外れる。**組み込みのフォールバックはない**
(1 つも設定されていなければ open はエラー)。その場のオーバーライドは
`WSM_SESSION_MANAGER`(設定済みのもののみ有効)。バイナリは設定されたパスで
起動する(PATH 非依存。Ghostty のようにログインシェルを介さない起動元でも
確実に動く)。

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
既知の制約: コンテナ内パスを $HOME 相対で組むため、クローンの置き場
(ghq のルート・`[[repo]]` の path) と worktree のルートは $HOME 配下に
あることを前提とする。

起動後、オーケストレーション層はコンテナへ `docker exec` で入るウィンドウ
(🐳) をセッションに追加する。責務の分割:

- DevContainer ロール: 起動済みコンテナの exec コマンドとコンテナ ID を
  組み立てて返す(docker・ラベル・remoteUser の知識はここに閉じる)。
  exec するシェルは設定 `devcontainer_shell`(既定 `zsh`)
- SessionManager ロール: add_window(session, `🐳`, command, dedup_key=コンテナ ID)
  でウィンドウを追加する(冪等。herdr は noop)
- 両者をつなぐのはオーケストレーション層のみ

現在の実装: devcontainers/cli + docker。

### Terminal

ワークスペースを開いたあと、セッションにアタッチしたタブを端末エミュレーターに
開かせるアダプタ。クライアントに属する。現在の実装は Ghostty (osascript 経由、
macOS のみ)。

契約は open_tab(attach_command) のみ。wsm は開いたタブを追跡しないため、
存在確認・削除は概念的に存在しない(他ロールと違い noop ですらない)。
attach_command は server の open 応答が返し、UI はそれをそのまま渡す
(決定事項を参照)。

ホスト以外(Transport が SSH のとき)ではアタッチできないため、Terminal は
何もしない。DevContainer からの open は「ホスト側にセッションを用意する」
ところまでが責務。

### Transport

クライアントから server への到達方法。`WSM_TRANSPORT` で明示指定がなければ、
`wsm-server` が PATH にあるかどうかで自動判別する。

- `local`: `wsm-server` を直接実行 (ホスト)
- `ssh`: SSH (`host.docker.internal`) 越しに `wsm-server` を実行 (DevContainer)

クライアントのロジックは transport に依存しない。分岐するのは server の呼び出し方と、
ホスト固有機能 (Terminal アダプタ、セッションマネージャー選択) の有効判定のみ。

制約: 環境変数は SSH を越えないため、`WSM_SESSION_MANAGER` の指定は
`ssh` transport では反映されない (ホスト側の既定が使われる)。

### Settings

マシン設定はホスト側の設定ファイル `~/.config/wsm/config.toml`
(`XDG_CONFIG_HOME` 準拠) に置き、server が読む。server は常にホストで動くため、
transport にかかわらず同じ設定が見える。フォーマットは TOML
(Rust の設定エコシステム標準。キーは snake_case でそのまま struct にマップできる)。

| キー | 内容 |
|---|---|
| `tmux_path` / `herdr_path` | セッションマネージャーの列挙 (バイナリの絶対パス、チルダ可)。出現順が選択 UI の並び順。未設定のマネージャーは選択不能 |
| `default_session_manager` | 既定のセッションマネージャー (未指定なら列挙の先頭) |
| `worktree_root` | worktree の置き場 (既定 `~/worktrees`) |
| `devcontainer_shell` | 🐳 ウィンドウで docker exec するシェル (既定 `zsh`) |
| `default_devcontainer_config` | フォールバック devcontainer 設定のパス |
| `[[tracker]]` | Tracker プラグインの列挙 (複数可)。キーは `name` (必須)、`path` (実行ファイル、必須)。列挙したものだけが存在する (フォールバックなし) |
| `default_tracker` | 既定のトラッカー (未指定なら列挙の先頭) |
| `[[repo]]` | ghq 管理外のリポジトリの登録 (複数可)。キーは `path` (クローンの場所、必須)、`host` (worktree 導出用、必須)、`ns` (必須)、`name` (省略時 path の basename)、`tracker` (省略時 default_tracker)。必須キーの欠落や形の不正は error JSON (黙って捨てない) |

優先順位: 環境変数 > 設定ファイル > 組み込み既定値。環境変数は
「その場のオーバーライド」(UI の `-m` フラグ等) にのみ使う。

設定ファイルに移せないものが 2 種ある。

- 接続情報 (`WSM_HOST` / `HOST_USER` / `HOST_SSH_KEY`): コンテナ内の UI が
  ホストへ到達する前に必要な値のため、DevContainer 側の環境変数で与える
- `WSM_TRANSPORT`: client が server に到達する方法の指定であり、server の設定では
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

## JSON API 契約 (wsm-server)

すべてのサブコマンドは JSON を stdout に返す。エラー時は
`{"error": "<message>"}` を stderr に出して非ゼロで終了する。

### 照会系

`list-repo-groups`
→ open な repo-group の一覧 (既定トラッカー)。id は不透明な文字列。
トラッカー未設定はエラー (`no tracker configured ...`)。
```json
[{"id": "1", "title": "..."}]
```

`list-repos --group <id|none>`
→ リポジトリ一覧。`none` は RepoStore (ghq + 設定 `[[repo]]`) の
全リポジトリ、id 指定時はその repo-group に属し RepoStore にもあるもの
(既定トラッカーに照会)。`active_count` はアクティブな Workspace 数。
```json
[{"ns_repo": "owner/repo", "active_count": 0}]
```

`list-issues --repo <ns_repo> [--parent <id>] [--cursor <token>]`
→ open な Issue の 1 ページ。`main` + 孤児 worktree (一覧の最初のページに
出ないがセッションが残っているもの。open な子 Issue や後続ページの Issue も
ここに来るため、`closed` は Tracker の実際の state。並びは worktree 一覧順)
は**トップレベルの最初のページにだけ**含まれる。`--parent` 指定時は
子 Issue のみ (階層のドリルダウン用)、`--cursor` 指定時は続きのページ。
`next_cursor` が非 null なら続きがある (ページの件数と並びはプラグインの
責務)。`has_children` が子 Issue の有無 (v0 しか知らないプラグインでは
常に false)。RepoStore で解決できないリポジトリはエラー
(`repository not found` / `ambiguous repository`。open /
list-devcontainer-configs も同様)。
```json
{"issues": [{"id": "main", "title": "...", "active": false, "closed": false, "devcontainer": "none", "has_children": false}],
 "next_cursor": null}
```

`list-workspaces`
→ 全リポジトリ横断のアクティブ Workspace 一覧。スキーマは list-issues に
`ns_repo` を加え、`has_children` を除いたもの (階層は選択 UX の概念で、
アクティブ一覧には関与しない)。

`list-session-managers`
→ 設定されたセッションマネージャーの一覧 (設定ファイルの出現順) と既定。
UI のマネージャー選択はこれを使う (選択肢のハードコードを持たない)。
```json
[{"name": "herdr", "default": true}, {"name": "tmux", "default": false}]
```

`list-trackers`
→ 設定されたトラッカーの一覧 (設定順) と診断。`installed` はプラグイン
実行ファイルの存在、`ready` / `diagnosis` / `protocol` は `info-v0` の
自己診断 (非対応・未インストールなら null)。UI の `wsm doctor` が使う。
診断コマンドなので [[tracker]] 未設定でもエラーにせず `[]` を返す。
```json
[{"name": "github", "path": "/Users/me/.local/bin/wsm-tracker-github",
  "default": true, "installed": true, "ready": true, "diagnosis": null,
  "protocol": ["list-repo-groups-v0", "..."]}]
```

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
  server (ホスト側) が SessionManager 実装に応じて組み立てる
- `session` / `path`: 参考情報。UI のアタッチには使わない

`remove --repo <ns_repo> --issue <id>`
→ セッション・DevContainer を破棄。worktree の場合は worktree も削除する。
(open と同じ id を受けるため、フラグ名も `--issue` で対称にしている)
セッションとコンテナの破棄はパスに依存しないため、RepoStore で解決できない
リポジトリ (クローン消失後など) でもエラーにせず掃除する。
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
  - `issue`: 英数と `-`(先頭は英数。`main` は予約)。トラッカーが発行する
    不透明な id を許容する(GitHub の `42`、Jira の `CHH-111` など)。
    先頭 `-` の禁止でオプション注入を、`.` `/` の禁止でトラバーサルを弾く
  - `group`: issue と同じ不透明な id の文法(`none` は list-repos の特別値)
  - `parent`: issue と同じ文法。番兵値 `main` は親になれない
  - `cursor`: 英数と `+/=_-`(先頭は英数)。プラグイン発行の不透明な token
  - `--config`: 検証しない(ローカルパスを許容する)
  - 空文字の値は未指定と同じ扱い(`--repo ""` は `--repo required`)
  - 同名フラグの重複は後勝ち
  - 違反時は `{"error":"--<flag> required"}` または
    `{"error":"Invalid <name>: <value>"}` を stderr に出して非ゼロ終了
- 既知の制約
  - open な Issue の 1 ページの件数と並びはプラグインの責務。続きは
    `list-issues-v2` のページングで取れる (api 版は 1 ページ 50 件・
    新しい順。v2 非対応の gh 版は先頭 50 件のみ)。かつての「タブを含む
    タイトル非対応」はプラグイン契約 (JSON 1 ドキュメント) への移行で解消した

## Tracker プラグイン契約

Tracker の実装は外部コマンドのプラグイン。wsm-server とプラグインは
この契約だけで結合し、互いに独立して更新できる(運用に合わせた修正が
wsm 本体のリリースを要しないこと、が目的)。

実装例: `examples/wsm-tracker-demo`(階層付きダミーデータを返すシェル
スクリプト。UI 開発の治具を兼ねる)。契約の全動詞をシェル 1 枚で
満たせることの見本で、配布物ではない。

### 役割と性質

- 照会のみ。読み取り専用で、副作用を持たないこと
- 1 つの実行ファイル。wsm-server がサブプロセスとして起動する
  (1 呼び出し = 1 動詞)。実装言語は問わない
- **非対話であること**。TTY は与えられない。認証切れ等でプロンプトを
  出して待たず、即座に非ゼロで失敗すること
- 認証情報・接続先などトラッカー固有の設定はプラグインが自分で読む。
  wsm は渡さないし、関知しない
- wsm 側の識別子 `ns/repo` とトラッカー側の識別子 (Jira のプロジェクト
  キー等) の対応付けはプラグイン自身の設定の責務。wsm は自分の識別子
  しか話さない

### 動詞 (v0)

| 呼び出し | stdout (JSON 1 ドキュメント) |
|---|---|
| `list-repo-groups-v0` | `[{"id": "...", "title": "..."}]` — open な repo-group。UI の表示順で返す |
| `repo-group-repos-v0 --group <id>` | `["ns/repo", ...]` — repo-group 所属のリポジトリ |
| `list-issues-v2 --repo <ns/repo> [--parent <id>] [--cursor <token>]` | `{"issues": [{"id", "title", "has_children"}], "next_cursor": "<token>" \| null}` — open な Issue の 1 ページ |
| `list-issues-v1 --repo <ns/repo> [--parent <id>]` | `[{"id": "...", "title": "...", "has_children": bool}]` — v2 非対応プラグイン向け (ページングなしの全件) |
| `list-issues-v0 --repo <ns/repo>` | `[{"id": "...", "title": "..."}]` — v1 非対応プラグイン向け (平坦な一覧) |
| `issue-v0 --repo <ns/repo> --id <id>` | `{"title": "...", "state": "open" \| "closed"}` — 単一 Issue |
| `info-v0` | `{"name": "...", "protocol": ["<動詞>", ...], "ready": true\|false, "diagnosis": "..."}` — 自己診断 |

Issue の階層 (`--parent` / `has_children`):

- `--parent` 省略時は親を持たない open Issue、指定時はその子の open Issue を
  返す。`has_children` (省略時 false) が子を持つことを示し、UI はこれで
  ドリルダウンを提供する
- 階層は**選択 UX とトラッカー側の関係**であり、Workspace のモデルには
  影響しない (どの階層の Issue を選んでも worktree・ブランチ・セッションの
  導出は平坦な id に対して同じ)
- 階層概念のないトラッカーは `has_children` を常に false にすれば、
  平坦な一覧と同じ UX に退化する

ページング (`list-issues-v2` の `--cursor` / `next_cursor`):

- **1 ページの件数はプラグインの責務** (契約は決めない)。続きがあるときは
  `next_cursor` に不透明な token を返し、wsm はそれを次の呼び出しの
  `--cursor` にそのまま渡す。UI は「… さらに読み込む」で 1 ページずつ足す
- ページングしない自由もある: `next_cursor` を常に null にすれば
  「プラグインが決めた件数の単一ページ」になる。v2 自体を実装しない自由も
  ある (下記フォールバック)
- cursor はプラグインの出力から引数へ還流するため、wsm が形を検証する
  (英数と `+/=_-`、先頭は英数。違反は「続きなし」に落とす)

wsm は v2 → v1 → v0 の順に試す (未知の動詞 → 非ゼロ、で非対応を検知):

- v1 へのフォールバックは `has_children` 付きの全件 (ページングなし)、
  v0 へのフォールバックは平坦な一覧で `has_children` は false に補われる
- 下位動詞で表現できない照会のフォールバックは空 (`--parent` は v0 で、
  `--cursor` は v1 以下で表現できない)

`info-v0` の意味論 (他の動詞と違う点):

- 「トラッカーが使えない状態」(未ログイン・スコープ不足等) は **ready:false
  のデータ**として返す。info 自体が非ゼロ終了するのは info を実行できない
  ときだけ
- `diagnosis` は人間向けの原因と修復手順 (`ready: false` のときに必須。
  複数行可 — 表示側が 1 行に潰す)。`protocol` は対応する動詞の列挙
  (info-v0 自身を含む)
- wsm はこれを診断 (`list-trackers` / `wsm doctor`) にだけ使う。照会の
  通常経路では呼ばない (照会は縮退契約で守られており、毎回のプローブは
  遅くする価値がない)
- info-v0 非対応のプラグイン (未知の動詞に非ゼロで応答) は「installed だが
  ready 不明」として扱う。対応は必須ではない

- 出力は UTF-8 の JSON 1 ドキュメント (行区切り JSON ではない)。タイトルの
  タブ・改行は JSON エスケープで表現できるため、形式上の制約はない
- `state` は中立語彙 `open` / `closed` (実装固有の表記を持ち込まない)
- グルーピング概念を持たないトラッカーは `list-repo-groups-v0` で `[]` を返す
- 取得件数の上限やページングはプラグインの責務

### 成功・失敗の契約

- 成功: exit 0 + 完全な JSON。部分的な JSON を出して失敗してはならない
- 失敗: 非ゼロ終了。stderr は診断用で、wsm は解釈しない
- wsm 側の扱いは照会の縮退契約と同じ: プラグインの失敗は「取得できなかった
  部分を除いた結果」に畳む (プラグインが壊れても wsm 全体は使える)

### 信頼境界

プラグインの出力は信頼しない入力として扱う (wsm 側の義務):

- `id` は Issue id の文法 (英数と `-`、先頭英数) で検証し、違反する要素と
  番兵値 `main` は捨てる。id はブランチ名・セッション名・Docker ラベルに
  流れ込むため、ここが注入対策の関所
- `ns/repo` は RepoRef の文法で検証する (`repo-group-repos-v0` の応答)
- `title` は表示にしか使わないため任意の文字列を許す

逆にプラグインが前提にしてよいこと: 引数は wsm が検証済みの形でのみ渡る。
動詞・フラグは上記以外来ない。未知の動詞には Usage を stderr に出して
非ゼロで終了すればよい (前方互換の逃げ道)。

### バージョニング (動詞名 = バージョン)

wsm とプラグインは別々に更新されるため、契約の変更は動詞名で表現する。
握手プロトコルは持たない。

- **既存の動詞の契約は変えない。** 非互換な変更 (フィールドの意味・型の
  変更、必須フィールドの追加、フラグの意味変更) は `-v1` 等の新動詞を作る
- 互換な変更はバージョンを上げない: 応答への**任意**フィールドの追加
  (wsm は知らないフィールドを無視する)、新動詞の追加 (古いプラグインは
  非ゼロで断る)
- 動詞がずれたときの壊れ方は「未知の動詞 → 非ゼロ → 照会が空に縮退」で、
  静かに間違った動作をするのではなく見えて失敗する
- `info-v0` はこの「互換な新動詞の追加」の実例 (診断のために後から足した。
  非対応の既存プラグインは非ゼロで断るだけで壊れない)。起動方法など動詞で
  表現できない変更が必要になったら、info の応答を握手に使う移行経路がある
- 動詞の**廃止**は非互換だが検知可能な変更 (未知の動詞 → 非ゼロ → 縮退)。
  repo-group への改名 (旧 `list-projects-v0` / `project-repos-v0` の廃止) は
  これを使った。運用者が単一で両側を束ねてリリースできる場合に限る手段
- `list-issues-v1` (階層対応) が v-bump の最初の実例。v0 に `--parent` を
  足す案は、フラグを知らない旧プラグインが**黙ってトップレベル一覧を返す**
  (静かに間違う) ため不可で、新動詞が必須だった。v0 は wsm 側の
  フォールバック先として契約に残り、旧プラグインは無修正で動き続ける

## 決定事項

### エラーは必ずちょうど 1 つの error JSON で返す

失敗時の契約は「stderr にちょうど 1 つの `{"error": ...}` を出して
非ゼロ終了する」。外部コマンド自体の起動失敗(gh 未ログイン、docker
daemon 停止など)でも「JSON なしの無言の失敗」はしない: 照会系は取得
できなかった部分を除いた結果で成功し、変更系はこの形の error JSON で
失敗する。契約テストは stderr 全体を 1 つの JSON としてパースすることで
これを固定する(無言の失敗も 2 行出力も落ちる)。

(経緯: 旧 zsh 版はこの契約を満たせない既知の欠陥 — `set -e` /
`pipefail` による無言の exit 1、devcontainer 失敗時の error JSON 2 行 —
を持ち、契約テストを「両実装が通る」範囲に絞っていた。zsh 版の削除で
この制約は解消し、テストで固定できるようになった)

### 配布は GitHub Releases で行う

タグ `v*` の push で GitHub Actions が契約テストを通してからビルドし、
成果物を Release に添付する (.github/workflows/release.yml)。

- 成果物: `wsm-server-aarch64-apple-darwin`(Rust サーバー、macOS arm64
  ホスト用)と `wsm`(zsh クライアント。ホスト・DevContainer 共通)
- dotfiles は chezmoi external で `releases/latest/download/<asset>` から
  取得する(`bin/` 直取りは廃止)
- 将来 Rust クライアントを配るときは `wsm-client-<target>`(linux
  amd64 / arm64 等)を成果物に追加する。クロスビルドが必要になるのは
  この時点で、Releases 方式を選んだ主な理由

### JSON API を書き換えの境界とする

実装の書き換えは JSON API を仕様として行い、クライアントは変更なしで
差し替えられること(UI は PATH 上の `wsm-server` を呼ぶだけなので無変更。
開発中のビルドは `PATH="$PWD/target/debug:$PATH"` の前置で UI に掴ませる)。

(適用済み: server は zsh 版から Rust 版 `crates/server` へこの境界で
書き換えた。並存期は契約テストを両実装の受け入れ条件とし、配布・日常利用の
切り替え後に zsh 版 `bin/wsm-server` を削除した。UI の Rust 化は未定)

### 契約テストを受け入れ条件とする

JSON API の契約テストを `crates/server/tests/contract/` に置く。
外部コマンド (gh, ghq, git, tmux, herdr, docker, devcontainer) は PATH 先頭の
フェイクに差し替え、隔離した一時 HOME の下で「応答」と「呼び出しログ」の
両面から挙動を検証する。

- テスト対象は既定でビルドしたバイナリ。`WSM_SERVER_BIN=<path>` で
  別のビルド (リリースバイナリ等) に差し替えて同じスイートを回せる
- 挙動の変更は契約テストの変更として現れること。テストにない挙動は
  契約ではない
- JSON の比較は意味比較 (パース後の等価判定)。整形の差は契約に含めない
- 外部コマンドとの会話 (引数列) も契約の一部 (フェイクの応答が
  それを前提にするため)
- テストは Arrange-Act-Assert パターンで構造化する

### ロール契約を実装追加の境界とする

外部ツール連携は必ずロール契約(trait)越しに行い、特定ツールの実装は
契約の 1 実装として追加・削除できること。実装の追加が満たすべき条件は
「ロールと実装」節の設計制約の通り。想定する追加の例:

- Tracker: Jira(リポジトリは GitLab、という組み合わせも含む)。
  プラグイン契約に切り出し済みで、wsm 本体の変更なしに追加できる
  (「Tracker プラグイン契約」節)
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
修正が必要になる。server が `attach_command` を返し、UI はそれを Terminal に
渡すだけにする。実装を追加したときに変わるのは server 側だけになる。
(適用済み。かつて UI が分岐に使っていた `manager` フィールドはこのとき廃止した)

### herdr は「リポジトリセッション + Issue workspace」にマップする

tmux のように Workspace ごとにフラットなセッションを作るのではなく、
herdr 本来のモデル (セッションの中に workspace) に合わせる。
挙動の決定 (2026-07-07):

- main の remove は、main 以外の workspace が残っている間は拒否する
  (Issue id が不透明な文字列になったため、ラベルの形で wsm 管理かは
  判別しない。wsm 外で作られた workspace も「開いている作業」として保護する)
- 最後の Issue workspace を閉じてセッションが空になったら stop + delete で畳む
- open 時は該当 workspace にフォーカスを移す
- Issue workspace のラベルは Issue id のみ (`42`, `CHH-111`)

### リポジトリの識別子は ns/repo、host は RepoStore のメタ情報

複数 host (GitLab 等) に対応しても、識別子・Target 構文・JSON API・
セッション名・Docker ラベルは `<ns>/<repo>` のまま変えない。host は
RepoEntry のメタ情報として worktree のパス導出にだけ使う。その代わり
「ns/repo は全 host・全ソースを横断して一意」を個人ツールの規約とし、
重複は lookup 時に `ambiguous repository` のエラーにする。
(識別子に host を含める案は、Target 構文と JSON API の互換を壊し、
毎回 host を書かせる割に個人ツールでは衝突がほぼ起きないため見送った)

### セッションマネージャーは設定で列挙する(フォールバックなし)

マネージャーの存在とバイナリの場所はマシン依存の値なので、config.toml に
パス付きで列挙する。ツール側は既定 (tmux) を持たない。選択 UI の並び順は
設定ファイルの記述順、既定は `default_session_manager` で明示する
(未指定なら列挙の先頭)。

### 旧形式 tmux セッション名は廃止済み

旧形式 `<ns>/<repo>(-<id>)` のセッション名と、その検出・アタッチ・削除の
互換コードは削除した。セッション名は常に新形式 `<ns>.<repo>(-<id>)`。
Rust / Go 版にも旧形式は持ち込まない。

### UI は単一の実行ファイルにする

wsm.tmpl (zshrc 関数) と wsm-client の二重実装を、単一の実行ファイル `wsm` に
統合する。`wsm` はシェル状態に触れないため、shell 関数である必要がない。

- transport は自動判別する: `wsm-server` が PATH にあればローカル実行、
  なければ SSH。`WSM_TRANSPORT` で明示指定も可能
- ホスト固有の処理 (Terminal アダプタ) のみ実行環境で分岐する
- zshrc 側には環境変数の設定だけを残す

### 導出ロジックは server に集約する

パス・セッション名・ブランチ名の導出は server (ドメイン層) のみが持つ。
クライアントにもロール実装にも導出規則を置かない。Target 解決だけは
カレントディレクトリに依存するため クライアントの責務とする。

### トラッカーの知識はプラグインに閉じる

クライアントは Tracker を直接呼ばない。トラッカー固有の知識 (gh の
呼び出し・認証ユーザーの解決・API の形) は Tracker プラグインに閉じ、
server はプラグインの解決・起動・出力検証だけを行う。これにより
DevContainer からの SSH ホワイトリストは `wsm-server` エントリだけで
足り、運用に合わせたトラッカーの修正が wsm 本体のリリースを要しない。
(経緯: かつては server が gh を直接呼んでいた。`--user` フラグは owner
解決ごとプラグインに移して廃止)

### 個人・マシン依存の値はツールに焼き込まない

フォールバック devcontainer 設定のパスなど、個人・マシン依存の値は
環境変数として dotfiles 側で設定する。ツール側の既定値は「なし」。

### パス配置とブランチ規則は規約として固定する

以下は設定にせず、ツールの規約とする(個人ツールであり、可変にする
メンテナンスコストに見合わないため)。

- リポジトリは ghq のルート配下 (`ghq root` で解決。既定 `~/ghq`。
  host は任意) か、設定 `[[repo]]` で登録した場所に置く
- worktree は設定 `worktree_root`(既定 `~/worktrees`)配下。
  配下の構造 `<host>/<ns_repo>/<id>` は固定 (host は RepoStore の
  メタ情報。クローンの置き場によらず共通)
- worktree のブランチは `feature/<id>`

## dotfiles との境界

wsm 本体は本リポジトリ、利用環境の構成は dotfiles が持つ。この一覧が
wsm リポジトリと dotfiles の契約になる。

| dotfiles に残るもの | 内容 |
|---|---|
| インストール導線 | `~/.local/bin/wsm-server`・`~/.local/bin/wsm-tracker-github`・`~/.local/bin/wsm` を chezmoi external で GitHub Releases (`releases/latest/download/<asset>`) から取得・配置する |
| 設定ファイルの配置 | `~/.config/wsm/config.toml` (マシン・個人依存の値) |
| SSH ホワイトリスト | `allowed-commands.sh` の `wsm-server` エントリ。wsm が必要とするのはこれのみ |
| SSH 鍵の配置 | DevContainer → ホストの鍵 (`~/.ssh/devcontainer`) |
| 配布の出し分け | DevContainer にはクライアント (`wsm`) のみ配り、サーバー (`wsm-server`) と設定ファイルは配らない |

## 書き換えの状態

server の書き換えは完了。Rust 版 (`crates/server`) が唯一の実装で、
zsh 版 (`bin/wsm-server`) は削除済み。JSON API の仕様は本ドキュメントと
契約テストが持つ。クライアント (`bin/wsm`) は zsh のまま。

適用済みの整理: legacy セッション名互換コードの削除、open 応答への
`attach_command` 追加と UI の `manager` 分岐の除去、🐳 ウィンドウの
add_window 化、gh 直呼びの解消、個人依存既定値の除去、クライアントの統合、
導出ロジックの server への集約、zsh サーバーの削除。

## 環境変数

マシン設定は config.toml が正 (Settings 節を参照)。環境変数の役割は
その場のオーバーライドと、設定ファイルに移せない値に限る。

| 変数 | 既定値 | 用途 |
|---|---|---|
| `WSM_SESSION_MANAGER` | `default_session_manager` (未指定なら列挙の先頭) | セッションマネージャーのオーバーライド (設定済みのもののみ有効)。UI の `-m` / fzf 選択が export する |
| `WSM_WORKTREE_ROOT` | (config.toml) | worktree 置き場のオーバーライド |
| `WSM_DEVCONTAINER_SHELL` | (config.toml) | 🐳 ウィンドウのシェルのオーバーライド |
| `WSM_DEFAULT_DEVCONTAINER_CONFIG` | (config.toml) | フォールバック devcontainer 設定のオーバーライド |
| `WSM_TRANSPORT` | 自動判別 | server への到達方法の明示指定 (`local` / `ssh`) |
| `WSM_HOST` | `host.docker.internal` | (ssh transport) SSH 接続先 |
| `HOST_USER` | なし (ssh transport では必須) | (ssh transport) SSH ユーザー |
| `HOST_SSH_KEY` | `devcontainer` | (ssh transport) `~/.ssh/` 配下の鍵名 |

## 前提ツール

この一覧は現在のロール実装セットの前提であり、実装を差し替えれば変わる。

**ホスト**: tmux または herdr, fzf, jq, ghq, git,
docker, devcontainer (devcontainers/cli)。
gh (GitHub CLI) は公式プラグイン `wsm-tracker-github` を使う場合の前提

**DevContainer**: fzf, jq, ssh
