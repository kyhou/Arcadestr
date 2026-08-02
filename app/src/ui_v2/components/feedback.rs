use leptos::ev::MouseEvent;
use leptos::prelude::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FeedbackLayout {
    Inline,
    Compact,
    #[default]
    Panel,
    FullPage,
}

impl FeedbackLayout {
    const fn class(self) -> &'static str {
        match self {
            Self::Inline => "arc-feedback-inline",
            Self::Compact => "arc-feedback-compact",
            Self::Panel => "arc-feedback-panel",
            Self::FullPage => "arc-feedback-full",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkeletonKind {
    Text,
    Panel,
    Card,
}

impl SkeletonKind {
    const fn class(self) -> &'static str {
        match self {
            Self::Text => "arc-skeleton-text",
            Self::Panel => "arc-skeleton-panel",
            Self::Card => "arc-skeleton-card",
        }
    }
}

#[component]
pub fn Skeleton(
    kind: SkeletonKind,
    #[prop(optional, into)] class: Option<String>,
) -> impl IntoView {
    let class = format!(
        "arc-skeleton {} {}",
        kind.class(),
        class.unwrap_or_default()
    );
    view! { <span class=class aria-hidden="true"></span> }
}

#[component]
pub fn InlineLoading(#[prop(into)] label: String) -> impl IntoView {
    view! {
        <span class="arc-inline-loading" role="status" aria-live="polite">
            <span class="arc-loading-mark" aria-hidden="true"></span>
            <span>{label}</span>
        </span>
    }
}

#[component]
pub fn LoadingState(
    #[prop(into)] title: String,
    #[prop(optional, into)] description: Option<String>,
    #[prop(optional)] layout: FeedbackLayout,
) -> impl IntoView {
    view! {
        <section
            class=format!("arc-feedback arc-feedback-loading {}", layout.class())
            role="status"
            aria-live="polite"
            aria-busy="true"
        >
            <span class="arc-loading-mark" aria-hidden="true"></span>
            <div>
                <h2>{title}</h2>
                {description.map(|description| view! { <p>{description}</p> })}
            </div>
        </section>
    }
}

#[component]
pub fn GameCardSkeleton(
    #[prop(optional)] announce: bool,
    #[prop(optional)] browse: bool,
) -> impl IntoView {
    view! {
        <article
            class="arc-game-card arc-game-card-skeleton"
            class:arc-game-card-browse=browse
            role=announce.then_some("status")
            aria-label=announce.then_some("Loading game")
            aria-hidden=(!announce).then_some("true")
        >
            <div class="arc-game-card-art">
                <Skeleton kind=SkeletonKind::Card class="arc-game-card-skeleton-art" />
                <Skeleton kind=SkeletonKind::Text class="arc-game-card-skeleton-title" />
            </div>
            <div class="arc-game-card-skeleton-copy">
                <Skeleton kind=SkeletonKind::Text class="arc-skeleton-short" />
                <Skeleton kind=SkeletonKind::Text />
                <Skeleton kind=SkeletonKind::Text class="arc-skeleton-chip" />
            </div>
            <Skeleton kind=SkeletonKind::Panel class="arc-game-card-skeleton-action" />
        </article>
    }
}

#[component]
pub fn EmptyState(
    #[prop(into)] title: String,
    #[prop(into)] description: String,
    #[prop(optional)] icon: Option<&'static str>,
    #[prop(optional)] primary_action: Option<AnyView>,
    #[prop(optional)] secondary_action: Option<AnyView>,
    #[prop(optional)] layout: FeedbackLayout,
) -> impl IntoView {
    let has_actions = feedback_has_actions(primary_action.is_some(), secondary_action.is_some());
    view! {
        <section class=format!("arc-feedback arc-feedback-empty {}", layout.class())>
            {icon.map(|icon| view! {
                <span class="material-symbols-outlined arc-feedback-icon" aria-hidden="true">{icon}</span>
            })}
            <div class="arc-feedback-copy">
                <h2>{title}</h2>
                <p>{description}</p>
            </div>
            {has_actions.then(|| view! {
                <div class="arc-feedback-actions">
                    {primary_action}
                    {secondary_action}
                </div>
            })}
        </section>
    }
}

pub const fn feedback_has_actions(primary: bool, secondary: bool) -> bool {
    primary || secondary
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartialRelayKind {
    Loading,
    Failed,
    ResultsAvailable,
    NoResultsYet,
}

pub fn partial_relay_copy(kind: PartialRelayKind, result_count: usize) -> (&'static str, String) {
    match kind {
        PartialRelayKind::Loading if result_count > 0 => (
            "Results still arriving",
            format!("Showing {result_count} result(s) while more relays respond."),
        ),
        PartialRelayKind::Loading => (
            "Waiting for relays",
            "No results have arrived yet. Connected relays are still loading.".to_string(),
        ),
        PartialRelayKind::Failed if result_count > 0 => (
            "Partial relay results",
            format!("Showing {result_count} result(s). The refresh failed, so results may be incomplete."),
        ),
        PartialRelayKind::Failed => (
            "Relay results unavailable",
            "No results are available yet because the refresh request failed.".to_string(),
        ),
        PartialRelayKind::ResultsAvailable => (
            "Partial results available",
            format!("Showing {result_count} result(s); this may not be the complete relay set."),
        ),
        PartialRelayKind::NoResultsYet => (
            "No results yet",
            "Relays have not returned matching results yet.".to_string(),
        ),
    }
}

#[component]
pub fn PartialRelayState(
    kind: PartialRelayKind,
    result_count: usize,
    #[prop(optional, into)] relay_status: Option<String>,
    #[prop(optional)] on_retry: Option<Callback<MouseEvent>>,
    #[prop(optional, into)] retry_busy: MaybeProp<bool>,
) -> impl IntoView {
    let (title, message) = partial_relay_copy(kind, result_count);
    let class = if kind == PartialRelayKind::Failed {
        "arc-feedback arc-relay-feedback arc-relay-feedback-warning"
    } else {
        "arc-feedback arc-relay-feedback"
    };

    view! {
        <section class=class role="status" aria-live="polite">
            <span class="material-symbols-outlined arc-feedback-icon" aria-hidden="true">
                {if kind == PartialRelayKind::Failed { "cloud_off" } else { "sync" }}
            </span>
            <div class="arc-feedback-copy">
                <h2>{title}</h2>
                <p>{message}</p>
                {relay_status.map(|status| view! { <small>{status}</small> })}
            </div>
            {on_retry.map(|retry| view! {
                <button
                    type="button"
                    class="v2-btn-secondary"
                    disabled=move || retry_busy.get().unwrap_or(false)
                    aria-busy=move || retry_busy.get().unwrap_or(false).then_some("true")
                    on:click=move |event| {
                        if !retry_busy.get_untracked().unwrap_or(false) {
                            retry.run(event);
                        }
                    }
                >
                    "Retry"
                </button>
            })}
        </section>
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ErrorSeverity {
    Inline,
    #[default]
    Recoverable,
    Blocking,
}

impl ErrorSeverity {
    const fn class(self) -> &'static str {
        match self {
            Self::Inline => "arc-error-inline",
            Self::Recoverable => "arc-error-panel",
            Self::Blocking => "arc-error-full",
        }
    }
}

pub const fn error_is_retryable(on_retry_present: bool) -> bool {
    on_retry_present
}

pub const fn retry_can_activate(retry_present: bool, busy: bool) -> bool {
    retry_present && !busy
}

#[component]
pub fn ErrorState(
    #[prop(into)] title: String,
    #[prop(into)] message: String,
    #[prop(optional, into)] technical_detail: Option<String>,
    #[prop(optional)] on_retry: Option<Callback<MouseEvent>>,
    #[prop(optional, into)] retry_busy: MaybeProp<bool>,
    #[prop(optional)] severity: ErrorSeverity,
) -> impl IntoView {
    let retryable = error_is_retryable(on_retry.is_some());
    view! {
        <section class=format!("arc-feedback arc-feedback-error {}", severity.class()) role="alert">
            <span class="material-symbols-outlined arc-feedback-icon" aria-hidden="true">"error"</span>
            <div class="arc-feedback-copy">
                <h2>{title}</h2>
                <p>{message}</p>
                {technical_detail.map(|detail| view! {
                    <details class="arc-error-detail">
                        <summary>"Technical details"</summary>
                        <p>{detail}</p>
                    </details>
                })}
            </div>
            {retryable.then(|| on_retry.map(|retry| view! {
                <button
                    type="button"
                    class="v2-btn-secondary"
                    disabled=move || retry_busy.get().unwrap_or(false)
                    aria-busy=move || retry_busy.get().unwrap_or(false).then_some("true")
                    on:click=move |event| {
                        if retry_can_activate(true, retry_busy.get_untracked().unwrap_or(false)) {
                            retry.run(event);
                        }
                    }
                >
                    "Try again"
                </button>
            }))}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_kinds_map_to_stable_geometry_classes() {
        assert_eq!(SkeletonKind::Text.class(), "arc-skeleton-text");
        assert_eq!(SkeletonKind::Panel.class(), "arc-skeleton-panel");
        assert_eq!(SkeletonKind::Card.class(), "arc-skeleton-card");
    }

    #[test]
    fn partial_relay_copy_preserves_loaded_results() {
        let (title, message) = partial_relay_copy(PartialRelayKind::Failed, 4);
        assert_eq!(title, "Partial relay results");
        assert!(message.contains("Showing 4 result(s)"));

        let (title, message) = partial_relay_copy(PartialRelayKind::Loading, 0);
        assert_eq!(title, "Waiting for relays");
        assert!(message.contains("still loading"));
    }

    #[test]
    fn retryability_depends_only_on_a_supplied_real_action() {
        assert!(error_is_retryable(true));
        assert!(!error_is_retryable(false));
        assert!(retry_can_activate(true, false));
        assert!(!retry_can_activate(true, true));
        assert!(!retry_can_activate(false, false));
    }

    #[test]
    fn empty_state_actions_render_only_when_supplied() {
        assert!(!feedback_has_actions(false, false));
        assert!(feedback_has_actions(true, false));
        assert!(feedback_has_actions(false, true));
    }

    #[test]
    fn loading_components_keep_accessible_busy_semantics() {
        let source = include_str!("feedback.rs");
        assert!(source.contains("aria-busy=\"true\""));
        assert!(source.contains("role=\"status\""));
        assert!(source.contains("aria-label=announce.then_some(\"Loading game\")"));
        assert!(source.contains("class:arc-game-card-browse=browse"));
    }
}
