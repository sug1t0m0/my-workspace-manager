//! wsm の共有ライブラリ。core と (将来の) client が共有する語彙を置く。
//! 副作用を持たない純粋なレイヤーのみ (プロセス起動や IO は core 側の infra)。

pub mod domains;
