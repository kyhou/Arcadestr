use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use arcadestr_core::adp_protocol::EXPERIMENTAL_STORE_PAGE_KIND;
use arcadestr_core::nip46::AppSignerState;
use arcadestr_core::signers::NostrSigner;
use arcadestr_core::store_page::{
    build_listing_store_page_replacement, build_store_page_event_builder, clone_store_page_draft,
    listing_coordinate, parse_store_page_event, parse_store_page_pointers,
    resolve_store_page_events, store_page_coordinate, validate_listing_revision,
    validate_store_page_association, validate_store_page_draft, validate_store_page_revision,
    ParsedStorePage, StorePageDraft, StorePagePointerAction, StorePageValidationDiagnostic,
};
use arcadestr_core::store_page_discovery::{
    preview_store_page_detail, StorePageBatchEnrichment, StorePageBatchService,
    StorePageDetailEnrichment, StorePageDetailPresentation, StorePageListingRef,
};
use arcadestr_core::store_page_repository::StorePageRepository;
use nostr::nips::nip19::FromBech32;
use nostr::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag};
use serde::{Deserialize, Serialize};

use crate::adp_commands::{
    ensure_publish_account_current, resolve_active_signer, verify_expected_publisher,
};
use crate::AppState;

const STORE_PAGE_PROPAGATION_THRESHOLD: usize = 2;

#[derive(Debug, Deserialize)]
pub struct StorePageEnrichmentRequest {
    pub generation: u64,
    pub listings: Vec<StorePageListingRef>,
}

