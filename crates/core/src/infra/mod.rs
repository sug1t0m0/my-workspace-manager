//! Infrastructure 層: プロセス起動 (exec) とマシン設定 (settings)。
//! 副作用の技術的な詳細をここに閉じ、上位レイヤーは語彙だけを使う。

pub mod exec;
pub mod settings;
