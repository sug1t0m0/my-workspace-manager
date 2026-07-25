#!/bin/sh
# bin/wsm の attached 分岐 (issue #5) の回帰テスト。
# 実行方法: sh tests/wsm_op_attached.sh (要 jq。macOS 以外は skip)
#
# contract テストのフェイク方式 (PATH 先頭にスタブを置き呼び出しを記録) に
# 合わせ、wsm-server と osascript をフェイクに差し替えて、open 応答の
# attached の真偽でタブを開く副作用 (osascript 呼び出し) が変わることを固定する。
set -eu

# _terminal_available は macOS (darwin) が前提。他 OS では分岐に入らないため skip
case "$(uname)" in
  Darwin) ;;
  *) echo "skip: macOS only (bin/wsm opens tabs only on darwin)"; exit 0 ;;
esac

repo_root=$(cd "$(dirname "$0")/.." && pwd)
# macOS の mktemp は引数なしだと TMPDIR を無視するためテンプレートで指定
tmp=$(mktemp -d "${TMPDIR:-/tmp}/wsm-op-attached.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

fakebin="$tmp/bin"
mkdir -p "$fakebin"

# wsm-server フェイク: FAKE_ATTACHED に応じた open 応答を返す
cat > "$fakebin/wsm-server" <<'EOF'
#!/bin/sh
printf '{"status":"ok","message":"opened","session":"owner_repo_42","attach_command":"tmux attach-session -t owner_repo_42","attached":%s}\n' "${FAKE_ATTACHED:-false}"
EOF

# osascript フェイク: 呼ばれたら MARKER に記録するだけ (実 Ghostty は触らない)
cat > "$fakebin/osascript" <<'EOF'
#!/bin/sh
: >> "$MARKER"
EOF

# fzf フェイク: _check_deps の存在確認用 (直接ターゲット指定では呼ばれない)
cat > "$fakebin/fzf" <<'EOF'
#!/bin/sh
exit 1
EOF

chmod +x "$fakebin/wsm-server" "$fakebin/osascript" "$fakebin/fzf"

PATH="$fakebin:$PATH"
export PATH
WSM_TRANSPORT=local
export WSM_TRANSPORT

fail=0

# --- attached=true → タブを開かない (osascript は呼ばれない) ---
MARKER="$tmp/marker_true"
export MARKER
out=$(FAKE_ATTACHED=true "$repo_root/bin/wsm" op owner/repo#42)
if [ -e "$MARKER" ]; then
  echo "FAIL: attached=true なのに osascript が呼ばれた (タブが開いてしまう)"
  fail=1
fi
case "$out" in
  *"アタッチ済み"*) ;;
  *) echo "FAIL: attached=true のスキップメッセージが出ていない: $out"; fail=1 ;;
esac

# --- attached=false → タブを開く (osascript が呼ばれる) ---
MARKER="$tmp/marker_false"
export MARKER
FAKE_ATTACHED=false "$repo_root/bin/wsm" op owner/repo#42 > /dev/null
if [ ! -e "$MARKER" ]; then
  echo "FAIL: attached=false なのに osascript が呼ばれていない (タブが開かない)"
  fail=1
fi

if [ "$fail" -eq 0 ]; then
  echo "ok: attached true/false の 2 ケース通過"
fi
exit "$fail"
