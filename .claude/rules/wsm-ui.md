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

## 回帰テスト (`tests/wsm_op_attached.sh`)

- POSIX sh で書き、`mktemp -d` の一時ディレクトリに PATH 先頭のフェイク
  (`wsm-server`, `osascript`, `fzf`) を置いて `bin/wsm` を直接実行する。
  実際の Ghostty やサーバーには触れない
- フェイクの応答は環境変数 (例 `FAKE_ATTACHED`) で切り替え、副作用はマーカーファイルで検出する
- 新しい分岐を固定するときは同じ方式でケースを足す。CI (ci.yml) で `sh tests/wsm_op_attached.sh`
  として実行される。macOS 以外では skip
