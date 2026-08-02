//! Shared UI v2 components.

pub mod blossom_media_upload;
pub mod button;
pub mod feedback;
pub mod game_artwork;
pub mod game_card;
pub mod logo;
pub mod nav_item;
pub mod page_container;
pub mod page_header;
pub mod status_chip;
pub mod store_page_detail;
pub mod tabs;
pub mod topbar;

pub use blossom_media_upload::BlossomMediaUpload;
pub use button::{Button, ButtonSize, ButtonVariant, IconButton};
pub use feedback::{
    EmptyState, ErrorSeverity, ErrorState, FeedbackLayout, GameCardSkeleton, InlineLoading,
    LoadingState, PartialRelayKind, PartialRelayState, Skeleton, SkeletonKind,
};
pub use game_artwork::{artwork_state_from_url, ArtworkRole, ArtworkState, GameArtwork};
pub use game_card::{
    GameCard, GameCardAccess, GameCardAction, GameCardActionPresentation, GameCardCampaign,
    GameCardDensity, GameCardPresentation, GameCardStatus, GameCardVisual, GameCardVisualContent,
    PlatformCompatibility,
};
pub use logo::ArcadestrLogo;
pub use nav_item::{MobileNavItem, NavItem};
pub use page_container::{ClippedPanel, PageContainer};
pub use page_header::PageHeader;
pub use status_chip::{StatusChip, StatusChipSize, StatusChipVariant};
pub use store_page_detail::StorePageRichDetail;
pub use tabs::{
    PageTabItem, PageTabSemantics, PageTabTarget, PageTabs, PublisherDestination, PublisherTabItem,
    PublisherTabs,
};
pub use topbar::TopBar;
