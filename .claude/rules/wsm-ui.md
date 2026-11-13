---
paths:
  - "bin/*"
  - "tests/*.sh"
---

# bin/wsm (zsh クライアント) の規約

UI は fzf 選択と表示整形、Terminal (タブを開く) だけを担う。状態の照会・変更と
パス・ブランチ・セッション名の導出は server の責務で、UI は JSON 応答をそのまま使う
(docs/wsm.md「導出ロジックは server に集約する」)。マネージャー分岐も持たない
(`attach_command` を Terminal に渡すだけ)。

## ピッカーの約束

- `_fzf_with_back` は fzf を `--expect=left` で起動し、返り値で 3 状態を区別する:
  `__back__` (← で 1 段戻る)、空 (ESC = フロー全体のキャンセル)、選択行
- `_pick_workspace` が返す特別値: `__fallback__` (グループの Issue 一覧が空 →
  リポジトリ起点フローへ)。内部の行キー `__self__` (この Issue を開く)、`__up__` (1 階層上)、
  `__more__` (… さらに読み込む) はピッカーの外に出さない
- 呼び出し側 (`op` の `case "$step"`) は `__back__` で前のステップへ、空で `exit 0`。
  新しいピッカーを足すときは `with-nth` で表示列を分け、返り値は `cut -f` でキー列だけを返す
- マーカー (`▸` 子あり、`↳` 孤児 workspace) は行頭に置く。行末は長いタイトルの切り詰めで
  見えなくなる

## zsh の罠 (過去に踏んだもの)

- `setopt typeset_silent` を外さない。ループ内の `local` 再宣言が `name=''` を stdout に
  印字し、コマンド置換で受ける関数の返り値を汚染する
- `path` と `status` を変数名に使わない (`$PATH` の別名、`$?` の読み取り専用別名)
- JSON や複数行の値は `echo` ではなく `printf '%s\n'` で出す
- `set -euo pipefail` 下で `fzf` のキャンセル (非ゼロ) を `|| true` で受ける
- `-m/--manager` の指定は SSH transport では server に届かない (警告を出すだけ)

## 回帰テスト (`tests/wsm_*.sh`)

| ファイル | 固定している分岐 |
|---|---|
| `tests/wsm_op_attached.sh` | open 応答の `attached` でタブを開くか (issue #5)。macOS 以外では skip |
| `tests/wsm_op_work_repo.sh` | 作業リポジトリ選択 (`_pick_work_repo`) と ← / ESC / 選択 (`_fzf_with_back`) (issue #12) |

- 新しいテストは `tests/wsm_op_work_repo.sh` を写して始める (フェイク一式、`run_op`、
  アサーション)。`tests/wsm_op_attached.sh` は写さない。`wsm op owner/repo#42` のように
  ターゲットを直接指定して fzf を通さないテストで、`fake_errors` の導入前に書いたため、
  下の偽 fzf と `fake_errors` の約束をまだ満たしていない (#24)
- POSIX sh で書き、`mktemp -d` の一時ディレクトリに PATH 先頭のフェイク
  (`wsm-server`, `fzf`, `osascript`) を置いて `bin/wsm` を丸ごと実行する。関数を直接呼ばない
  (関数の分け方を書き換えてもテストをそのまま使えるように)。実際の Ghostty やサーバーには触れない
- `ZDOTDIR` を空ディレクトリに向ける。`~/.zshenv` が PATH を前置すると、フェイクより先に
  本物の `wsm-server` / `fzf` が見つかる
- 変数の直後に日本語が続くときは `${var}` と書く。macOS の `/bin/sh` (bash 3.2) は UTF-8
  ロケールで日本語のバイトを変数名に含める。`set -u` のテスト本体では unbound variable で止まり、
  `set -u` のないフェイクでは値が黙って消えて文字が化ける (`$1` などの位置パラメータは 1 桁で
  切れるので影響しない)
- フェイクの応答は環境変数 (例 `FAKE_ATTACHED`、`FAKE_FZF_<画面名の大文字>`) で切り替え、
  呼び出しと副作用はファイルに記録して検出する
- 偽 fzf は `--prompt` で画面を見分ける。`--expect` のある画面では本物と同じ形 (1 行目 = 押したキー、
  2 行目 = 隠した列も含む行全体)、ない画面では行全体だけで答える。答えは画面ごとに `N` (N 行目) /
  `left` / `esc` を呼び出し順に書く。足りないときは ESC にせず失敗させる
- `left` は `_fzf_with_back` を通る画面 (`--expect=left`) でだけ書ける。fzf を直接呼ぶ画面
  (Session manager、Container、`wsm rm`) で書くと偽 fzf が失敗させる (本物は ← で閉じない)
- 新しい画面を通すときは、偽 fzf の prompt の `case` に画面名を足し、答えの変数
  `FAKE_FZF_<画面名の大文字>` を既定値・`export`・`start_case` の 3 か所に足す
  (`export` を忘れると、偽 fzf には「答えがない」と見える)。偽 fzf は `--opt=value` の形だけ
  読むので、bin/wsm 側も fzf のオプションはその形で渡す (`--multi` の複数選択は未対応)
- フェイクの想定外の使われ方 (知らない画面・サブコマンド・引数、答えの不足・不正) は
  `fake_errors` に書き、`bin/wsm` を実行するたびに確かめる。`bin/wsm` は fzf と server の
  失敗を捨てる経路があり、終了コードでは伝わらない
- ピッカーの分岐を変えたら、対応するケースを足す。分岐のまとまりごとに
  `tests/wsm_<コマンド>_<分岐>.sh` を作り、上の表に 1 行、CI (ci.yml) にステップを 1 つ足す。
  補助スクリプトは `wsm_` で始めない (`tests/run.sh` がテストとして実行する)
