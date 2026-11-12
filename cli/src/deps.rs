// CLI 外部依赖统一入口。
// 命令行解析。
pub use clap::{Parser, Subcommand};
// 核心业务与播放存储。
pub use kugou_core::application::commands::{AppCommand, TargetScope};
pub use kugou_core::application::coordinator::Coordinator;
pub use kugou_core::audio::controller::AudioController;
pub use kugou_core::domain::ids::TrackId;
pub use kugou_core::domain::track::AudioQuality;
pub use kugou_core::provider::kugou::{auth::KugouAuth, catalog::KugouCatalog, client::KugouClient, library::KugouLibrary, stream::KugouStream};
pub use kugou_core::storage::database::DatabaseStorage;
