#!/bin/sh
# bin/wsm の作業リポジトリ選択 (_pick_work_repo) と、ピッカーの ← / ESC / 選択
# (_fzf_with_back) の分岐の回帰テスト (issue #12)。
# 実行方法: sh tests/wsm_op_work_repo.sh (要 jq)
#
# tests/wsm_op_attached.sh と同じく PATH 先頭にフェイク (wsm-server, fzf) を置き、
# `wsm op -m tmux` を丸ごと実行する。グループ → Issue → 作業リポジトリの画面を
# 通し、出た画面と、server に頼んだ問い合わせ・open の引数で分岐を確かめる。
set -eu

repo_root=$(cd "$(dirname "$0")/.." && pwd)
# macOS の mktemp は引数なしだと TMPDIR を無視するためテンプレートで指定
tmp=$(mktemp -d "${TMPDIR:-/tmp}/wsm-op-work-repo.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

fakebin="$tmp/bin"
FAKE_DIR="$tmp/fake"
zdotdir="$tmp/zdotdir"
mkdir -p "$fakebin" "$FAKE_DIR" "$zdotdir"
export FAKE_DIR

# フェイクは想定外の使われ方 (知らない画面・サブコマンド・引数、答えの不足・不正) を
# fake_errors に書く。bin/wsm は fzf の失敗を `|| true` で、server の失敗を
# コマンド置換で捨てる経路があり、終了コードではテストに伝わらないため

# wsm-server フェイク: 呼ばれた引数を server.log に記録し、サブコマンドごとの
# 固定応答を返す。グループの Issue は owner/api#42 の 1 件。list-repos の応答は
# FAKE_HOME_REPOS (所属リポジトリの問い合わせ) と FAKE_GROUP_REPOS (グループの
# 問い合わせ) で切り替える
cat > "$fakebin/wsm-server" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_DIR/server.log"
case "$1" in
  list-repo-groups)
    echo '[{"tracker":"t1","id":"g1","title":"Group 1"}]' ;;
  list-group-issues)
    echo '{"issues":[{"id":"42","title":"Fix bug","repo":"owner/api","active":false,"closed":false}]}' ;;
  list-repos)
    case "$2" in
      --repo)  printf '%s\n' "$FAKE_HOME_REPOS" ;;
      --group) printf '%s\n' "$FAKE_GROUP_REPOS" ;;
      *)
        echo "wsm-server: 想定外の list-repos の引数 $*" >> "$FAKE_DIR/fake_errors"
        echo '{"error":"fake: unexpected list-repos args"}'; exit 1 ;;
    esac ;;
  list-devcontainer-configs)
    echo '[]' ;;
  open)
    echo '{"status":"ok","message":"opened","session":"s","attach_command":"true","attached":true}' ;;
  *)
    echo "wsm-server: 想定外のサブコマンド $*" >> "$FAKE_DIR/fake_errors"
    echo "{\"error\":\"fake: unexpected subcommand $1\"}"; exit 1 ;;
esac
EOF

# fzf フェイク: --prompt で画面を見分け、FAKE_FZF_<画面> の答えを返す。
# 答えは空白区切りで、その画面の n 回目の呼び出しに n 番目を使う。
#   数字 N .. N 行目を Enter で選ぶ
#   left .... ← を押す (--expect のある画面だけ。本物は --expect がないと ← で閉じない)
#   esc ..... ESC (出力なし、終了コード 130)
# 答えが足りない・不正なときは ESC にせず fake_errors に書く (打ち間違いで
# ESC のケースが誤って通るのを防ぐ)。オプションは bin/wsm が --opt=value の
# 形で渡す前提で読む。
# 出力は本物の fzf の --expect に合わせる: 1 行目 = 押したキー (Enter なら空行)、
# 2 行目 = 行全体 (--with-nth で隠した列も含む)。fzf 0.74.3 で確認した形。
# 呼ばれた画面を fzf.log に、受け取った候補とヘッダを fzf_in_<画面> /
# fzf_header_<画面> に残す (同じ画面が 2 回出たら最後の呼び出しの分)
cat > "$fakebin/fzf" <<'EOF'
#!/bin/sh
bad() {
  echo "fzf($screen): $1" >> "$FAKE_DIR/fake_errors"
  exit 2
}
prompt="" header="" expect=false screen=unknown
for arg in "$@"; do
  case "$arg" in
    --prompt=*) prompt=${arg#--prompt=} ;;
    --header=*) header=${arg#--header=} ;;
    --expect=*) expect=true ;;
  esac
