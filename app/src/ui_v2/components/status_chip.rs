use leptos::prelude::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusChipVariant {
    Neutral,
    Active,
    Success,
    Warning,
    Error,
    Pending,
    Unavailable,
    Public,
    Gated,
    TimedAccess,
    Owned,
    Installed,
    Downloading,
    UpdateAvailable,
    Draft,
    Published,
    Cancelled,
    Expired,
    Verified,
    Unverified,
}

impl StatusChipVariant {
    pub const fn class(self) -> &'static str {
        match self {
            Self::Neutral => "arc-status-neutral",
            Self::Active => "arc-status-active",
            Self::Success => "arc-status-success",
            Self::Warning => "arc-status-warning",
            Self::Error => "arc-status-error",
            Self::Pending => "arc-status-pending",
            Self::Unavailable => "arc-status-unavailable",
            Self::Public => "arc-status-public",
            Self::Gated => "arc-status-gated",
            Self::TimedAccess => "arc-status-timed",
            Self::Owned => "arc-status-owned",
            Self::Installed => "arc-status-installed",
            Self::Downloading => "arc-status-downloading",
            Self::UpdateAvailable => "arc-status-update",
            Self::Draft => "arc-status-draft",
            Self::Published => "arc-status-published",
            Self::Cancelled => "arc-status-cancelled",
            Self::Expired => "arc-status-expired",
            Self::Verified => "arc-status-verified",
            Self::Unverified => "arc-status-unverified",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StatusChipSize {
    Compact,
    #[default]
    Standard,
}

#[component]
pub fn StatusChip(
    #[prop(into)] label: String,
    variant: StatusChipVariant,
    icon: Option<&'static str>,
    #[prop(optional)] size: StatusChipSize,
) -> impl IntoView {
    let class = format!(
        "arc-status-chip {} {}",
        variant.class(),
        if size == StatusChipSize::Compact {
            "arc-status-compact"
        } else {
            "arc-status-standard"
        }
    );

    view! {
        <span class=class>
            {icon.map(|icon| view! {
                <span class="material-symbols-outlined" aria-hidden="true">{icon}</span>
            })}
            <span>{label}</span>
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_map_to_semantic_classes() {
        assert_eq!(StatusChipVariant::Public.class(), "arc-status-public");
        assert_eq!(StatusChipVariant::TimedAccess.class(), "arc-status-timed");
        assert_eq!(StatusChipVariant::Owned.class(), "arc-status-owned");
        assert_eq!(StatusChipVariant::Installed.class(), "arc-status-installed");
        assert_eq!(StatusChipVariant::Published.class(), "arc-status-published");
        assert_eq!(
            StatusChipVariant::Unverified.class(),
            "arc-status-unverified"
        );
    }

    #[test]
    fn compact_and_standard_sizes_are_distinct() {
        assert_ne!(StatusChipSize::Compact, StatusChipSize::Standard);
    }
}
