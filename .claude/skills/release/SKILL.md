---
name: release
description: wsm をリリースする。origin/main の先頭にタグ v* を打って push し、GitHub Actions が Release に成果物を添付するまでを確認する。
disable-model-invocation: true
argument-hint: "[vX.Y.Z]"
---

# /release — タグ push によるリリース

指定された引数: $ARGUMENTS

配布の仕組みは docs/wsm.md「配布は GitHub Releases で行う」。タグ `v*` を push すると
release.yml が契約テスト → ビルド → Release 作成 (`--generate-notes`) を行う。
このスキルは「次のバージョンを決めて origin/main にタグを打ち、Release が揃ったことを
確かめる」手順。タグを push する前には必ずユーザーの確認を取る。

引数が未指定 (空、または `$ARGUMENTS` のまま) なら手順 2 でバージョンを提案する。
指定されていれば手順 2 の提案を省いてその番号を使う (確認は取る)。指定された引数は
`^v[0-9]+\.[0-9]+\.[0-9]+$` の形であること。合わなければ (`v` 無しなど、release.yml が
反応しない形) 中止して指摘する。

## 1. 前提の確認

```bash
git fetch origin
git status --short                    # 作業ツリーが clean であること
git log --oneline origin/main..HEAD   # 未 push のコミットの検出
```

- タグは **origin/main の先頭** に打つ。作業ブランチの先頭ではない。したがって clean /
  未 push の確認はタグの指す先には影響せず、目的は「リリースに入れ忘れた変更の検知」。
  作業ツリーが汚れていれば中止。未 push のコミットがあれば一覧を示し、リリースに
  含めるべきものか確認してから進む (別 Issue の作業中なら、そのまま進んでよい)
- origin/main の先頭に対する CI が成功していることを確認する。未完了なら待ち、
  失敗なら中止する (main を直すのが先)。ローカルで契約テストを再実行はしない
  (CI が同じ内容を main の先頭に対して既に走らせている)

```bash
sha=$(git rev-parse origin/main)
gh run list --workflow ci --commit "$sha" --json databaseId,status,conclusion,url --jq '.[0]'
# status が completed で conclusion が success であること。
# in_progress なら: gh run watch <databaseId> --exit-status
```

## 2. バージョンの提案

```bash
last=$(git describe --tags --abbrev=0 origin/main)
git log --format='%s' "$last..origin/main"
```

直前タグ以降のコミットの接頭辞で上げ幅を決め、候補を 1 つ提案する:

- `feat:` または `perf:` が 1 件以上 → **マイナー** を上げる (v0.8.5 → v0.9.0)
- `fix:` / `docs:` / `test:` / `ci:` / `chore:` / `refactor:` のみ → **パッチ** を上げる
- docs/wsm.md「Tracker プラグイン契約」の動詞を増減した変更が含まれる → マイナー扱いの
  まま、手順 5 で Release ノートにプラグイン側への影響を追記する (0.x の間は
  メジャーを上げない)

コミット一覧と候補をユーザーに示し、**その番号でよいか確認する** (別の番号の指定も可)。
確認が取れるまでタグを打たない。

## 3. タグを打って push

```bash
git tag "$version" origin/main
git push origin "$version"
```

## 4. Release ワークフローの完了を待つ

タグ push の run は `headBranch` がタグ名になる。push 直後は run がまだ登録されて
いないことがあるので、`--branch "$version"` で絞り、現れるまで数秒おきに再試行する
(絞らずに `.[0]` を取ると前回リリースの completed を拾って誤判定する)。

```bash
gh run list --workflow release --branch "$version" --json databaseId,status --jq '.[0]'
gh run watch <databaseId> --exit-status
gh release view "$version" --json assets --jq '.assets[].name'
```

成果物が 4 点揃っていることを確認する:
`wsm-server-aarch64-apple-darwin`、`wsm-tracker-github-aarch64-apple-darwin`、
`wsm-tracker-github-api-aarch64-apple-darwin`、`wsm`。

## 5. Release ノート (必要なときだけ)

手順 2 でプラグイン契約の変更があった場合のみ、自動生成ノートの冒頭に
「プラグイン側への影響」(追加・廃止した動詞と、旧プラグインの見え方) を追記する。

```bash
gh release view "$version" --json body --jq .body > "$TMPDIR/notes.md"
# 冒頭に追記してから
gh release edit "$version" --notes-file "$TMPDIR/notes.md"
```

それ以外は自動生成ノートのままにする。

## 6. 完了報告

Release の URL と成果物 4 点を報告し、最後に 1 行だけ案内する:
「ローカルへの反映は dotfiles 側 (chezmoi) で取り込み、`wsm doctor` で確認してください」。
dotfiles の手順はこのリポジトリの範囲外なので、ここには書かない。

## 失敗したとき

- release ワークフローが落ちた (Release 未作成): 原因を直して main に入れたあと、
  タグを消して同じ番号で打ち直す。Release が作られていないので番号は再利用してよい

  ```bash
  git push --delete origin "$version"
  git tag -d "$version"
  ```

- Release は作られたが成果物が欠けた: Release とリモートタグを消し、ローカルタグも消して、
  原因を直してから打ち直す (`--yes` が無いと非対話では確認待ちで失敗する)

  ```bash
  gh release delete "$version" --cleanup-tag --yes
  git tag -d "$version"
  ```

- 手順 1 で CI が失敗していた: リリースを中止し、main を直すのが先
