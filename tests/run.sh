#!/bin/sh
# UI の回帰テスト (tests/wsm_*.sh) をすべて実行し、1 本でも失敗したら終了コード 1 を返す。
# 実行方法: sh tests/run.sh
#
# ループを CLAUDE.md に直接書かずこのファイルに置くのは、.claude/settings.json の
# 許可規則 `Bash(sh tests/*.sh)` に合う 1 行で回せるようにするため。
# 名前を wsm_ で始めないので、このファイル自身はテストとして拾われない。
cd "$(dirname "$0")/.." || exit 1
rc=0
for t in tests/wsm_*.sh; do
  sh "$t" || rc=1
done
exit "$rc"
