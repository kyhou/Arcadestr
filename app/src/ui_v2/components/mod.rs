//! Shared UI v2 components.

pub mod game_card;
pub mod nav_item;
pub mod page_header;
pub mod topbar;

pub use game_card::{
    GameCard, GameCardAccess, GameCardAction, GameCardCampaign, GameCardPresentation,
    PlatformCompatibility,
};
pub use nav_item::{MobileNavItem, NavItem};
pub use page_header::PageHeader;
pub use topbar::TopBar;
