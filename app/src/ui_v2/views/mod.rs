//! UI v2 view modules.

pub mod browse_games;
pub mod store_front;
pub mod game_detail;
pub mod library;
pub mod login;
pub mod marketplace_loader;
pub mod profile;
pub mod publish;
pub mod social;

pub use browse_games::BrowseGamesView;
pub use game_detail::GameDetailView;
pub use library::LibraryView;
pub use login::LoginV2View;
pub use profile::ProfileV2View;
pub use publish::PublishV2View;
pub use social::SocialView;
pub use store_front::StoreFrontView;
