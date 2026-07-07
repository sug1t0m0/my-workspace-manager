//! ロール層: 外部ツール連携の実装。ロール同士は互いを知らない
//! (合成はオーケストレーション層 = commands モジュールが行う)。

pub mod devcontainer;
pub mod repostore;
pub mod session;
pub mod tracker;
pub mod worktree;