done
case "$prompt" in
  "Repo group> ") screen=group;     answers=${FAKE_FZF_GROUP:-} ;;
  "Issue> ")      screen=issue;     answers=${FAKE_FZF_ISSUE:-} ;;
  "Work repo> ")  screen=work_repo; answers=${FAKE_FZF_WORK_REPO:-} ;;
  *)              bad "想定外の画面 (prompt=$prompt)" ;;
esac
cat > "$FAKE_DIR/fzf_in_$screen"
printf '%s\n' "$header" > "$FAKE_DIR/fzf_header_$screen"
echo "$screen" >> "$FAKE_DIR/fzf.log"
n=$(grep -cx "$screen" "$FAKE_DIR/fzf.log")
answer=$(printf '%s\n' $answers | sed -n "${n}p")
case "$answer" in
  esc)
    exit 130 ;;
  left)
    [ "$expect" = true ] || bad "${n} 回目の答え left は --expect のない画面では使えない"
    key=left
    line=$(sed -n 1p "$FAKE_DIR/fzf_in_$screen") ;;
  "")
    var=FAKE_FZF_$(printf '%s' "$screen" | tr '[:lower:]' '[:upper:]')
    bad "${n} 回目の呼び出しに答えがない (画面が想定より多く出たか、esc の書き忘れか、${var} の export 漏れ)" ;;
  *[!0-9]*)
    bad "${n} 回目の答え「${answer}」が不正 (数字 / left / esc を答えに書く)" ;;
  *)
    key=""
    line=$(sed -n "${answer}p" "$FAKE_DIR/fzf_in_$screen")
    [ -n "$line" ] || bad "${answer} 行目の候補がない" ;;
esac
if [ "$expect" = true ]; then
  printf '%s\n' "$key"
fi
printf '%s\n' "$line"
EOF

# osascript フェイク: open 応答は attached=true なのでタブは開かないはずだが、
# 万一呼ばれても実 Ghostty に触れないよう差し替えておく
cat > "$fakebin/osascript" <<'EOF'
#!/bin/sh
exit 0
EOF

chmod +x "$fakebin/wsm-server" "$fakebin/fzf" "$fakebin/osascript"

PATH="$fakebin:$PATH"
# bin/wsm (zsh) にユーザーの ~/.zshenv を読ませない。そこで PATH を前置されると、
# フェイクより先に本物の wsm-server / fzf が見つかってしまう
ZDOTDIR="$zdotdir"
WSM_TRANSPORT=local
FAKE_HOME_REPOS='[]'
FAKE_GROUP_REPOS='[]'
FAKE_FZF_GROUP=""
FAKE_FZF_ISSUE=""
FAKE_FZF_WORK_REPO=""
export PATH ZDOTDIR WSM_TRANSPORT FAKE_HOME_REPOS FAKE_GROUP_REPOS \
  FAKE_FZF_GROUP FAKE_FZF_ISSUE FAKE_FZF_WORK_REPO

failed=0
cases=0
case_name=""

