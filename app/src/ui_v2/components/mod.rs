//! Shared UI v2 components.

pub mod aria;
pub mod blossom_media_upload;
pub mod button;
pub mod dialog;
pub mod feedback;
pub mod game_artwork;
pub mod game_card;
pub mod logo;
pub mod modal_background;
pub mod nav_item;
pub mod page_container;
pub mod page_header;
pub mod status_chip;
pub mod store_page_detail;
pub mod tabs;
pub mod topbar;
pub mod transient;
pub mod unsaved_changes_dialog;

pub use aria::aria_bool;
pub use blossom_media_upload::BlossomMediaUpload;
pub use button::{Button, ButtonSize, ButtonVariant, IconButton};
pub use dialog::{
    focus_restoration_target, initial_focus_target, resolve_close, Dialog, DialogCloseAction,
    DialogCloseButtonPolicy, DialogClosePolicy, DialogCloseRequest, DialogCloseSource,
    DialogDismissal, DialogFocusRestoration, DialogFocusTarget, DialogInitialFocus,
    DialogInitialFocusKind, DialogSourcePolicy, DialogTone, DialogWidth, ROUTE_FOCUS_FALLBACK_ID,
};
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
pub use modal_background::{scroll_lock_transition, ScrollLockAction, MODAL_OPEN_CLASS};
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
pub use transient::{
    close_transient_on_outside_pointer, close_transient_when_modal_opens, focus_transient_invoker,
    notify_modal_opened, should_close_for_modal, should_close_on_escape,
    should_close_on_outside_pointer,
};
pub use unsaved_changes_dialog::{
    create_game_dirty, guard_navigation, resolve_guard, set_create_game_dirty, GuardResolution,
    NavigationGuard, UnsavedChangesDialog, UnsavedWork,
};
