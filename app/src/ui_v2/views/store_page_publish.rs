use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use arcadestr_core::store_page::{
    AccessibilityFeature, LanguageSupport, PlatformRequirement, RequirementTier, StorePageDraft,
    StorePageMediaItem, StorePageSection,
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::campaign_management::{accepts_account_response, listing_coordinate};
use crate::models::{GameDetailPresentation, GameListing, StorePageListingRef};
use crate::tauri_bridge::{
    invoke_clone_store_page, invoke_load_publisher_store_page_editor, invoke_publish_store_page,
    invoke_retry_store_page_pointer_sync, invoke_validate_store_page_draft, ListingPointerMutation,
    PublishStorePageResponse, PublisherStorePageListingRevision, StorePageListingMutation,
};
use crate::ui_v2::components::StorePageRichDetail;

#[derive(Clone)]
struct CachedDraft {
    draft: StorePageDraft,
    baseline: StorePageDraft,
    raw: RawDraftFields,
    input_dirty: bool,
}

#[derive(Clone)]
struct RawDraftFields {
    associations: String,
    media: String,
    sections: String,
    languages: String,
    requirements: String,
    accessibility: String,
}

#[derive(Clone)]
struct PublisherRecovery {
    response: PublishStorePageResponse,
    mutations: Vec<StorePageListingMutation>,
    draft: StorePageDraft,
}

thread_local! {
    static PUBLISHER_STORE_PAGE_DRAFTS: RefCell<HashMap<String, CachedDraft>> = RefCell::new(HashMap::new());
    static PUBLISHER_STORE_PAGE_RECOVERY: RefCell<HashMap<String, PublisherRecovery>> = RefCell::new(HashMap::new());
}

fn draft_key(publisher: &str, listing_coordinate: &str) -> String {
    format!("{publisher}|{listing_coordinate}")
}

fn cached_draft(key: &str) -> Option<CachedDraft> {
    PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| drafts.borrow().get(key).cloned())
}

fn retain_account_drafts(publisher: &str) {
    PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| {
        drafts
            .borrow_mut()
            .retain(|key, _| key.starts_with(&format!("{publisher}|")));
    });
}

fn save_cached_draft(
    key: &str,
    draft: StorePageDraft,
    baseline: StorePageDraft,
    raw: RawDraftFields,
    input_dirty: bool,
) {
    PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| {
        drafts.borrow_mut().insert(
            key.to_string(),
            CachedDraft {
                draft,
                baseline,
                raw,
                input_dirty,
            },
        );
    });
}

fn recovery(key: &str) -> Option<PublisherRecovery> {
    PUBLISHER_STORE_PAGE_RECOVERY.with(|recoveries| recoveries.borrow().get(key).cloned())
}

fn save_recovery(key: &str, recovery: PublisherRecovery) {
    PUBLISHER_STORE_PAGE_RECOVERY.with(|recoveries| {
        recoveries.borrow_mut().insert(key.to_string(), recovery);
    });
}

fn clear_recovery(key: &str) {
    PUBLISHER_STORE_PAGE_RECOVERY.with(|recoveries| {
        recoveries.borrow_mut().remove(key);
    });
}

fn seed_new_draft_association(
    draft: &mut StorePageDraft,
    raw: &mut RawDraftFields,
    coordinate: &str,
    event_id: Option<&str>,
) {
    let Some(event_id) = event_id else {
        return;
    };
    let association = format!("{coordinate}\t{event_id}\tlink");
    if raw.associations.trim().is_empty()
        && draft.listing_coordinates.is_empty()
        && draft.loaded_event_id.is_none()
    {
        raw.associations = association;
        draft.listing_coordinates = vec![coordinate.to_string()];
    }
}

fn csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn optional(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn sync_compact_tags(draft: &mut StorePageDraft) {
    draft.compact_tags.title = draft.content.basic.title.clone();
    draft.compact_tags.summary = draft.content.basic.summary.clone();
    draft.compact_tags.developer = draft.content.basic.developer.clone();
    draft.compact_tags.publisher = draft.content.basic.publisher.clone();
    draft.compact_tags.release_date = draft.content.basic.release_date.clone();
    draft.compact_tags.genres = draft.content.discovery.genres.clone().unwrap_or_default();
    draft.compact_tags.features = draft.content.discovery.features.clone().unwrap_or_default();
    draft.compact_tags.languages = draft.content.languages.clone().unwrap_or_default();
    draft.compact_tags.website = draft.content.links.website.clone();
    draft.compact_tags.support = draft.content.links.support.clone();
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected true or false, found `{value}`")),
    }
}

fn parse_dimension(value: &str, line: usize, name: &str) -> Result<Option<u32>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<u32>()
        .map(Some)
        .map_err(|_| format!("Media line {line} has an invalid {name}"))
}

fn escape_cell(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            character => escaped.push(character),
        }
    }
    escaped
}

fn unescape_cell(value: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => decoded.push('\\'),
            Some('t') => decoded.push('\t'),
            Some('n') => decoded.push('\n'),
            Some('r') => decoded.push('\r'),
            Some(other) => return Err(format!("unsupported escape sequence: \\{other}")),
            None => return Err("unfinished escape sequence".to_string()),
        }
    }
    Ok(decoded)
}

fn parse_media(value: &str) -> Result<Vec<StorePageMediaItem>, String> {
    value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 9 {
                return Err(format!(
                    "Media line {} needs 9 tab-separated fields",
                    index + 1
                ));
            }
            Ok(StorePageMediaItem {
                id: fields[0].to_string(),
                media_type: fields[1].to_string(),
                role: fields[2].to_string(),
                url: fields[3].to_string(),
                thumbnail_url: fields
                    .get(4)
                    .and_then(|value| optional((*value).to_string())),
                alt: fields
                    .get(5)
                    .and_then(|value| optional((*value).to_string())),
                caption: fields
                    .get(6)
                    .and_then(|value| optional((*value).to_string())),
                width: parse_dimension(fields[7], index + 1, "width")?,
                height: parse_dimension(fields[8], index + 1, "height")?,
            })
        })
        .collect()
}

