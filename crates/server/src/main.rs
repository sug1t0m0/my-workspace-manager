// wsm-server: Workspace Manager core (JSON API)
// 仕様は docs/wsm.md。UI 層 (bin/wsm) はこの JSON API だけに依存し、
// server の実装を無変更で差し替えられる。
//
// レイヤー (docs/wsm.md のレイヤー構成に対応):
//   presentations … CLI の入出力 (引数 → ドメイン型、結果 → JSON)
//   usecases      … オーケストレーション (依存の順序と合成ビュー)
//   roles         … 外部ツール連携 (Tracker / RepoStore / SessionManager / ...)
//   infra         … プロセス起動・マシン設定
//   wsm-shared::domains … 識別子と導出規則 (純粋、client と共有)

mod infra;
mod presentations;
mod roles;
mod usecases;

use std::process::ExitCode;

fn main() -> ExitCode {
    presentations::cli::main()
}
