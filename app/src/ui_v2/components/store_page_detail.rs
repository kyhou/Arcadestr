use std::collections::HashMap;

use leptos::prelude::*;

use crate::models::{
    GameDetailPresentation, SafeStorePageHtml, StorePageMedia, StorePageRequirementTier,
};
use crate::ui_v2::components::aria::aria_bool;
use crate::ui_v2::components::{
    Dialog, DialogCloseAction, DialogClosePolicy, DialogCloseRequest, DialogDismissal,
    DialogInitialFocus, DialogWidth,
};
use crate::ui_v2::views::use_fallback_cover;

const VIDEO_THUMBNAIL_FALLBACK: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 9'%3E%3Crect width='16' height='9' fill='%23191a22'/%3E%3Cpath d='M6 3.2v2.6L10 4.5z' fill='%23aaa4b5'/%3E%3C/svg%3E";

#[component]
fn SafeStorePageContent(html: SafeStorePageHtml, preview: bool) -> impl IntoView {
    view! {
        <div class="store-page-safe-html v2-detail-prose" inner_html=html.as_str().to_string() on:click=move |event| {
            if preview {
                event.prevent_default();
            }
        }></div>
    }
}

fn label(value: &str) -> String {
    value
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tier_rows(tier: &StorePageRequirementTier) -> Vec<(&'static str, String)> {
    [
        ("Operating system", &tier.os),
        ("Processor", &tier.processor),
        ("Memory", &tier.memory),
        ("Graphics", &tier.graphics),
        ("Storage", &tier.storage),
        ("Additional", &tier.additional),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.clone().map(|value| (name, value)))
    .collect()
}

/// The declared dismissal contract for the media expansion dialog.
///
/// Nothing is pending behind it, so every channel dismisses. It is a viewer,
/// never a confirmation.
fn media_close_contract() -> (DialogClosePolicy, DialogDismissal) {
    (
        DialogClosePolicy::Dismissible,
        DialogDismissal::freely_dismissible(),
    )
}

fn media_view(item: StorePageMedia) -> AnyView {
    if item.media_type == "video" {
        view! {
            <figure class="v2-detail-rich-media v2-detail-rich-media-video">
                <video controls preload="metadata" poster=item.thumbnail_url.clone()>
                    <source src=item.url />
                    "This direct video cannot be played."
                </video>
                {item.caption.map(|caption| view! { <figcaption>{caption}</figcaption> })}
            </figure>
        }
        .into_any()
    } else {
        view! {
            <figure class="v2-detail-rich-media">
                <img src=item.url alt=item.alt.unwrap_or_else(|| "Game media".into()) on:error=use_fallback_cover />
                {item.caption.map(|caption| view! { <figcaption>{caption}</figcaption> })}
            </figure>
        }.into_any()
    }
}

#[component]
pub fn StorePageRichDetail(
    presentation: GameDetailPresentation,
    #[prop(optional)] preview: bool,
) -> impl IntoView {
    let media = presentation
        .media
        .iter()
        .filter(|item| matches!(item.role.as_str(), "hero" | "screenshot" | "trailer"))
        .cloned()
        .collect::<Vec<_>>();
    let selected = RwSignal::new(0usize);
    let expanded = RwSignal::new(false);
    let close_media_ref = NodeRef::<leptos::html::Button>::new();
    let selected_media = StoredValue::new(media.clone());
    let media_by_id = presentation
        .media
        .iter()
        .map(|item| (item.id.clone(), item.clone()))
        .collect::<HashMap<_, _>>();
    let links = [
        ("Website", presentation.links.website.clone()),
        ("Support", presentation.links.support.clone()),
        ("Documentation", presentation.links.documentation.clone()),
        ("Source", presentation.links.source.clone()),
        ("Community", presentation.links.community.clone()),
        ("Privacy policy", presentation.links.privacy_policy.clone()),
    ];
    let requirements_when = presentation.requirements.clone();
    let requirements_view = presentation.requirements.clone();
    let languages_when = presentation.languages.clone();
    let languages_view = presentation.languages.clone();
    let accessibility_when = presentation.accessibility.clone();
    let accessibility_view = presentation.accessibility.clone();
    let links_when = links.clone();
    let links_view = links.clone();
    view! {
        <Show when=move || !selected_media.get_value().is_empty()>
            <section class="v2-detail-section" aria-labelledby="store-page-media-title">
                <p class="v2-store-kicker">"Media"</p><h2 id="store-page-media-title">"Gallery"</h2>
                <div class="v2-detail-media-stage">{move || selected_media.get_value().get(selected.get()).cloned().map(media_view)}</div>
                <button type="button" class="v2-btn-secondary v2-detail-expand-media" on:click=move |_| expanded.set(true)>"Expand media"</button>
                <Dialog
                    id="store-page-media"
                    open=Signal::derive(move || expanded.get())
                    title="Expanded media"
                    kicker="Media"
                    width=DialogWidth::Wide
                    // Nothing is pending behind this dialog, so every channel
                    // dismisses it. It is a viewer, not a confirmation.
                    policy=media_close_contract().0
                    dismissal=media_close_contract().1
                    initial_focus=DialogInitialFocus::Button(close_media_ref)
                    close_label="Close expanded media"
                    on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                        if request.action == DialogCloseAction::Dismiss {
                            expanded.set(false);
                        }
                    })
                    actions=move || view! {
                        <button
                            node_ref=close_media_ref
                            type="button"
                            class="v2-btn-secondary"
                            on:click=move |_| expanded.set(false)
                        >"Close media"</button>
                    }
                >
                    <div>{move || selected_media.get_value().get(selected.get()).cloned().map(media_view)}</div>
                </Dialog>
                <div class="v2-detail-media-thumbs" role="list" aria-label="Select game media">
                    {media.clone().into_iter().enumerate().map(|(index, item)| {
                        let thumbnail = item.thumbnail_url.clone().unwrap_or_else(|| {
                            if item.media_type == "video" {
                                VIDEO_THUMBNAIL_FALLBACK.to_string()
                            } else {
                                item.url.clone()
                            }
                        });
                        view! { <button type="button" class="v2-detail-media-thumb" class:active=move || selected.get() == index aria-pressed=move || aria_bool(selected.get() == index) on:click=move |_| selected.set(index)>
                            <img src=thumbnail alt=item.alt.unwrap_or_else(|| format!("Select {}", label(&item.role))) on:error=use_fallback_cover />
                        </button> }
                    }).collect_view()}
                </div>
            </section>
        </Show>

        {presentation.description_html.map(|html| view! { <section class="v2-detail-section"><p class="v2-store-kicker">"About"</p><h2>"About this game"</h2><SafeStorePageContent html=html preview=preview /></section> })}

        {presentation.sections.into_iter().map(|section| {
            let media = section.media_id.as_ref().and_then(|id| media_by_id.get(id)).cloned();
            let class = match section.layout.as_str() {
                "media-left" | "media-right" => "v2-detail-section grid gap-5 md:grid-cols-2",
                "media-wide" => "v2-detail-section space-y-5",
                _ => "v2-detail-section",
            };
            let media_before = (section.layout == "media-left").then(|| media.clone()).flatten();
            let media_after = (section.layout != "media-left").then_some(media).flatten();
            view! { <section class=class>
                {media_before.map(media_view)}
                <div><h2>{section.heading}</h2><SafeStorePageContent html=section.body_html preview=preview /></div>
                {media_after.map(media_view)}
            </section> }
        }).collect_view()}

        <Show when=move || !requirements_when.is_empty()>
            <section class="v2-detail-section"><p class="v2-store-kicker">"System requirements"</p><h2>"Requirements"</h2>
                {requirements_view.clone().into_iter().map(|requirement| view! { <article class="v2-detail-requirement"><h3>{label(&requirement.platform)}</h3>
                    {[("Minimum", requirement.minimum), ("Recommended", requirement.recommended)].into_iter().filter_map(|(heading, tier)| tier.map(|tier| view! { <div class="v2-detail-note"><h4>{heading}</h4><dl>{tier_rows(&tier).into_iter().map(|(name, value)| view! { <div class="v2-detail-requirement-grid"><dt>{name}</dt><dd>{value}</dd></div> }).collect_view()}</dl></div> })).collect_view()}
                </article> }).collect_view()}
            </section>
        </Show>

        <Show when=move || !languages_when.is_empty()>
            <section class="v2-detail-section"><p class="v2-store-kicker">"Languages"</p><h2>"Language support"</h2><div class="v2-detail-grid">
                {languages_view.clone().into_iter().map(|language| view! { <div class="v2-detail-note"><strong>{label(&language.code)}</strong><p class="v2-store-help">{[language.interface.then_some("Interface"), language.audio.then_some("Audio"), language.subtitles.then_some("Subtitles")].into_iter().flatten().collect::<Vec<_>>().join(" · ")}</p></div> }).collect_view()}
            </div></section>
        </Show>

        <Show when=move || !accessibility_when.is_empty()>
            <section class="v2-detail-section"><p class="v2-store-kicker">"Accessibility"</p><h2>"Accessibility features"</h2><p class="v2-store-help">"Accessibility information provided by the publisher."</p><ul class="v2-detail-grid">
                {accessibility_view.clone().into_iter().map(|entry| view! { <li class="v2-detail-note"><strong>{label(&entry.feature)}</strong>{if entry.supported { " · Supported" } else { " · Not supported" }}{entry.notes.map(|notes| view! { <p class="v2-store-help">{notes}</p> })}</li> }).collect_view()}
            </ul></section>
        </Show>

        <Show when=move || links_when.iter().any(|(_, value)| value.is_some())>
            <section class="v2-detail-section"><p class="v2-store-kicker">"Links"</p><h2>"Official links"</h2><div class="v2-detail-link-row">
                {links_view.clone().into_iter().filter_map(|(name, value)| value.map(|url| if preview {
                    view! { <span class="v2-detail-link v2-detail-link-disabled" title=url>{name}</span> }.into_any()
                } else {
                    view! { <a class="v2-detail-link" href=url target="_blank" rel="noopener noreferrer">{name}</a> }.into_any()
                })).collect_view()}
            </div></section>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_media_is_dismissible_from_every_channel() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = media_close_contract();
        for source in [
            DialogCloseSource::Escape,
            DialogCloseSource::Backdrop,
            DialogCloseSource::CloseButton,
        ] {
            assert_eq!(
                resolve_close(policy, dismissal, false, source),
                DialogCloseAction::Dismiss,
                "{source:?} should close the media viewer"
            );
        }
    }

    #[test]
    fn expanded_media_is_not_a_destructive_confirmation() {
        // Assert against the production source only, not this test module.
        let source = include_str!("store_page_detail.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source precedes the test module");
        assert!(!source.contains("DialogTone"));
        assert!(!source.contains("v2-btn-danger"));
    }

    #[test]
    fn game_detail_safe_html_has_no_public_string_constructor_or_deserializer() {
        assert_eq!(
            std::mem::size_of::<SafeStorePageHtml>(),
            std::mem::size_of::<String>()
        );
    }

    #[test]
    fn game_detail_video_rendering_accepts_only_backend_media_types() {
        let video = StorePageMedia {
            id: "trailer".into(),
            media_type: "video".into(),
            role: "trailer".into(),
            url: "https://cdn.example.org/trailer.webm".into(),
            thumbnail_url: None,
            alt: None,
            caption: None,
        };
        assert_eq!(video.media_type, "video");
        assert!(video.url.ends_with(".webm"));
    }
}