fn format_media(media: &[StorePageMediaItem]) -> String {
    media
        .iter()
        .map(|item| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                item.id,
                item.media_type,
                item.role,
                item.url,
                item.thumbnail_url.clone().unwrap_or_default(),
                item.alt.clone().unwrap_or_default(),
                item.caption.clone().unwrap_or_default(),
                item.width
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                item.height
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_sections(value: &str) -> Result<Vec<StorePageSection>, String> {
    value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            let fields = line.splitn(5, '\t').collect::<Vec<_>>();
            if fields.len() != 5 {
                return Err(format!(
                    "Section line {} needs id|heading|layout|media_id|Markdown",
                    index + 1
                ));
            }
            Ok(StorePageSection {
                id: fields[0].trim().to_string(),
                heading: unescape_cell(fields[1])?,
                layout: fields[2].trim().to_string(),
                media_id: optional(fields[3].trim().to_string()),
                body_markdown: unescape_cell(fields[4])?,
            })
        })
        .collect()
}

fn format_sections(sections: &[StorePageSection]) -> String {
    sections
        .iter()
        .map(|section| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                section.id,
                escape_cell(&section.heading),
                section.layout,
                section.media_id.clone().unwrap_or_default(),
                escape_cell(&section.body_markdown)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_languages(value: &str) -> Result<Vec<LanguageSupport>, String> {
    value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(format!(
                    "Language line {} needs code|interface|audio|subtitles",
                    index + 1
                ));
            }
            Ok(LanguageSupport {
                code: fields[0].to_string(),
                interface: parse_bool(fields[1])
                    .map_err(|error| format!("Language line {}: {error}", index + 1))?,
                audio: parse_bool(fields[2])
                    .map_err(|error| format!("Language line {}: {error}", index + 1))?,
                subtitles: parse_bool(fields[3])
                    .map_err(|error| format!("Language line {}: {error}", index + 1))?,
            })
        })
        .collect()
}

