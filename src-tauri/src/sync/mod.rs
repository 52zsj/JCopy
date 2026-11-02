pub mod crypto;
pub mod engine;
pub mod lan;
pub mod protocol;

// 只有 SyncClip 被 crate 外部直接用（clipboard.rs 构造局域网广播载荷），
// 其余类型在 sync 内部走 protocol:: 路径，不需要再导出
pub use protocol::SyncClip;
