---
paths:
  - "crates/tracker-*/**"
  - "crates/server/src/roles/tracker.rs"
  - "examples/**"
---

# Tracker プラグインの変更・追加

契約の正は docs/wsm.md「Tracker プラグイン契約」。役割と性質 (読み取り専用・非対話・
設定は環境変数)、動詞の表、成功・失敗の契約、信頼境界、バージョニングはそこを読む。
ここには docs に無い運用上の知識と、変更時のチェックリストだけを置く。
見本は `examples/wsm-tracker-demo` (シェル 1 枚で全動詞を満たす)。

## 変更前に確認すること

- 変更が互換か非互換かを docs の「バージョニング (動詞名 = バージョン)」節で判定する。
  非互換なら **新しい動詞** (`-v1` 等) を作り、旧動詞はフォールバック先として残す。
  server 側のフォールバック順 (v2 → v1 → v0) は `crates/server/src/roles/tracker.rs`
- 新しい動詞は `PROTOCOL` 配列と `USAGE` 文字列の **両方** に足す (`info-v0` の `protocol` が
  対応動詞の宣言になる。片方だけだと診断と実挙動がずれる)

## docs に無い実装の知識

- gh 版は gh を `GH_PROMPT_DISABLED=1` で起動して非対話を保証する。api 版は認証だけ
  `gh auth token` に借り、プロセス内でキャッシュする
- `has_children` は api 版では **open な子だけ** を数える (掘った先が空になる `▸` を出さない)
- `repo` フィールドは `list-group-issues-v0` では常に必須、他の動詞では所属リポジトリが
  照会先と違うとき (リポジトリ横断の子 Issue) に付ける
- server 側 (`roles/tracker.rs`) はプラグインの出力を信頼せず id / repo の形を検証して
  不正要素を捨てる。プラグイン側は引数が検証済みであることを前提にしてよい

## 同時に更新するもの

- 契約テスト `crates/tracker-*/tests/contract.rs` (書き方は `.claude/rules/contract-tests.md`)
- docs/wsm.md「Tracker プラグイン契約」の動詞表 (契約を変えた場合)
- `examples/wsm-tracker-demo` (新動詞を足した場合。UI 開発の治具を兼ねる)
- 配布物を増やす場合は `.github/workflows/release.yml` の成果物と README のインストール節