fn format_languages(languages: &[LanguageSupport]) -> String {
    languages
        .iter()
        .map(|entry| {
            format!(
                "{}\t{}\t{}\t{}",
                entry.code, entry.interface, entry.audio, entry.subtitles
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_accessibility(value: &str) -> Result<Vec<AccessibilityFeature>, String> {
    value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            let fields = line.splitn(3, '\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() < 2 {
                return Err(format!(
                    "Accessibility line {} needs feature|supported|notes",
                    index + 1
                ));
            }
            Ok(AccessibilityFeature {
                feature: fields[0].to_string(),
                supported: parse_bool(fields[1])
                    .map_err(|error| format!("Accessibility line {}: {error}", index + 1))?,
                notes: fields
                    .get(2)
                    .and_then(|value| optional((*value).to_string())),
            })
        })
        .collect()
}

fn format_accessibility(entries: &[AccessibilityFeature]) -> String {
    entries
        .iter()
        .map(|entry| {
            format!(
                "{}\t{}\t{}",
                entry.feature,
                entry.supported,
                entry.notes.clone().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_requirements(value: &str) -> Result<BTreeMap<String, PlatformRequirement>, String> {
    let mut requirements = BTreeMap::new();
    for (index, line) in value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 8 || !matches!(fields[1], "minimum" | "recommended") {
            return Err(format!("Requirement line {} needs platform|minimum-or-recommended|os|processor|memory|graphics|storage|additional", index + 1));
        }
        let tier = RequirementTier {
            os: optional(fields[2].to_string()),
            processor: optional(fields[3].to_string()),
            memory: optional(fields[4].to_string()),
            graphics: optional(fields[5].to_string()),
            storage: optional(fields[6].to_string()),
            additional: optional(fields[7].to_string()),
        };
        let entry = requirements
            .entry(fields[0].to_string())
            .or_insert_with(PlatformRequirement::default);
        if fields[1] == "minimum" {
            entry.minimum = Some(tier);
        } else {
            entry.recommended = Some(tier);
        }
    }
    Ok(requirements)
}

fn format_requirements(requirements: &BTreeMap<String, PlatformRequirement>) -> String {
    let mut lines = Vec::new();
    for (platform, requirement) in requirements {
        for (name, tier) in [
            ("minimum", requirement.minimum.as_ref()),
            ("recommended", requirement.recommended.as_ref()),
        ] {
            if let Some(tier) = tier {
                lines.push(format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    platform,
                    name,
                    tier.os.clone().unwrap_or_default(),
                    tier.processor.clone().unwrap_or_default(),
                    tier.memory.clone().unwrap_or_default(),
                    tier.graphics.clone().unwrap_or_default(),
                    tier.storage.clone().unwrap_or_default(),
                    tier.additional.clone().unwrap_or_default()
                ));
            }
        }
    }
    lines.join("\n")
}

fn parse_associations(value: &str) -> Result<Vec<StorePageListingMutation>, String> {
    value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() < 3 {
                return Err(format!(
                    "Association line {} needs coordinate, event ID, and link, unlink, or review",
                    index + 1
                ));
            }
            let action = match fields[2] {
                "link" => ListingPointerMutation::Link,
                "unlink" => ListingPointerMutation::Unlink,
                "review" => ListingPointerMutation::Review,
                _ => {
                    return Err(format!(
                        "Association line {} has an invalid action",
                        index + 1
                    ))
                }
            };
            Ok(StorePageListingMutation {
                listing_coordinate: fields[0].to_string(),
                expected_event_id: fields[1].to_string(),
                action,
                relay_hint: fields
                    .get(3)
                    .and_then(|value| optional((*value).to_string())),
                published_event_id: None,
            })
        })
        .collect()
}

fn format_associations(listings: &[PublisherStorePageListingRevision]) -> String {
    listings
        .iter()
        .map(|listing| {
            format!(
                "{}\t{}\t{}",
                listing.listing_coordinate,
                listing.event_id,
                if listing.reciprocal { "link" } else { "review" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn retryable_mutations(
    result: &PublishStorePageResponse,
    mutations: &[StorePageListingMutation],
) -> Vec<StorePageListingMutation> {
    result
        .listing_updates
        .iter()
        .filter(|outcome| !outcome.published || !outcome.propagation_confirmed)
        .filter_map(|outcome| {
            mutations
                .iter()
                .find(|mutation| mutation.listing_coordinate == outcome.listing_coordinate)
                .cloned()
                .map(|mut mutation| {
                    if outcome.published {
                        mutation.published_event_id = outcome.replacement_event_id.clone();
                    }
                    mutation
                })
        })
        .collect()
}

fn preview_commerce_label(
    price: f64,
    currency: &str,
    acquisition: &crate::models::AcquisitionPolicy,
) -> String {
    let access = match acquisition {
        crate::models::AcquisitionPolicy::Public => "Public",
        crate::models::AcquisitionPolicy::Gated => "Gated",
        crate::models::AcquisitionPolicy::TimedAccess { .. } => "Timed access",
    };
    format!("{price} {currency} · {access}")
}

#[component]
pub fn StorePageEditorView(
    listing: GameListing,
    on_back: Callback<()>,
    on_saved: Callback<GameListing>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let publisher = listing.publisher_npub.clone();
    let coordinate = listing_coordinate(&listing);
    let key = draft_key(&publisher, &coordinate);
    let cached = cached_draft(&key);
    let recovered = recovery(&key);
    let fallback = StorePageDraft::new(listing.id.clone(), vec![coordinate.clone()]);
    let mut initial_draft = cached
        .as_ref()
        .map_or_else(|| fallback.clone(), |entry| entry.draft.clone());
    let baseline = RwSignal::new(
        cached
            .as_ref()
            .map_or_else(|| fallback, |entry| entry.baseline.clone()),
    );
    let listings = RwSignal::new(Vec::<PublisherStorePageListingRevision>::new());
    let selected_association = listing
        .event_id
        .as_ref()
        .map(|event_id| format!("{coordinate}\t{event_id}\tlink"))
        .unwrap_or_default();
    let default_raw = RawDraftFields {
        associations: selected_association.clone(),
        media: format_media(&initial_draft.content.media),
        sections: format_sections(&initial_draft.content.sections),
        languages: format_languages(
            initial_draft
                .content
                .languages
                .as_deref()
                .unwrap_or_default(),
        ),
        requirements: format_requirements(&initial_draft.content.requirements),
        accessibility: format_accessibility(&initial_draft.content.accessibility),
    };
    let mut raw = cached
        .as_ref()
        .map_or(default_raw, |entry| entry.raw.clone());
    seed_new_draft_association(
        &mut initial_draft,
        &mut raw,
        &coordinate,
        listing.event_id.as_deref(),
    );
    let draft = RwSignal::new(initial_draft);
    let associations_text = RwSignal::new(raw.associations);
    let media_text = RwSignal::new(raw.media);
    let sections_text = RwSignal::new(raw.sections);
    let languages_text = RwSignal::new(raw.languages);
    let requirements_text = RwSignal::new(raw.requirements);
    let accessibility_text = RwSignal::new(raw.accessibility);
    let input_dirty = RwSignal::new(cached.as_ref().is_some_and(|entry| entry.input_dirty));
    let association_review_required = RwSignal::new(false);
    let preview = RwSignal::new(None::<GameDetailPresentation>);
    let diagnostics = RwSignal::new(Vec::<String>::new());
    let loading = RwSignal::new(true);
    let validating = RwSignal::new(false);
    let publishing = RwSignal::new(false);
    let parser_error = RwSignal::new(None::<String>);
    let message = RwSignal::new(None::<String>);
    let partial = RwSignal::new(recovered.as_ref().map(|state| state.response.clone()));
    let transaction_mutations = RwSignal::new(
        recovered
            .as_ref()
            .map_or_else(Vec::new, |state| state.mutations.clone()),
    );
    let transaction_draft = RwSignal::new(recovered.as_ref().map(|state| state.draft.clone()));
    let show_discard = RwSignal::new(false);
    let clone_id = RwSignal::new(String::new());
    let link_existing_id = RwSignal::new(String::new());
    let operation_generation = RwSignal::new(0_u64);
    let operation_account = RwSignal::new(auth.npub.get_untracked());

    Effect::new({
        let key = key.clone();
        move |_| {
            save_cached_draft(
                &key,
                draft.get(),
                baseline.get_untracked(),
                RawDraftFields {
                    associations: associations_text.get(),
                    media: media_text.get(),
                    sections: sections_text.get(),
                    languages: languages_text.get(),
                    requirements: requirements_text.get(),
                    accessibility: accessibility_text.get(),
                },
                input_dirty.get(),
            )
        }
    });

    Effect::new(move |_| {
        if let Ok(value) = parse_media(&media_text.get()) {
            draft.update(|draft| draft.content.media = value);
        }
    });
    Effect::new(move |_| {
        if let Ok(value) = parse_sections(&sections_text.get()) {
            draft.update(|draft| draft.content.sections = value);
        }
    });
    Effect::new(move |_| {
        if let Ok(value) = parse_languages(&languages_text.get()) {
            draft.update(|draft| draft.content.languages = (!value.is_empty()).then_some(value));
        }
    });
    Effect::new(move |_| {
        if let Ok(value) = parse_requirements(&requirements_text.get()) {
            draft.update(|draft| draft.content.requirements = value);
        }
    });
    Effect::new(move |_| {
        if let Ok(value) = parse_accessibility(&accessibility_text.get()) {
            draft.update(|draft| draft.content.accessibility = value);
        }
    });
    Effect::new(move |_| {
        if let Ok(value) = parse_associations(&associations_text.get()) {
            draft.update(|draft| {
                draft.listing_coordinates = value
                    .iter()
                    .filter(|mutation| {
                        matches!(
                            mutation.action,
                            ListingPointerMutation::Link | ListingPointerMutation::Review
                        )
                    })
                    .map(|mutation| mutation.listing_coordinate.clone())
                    .collect();
            });
        }
    });

    Effect::new({
        let publisher = publisher.clone();
        move |_| {
            let current = auth.npub.get();
            if current.as_deref() != Some(publisher.as_str()) {
                operation_generation.update(|value| *value = value.wrapping_add(1));
                operation_account.set(current.clone());
                preview.set(None);
                loading.set(false);
                validating.set(false);
                publishing.set(false);
                message.set(Some(
                    "Account changed. Switch back to the publisher account and reload before publishing."
                        .to_string(),
                ));
                if let Some(current) = current {
                    retain_account_drafts(&current);
                } else {
                    PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| drafts.borrow_mut().clear());
                }
            }
        }
    });

    Effect::new({
        let publisher = publisher.clone();
        let coordinate = coordinate.clone();
        let event_id = listing.event_id.clone();
        move |_| {
            let Some(event_id) = event_id.clone() else {
                loading.set(false);
                message.set(Some(
                    "The current signed listing event ID is unavailable.".to_string(),
                ));
                return;
            };
            let Some(initiating_account) = auth.npub.get_untracked() else {
                loading.set(false);
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            spawn_local({
                let publisher = publisher.clone();
                let coordinate = coordinate.clone();
                async move {
                    let result = invoke_load_publisher_store_page_editor(
                        publisher,
                        StorePageListingRef {
                            listing_coordinate: coordinate,
                            listing_event_id: event_id,
                        },
                        None,
                    )
                    .await;
                    if !accepts_account_response(
                        auth.npub.get_untracked().as_deref(),
                        &initiating_account,
                        operation_generation.get_untracked(),
                        generation,
                    ) {
                        return;
                    }
                    match result {
                        Ok(state) => {
                            let requires_association_review = !state.diagnostics.is_empty();
                            listings.set(state.listings.clone());
                            diagnostics.set(state.diagnostics);
                            association_review_required.set(requires_association_review);
                            if draft.get_untracked() == baseline.get_untracked()
                                && !input_dirty.get_untracked()
                            {
                                associations_text.set(format_associations(&state.listings));
                                draft.set(state.draft.clone());
                                baseline.set(state.baseline_draft);
                                media_text.set(format_media(&state.draft.content.media));
                                sections_text.set(format_sections(&state.draft.content.sections));
                                languages_text.set(format_languages(
                                    state.draft.content.languages.as_deref().unwrap_or_default(),
                                ));
                                requirements_text
                                    .set(format_requirements(&state.draft.content.requirements));
                                accessibility_text
                                    .set(format_accessibility(&state.draft.content.accessibility));
                            }
                        }
                        Err(error) => message.set(Some(error)),
                    }
                    loading.set(false);
                }
            });
        }
    });

    let update_complex = move || -> Result<(), String> {
        let media = parse_media(&media_text.get_untracked())?;
        let sections = parse_sections(&sections_text.get_untracked())?;
        let languages = parse_languages(&languages_text.get_untracked())?;
        let requirements = parse_requirements(&requirements_text.get_untracked())?;
        let accessibility = parse_accessibility(&accessibility_text.get_untracked())?;
        let mutations = parse_associations(&associations_text.get_untracked())?;
        draft.update(|draft| {
            draft.content.media = media;
            draft.content.sections = sections;
            draft.content.languages = (!languages.is_empty()).then_some(languages);
            draft.content.requirements = requirements;
            draft.content.accessibility = accessibility;
            draft.listing_coordinates = mutations
                .iter()
                .filter(|mutation| {
                    matches!(
                        mutation.action,
                        ListingPointerMutation::Link | ListingPointerMutation::Review
                    )
                })
                .map(|mutation| mutation.listing_coordinate.clone())
                .collect();
            sync_compact_tags(draft);
        });
        parser_error.set(None);
        Ok(())
    };

    let run_validation = Callback::new({
        let publisher = publisher.clone();
        let selected_coordinate = coordinate.clone();
        move |_: ()| {
            if let Err(error) = update_complex() {
                parser_error.set(Some(error));
                return;
            }
            let Ok(mutations) = parse_associations(&associations_text.get_untracked()) else {
                return;
            };
            let Some(preview_listing) = mutations
                .iter()
                .find(|mutation| {
                    mutation.action == ListingPointerMutation::Link
                        && mutation.listing_coordinate == selected_coordinate
                })
                .map(|mutation| PublisherStorePageListingRevision {
                    listing_coordinate: mutation.listing_coordinate.clone(),
                    event_id: mutation.expected_event_id.clone(),
                    reciprocal: true,
                })
            else {
                message.set(Some(
                    "The selected listing must remain linked to preview its authoritative commerce."
                        .to_string(),
                ));
                return;
            };
            validating.set(true);
            diagnostics.set(Vec::new());
            let request_draft = draft.get_untracked();
            let publisher = publisher.clone();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                validating.set(false);
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            spawn_local(async move {
                let result = invoke_validate_store_page_draft(
                    publisher,
                    request_draft,
                    preview_listing,
                    mutations,
                )
                .await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    validating.set(false);
                    return;
                }
                match result {
                    Ok(result) => {
                        diagnostics.set(
                            result
                                .diagnostics
                                .into_iter()
                                .map(|diagnostic| diagnostic.message)
                                .collect(),
                        );
                        preview.set(result.preview);
                    }
                    Err(error) => message.set(Some(error)),
                }
                validating.set(false);
            });
        }
    });

    let publish = Callback::new({
        let publisher = publisher.clone();
        let listing_for_saved = listing.clone();
        let selected_coordinate = coordinate.clone();
        let recovery_key = key.clone();
        move |_: ()| {
            if let Err(error) = update_complex() {
                parser_error.set(Some(error));
                return;
            }
            if auth.npub.get_untracked().as_deref() != Some(publisher.as_str()) {
                message.set(Some(
                    "Switch back to the publisher account before publishing.".to_string(),
                ));
                return;
            }
            if association_review_required.get_untracked() {
                message.set(Some(
                    "Review and edit the association rows before publishing this incomplete relationship."
                        .to_string(),
                ));
                return;
            }
            let Ok(mutations) = parse_associations(&associations_text.get_untracked()) else {
                return;
            };
            let request_draft = draft.get_untracked();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            publishing.set(true);
            partial.set(None);
            transaction_mutations.set(mutations.clone());
            transaction_draft.set(Some(request_draft.clone()));
            let publisher = publisher.clone();
            let listing_for_saved = listing_for_saved.clone();
            let selected_coordinate = selected_coordinate.clone();
            let recovery_key = recovery_key.clone();
            spawn_local(async move {
                let result =
                    invoke_publish_store_page(publisher, request_draft.clone(), mutations).await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    if let Ok(mut response) = result {
                        response.complete = false;
                        response.retryable = true;
                        save_recovery(
                            &recovery_key,
                            PublisherRecovery {
                                response: response.clone(),
                                mutations: transaction_mutations.get_untracked(),
                                draft: request_draft,
                            },
                        );
                        partial.set(Some(response));
                    }
                    publishing.set(false);
                    return;
                }
                match result {
                    Ok(result) => {
                        let mut updated_listing = listing_for_saved;
                        if let Some(event_id) = result
                            .listing_updates
                            .iter()
                            .find(|outcome| {
                                outcome.published
                                    && outcome.listing_coordinate == selected_coordinate
                            })
                            .and_then(|outcome| outcome.replacement_event_id.clone())
                        {
                            updated_listing.event_id = Some(event_id);
                        }
                        if result.complete {
                            clear_recovery(&recovery_key);
                            baseline.set(request_draft.clone());
                            draft.set(request_draft);
                            input_dirty.set(false);
                            message.set(Some(
                                "Store Page and listing pointers published.".to_string(),
                            ));
                            on_saved.run(updated_listing);
                        } else {
                            message.set(Some("Store Page publication is incomplete. Review and retry failed pointer updates.".to_string()));
                            save_recovery(
                                &recovery_key,
                                PublisherRecovery {
                                    response: result.clone(),
                                    mutations: transaction_mutations.get_untracked(),
                                    draft: request_draft,
                                },
                            );
                            partial.set(Some(result));
                        }
                    }
                    Err(error) => message.set(Some(error)),
                }
                publishing.set(false);
            });
        }
    });

    let retry = Callback::new({
        let publisher = publisher.clone();
        let listing_for_saved = listing.clone();
        let selected_coordinate = coordinate.clone();
        let recovery_key = key.clone();
        move |_: ()| {
            let Some(result) = partial.get_untracked() else {
                return;
            };
            let Some(page) = result.store_page.as_ref() else {
                return;
            };
            let all_mutations = transaction_mutations.get_untracked();
            if all_mutations.is_empty() {
                message.set(Some(
                    "The original publication transaction is unavailable.".to_string(),
                ));
                return;
            }
            let retry_mutations = retryable_mutations(&result, &all_mutations);
            let selected_existing_event_id = result
                .listing_updates
                .iter()
                .find(|outcome| {
                    outcome.published && outcome.listing_coordinate == selected_coordinate
                })
                .and_then(|outcome| outcome.replacement_event_id.clone());
            let store_page_coordinate = result.store_page_coordinate.clone();
            let store_page_event_id = page.event_id.clone();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            publishing.set(true);
            let publisher = publisher.clone();
            let listing_for_saved = listing_for_saved.clone();
            let selected_coordinate = selected_coordinate.clone();
            let recovery_key = recovery_key.clone();
            spawn_local(async move {
                let result = invoke_retry_store_page_pointer_sync(
                    publisher,
                    store_page_coordinate,
                    store_page_event_id,
                    retry_mutations,
                )
                .await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    if let Ok(response) = result {
                        save_recovery(
                            &recovery_key,
                            PublisherRecovery {
                                response: response.clone(),
                                mutations: transaction_mutations.get_untracked(),
                                draft: transaction_draft
                                    .get_untracked()
                                    .unwrap_or_else(|| draft.get_untracked()),
                            },
                        );
                        partial.set(Some(response));
                    }
                    publishing.set(false);
                    return;
                }
                match result {
                    Ok(retried) if retried.retry_scope_complete => {
                        clear_recovery(&recovery_key);
                        let mut updated_listing = listing_for_saved;
                        if let Some(event_id) = retried
                            .listing_updates
                            .iter()
                            .find(|outcome| {
                                outcome.published
                                    && outcome.listing_coordinate == selected_coordinate
                            })
                            .and_then(|outcome| outcome.replacement_event_id.clone())
                            .or(selected_existing_event_id)
                        {
                            updated_listing.event_id = Some(event_id);
                        }
                        if let Some(published_draft) = transaction_draft.get_untracked() {
                            baseline.set(published_draft.clone());
                            draft.set(published_draft);
                            input_dirty.set(false);
                        }
                        partial.set(None);
                        message.set(Some(
                            "Listing pointer synchronization completed.".to_string(),
                        ));
                        on_saved.run(updated_listing);
                    }
                    Ok(retried) => {
                        save_recovery(
                            &recovery_key,
                            PublisherRecovery {
                                response: retried.clone(),
                                mutations: transaction_mutations.get_untracked(),
                                draft: transaction_draft
                                    .get_untracked()
                                    .unwrap_or_else(|| draft.get_untracked()),
                            },
                        );
                        partial.set(Some(retried));
                    }
                    Err(error) => message.set(Some(error)),
                }
                publishing.set(false);
            });
        }
    });

    let clone_page = Callback::new({
        move |_: ()| {
            let presentation_id = clone_id.get_untracked();
            if presentation_id.trim().is_empty() {
                message.set(Some(
                    "Enter a new presentation ID before cloning.".to_string(),
                ));
                return;
            }
            let source = draft.get_untracked();
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            spawn_local(async move {
                let result = invoke_clone_store_page(source, presentation_id).await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    return;
                }
                match result {
                    Ok(cloned) => {
                        associations_text.set(String::new());
                        draft.set(cloned);
                        preview.set(None);
                        message.set(Some("Clone created locally. Add explicit listing associations before publishing.".to_string()));
                    }
                    Err(error) => message.set(Some(error)),
                }
            });
        }
    });

    let link_existing = Callback::new({
        let publisher = publisher.clone();
        let coordinate = coordinate.clone();
        let event_id = listing.event_id.clone();
        move |_: ()| {
            if draft.get_untracked() != baseline.get_untracked() || input_dirty.get_untracked() {
                message.set(Some(
                    "Discard or publish the current draft before loading another Store Page."
                        .to_string(),
                ));
                return;
            }
            let presentation_id = link_existing_id.get_untracked();
            let Some(event_id) = event_id.clone() else {
                message.set(Some(
                    "The selected listing event ID is unavailable.".to_string(),
                ));
                return;
            };
            if presentation_id.trim().is_empty() {
                message.set(Some("Enter an existing presentation ID.".to_string()));
                return;
            }
            let Some(initiating_account) = auth.npub.get_untracked() else {
                return;
            };
            if initiating_account != publisher {
                message.set(Some(
                    "Switch back to the publisher account before linking.".to_string(),
                ));
                return;
            }
            operation_generation.update(|value| *value = value.wrapping_add(1));
            let generation = operation_generation.get_untracked();
            loading.set(true);
            let publisher = publisher.clone();
            let coordinate = coordinate.clone();
            spawn_local(async move {
                let result = invoke_load_publisher_store_page_editor(
                    publisher,
                    StorePageListingRef {
                        listing_coordinate: coordinate,
                        listing_event_id: event_id,
                    },
                    Some(presentation_id),
                )
                .await;
                if !accepts_account_response(
                    auth.npub.get_untracked().as_deref(),
                    &initiating_account,
                    operation_generation.get_untracked(),
                    generation,
                ) {
                    return;
                }
                match result {
                    Ok(state) => {
                        let requires_association_review = !state.diagnostics.is_empty();
                        associations_text.set(format_associations(&state.listings));
                        listings.set(state.listings);
                        diagnostics.set(state.diagnostics);
                        association_review_required.set(requires_association_review);
                        draft.set(state.draft.clone());
                        baseline.set(state.baseline_draft);
                        input_dirty.set(false);
                        media_text.set(format_media(&state.draft.content.media));
                        sections_text.set(format_sections(&state.draft.content.sections));
                        languages_text.set(format_languages(
                            state.draft.content.languages.as_deref().unwrap_or_default(),
                        ));
                        requirements_text
                            .set(format_requirements(&state.draft.content.requirements));
                        accessibility_text
                            .set(format_accessibility(&state.draft.content.accessibility));
                        message.set(Some(
                            "Existing Store Page loaded locally. Publishing will add the selected listing reciprocally."
                                .to_string(),
                        ));
                    }
                    Err(error) => message.set(Some(error)),
                }
                loading.set(false);
            });
        }
    });

    let on_back_click = move |_| {
        if publishing.get_untracked() {
            message.set(Some(
                "Wait for the active publication request to finish before leaving.".to_string(),
            ));
            return;
        }
        if partial.get_untracked().is_some() {
            message.set(Some(
                "Resolve or retry the incomplete publication before leaving this editor. Recovery state has been retained."
                    .to_string(),
            ));
            return;
        }
        if draft.get_untracked() != baseline.get_untracked() || input_dirty.get_untracked() {
            show_discard.set(true);
        } else {
            on_back.run(());
        }
    };

    view! {
        <section class="v2-publisher-studio">
            <button class="v2-btn-secondary v2-publisher-back" on:click=on_back_click>"Back to game management"</button>
            <header class="v2-publisher-game-hero">
                <div><p class="v2-publisher-kicker">"Store Page editor"</p><h1>{listing.title.clone()}</h1><p class="text-sm text-on-surface-variant">"Drafts stay local until Publish is selected."</p></div>
            </header>
            {move || loading.get().then(|| view! { <p>"Loading current Store Page and signed listings..."</p> })}
            {move || message.get().map(|value| view! { <p class="rounded-xl bg-surface-container-high p-3" role="status">{value}</p> })}
            {move || parser_error.get().map(|value| view! { <p class="text-error" role="alert">{value}</p> })}
            {move || (!diagnostics.get().is_empty()).then(|| view! { <div class="rounded-xl border border-error p-3" role="alert"><h2>"Validation"</h2><ul>{diagnostics.get().into_iter().map(|item| view! { <li>{item}</li> }).collect_view()}</ul></div> })}
            {move || partial.get().map(|result| {
                let page_status = result.store_page.as_ref().map(|page| {
                    format!(
                        "Store Page {}: accepted by {} relay(s), propagation {}",
                        page.event_id,
                        page.success_count,
                        if page.propagation_confirmed { "confirmed" } else { "not confirmed" }
                    )
                }).unwrap_or_else(|| "Store Page was not republished during this retry.".to_string());
                view! { <section class="v2-publisher-panel border border-tertiary" aria-labelledby="partial-store-page-title">
                    <h2 id="partial-store-page-title">"Incomplete publication"</h2>
                    <p>{page_status}</p>
                    {result.cache_error.map(|error| view! { <p class="text-error">{format!("Local cache update failed: {error}")}</p> })}
                    <ul class="mt-3 space-y-2">{result.listing_updates.into_iter().map(|outcome| view! { <li class="rounded-xl bg-surface-container-low p-3"><strong>{outcome.listing_coordinate}</strong><p>{format!("{:?}: published={}, propagation={}", outcome.action, outcome.published, outcome.propagation_confirmed)}</p>{outcome.error.map(|error| view! { <p class="text-error">{error}</p> })}</li> }).collect_view()}</ul>
                    <button class="v2-btn-secondary mt-3" type="button" disabled=move || publishing.get() on:click=move |_| retry.run(())>"Retry incomplete synchronization"</button>
                </section> }
            })}

            <div class="v2-publisher-management-layout">
            <main class="v2-publisher-main space-y-5">
                <fieldset class="contents" disabled=move || publishing.get() || partial.get().is_some()>
                <section class="v2-publisher-panel grid gap-4 sm:grid-cols-2">
                    <h2 class="sm:col-span-2">"Identity and discovery"</h2>
                    <label>"Presentation ID"<input class="v2-input" disabled=move || draft.get().loaded_event_id.is_some() prop:value=move || draft.get().presentation_id on:input=move |event| draft.update(|draft| draft.presentation_id = event_target_value(&event)) /></label>
                    <label>"Release date"<input class="v2-input" prop:value=move || draft.get().content.basic.release_date.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.basic.release_date = optional(event_target_value(&event))) /></label>
                    <label>"Title"<input class="v2-input" prop:value=move || draft.get().content.basic.title.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.basic.title = optional(event_target_value(&event))) /></label>
                    <label>"Summary"<input class="v2-input" prop:value=move || draft.get().content.basic.summary.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.basic.summary = optional(event_target_value(&event))) /></label>
                    <label>"Developer display name"<input class="v2-input" prop:value=move || draft.get().content.basic.developer.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.basic.developer = optional(event_target_value(&event))) /></label>
                    <label>"Publisher display name"<input class="v2-input" prop:value=move || draft.get().content.basic.publisher.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.basic.publisher = optional(event_target_value(&event))) /></label>
                    <label>"Genres (comma separated)"<input class="v2-input" prop:value=move || draft.get().content.discovery.genres.unwrap_or_default().join(", ") on:input=move |event| draft.update(|draft| { let values = csv(&event_target_value(&event)); draft.content.discovery.genres = (!values.is_empty()).then_some(values); }) /></label>
                    <label>"Features (comma separated)"<input class="v2-input" prop:value=move || draft.get().content.discovery.features.unwrap_or_default().join(", ") on:input=move |event| draft.update(|draft| { let values = csv(&event_target_value(&event)); draft.content.discovery.features = (!values.is_empty()).then_some(values); }) /></label>
                </section>

                <section class="v2-publisher-panel"><h2>"Description"</h2><label>"Markdown"<textarea class="v2-input min-h-48" prop:value=move || draft.get().content.description_markdown on:input=move |event| draft.update(|draft| draft.content.description_markdown = event_target_value(&event)) /></label></section>

                <section class="v2-publisher-panel"><h2>"Associated listings"</h2><p class="text-sm text-on-surface-variant">"One tab-separated row per listing: coordinate, current event ID, link, unlink, or review, and an optional public wss relay. Every review row must be changed explicitly before publishing."</p><textarea class="v2-input min-h-32" prop:value=move || associations_text.get() on:input=move |event| { let value = event_target_value(&event); input_dirty.set(true); association_review_required.set(match parse_associations(&value) { Ok(rows) => rows.iter().any(|row| row.action == ListingPointerMutation::Review), Err(_) => true }); associations_text.set(value); } /></section>

                <section class="v2-publisher-panel"><h2>"Media"</h2><p class="text-sm text-on-surface-variant">"Tab-separated rows: id, image or video, role, HTTPS URL, thumbnail URL, alt text, caption, width, height. Direct trailers must be MP4 or WebM."</p><textarea class="v2-input min-h-40" prop:value=move || media_text.get() on:input=move |event| { input_dirty.set(true); media_text.set(event_target_value(&event)); } /></section>

                <section class="v2-publisher-panel"><h2>"Feature sections"</h2><p class="text-sm text-on-surface-variant">"Tab-separated rows: id, heading, layout, media ID, Markdown. Use \\n for a line break."</p><textarea class="v2-input min-h-40" prop:value=move || sections_text.get() on:input=move |event| { input_dirty.set(true); sections_text.set(event_target_value(&event)); } /></section>

                <section class="v2-publisher-panel"><h2>"Platform requirements"</h2><p class="text-sm text-on-surface-variant">"Tab-separated: platform, minimum or recommended, os, processor, memory, graphics, storage, additional."</p><textarea class="v2-input min-h-32" prop:value=move || requirements_text.get() on:input=move |event| { input_dirty.set(true); requirements_text.set(event_target_value(&event)); } /></section>

                <section class="v2-publisher-panel"><h2>"Languages"</h2><p class="text-sm text-on-surface-variant">"Tab-separated: code, interface, audio, subtitles using true or false."</p><textarea class="v2-input min-h-32" prop:value=move || languages_text.get() on:input=move |event| { input_dirty.set(true); languages_text.set(event_target_value(&event)); } /></section>

                <section class="v2-publisher-panel"><h2>"Accessibility claims"</h2><p class="text-sm text-on-surface-variant">"Tab-separated: feature, supported, notes."</p><textarea class="v2-input min-h-32" prop:value=move || accessibility_text.get() on:input=move |event| { input_dirty.set(true); accessibility_text.set(event_target_value(&event)); } /></section>

                <section class="v2-publisher-panel grid gap-4 sm:grid-cols-2"><h2 class="sm:col-span-2">"External links"</h2>
                    <label>"Website"<input class="v2-input" prop:value=move || draft.get().content.links.website.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.website = optional(event_target_value(&event))) /></label>
                    <label>"Support"<input class="v2-input" prop:value=move || draft.get().content.links.support.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.support = optional(event_target_value(&event))) /></label>
                    <label>"Documentation"<input class="v2-input" prop:value=move || draft.get().content.links.documentation.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.documentation = optional(event_target_value(&event))) /></label>
                    <label>"Source"<input class="v2-input" prop:value=move || draft.get().content.links.source.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.source = optional(event_target_value(&event))) /></label>
                    <label>"Community"<input class="v2-input" prop:value=move || draft.get().content.links.community.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.community = optional(event_target_value(&event))) /></label>
                    <label>"Privacy policy"<input class="v2-input" prop:value=move || draft.get().content.links.privacy_policy.unwrap_or_default() on:input=move |event| draft.update(|draft| draft.content.links.privacy_policy = optional(event_target_value(&event))) /></label>
                </section>

                <section class="v2-publisher-panel"><h2>"Clone presentation"</h2><p class="text-sm text-on-surface-variant">"Cloning copies presentation fields, clears associations, and starts a new optimistic-concurrency identity."</p><div class="flex gap-2"><input class="v2-input" placeholder="new-presentation-id" prop:value=move || clone_id.get() on:input=move |event| clone_id.set(event_target_value(&event)) /><button class="v2-btn-secondary" type="button" on:click=move |_| clone_page.run(())>"Clone locally"</button></div></section>

                <section class="v2-publisher-panel"><h2>"Link existing Store Page"</h2><p class="text-sm text-on-surface-variant">"Load a Store Page owned by this publisher and add the selected listing as an explicit reciprocal association."</p><div class="flex gap-2"><input class="v2-input" placeholder="existing-presentation-id" prop:value=move || link_existing_id.get() on:input=move |event| link_existing_id.set(event_target_value(&event)) /><button class="v2-btn-secondary" type="button" on:click=move |_| link_existing.run(())>"Load existing"</button></div></section>

                <section class="v2-publisher-panel"><div class="flex flex-wrap gap-3"><button class="v2-btn-secondary" type="button" disabled=move || validating.get() on:click=move |_| run_validation.run(())>{move || if validating.get() { "Validating..." } else { "Validate and preview" }}</button><button class="v2-btn-primary" type="button" disabled=move || publishing.get() on:click=move |_| publish.run(())>{move || if publishing.get() { "Publishing..." } else { "Publish Store Page" }}</button></div></section>
                </fieldset>

                {move || preview.get().map(|presentation| view! {
                    <section class="rounded-2xl border-2 border-primary p-4" aria-label="Store Page preview">
                        <p class="v2-publisher-kicker">"Preview mode · links and commerce actions disabled"</p>
                        <h2>{presentation.title.clone().unwrap_or_else(|| listing.title.clone())}</h2>
                        <p>{presentation.summary.clone().unwrap_or_else(|| listing.description.clone())}</p>
                        <div class="my-4 rounded-xl bg-surface-container-high p-3"><strong>"Authoritative listing commerce"</strong><p>{preview_commerce_label(listing.price, &listing.currency, &listing.acquisition)}</p></div>
                        <StorePageRichDetail presentation=presentation preview=true />
                    </section>
                })}
            </main>
            <aside class="v2-publisher-panel v2-publisher-sidebar"><h2>"Draft status"</h2><p>{move || if draft.get() == baseline.get() { "No unsaved changes" } else { "Unsaved local changes" }}</p><p class="text-sm text-on-surface-variant">"Validation and preview use the same core sanitizer and buyer renderer as published Store Pages."</p></aside>
            </div>

            <Show when=move || show_discard.get()>
                <dialog open class="m-auto rounded-2xl bg-surface-container-high p-6 text-on-surface backdrop:bg-black/70"><h2>"Discard Store Page draft?"</h2><p>"Unsaved changes will be removed for this game."</p><div class="mt-4 flex gap-3"><button class="v2-btn-secondary" autofocus on:click=move |_| show_discard.set(false)>"Keep editing"</button><button class="v2-btn-primary" on:click={let key = key.clone(); move |_| { PUBLISHER_STORE_PAGE_DRAFTS.with(|drafts| { drafts.borrow_mut().remove(&key); }); on_back.run(()); }}>"Discard changes"</button></div></dialog>
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tauri_bridge::{EventPublishOutcome, ListingPointerPublishOutcome};

    #[test]
    fn structured_editor_parsers_preserve_supported_v1_fields() {
        let media = parse_media("hero\timage\thero\thttps://cdn.example/hero.png\t\talt | text\tcaption | text\t1920\t1080").expect("media");
        assert_eq!(media[0].width, Some(1920));
        assert_eq!(media[0].alt.as_deref(), Some("alt | text"));
        assert_eq!(
            parse_sections("intro\tIntro\ttext-only\t\tHello **world**")
                .expect("section")
                .len(),
            1
        );
        assert_eq!(
            parse_languages("en\ttrue\tfalse\ttrue").expect("language")[0].code,
            "en"
        );
        assert!(
            parse_requirements("linux-x86_64\tminimum\tLinux\tCPU\t8 GB\tGPU\t2 GB\t")
                .expect("requirements")
                .contains_key("linux-x86_64")
        );
        assert!(
            parse_accessibility("subtitles\ttrue\tConfigurable").expect("accessibility")[0]
                .supported
        );
    }

    #[test]
    fn association_editor_distinguishes_link_unlink_and_review() {
        let values = parse_associations(
            "30402:pub:a\tevent-a\tlink\n30402:pub:b\tevent-b\tunlink\n30402:pub:c\tevent-c\treview",
        )
        .expect("associations");
        assert_eq!(values[0].action, ListingPointerMutation::Link);
        assert_eq!(values[1].action, ListingPointerMutation::Unlink);
        assert_eq!(values[2].action, ListingPointerMutation::Review);
    }

    #[test]
    fn editor_rejects_ambiguous_boolean_claims() {
        assert!(parse_languages("en\tflase\tfalse\ttrue").is_err());
        assert!(parse_accessibility("subtitles\tyes\tConfigurable").is_err());
    }

    #[test]
    fn section_rows_round_trip_markdown_losslessly() {
        let section = StorePageSection {
            id: "details".into(),
            heading: "Heading\twith tab".into(),
            body_markdown: "literal \\n and real\nnewline\tindent".into(),
            media_id: None,
            layout: "text-only".into(),
        };
        assert_eq!(
            parse_sections(&format_sections(std::slice::from_ref(&section))).expect("round trip"),
            vec![section]
        );
    }

    #[test]
    fn publisher_draft_cache_is_scoped_by_account() {
        let mut draft = StorePageDraft::new("page".into(), vec!["listing".into()]);
        let baseline = draft.clone();
        draft.content.description_markdown = "unsaved".into();
        save_cached_draft(
            "npub-a|listing",
            draft.clone(),
            baseline,
            RawDraftFields {
                associations: "invalid raw input".into(),
                media: String::new(),
                sections: String::new(),
                languages: String::new(),
                requirements: String::new(),
                accessibility: String::new(),
            },
            true,
        );
        retain_account_drafts("npub-b");
        assert!(cached_draft("npub-a|listing").is_none());
        assert!(cached_draft("npub-b|listing").is_none());
    }

    #[test]
    fn new_draft_keeps_selected_listing_association_before_load() {
        let coordinate = "30402:publisher:game";
        let mut draft = StorePageDraft::new("game".into(), Vec::new());
        let mut raw = RawDraftFields {
            associations: String::new(),
            media: String::new(),
            sections: String::new(),
            languages: String::new(),
            requirements: String::new(),
            accessibility: String::new(),
        };

        seed_new_draft_association(&mut draft, &mut raw, coordinate, Some("listing-event"));

        assert_eq!(draft.listing_coordinates, vec![coordinate.to_string()]);
        assert_eq!(
            raw.associations,
            format!("{coordinate}\tlisting-event\tlink")
        );
    }

    #[test]
    fn retry_after_partial_success_only_targets_incomplete_pointers() {
        let mutations = vec![
            StorePageListingMutation {
                listing_coordinate: "listing-a".into(),
                expected_event_id: "a".into(),
                action: ListingPointerMutation::Link,
                relay_hint: None,
                published_event_id: None,
            },
            StorePageListingMutation {
                listing_coordinate: "listing-b".into(),
                expected_event_id: "b".into(),
                action: ListingPointerMutation::Link,
                relay_hint: None,
                published_event_id: None,
            },
        ];
        let result = PublishStorePageResponse {
            store_page_coordinate: "page".into(),
            store_page: Some(EventPublishOutcome {
                event_id: "page-event".into(),
                success_count: 2,
                failure_count: 0,
                propagation_confirmed: true,
            }),
            listing_updates: vec![
                ListingPointerPublishOutcome {
                    listing_coordinate: "listing-a".into(),
                    action: ListingPointerMutation::Link,
                    replacement_event_id: Some("new-a".into()),
                    published: true,
                    propagation_confirmed: true,
                    error: None,
                },
                ListingPointerPublishOutcome {
                    listing_coordinate: "listing-b".into(),
                    action: ListingPointerMutation::Link,
                    replacement_event_id: None,
                    published: false,
                    propagation_confirmed: false,
                    error: Some("failed".into()),
                },
            ],
            complete: false,
            retryable: true,
            cache_error: None,
            retry_scope_complete: false,
        };

        let retry = retryable_mutations(&result, &mutations);
        assert_eq!(retry, vec![mutations[1].clone()]);
    }

    #[test]
    fn preview_commerce_is_derived_only_from_listing_values() {
        assert_eq!(
            preview_commerce_label(19.99, "USD", &crate::models::AcquisitionPolicy::Gated),
            "19.99 USD · Gated"
        );
    }

    #[test]
    fn account_generation_rejects_late_editor_response() {
        assert!(!accepts_account_response(
            Some("npub-new"),
            "npub-old",
            2,
            1
        ));
    }
}
