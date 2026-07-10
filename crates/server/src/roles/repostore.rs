//! RepoStore ロール: ローカルにあるリポジトリの列挙とクローン本体のパス解決。
//! 読み取り専用。ソースは 2 つ: ghq (`ghq list`、任意の host) と、設定
//! `[[repo]]` で登録された ghq 管理外のクローン。
//!
//! リポジトリの識別子は `<ns>/<repo>` で、host はストアが解決するメタ情報。
//! ns/repo は全ソース・全 host を横断して一意であることを規約とし、
//! 重複はエラーにする。出力の形が不正な行は捨てる。

use crate::infra::{exec, settings};
use std::path::{Path, PathBuf};
use wsm_shared::domains::{self as domain, RepoEntry, RepoRef};

/// ghq のルート (`ghq root` を尊重。取得できなければ ~/ghq)。
pub fn root(home: &Path) -> PathBuf {
    exec::stdout_if_ok("ghq", &["root"])
        .and_then(|out| {
            out.lines().next().map(str::trim).filter(|line| !line.is_empty()).map(PathBuf::from)
        })
        .unwrap_or_else(|| home.join("ghq"))
}

/// ストアの全エントリ (ghq の出力順 → 設定 `[[repo]]` の記述順)。
pub fn entries(home: &Path) -> Result<Vec<RepoEntry>, String> {
    let mut entries = ghq_entries(&root(home));
    entries.extend(settings::custom_repos(home)?);
    Ok(entries)
}

/// 識別子 `<ns>/<repo>` からエントリを探す。Ok(None) = 未登録 (Issue の照会は
/// ローカルクローンなしでも成立するため、呼び出し側が扱いを決める)。
/// 複数の host にまたがる重複と設定の誤りはエラー (識別子の一意性が規約)。
pub fn find(home: &Path, repo: &RepoRef) -> Result<Option<RepoEntry>, String> {
    let mut matches: Vec<RepoEntry> =
        entries(home)?.into_iter().filter(|entry| entry.repo == *repo).collect();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        _ => Err(format!(
            "ambiguous repository: {} ({})",
            repo.ns_repo(),
            matches.iter().map(|entry| entry.host.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// find() の必須版 (open など、クローンの実体が要る操作に使う)。
pub fn lookup(home: &Path, repo: &RepoRef) -> Result<RepoEntry, String> {
    find(home, repo)?.ok_or_else(|| format!("repository not found: {}", repo.ns_repo()))
}

fn ghq_entries(root: &Path) -> Vec<RepoEntry> {
    exec::stdout_if_ok("ghq", &["list"])
        .map(|out| out.lines().filter_map(|line| ghq_entry(root, line)).collect())
        .unwrap_or_default()
}

/// `ghq list` の 1 行 `<host>/<ns>/<repo>` をエントリにする。サブグループ等で
/// 3 セグメントを超える行は repo 名の検証で落ちる (現状の非対応を明示)。
fn ghq_entry(root: &Path, line: &str) -> Option<RepoEntry> {
    let mut parts = line.splitn(3, '/');
    let (host, ns, name) = (parts.next()?, parts.next()?, parts.next()?);
    if !domain::is_valid_host(host) {
        return None;
    }
    let repo = RepoRef::parse(&format!("{ns}/{name}"))?;
    Some(RepoEntry {
        clone_path: root.join(host).join(ns).join(name),
        host: host.to_owned(),
        repo,
        tracker: None, // ghq のエントリは常に既定トラッカー
    })
}