# ケースの開始 (共通の Arrange): 記録を消し、フェイクを既定に戻す。グループと
# Issue の画面は 1 行目 (Group 1、owner/api#42) を選んで作業リポジトリの画面まで進む
start_case() {
  case_name=$1
  cases=$((cases + 1))
  rm -f "$FAKE_DIR"/*
  : > "$FAKE_DIR/server.log"
  : > "$FAKE_DIR/fzf.log"
  FAKE_HOME_REPOS='[]'
  FAKE_GROUP_REPOS='[]'
  FAKE_FZF_GROUP=1
  FAKE_FZF_ISSUE=1
  FAKE_FZF_WORK_REPO=1
}

# -m tmux はセッションマネージャーの画面を飛ばし、グループの画面から始めるため
run_op() {
  if "$repo_root/bin/wsm" op -m tmux > "$tmp/stdout" 2> "$tmp/stderr"; then
    exit_code=0
  else
    exit_code=$?
  fi
  if [ -s "$FAKE_DIR/fake_errors" ]; then
    fail "フェイクの想定外の使われ方: $(tr '\n' ';' < "$FAKE_DIR/fake_errors")"
  fi
}

fail() {
  echo "FAIL [$case_name]: $1"
  failed=1
}

assert_exit() {
  [ "$exit_code" -eq "$1" ] || fail "終了コードが $1 のはずが $exit_code (stderr: $(cat "$tmp/stderr"))"
}

assert_exit_nonzero() {
  [ "$exit_code" -ne 0 ] || fail "失敗で終わるはずが終了コード 0"
}

# server に $1 (サブコマンドと引数の全体) で問い合わせたこと
assert_called() {
  grep -Fqx "$1" "$FAKE_DIR/server.log" \
    || fail "「$1」が呼ばれていない (server: $(tr '\n' ';' < "$FAKE_DIR/server.log"))"
}

assert_opened() {
  assert_called "open --repo $1 --issue $2"
}

assert_not_opened() {
  if grep -q '^open ' "$FAKE_DIR/server.log"; then
    fail "open が呼ばれた: $(grep '^open ' "$FAKE_DIR/server.log")"
  fi
}

# 画面 $1 が $2 回出たこと
assert_screen_count() {
  n=$(grep -cx "$1" "$FAKE_DIR/fzf.log" || true)
  [ "$n" -eq "$2" ] || fail "$1 の画面が $2 回のはずが $n 回"
}

# 以下の画面の中身の確認は、画面が出ていなければ FAIL にして次へ進む
# (set -e でスクリプトごと止まると、残りのケースの結果が見えなくなる)

# 画面 $1 の候補の並び (キー列) が $2 (改行区切り) であること
assert_candidates() {
  [ -f "$FAKE_DIR/fzf_in_$1" ] || { fail "$1 の画面が出ていないので候補を確かめられない"; return 0; }
  actual=$(cut -f1 "$FAKE_DIR/fzf_in_$1")
  [ "$actual" = "$2" ] || fail "$1 の候補が [$2] のはずが [$actual]"
}

assert_header_contains() {
  [ -f "$FAKE_DIR/fzf_header_$1" ] || { fail "$1 の画面が出ていないのでヘッダ「$2」を確かめられない"; return 0; }
  grep -Fq "$2" "$FAKE_DIR/fzf_header_$1" \
    || fail "$1 のヘッダに「$2」がない: $(cat "$FAKE_DIR/fzf_header_$1")"
}

assert_header_not_contains() {
  [ -f "$FAKE_DIR/fzf_header_$1" ] || { fail "$1 の画面が出ていないのでヘッダに「$2」がないことを確かめられない"; return 0; }
  if grep -Fq "$2" "$FAKE_DIR/fzf_header_$1"; then
    fail "$1 のヘッダに「$2」がある: $(cat "$FAKE_DIR/fzf_header_$1")"
  fi
}

assert_stderr_contains() {
  grep -Fq "$1" "$tmp/stderr" || fail "stderr に「$1」がない: $(cat "$tmp/stderr")"
}

# ===== _pick_work_repo: 候補の数と所属リポジトリの有無による分岐 =====

# --- 所属のみ: 候補 1 件 → 画面を出さずに所属リポジトリで確定 ---
# グループの問い合わせにも所属リポジトリが返るが、候補として重複させない
start_case "所属のみ"
# Arrange
FAKE_HOME_REPOS='[{"ns_repo":"owner/api","active_count":0}]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/api","active_count":0}]'
# Act
run_op
# Assert
assert_exit 0
assert_screen_count work_repo 0
assert_opened owner/api 42

# --- 0 件: 開けるリポジトリがない → 画面を出さずにエラー ---
start_case "0 件"
# Arrange
FAKE_HOME_REPOS='[]'
FAKE_GROUP_REPOS='[]'
# Act
run_op
# Assert
assert_exit_nonzero
assert_stderr_contains "No local repositories in this group"
assert_screen_count work_repo 0
assert_not_opened

# --- 所属 + グループ: 画面を出し、所属リポジトリを先頭に並べる ---
# 所属と違うリポジトリを選ぶと、workspace id は所属リポジトリ名の接頭辞付き (api-42)
start_case "所属 + グループ"
# Arrange
FAKE_HOME_REPOS='[{"ns_repo":"owner/api","active_count":1}]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/api","active_count":1},{"ns_repo":"owner/web","active_count":0}]'
FAKE_FZF_WORK_REPO=2
# Act
run_op
# Assert
assert_exit 0
assert_called "list-repos --repo owner/api"
assert_called "list-repos --group g1 --tracker t1"
assert_screen_count work_repo 1
assert_candidates work_repo "owner/api
owner/web"
assert_header_contains work_repo "Issue の所属: owner/api"
assert_header_not_contains work_repo "ローカルになし"
assert_opened owner/web api-42

# --- グループのみ 1 件: 所属リポジトリがローカルにない → 残る 1 件で確定 ---
start_case "グループのみ 1 件"
# Arrange
FAKE_HOME_REPOS='[]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/web","active_count":0}]'
# Act
run_op
# Assert
assert_exit 0
assert_screen_count work_repo 0
assert_opened owner/web api-42

# --- グループのみ複数: 画面を出し、ヘッダで所属がローカルにないことを示す ---
start_case "グループのみ複数"
# Arrange
FAKE_HOME_REPOS='[]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/web","active_count":0},{"ns_repo":"owner/cli","active_count":0}]'
FAKE_FZF_WORK_REPO=2
# Act
run_op
# Assert
assert_exit 0
assert_called "list-repos --repo owner/api"
assert_called "list-repos --group g1 --tracker t1"
assert_screen_count work_repo 1
assert_candidates work_repo "owner/web
owner/cli"
assert_header_contains work_repo "ローカルになし"
assert_opened owner/cli api-42

# --- 古い server: --repo を無視して全件を返しても、所属の有無は ns_repo で判定する ---
# 全件に所属 (owner/api) を含めないのは、絞り込みが抜けたとき先頭の他リポジトリを
# 所属と取り違えて「ローカルにある」と判定する退行を見るため
start_case "古い server"
# Arrange
FAKE_HOME_REPOS='[{"ns_repo":"owner/web","active_count":0},{"ns_repo":"owner/cli","active_count":0}]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/web","active_count":0},{"ns_repo":"owner/cli","active_count":0}]'
FAKE_FZF_WORK_REPO=1
# Act
run_op
# Assert
assert_exit 0
assert_candidates work_repo "owner/web
owner/cli"
assert_header_contains work_repo "ローカルになし"
assert_opened owner/web api-42

# ===== _fzf_with_back: 作業リポジトリの画面での ← / ESC / 選択 =====

# --- 選択: 選んだ行のリポジトリで開く (先頭ではない 2 行目を選ぶ) ---
start_case "選択"
# Arrange
FAKE_HOME_REPOS='[{"ns_repo":"owner/api","active_count":0}]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/web","active_count":0}]'
FAKE_FZF_WORK_REPO=2
# Act
run_op
# Assert
assert_exit 0
assert_opened owner/web api-42

# --- ←: 1 つ前の Issue の画面に戻る (グループまでは戻らない)。戻った先で ESC して終える ---
start_case "←"
# Arrange
FAKE_HOME_REPOS='[{"ns_repo":"owner/api","active_count":0}]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/web","active_count":0}]'
FAKE_FZF_ISSUE="1 esc"
FAKE_FZF_WORK_REPO=left
# Act
run_op
# Assert
assert_exit 0
assert_screen_count group 1
assert_screen_count issue 2
assert_screen_count work_repo 1
assert_not_opened

# --- ESC: 前の画面に戻らず、何も開かずに終わる ---
start_case "ESC"
# Arrange
FAKE_HOME_REPOS='[{"ns_repo":"owner/api","active_count":0}]'
FAKE_GROUP_REPOS='[{"ns_repo":"owner/web","active_count":0}]'
FAKE_FZF_WORK_REPO=esc
# Act
run_op
# Assert
assert_exit 0
assert_screen_count issue 1
assert_screen_count work_repo 1
assert_not_opened

if [ "$failed" -eq 0 ]; then
  echo "ok: 作業リポジトリ選択の $cases ケース通過"
fi
exit "$failed"
