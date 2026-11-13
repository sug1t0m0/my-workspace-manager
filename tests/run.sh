#!/bin/sh
# UI の回帰テスト (tests/wsm_*.sh) をすべて実行し、1 本でも失敗したら終了コード 1 を返す。
# 実行方法: sh tests/run.sh (CI も同じコマンドで回す)
#
# ループを CLAUDE.md や ci.yml に書かずこのファイルに置くのは、
# - .claude/settings.json の許可規則 `Bash(sh tests/*.sh)` に合う 1 行で回せるように
# - テストを足しても ci.yml を直さずに済むように (ステップを足し忘れると、
#   そのテストは CI で黙って走らない)
# するため。名前を wsm_ で始めないので、このファイル自身はテストとして拾われない。
#
# 各テストの出力の前にファイル名を、落ちた直後に終了コードを出し、最後に落ちたファイルを
# 並べる。GitHub Actions の上では、落ちたファイルを ::error:: で PR のチェック結果にも
# 注釈として出す。
cd "$(dirname "$0")/.." || exit 1
count=0
# 落ちたファイルは位置パラメータに積む (空白区切りの文字列にすると、名前の空白や
# * で割れたり化けたりする)。for の対象はループの始めに展開済みなので影響しない
set --
for t in tests/wsm_*.sh; do
  count=$((count + 1))
  echo "== $t"
  sh "$t" || { echo "-> 失敗 (終了コード $?)"; set -- "$@" "$t"; }
done

if [ "$#" -eq 0 ]; then
  echo "ok: UI の回帰テスト ${count} 本すべて通過"
  exit 0
fi
for t in "$@"; do
  echo "FAIL: $t"
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::error file=$t,title=UI の回帰テストの失敗::$t が失敗した"
  fi
done
exit 1