#[derive(Debug, Deserialize)]
pub struct StorePageDetailRequest {
    pub generation: u64,
    pub listing: StorePageListingRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageListingRevision {
    pub listing_coordinate: String,
    pub event_id: String,
    pub reciprocal: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoadPublisherStorePageRequest {
    pub expected_publisher_npub: String,
    pub listing: StorePageListingRef,
    #[serde(default)]
    pub presentation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublisherStorePageEditorState {
    pub draft: StorePageDraft,
    pub baseline_draft: StorePageDraft,
    pub listings: Vec<StorePageListingRevision>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ValidateStorePageDraftRequest {
    pub expected_publisher_npub: String,
    pub draft: StorePageDraft,
    pub preview_listing: StorePageListingRevision,
    pub listing_mutations: Vec<StorePageListingMutation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidateStorePageDraftResponse {
    pub valid: bool,
    pub diagnostics: Vec<StorePageValidationDiagnostic>,
    pub preview: Option<StorePageDetailPresentation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingPointerMutation {
    Link,
    Unlink,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePageListingMutation {
    pub listing_coordinate: String,
    pub expected_event_id: String,
    pub action: ListingPointerMutation,
    pub relay_hint: Option<String>,
    #[serde(default)]
    pub published_event_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishStorePageRequest {
    pub expected_publisher_npub: String,
    pub draft: StorePageDraft,
    pub listing_mutations: Vec<StorePageListingMutation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloneStorePageRequest {
    pub source: StorePageDraft,
    pub presentation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetryStorePagePointersRequest {
    pub expected_publisher_npub: String,
    pub store_page_coordinate: String,
    pub store_page_event_id: String,
    pub listing_mutations: Vec<StorePageListingMutation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventPublishOutcome {
    pub event_id: String,
    pub success_count: usize,
    pub failure_count: usize,
    pub propagation_confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListingPointerPublishOutcome {
    pub listing_coordinate: String,
    pub action: ListingPointerMutation,
    pub replacement_event_id: Option<String>,
    pub published: bool,
    pub propagation_confirmed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishStorePageResponse {
    pub store_page_coordinate: String,
    pub store_page: Option<EventPublishOutcome>,
    pub listing_updates: Vec<ListingPointerPublishOutcome>,
    pub complete: bool,
    pub retryable: bool,
    pub cache_error: Option<String>,
    pub retry_scope_complete: bool,
}

#[tauri::command]
pub async fn enrich_store_pages(
    state: tauri::State<'_, AppState>,
    request: StorePageEnrichmentRequest,
) -> Result<StorePageBatchEnrichment, String> {
    let relay_manager = {
        let nostr = state.nostr.lock().await;
        nostr.relay_manager()
    };
    let repository = StorePageRepository::new(state.database.pool().clone());
    StorePageBatchService::new(&relay_manager, &repository)
        .enrich(request.generation, &request.listings)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn enrich_store_page_detail(
    state: tauri::State<'_, AppState>,
    request: StorePageDetailRequest,
) -> Result<StorePageDetailEnrichment, String> {
    let relay_manager = {
        let nostr = state.nostr.lock().await;
        nostr.relay_manager()
    };
    let repository = StorePageRepository::new(state.database.pool().clone());
    StorePageBatchService::new(&relay_manager, &repository)
        .enrich_detail(request.generation, &request.listing)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn load_publisher_store_page_editor(
    state: tauri::State<'_, AppState>,
    request: LoadPublisherStorePageRequest,
) -> Result<PublisherStorePageEditorState, String> {
    let publisher = PublicKey::from_bech32(&request.expected_publisher_npub)
        .map_err(|error| format!("invalid publisher npub: {error}"))?;
    let relay_manager = relay_manager(&state).await;
    let repository = StorePageRepository::new(state.database.pool().clone());
    let listing = fetch_current_listing(
        &relay_manager,
        &repository,
        &request.listing.listing_coordinate,
    )
    .await?;
    if listing.pubkey != publisher {
        return Err("the selected listing is not owned by the active publisher".to_string());
    }
    validate_listing_revision(
        &request.listing.listing_coordinate,
        &request.listing.listing_event_id,
        &listing.id.to_hex(),
    )
    .map_err(|error| error.to_string())?;

    let pointer_report = parse_store_page_pointers(&listing).map_err(|error| error.to_string())?;
    let mut diagnostics = pointer_report
        .diagnostics
        .iter()
        .map(|diagnostic| format!("{diagnostic:?}"))
        .collect::<Vec<_>>();
    let mut listings = vec![StorePageListingRevision {
        listing_coordinate: request.listing.listing_coordinate.clone(),
        event_id: listing.id.to_hex(),
        reciprocal: true,
    }];

    let requested_existing = request.presentation_id.clone();
    let target_presentation_id = requested_existing.clone().or_else(|| {
        pointer_report
            .active
            .as_ref()
            .map(|pointer| pointer.presentation_id.clone())
    });
    let (draft, baseline_draft) = if let Some(presentation_id) = target_presentation_id {
        let page =
            fetch_current_store_page(&relay_manager, &repository, publisher, &presentation_id)
                .await?
                .ok_or_else(|| "the requested Store Page could not be loaded".to_string())?;
        if requested_existing.is_none() {
            if let Err(error) = validate_store_page_association(&listing, &page) {
                diagnostics.push(format!("Reciprocal association is incomplete: {error}"));
            }
        }
        for coordinate in &page.listing_coordinates {
            if coordinate == &request.listing.listing_coordinate {
                continue;
            }
            let associated = fetch_current_listing(&relay_manager, &repository, coordinate).await?;
            if associated.pubkey != publisher {
                return Err(format!(
                    "associated listing is not owned by the publisher: {coordinate}"
                ));
            }
            let reciprocal = parse_store_page_pointers(&associated)
                .ok()
                .and_then(|report| report.active)
                .is_some_and(|associated_pointer| {
                    associated_pointer.coordinate
                        == store_page_coordinate(publisher, &page.presentation_id)
                });
            if !reciprocal {
                diagnostics.push(format!(
                    "Listing is no longer reciprocally linked and requires an explicit relink or removal decision: {coordinate}"
                ));
            }
            listings.push(StorePageListingRevision {
                listing_coordinate: coordinate.clone(),
                event_id: associated.id.to_hex(),
                reciprocal,
            });
        }
        let baseline_draft = StorePageDraft {
            presentation_id: page.presentation_id.clone(),
            listing_coordinates: page.listing_coordinates.clone(),
            content: page.content.clone(),
            compact_tags: page.compact_tags.clone(),
            loaded_event_id: Some(page.event.id.to_hex()),
        };
        let draft = StorePageDraft {
            presentation_id: page.presentation_id,
            listing_coordinates: listings
                .iter()
                .map(|listing| listing.listing_coordinate.clone())
                .collect(),
            content: page.content,
            compact_tags: page.compact_tags,
            loaded_event_id: Some(page.event.id.to_hex()),
        };
        (draft, baseline_draft)
    } else {
        let presentation_id = request
            .listing
            .listing_coordinate
            .splitn(3, ':')
            .nth(2)
            .unwrap_or("store-page")
            .to_string();
        let draft = StorePageDraft::new(presentation_id, vec![request.listing.listing_coordinate]);
        (draft.clone(), draft)
    };

    Ok(PublisherStorePageEditorState {
        draft,
        baseline_draft,
        listings,
        diagnostics,
    })
}

#[tauri::command]
pub async fn validate_store_page_draft_command(
    state: tauri::State<'_, AppState>,
    request: ValidateStorePageDraftRequest,
) -> Result<ValidateStorePageDraftResponse, String> {
    let publisher = PublicKey::from_bech32(&request.expected_publisher_npub)
        .map_err(|error| format!("invalid publisher npub: {error}"))?;
    let params = request.draft.build_params(publisher);
    let validated = match validate_store_page_draft(&params) {
        Ok(validated) => validated,
        Err(error) => {
            return Ok(ValidateStorePageDraftResponse {
                valid: false,
                diagnostics: vec![StorePageValidationDiagnostic::from_error(&error)],
                preview: None,
            });
        }
    };
    let policy_diagnostics = validated
        .diagnostics
        .iter()
        .map(|diagnostic| StorePageValidationDiagnostic {
            code: "sanitization_adjustment".to_string(),
            message: format!("{diagnostic:?}"),
        })
        .collect::<Vec<_>>();
    let relay_manager = relay_manager(&state).await;
    let repository = StorePageRepository::new(state.database.pool().clone());
    if let Err(message) = validate_association_mutations(&request.draft, &request.listing_mutations)
    {
        return Ok(ValidateStorePageDraftResponse {
            valid: false,
            diagnostics: vec![StorePageValidationDiagnostic {
                code: "invalid_associations".to_string(),
                message,
            }],
            preview: None,
        });
    }
    for mutation in &request.listing_mutations {
        let current =
            fetch_current_listing(&relay_manager, &repository, &mutation.listing_coordinate)
                .await?;
        if current.pubkey != publisher {
            return Ok(ValidateStorePageDraftResponse {
                valid: false,
                diagnostics: vec![StorePageValidationDiagnostic {
                    code: "wrong_listing_publisher".to_string(),
                    message: format!(
                        "Listing is not owned by the active publisher: {}",
                        mutation.listing_coordinate
                    ),
                }],
                preview: None,
            });
        }
        if let Err(error) = validate_listing_revision(
            &mutation.listing_coordinate,
            &mutation.expected_event_id,
            &current.id.to_hex(),
        ) {
            return Ok(ValidateStorePageDraftResponse {
                valid: false,
                diagnostics: vec![StorePageValidationDiagnostic {
                    code: "stale_listing".to_string(),
                    message: error.to_string(),
                }],
                preview: None,
            });
        }
    }
    let listing = fetch_current_listing(
        &relay_manager,
        &repository,
        &request.preview_listing.listing_coordinate,
    )
    .await?;
    if listing.pubkey != publisher {
        return Err("the preview listing is not owned by the active publisher".to_string());
    }
    validate_listing_revision(
        &request.preview_listing.listing_coordinate,
        &request.preview_listing.event_id,
        &listing.id.to_hex(),
    )
    .map_err(|error| error.to_string())?;
    match preview_store_page_detail(&params, &listing) {
        Ok(preview) => Ok(ValidateStorePageDraftResponse {
            valid: true,
            diagnostics: policy_diagnostics,
            preview: Some(preview),
        }),
        Err(error) => Ok(ValidateStorePageDraftResponse {
            valid: false,
            diagnostics: vec![StorePageValidationDiagnostic {
                code: "preview".to_string(),
                message: error.to_string(),
            }],
            preview: None,
        }),
    }
}

#[tauri::command]
pub async fn clone_store_page(request: CloneStorePageRequest) -> Result<StorePageDraft, String> {
    if request.presentation_id.trim().is_empty() {
        return Err("a new presentation ID is required".to_string());
    }
    Ok(clone_store_page_draft(
        &request.source,
        request.presentation_id,
    ))
}

#[tauri::command]
pub async fn publish_store_page(
    state: tauri::State<'_, AppState>,
    signer_state: tauri::State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    request: PublishStorePageRequest,
) -> Result<PublishStorePageResponse, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = resolve_active_signer(signer_state.inner(), &auth).await?;
    let publisher = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    verify_expected_publisher(&request.expected_publisher_npub, publisher)?;
    validate_publish_request(publisher, &request)?;
    let relay_manager = relay_manager(&state).await;
    let repository = StorePageRepository::new(state.database.pool().clone());
    preflight_publish(&relay_manager, &repository, publisher, &request).await?;

    let current_page = fetch_current_store_page(
        &relay_manager,
        &repository,
        publisher,
        &request.draft.presentation_id,
    )
    .await?;
    validate_store_page_revision(
        request.draft.loaded_event_id.as_deref(),
        current_page
            .as_ref()
            .map(|page| page.event.id.to_hex())
            .as_deref(),
    )
    .map_err(|error| error.to_string())?;
    for mutation in &request.listing_mutations {
        let current =
            fetch_current_listing(&relay_manager, &repository, &mutation.listing_coordinate)
                .await?;
        if current.pubkey != publisher {
            return Err(format!(
                "listing is not owned by the active publisher: {}",
                mutation.listing_coordinate
            ));
        }
        validate_listing_revision(
            &mutation.listing_coordinate,
            &mutation.expected_event_id,
            &current.id.to_hex(),
        )
        .map_err(|error| error.to_string())?;
    }
    ensure_publish_account_current(&state, signer_state.inner(), publisher).await?;

    let signed_page = signer
        .sign_event(
            build_store_page_event_builder(&request.draft.build_params(publisher))
                .map_err(|error| error.to_string())?
                .build(publisher),
        )
        .await
        .map_err(|error| error.to_string())?;
    let parsed_page = parse_store_page_event(&signed_page).map_err(|error| error.to_string())?;
    let send = relay_manager
        .send_event(&signed_page)
        .await
        .map_err(|error| error.to_string())?;
    if send.success_count == 0 {
        return Err("no relay accepted the Store Page event".to_string());
    }
    let page_confirmed = confirm_exact_event(
        &relay_manager,
        store_page_filter(publisher, &request.draft.presentation_id),
        &signed_page.id.to_hex(),
    )
    .await;
    let cache_error = if page_confirmed {
        repository
            .upsert_valid(&parsed_page)
            .await
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };

    let listing_updates = publish_pointer_updates(
        &state,
        signer_state.inner(),
        &relay_manager,
        &repository,
        signer.as_ref(),
        publisher,
        &parsed_page,
        &request.listing_mutations,
    )
    .await;
    let complete = page_confirmed
        && cache_error.is_none()
        && listing_updates
            .iter()
            .all(|outcome| outcome.published && outcome.propagation_confirmed);
    let retryable = !complete;
    Ok(PublishStorePageResponse {
        store_page_coordinate: store_page_coordinate(publisher, &request.draft.presentation_id),
        store_page: Some(EventPublishOutcome {
            event_id: signed_page.id.to_hex(),
            success_count: send.success_count,
            failure_count: send.failure_count,
            propagation_confirmed: page_confirmed,
        }),
        listing_updates,
        complete,
        retryable,
        cache_error,
        retry_scope_complete: false,
    })
}

#[tauri::command]
pub async fn retry_store_page_pointer_sync(
    state: tauri::State<'_, AppState>,
    signer_state: tauri::State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    request: RetryStorePagePointersRequest,
) -> Result<PublishStorePageResponse, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = resolve_active_signer(signer_state.inner(), &auth).await?;
    let publisher = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    verify_expected_publisher(&request.expected_publisher_npub, publisher)?;
    let (_, coordinate_publisher, presentation_id) =
        parse_coordinate(&request.store_page_coordinate, EXPERIMENTAL_STORE_PAGE_KIND)?;
    if coordinate_publisher != publisher {
        return Err("the Store Page is not owned by the active publisher".to_string());
    }
    let relay_manager = relay_manager(&state).await;
    let repository = StorePageRepository::new(state.database.pool().clone());
    let page = fetch_current_store_page(&relay_manager, &repository, publisher, &presentation_id)
        .await?
        .ok_or_else(|| "the Store Page no longer exists".to_string())?;
    validate_store_page_revision(
        Some(&request.store_page_event_id),
        Some(&page.event.id.to_hex()),
    )
    .map_err(|error| error.to_string())?;
    validate_retry_mutations(&page, &request.listing_mutations)?;
    let page_confirmed = confirm_exact_event(
        &relay_manager,
        store_page_filter(publisher, &presentation_id),
        &request.store_page_event_id,
    )
    .await;
    let cache_error = if page_confirmed {
        repository
            .upsert_valid(&page)
            .await
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    let listing_updates = publish_pointer_updates(
        &state,
        signer_state.inner(),
        &relay_manager,
        &repository,
        signer.as_ref(),
        publisher,
        &page,
        &request.listing_mutations,
    )
    .await;
    let retry_scope_complete = page_confirmed
        && cache_error.is_none()
        && listing_updates
            .iter()
            .all(|outcome| outcome.published && outcome.propagation_confirmed);
    Ok(PublishStorePageResponse {
        store_page_coordinate: request.store_page_coordinate,
        store_page: Some(EventPublishOutcome {
            event_id: request.store_page_event_id,
            success_count: 0,
            failure_count: 0,
            propagation_confirmed: page_confirmed,
        }),
        retryable: !retry_scope_complete,
        listing_updates,
        complete: false,
        cache_error,
        retry_scope_complete,
    })
}

async fn relay_manager(
    state: &tauri::State<'_, AppState>,
) -> arcadestr_core::relay_manager::RelayManager {
    let relay_manager = {
        let nostr = state.nostr.lock().await;
        nostr.relay_manager()
    };
    let manager = relay_manager.lock().await.clone();
    manager
}

fn parse_coordinate(
    coordinate: &str,
    expected_kind: u16,
) -> Result<(u16, PublicKey, String), String> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "invalid coordinate kind".to_string())?;
    let publisher = parts
        .next()
        .and_then(|value| PublicKey::from_hex(value).ok())
        .ok_or_else(|| "invalid coordinate publisher".to_string())?;
    let identifier = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "invalid coordinate identifier".to_string())?
        .to_string();
    if kind != expected_kind {
        return Err(format!("coordinate must use kind:{expected_kind}"));
    }
    Ok((kind, publisher, identifier))
}

async fn fetch_current_listing(
    relay_manager: &arcadestr_core::relay_manager::RelayManager,
    repository: &StorePageRepository,
    coordinate: &str,
) -> Result<Event, String> {
    let (_, publisher, identifier) = parse_coordinate(coordinate, 30402)?;
    let mut events = relay_manager
        .fetch_events(
            Filter::new()
                .kind(Kind::Custom(30402))
                .author(publisher)
                .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [identifier]),
        )
        .await
        .map_err(|error| error.to_string())?;
    if events.is_empty() {
        return Err(format!(
            "no live relay evidence is available for listing: {coordinate}"
        ));
    }
    if let Some(cached) = repository
        .load_listing_event(coordinate)
        .await
        .map_err(|error| error.to_string())?
    {
        events.push(cached);
    }
    let current = events
        .into_iter()
        .filter(|event| {
            event.verify().is_ok()
                && event.kind.as_u16() == 30402
                && event.pubkey == publisher
                && listing_coordinate(event).as_deref() == Ok(coordinate)
        })
        .reduce(|current, candidate| {
            if arcadestr_core::is_replaceable_event_newer(
                candidate.created_at.as_secs(),
                Some(candidate.id.to_hex().as_str()),
                current.created_at.as_secs(),
                Some(current.id.to_hex().as_str()),
            ) {
                candidate
            } else {
                current
            }
        })
        .ok_or_else(|| format!("current listing was not found: {coordinate}"))?;
    let _ = repository
        .upsert_listing_event(&current)
        .await
        .map_err(|error| error.to_string())?;
    Ok(current)
}

async fn fetch_current_store_page(
    relay_manager: &arcadestr_core::relay_manager::RelayManager,
    repository: &StorePageRepository,
    publisher: PublicKey,
    presentation_id: &str,
) -> Result<Option<ParsedStorePage>, String> {
    let mut events = relay_manager
        .fetch_events(store_page_filter(publisher, presentation_id))
        .await
        .map_err(|error| error.to_string())?;
    let coordinate = store_page_coordinate(publisher, presentation_id);
    let cached = repository
        .load(&coordinate)
        .await
        .map_err(|error| error.to_string())?;
    if events.is_empty()
        && matches!(
            &cached,
            arcadestr_core::store_page_repository::StorePageCacheLookup::Current(_)
        )
    {
        return Err(format!(
            "no live relay evidence is available for Store Page: {coordinate}"
        ));
    }
    if let arcadestr_core::store_page_repository::StorePageCacheLookup::Current(entry) = cached {
        events.push(entry.parsed.event);
    }
    Ok(resolve_store_page_events(
        events.iter(),
        publisher,
        presentation_id,
    ))
}

fn store_page_filter(publisher: PublicKey, presentation_id: &str) -> Filter {
    Filter::new()
        .kind(Kind::Custom(EXPERIMENTAL_STORE_PAGE_KIND))
        .author(publisher)
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::D),
            [presentation_id.to_string()],
        )
}

fn validate_publish_request(
    publisher: PublicKey,
    request: &PublishStorePageRequest,
) -> Result<(), String> {
    validate_store_page_draft(&request.draft.build_params(publisher))
        .map_err(|error| error.to_string())?;
    validate_association_mutations(&request.draft, &request.listing_mutations)
}

fn validate_association_mutations(
    draft: &StorePageDraft,
    mutations: &[StorePageListingMutation],
) -> Result<(), String> {
    if mutations
        .iter()
        .any(|mutation| mutation.action == ListingPointerMutation::Review)
    {
        return Err(
            "every nonreciprocal listing requires an explicit link or unlink decision".to_string(),
        );
    }
    let links = mutations
        .iter()
        .filter(|mutation| mutation.action == ListingPointerMutation::Link)
        .map(|mutation| mutation.listing_coordinate.clone())
        .collect::<HashSet<_>>();
    let associations = draft
        .listing_coordinates
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if links != associations {
        return Err(
            "Store Page associations must exactly match requested listing links".to_string(),
        );
    }
    if mutations.len()
        != mutations
            .iter()
            .map(|mutation| &mutation.listing_coordinate)
            .collect::<HashSet<_>>()
            .len()
    {
        return Err("duplicate listing pointer mutations are not allowed".to_string());
    }
    Ok(())
}

fn validate_retry_mutations(
    page: &ParsedStorePage,
    mutations: &[StorePageListingMutation],
) -> Result<(), String> {
    for mutation in mutations {
        let associated = page
            .listing_coordinates
            .contains(&mutation.listing_coordinate);
        match mutation.action {
            ListingPointerMutation::Link if !associated => {
                return Err(format!(
                    "retry link is absent from the published Store Page: {}",
                    mutation.listing_coordinate
                ));
            }
            ListingPointerMutation::Unlink if associated => {
                return Err(format!(
                    "retry unlink is still present in the published Store Page: {}",
                    mutation.listing_coordinate
                ));
            }
            ListingPointerMutation::Review => {
                return Err("retry contains an unresolved association decision".to_string());
            }
            _ => {}
        }
    }
    Ok(())
}

async fn preflight_publish(
    relay_manager: &arcadestr_core::relay_manager::RelayManager,
    repository: &StorePageRepository,
    publisher: PublicKey,
    request: &PublishStorePageRequest,
) -> Result<(), String> {
    let current_page = fetch_current_store_page(
        relay_manager,
        repository,
        publisher,
        &request.draft.presentation_id,
    )
    .await?;
    validate_store_page_revision(
        request.draft.loaded_event_id.as_deref(),
        current_page
            .as_ref()
            .map(|page| page.event.id.to_hex())
            .as_deref(),
    )
    .map_err(|error| error.to_string())?;
    if let Some(current_page) = &current_page {
        let desired = request
            .draft
            .listing_coordinates
            .iter()
            .collect::<HashSet<_>>();
        for removed in current_page
            .listing_coordinates
            .iter()
            .filter(|coordinate| !desired.contains(coordinate))
        {
            let listing = fetch_current_listing(relay_manager, repository, removed).await?;
            let points_to_current_page = parse_store_page_pointers(&listing)
                .ok()
                .and_then(|report| report.active)
                .is_some_and(|pointer| {
                    pointer.coordinate
                        == store_page_coordinate(publisher, &current_page.presentation_id)
                });
            if points_to_current_page
                && !request.listing_mutations.iter().any(|mutation| {
                    mutation.listing_coordinate == *removed
                        && mutation.action == ListingPointerMutation::Unlink
                })
            {
                return Err(format!(
                    "removing Store Page association requires an explicit unlink: {removed}"
                ));
            }
        }
    }
    for mutation in &request.listing_mutations {
        let listing =
            fetch_current_listing(relay_manager, repository, &mutation.listing_coordinate).await?;
        if listing.pubkey != publisher {
            return Err(format!(
                "listing is not owned by the active publisher: {}",
                mutation.listing_coordinate
            ));
        }
        validate_listing_revision(
            &mutation.listing_coordinate,
            &mutation.expected_event_id,
            &listing.id.to_hex(),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn publish_pointer_updates(
    state: &tauri::State<'_, AppState>,
    signer_state: &Arc<tokio::sync::Mutex<AppSignerState>>,
    relay_manager: &arcadestr_core::relay_manager::RelayManager,
    repository: &StorePageRepository,
    signer: &dyn NostrSigner,
    publisher: PublicKey,
    page: &ParsedStorePage,
    mutations: &[StorePageListingMutation],
) -> Vec<ListingPointerPublishOutcome> {
    let mut outcomes = Vec::with_capacity(mutations.len());
    for mutation in mutations {
        let result = publish_pointer_update(
            state,
            signer_state,
            relay_manager,
            repository,
            signer,
            publisher,
            page,
            mutation,
        )
        .await;
        outcomes.push(match result {
            Ok((event_id, confirmed)) => ListingPointerPublishOutcome {
                listing_coordinate: mutation.listing_coordinate.clone(),
                action: mutation.action,
                replacement_event_id: Some(event_id),
                published: true,
                propagation_confirmed: confirmed,
                error: (!confirmed).then(|| "listing propagation was not confirmed".to_string()),
            },
            Err(error) => ListingPointerPublishOutcome {
                listing_coordinate: mutation.listing_coordinate.clone(),
                action: mutation.action,
                replacement_event_id: None,
                published: false,
                propagation_confirmed: false,
                error: Some(error),
            },
        });
    }
    outcomes
}

async fn publish_pointer_update(
    state: &tauri::State<'_, AppState>,
    signer_state: &Arc<tokio::sync::Mutex<AppSignerState>>,
    relay_manager: &arcadestr_core::relay_manager::RelayManager,
    repository: &StorePageRepository,
    signer: &dyn NostrSigner,
    publisher: PublicKey,
    page: &ParsedStorePage,
    mutation: &StorePageListingMutation,
) -> Result<(String, bool), String> {
    let listing =
        fetch_current_listing(relay_manager, repository, &mutation.listing_coordinate).await?;
    if listing.pubkey != publisher {
        return Err("listing is not owned by the active publisher".to_string());
    }
    if let Some(published_event_id) = &mutation.published_event_id {
        ensure_store_page_current(relay_manager, repository, publisher, page).await?;
        if listing.id.to_hex() != *published_event_id {
            return Err(
                "the listing changed after its pointer replacement was published".to_string(),
            );
        }
        match mutation.action {
            ListingPointerMutation::Link => {
                validate_store_page_association(&listing, page)
                    .map_err(|error| error.to_string())?;
            }
            ListingPointerMutation::Unlink => {
                let page_coordinate = store_page_coordinate(publisher, &page.presentation_id);
                if parse_store_page_pointers(&listing)
                    .ok()
                    .and_then(|report| report.active)
                    .is_some_and(|pointer| pointer.coordinate == page_coordinate)
                {
                    return Err("the published listing still points to the Store Page".to_string());
                }
            }
            ListingPointerMutation::Review => {
                return Err("listing association still requires review".to_string())
            }
        }
        let (_, author, identifier) = parse_coordinate(&mutation.listing_coordinate, 30402)?;
        let confirmed = confirm_exact_event(
            relay_manager,
            Filter::new()
                .kind(Kind::Custom(30402))
                .author(author)
                .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [identifier]),
            published_event_id,
        )
        .await;
        return Ok((published_event_id.clone(), confirmed));
    }
    validate_listing_revision(
        &mutation.listing_coordinate,
        &mutation.expected_event_id,
        &listing.id.to_hex(),
    )
    .map_err(|error| error.to_string())?;
    if mutation.action == ListingPointerMutation::Unlink {
        let expected_page = store_page_coordinate(publisher, &page.presentation_id);
        let current_pointer = parse_store_page_pointers(&listing)
            .map_err(|error| error.to_string())?
            .active;
        if current_pointer
            .as_ref()
            .is_none_or(|pointer| pointer.coordinate != expected_page)
        {
            let (_, author, identifier) = parse_coordinate(&mutation.listing_coordinate, 30402)?;
            let confirmed = confirm_exact_event(
                relay_manager,
                Filter::new()
                    .kind(Kind::Custom(30402))
                    .author(author)
                    .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [identifier]),
                &listing.id.to_hex(),
            )
            .await;
            return Ok((listing.id.to_hex(), confirmed));
        }
    }
    ensure_publish_account_current(state, signer_state, publisher).await?;
    ensure_store_page_current(relay_manager, repository, publisher, page).await?;
    ensure_publish_account_current(state, signer_state, publisher).await?;
    let action = match mutation.action {
        ListingPointerMutation::Link => StorePagePointerAction::Link {
            store_page_coordinate: store_page_coordinate(publisher, &page.presentation_id),
            relay_hint: mutation.relay_hint.clone(),
        },
        ListingPointerMutation::Unlink => StorePagePointerAction::Unlink,
        ListingPointerMutation::Review => {
            return Err("listing association still requires review".to_string())
        }
    };
    let replacement = signer
        .sign_event(
            build_listing_store_page_replacement(&listing, publisher, &action)
                .map_err(|error| error.to_string())?
                .build(publisher),
        )
        .await
        .map_err(|error| error.to_string())?;
    if mutation.action == ListingPointerMutation::Link {
        validate_store_page_association(&replacement, page).map_err(|error| error.to_string())?;
    }
    let send = relay_manager
        .send_event(&replacement)
        .await
        .map_err(|error| error.to_string())?;
    if send.success_count == 0 {
        return Err("no relay accepted the listing pointer replacement".to_string());
    }
    let (_, author, identifier) = parse_coordinate(&mutation.listing_coordinate, 30402)?;
    let confirmed = confirm_exact_event(
        relay_manager,
        Filter::new()
            .kind(Kind::Custom(30402))
            .author(author)
            .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [identifier]),
        &replacement.id.to_hex(),
    )
    .await;
    Ok((replacement.id.to_hex(), confirmed))
}

async fn ensure_store_page_current(
    relay_manager: &arcadestr_core::relay_manager::RelayManager,
    repository: &StorePageRepository,
    publisher: PublicKey,
    page: &ParsedStorePage,
) -> Result<(), String> {
    let current =
        fetch_current_store_page(relay_manager, repository, publisher, &page.presentation_id)
            .await?
            .ok_or_else(|| "the Store Page no longer exists".to_string())?;
    validate_store_page_revision(
        Some(&page.event.id.to_hex()),
        Some(&current.event.id.to_hex()),
    )
    .map_err(|error| error.to_string())
}

async fn confirm_exact_event(
    relay_manager: &arcadestr_core::relay_manager::RelayManager,
    filter: Filter,
    event_id: &str,
) -> bool {
    let expected = event_id.to_string();
    let seen = Arc::new(Mutex::new(HashSet::new()));
    let callback_seen = Arc::clone(&seen);
    let result = relay_manager
        .fetch_events_streaming(filter, 10, 5, move |relay, events| {
            if current_valid_replacement_id(&events, &expected).as_deref()
                == Some(expected.as_str())
            {
                if let Ok(mut seen) = callback_seen.lock() {
                    seen.insert(relay);
                }
            }
        })
        .await;
    result.is_ok()
        && seen
            .lock()
            .is_ok_and(|seen| seen.len() >= STORE_PAGE_PROPAGATION_THRESHOLD)
}

fn current_valid_replacement_id(events: &[Event], expected_event_id: &str) -> Option<String> {
    let expected_identity = events
        .iter()
        .find(|event| event.id.to_hex() == expected_event_id)
        .and_then(event_identity)?;
    events
        .iter()
        .filter(|event| {
            event_identity(event).as_ref() == Some(&expected_identity)
                && match event.kind.as_u16() {
                    EXPERIMENTAL_STORE_PAGE_KIND => parse_store_page_event(event).is_ok(),
                    30402 => event.verify().is_ok() && listing_coordinate(event).is_ok(),
                    _ => false,
                }
        })
        .reduce(|current, candidate| {
            if arcadestr_core::is_replaceable_event_newer(
                candidate.created_at.as_secs(),
                Some(candidate.id.to_hex().as_str()),
                current.created_at.as_secs(),
                Some(current.id.to_hex().as_str()),
            ) {
                candidate
            } else {
                current
            }
        })
        .map(|event| event.id.to_hex())
}

fn event_identity(event: &Event) -> Option<(u16, PublicKey, String)> {
    let identifiers = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.clone().to_vec();
            (values.len() == 2 && values.first().is_some_and(|name| name == "d"))
                .then(|| values[1].clone())
        })
        .collect::<Vec<_>>();
    let [identifier] = identifiers.as_slice() else {
        return None;
    };
    Some((event.kind.as_u16(), event.pubkey, identifier.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag, Timestamp};

    #[test]
    fn store_page_batch_request_deserializes_generation_and_listing_refs() {
        let request: StorePageEnrichmentRequest = serde_json::from_value(serde_json::json!({
            "generation": 7,
            "listings": [{
                "listing_coordinate": "30402:publisher:game",
                "listing_event_id": "event"
            }]
        }))
        .expect("request");
        assert_eq!(request.generation, 7);
        assert_eq!(request.listings.len(), 1);
    }

    #[tokio::test]
    async fn clone_command_clears_associations_and_loaded_revision() {
        let keys = Keys::generate();
        let mut source = StorePageDraft::new(
            "source".to_string(),
            vec![format!("30402:{}:game", keys.public_key().to_hex())],
        );
        source.loaded_event_id = Some("loaded".to_string());
        source.content.basic.title = Some("Copied title".to_string());

        let cloned = clone_store_page(CloneStorePageRequest {
            source,
            presentation_id: "clone".to_string(),
        })
        .await
        .expect("clone");

        assert_eq!(cloned.presentation_id, "clone");
        assert!(cloned.listing_coordinates.is_empty());
        assert_eq!(cloned.loaded_event_id, None);
        assert_eq!(cloned.content.basic.title.as_deref(), Some("Copied title"));
    }

    #[test]
    fn publish_request_requires_pointer_links_to_match_page_associations() {
        let keys = Keys::generate();
        let coordinate = format!("30402:{}:game", keys.public_key().to_hex());
        let draft = StorePageDraft::new("page".to_string(), vec![coordinate.clone()]);
        let request = PublishStorePageRequest {
            expected_publisher_npub: "unused".to_string(),
            draft,
            listing_mutations: vec![StorePageListingMutation {
                listing_coordinate: coordinate,
                expected_event_id: "event".to_string(),
                action: ListingPointerMutation::Unlink,
                relay_hint: None,
                published_event_id: None,
            }],
        };

        assert_eq!(
            validate_publish_request(keys.public_key(), &request),
            Err("Store Page associations must exactly match requested listing links".to_string())
        );
    }

    #[test]
    fn partial_pointer_failure_is_retryable_and_never_complete() {
        let response = PublishStorePageResponse {
            store_page_coordinate: "30407:publisher:page".to_string(),
            store_page: Some(EventPublishOutcome {
                event_id: "page-event".to_string(),
                success_count: 2,
                failure_count: 0,
                propagation_confirmed: true,
            }),
            listing_updates: vec![ListingPointerPublishOutcome {
                listing_coordinate: "30402:publisher:game".to_string(),
                action: ListingPointerMutation::Link,
                replacement_event_id: None,
                published: false,
                propagation_confirmed: false,
                error: Some("relay rejected pointer".to_string()),
            }],
            complete: false,
            retryable: true,
            cache_error: None,
            retry_scope_complete: false,
        };

        assert!(!response.complete);
        assert!(response.retryable);
        assert_eq!(
            response.store_page.expect("published page").event_id,
            "page-event"
        );
    }

    #[test]
    fn propagation_requires_expected_event_to_be_current_replacement() {
        let keys = Keys::generate();
        let older = EventBuilder::new(Kind::Custom(30402), "older")
            .tags([Tag::parse(["d", "game"]).expect("d tag")])
            .custom_created_at(Timestamp::from(10))
            .sign_with_keys(&keys)
            .expect("older");
        let newer = EventBuilder::new(Kind::Custom(30402), "newer")
            .tags([Tag::parse(["d", "game"]).expect("d tag")])
            .custom_created_at(Timestamp::from(20))
            .sign_with_keys(&keys)
            .expect("newer");

        assert_eq!(
            current_valid_replacement_id(&[older.clone(), newer.clone()], &older.id.to_hex()),
            Some(newer.id.to_hex())
        );
    }
}
