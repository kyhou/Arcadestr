//! Pure publisher campaign-management rules used by the v2 Publish views.

use std::cmp::Reverse;
use std::collections::HashMap;

use arcadestr_core::is_replaceable_event_newer;

use crate::models::{CampaignPointer, GameListing, ListingSource};
use crate::tauri_bridge::{PublishCampaignRequest, PublishCampaignResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignEditorType {
    FreeClaim,
    DiscountedPrice,
    TimedAccessPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignForm {
    pub campaign_id: String,
    pub starts_at: String,
    pub ends_at: String,
    pub campaign_type: CampaignEditorType,
    pub update_listing_pointer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignValidationError {
    MissingCampaignId,
    MissingStart,
    MissingEnd,
    InvalidStart,
    InvalidEnd,
    EndMustFollowStart,
    UnsupportedCampaignType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignPublishOutcome {
    Success,
    PartialSuccess(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignPointerUpdatePlan {
    None,
    AddWithCampaignPublish,
    RemoveAfterCampaignPublish,
}

impl CampaignForm {
    pub fn new(campaign_id: String) -> Self {
        Self {
            campaign_id,
            starts_at: String::new(),
            ends_at: String::new(),
            campaign_type: CampaignEditorType::FreeClaim,
            update_listing_pointer: true,
        }
    }
}

pub fn listing_coordinate(listing: &GameListing) -> String {
    format!("30402:{}:{}", listing.publisher_npub, listing.id)
}

pub fn current_user_listings(
    listings: impl IntoIterator<Item = GameListing>,
    publisher_npub: &str,
) -> Vec<GameListing> {
    let mut latest = HashMap::<String, GameListing>::new();
    for listing in listings.into_iter().filter(|listing| {
        listing.publisher_npub == publisher_npub && listing.source == ListingSource::Nip99Listing
    }) {
        let coordinate = listing_coordinate(&listing);
        if latest.get(&coordinate).map_or(true, |current| {
            is_replaceable_event_newer(
                listing.created_at,
                listing.event_id.as_deref(),
                current.created_at,
                current.event_id.as_deref(),
            )
        }) {
            latest.insert(coordinate, listing);
        }
    }
    let mut listings = latest.into_values().collect::<Vec<_>>();
    listings.sort_by_key(|listing| Reverse(listing.created_at));
    listings
}

pub fn campaign_terms_editable(classification: &str) -> bool {
    classification == "upcoming"
}

pub fn campaign_publish_outcome(response: &PublishCampaignResponse) -> CampaignPublishOutcome {
    match &response.pointer_update_error {
        Some(error) => CampaignPublishOutcome::PartialSuccess(error.clone()),
        None => CampaignPublishOutcome::Success,
    }
}

pub fn campaign_pointer_update_plan(
    initially_points_to_campaign: bool,
    update_listing_pointer: bool,
) -> CampaignPointerUpdatePlan {
    match (initially_points_to_campaign, update_listing_pointer) {
        (true, false) => CampaignPointerUpdatePlan::RemoveAfterCampaignPublish,
        (_, true) => CampaignPointerUpdatePlan::AddWithCampaignPublish,
        (false, false) => CampaignPointerUpdatePlan::None,
    }
}

pub fn apply_campaign_pointer_mutation(
    listing: &GameListing,
    root_event_id: &str,
    listing_event_id: &str,
    remove: bool,
) -> GameListing {
    let mut updated = listing.clone();
    if remove {
        updated
            .campaigns
            .retain(|pointer| pointer.root_event_id != root_event_id);
    } else if !updated
        .campaigns
        .iter()
        .any(|pointer| pointer.root_event_id == root_event_id)
    {
        updated.campaigns.push(CampaignPointer {
            root_event_id: root_event_id.to_string(),
            relay_hint: None,
        });
    }
    updated.event_id = Some(listing_event_id.to_string());
    updated
}

pub fn apply_campaign_response_pointer_mutation(
    listing: &GameListing,
    response: &PublishCampaignResponse,
    remove: bool,
    pointer_update_requested: bool,
) -> Option<GameListing> {
    if !pointer_update_requested || response.pointer_update_error.is_some() {
        return None;
    }
    let listing_event_id = response.listing_event_id.as_deref()?;
    Some(apply_campaign_pointer_mutation(
        listing,
        &response.root_event_id,
        listing_event_id,
        remove,
    ))
}

pub fn generated_campaign_id(now_unix: u64, suffix: &str) -> String {
    format!("promo-{}-{}", unix_day(now_unix), sanitize_suffix(suffix))
}

pub fn validate_campaign_form(form: &CampaignForm) -> Result<(u64, u64), CampaignValidationError> {
    if form.campaign_id.trim().is_empty() {
        return Err(CampaignValidationError::MissingCampaignId);
    }
    if !matches!(form.campaign_type, CampaignEditorType::FreeClaim) {
        return Err(CampaignValidationError::UnsupportedCampaignType);
    }
    if form.starts_at.trim().is_empty() {
        return Err(CampaignValidationError::MissingStart);
    }
    if form.ends_at.trim().is_empty() {
        return Err(CampaignValidationError::MissingEnd);
    }
    let starts =
        datetime_local_to_unix(&form.starts_at).ok_or(CampaignValidationError::InvalidStart)?;
    let ends = datetime_local_to_unix(&form.ends_at).ok_or(CampaignValidationError::InvalidEnd)?;
    if starts >= ends {
        return Err(CampaignValidationError::EndMustFollowStart);
    }
    Ok((starts, ends))
}

pub fn build_campaign_request(
    publisher_npub: String,
    listing_id: String,
    form: &CampaignForm,
    predecessor_event_id: Option<String>,
) -> Result<PublishCampaignRequest, CampaignValidationError> {
    let (starts_at, ends_at) = validate_campaign_form(form)?;
    Ok(PublishCampaignRequest {
        publisher_npub,
        listing_id,
        campaign_id: form.campaign_id.clone(),
        starts_at: Some(starts_at),
        ends_at: Some(ends_at),
        predecessor_event_id,
        cancel: false,
        update_listing_pointer: form.update_listing_pointer,
    })
}

pub fn build_cancel_request(
    publisher_npub: String,
    listing_id: String,
    campaign_id: String,
    predecessor_event_id: String,
    update_listing_pointer: bool,
) -> PublishCampaignRequest {
    PublishCampaignRequest {
        publisher_npub,
        listing_id,
        campaign_id,
        starts_at: None,
        ends_at: None,
        predecessor_event_id: Some(predecessor_event_id),
        cancel: true,
        update_listing_pointer,
    }
}

pub fn campaign_status(classification: &str) -> &'static str {
    match classification {
        "upcoming" => "Upcoming",
        "active" => "Active",
        "ended" => "Ended",
        "cancelled" => "Cancelled",
        _ => "Invalid",
    }
}

fn sanitize_suffix(value: &str) -> String {
    let suffix: String = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(12)
        .collect();
    if suffix.is_empty() {
        "campaign".to_string()
    } else {
        suffix
    }
}

fn unix_day(now_unix: u64) -> String {
    let days = now_unix / 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + if month <= 2 { 1 } else { 0 }, month, day)
}

#[cfg(target_arch = "wasm32")]
pub fn datetime_local_to_unix(value: &str) -> Option<u64> {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(value));
    let milliseconds = date.get_time();
    (milliseconds.is_finite() && milliseconds >= 0.0).then_some((milliseconds / 1000.0) as u64)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn datetime_local_to_unix(value: &str) -> Option<u64> {
    let (date, time) = value.split_once('T')?;
    let mut date_parts = date.split('-').map(|part| part.parse::<i64>().ok());
    let year = date_parts.next()??;
    let month = date_parts.next()??;
    let day = date_parts.next()??;
    let mut time_parts = time.split(':').map(|part| part.parse::<u64>().ok());
    let hour = time_parts.next()??;
    let minute = time_parts.next()??;
    if month == 0 || month > 12 || day == 0 || day > 31 || hour > 23 || minute > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    Some((days * 86_400 + hour * 3_600 + minute * 60) as u64)
}

#[cfg(not(target_arch = "wasm32"))]
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<u64> {
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    (days >= 0).then_some(days as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(created_at: u64, event_id: &str) -> GameListing {
        GameListing {
            id: "same-game".into(),
            source: ListingSource::Nip99Listing,
            title: event_id.into(),
            description: String::new(),
            images: Vec::new(),
            download_url: String::new(),
            price: 0.0,
            currency: "SATS".into(),
            price_sats: 0,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub: "npub1publisher".into(),
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: Some(event_id.into()),
            created_at,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    #[test]
    fn current_user_listings_rejects_stale_listing_after_newer_listing() {
        let stale = listing(10, "old");
        let newer = listing(20, "new");

        let visible = current_user_listings([newer.clone(), stale], "npub1publisher");

        assert_eq!(visible, vec![newer]);
    }

    #[test]
    fn current_user_listings_uses_lower_event_id_for_equal_timestamps_in_both_orders() {
        let lower_id = listing(20, "aaa");
        let higher_id = listing(20, "bbb");

        for arrivals in [
            [higher_id.clone(), lower_id.clone()],
            [lower_id.clone(), higher_id.clone()],
        ] {
            assert_eq!(
                current_user_listings(arrivals, "npub1publisher"),
                vec![lower_id.clone()]
            );
        }
    }

    #[test]
    fn generated_campaign_id_is_url_safe_and_date_prefixed() {
        let id = generated_campaign_id(1_752_806_400, "a/b C");
        assert_eq!(id, "promo-2025-07-18-abC");
    }

    #[test]
    fn datetime_local_conversion_is_deterministic() {
        assert_eq!(
            datetime_local_to_unix("2026-07-18T12:30"),
            Some(1_784_377_800)
        );
    }

    #[test]
    fn end_before_start_is_rejected() {
        let form = CampaignForm {
            campaign_id: "promo-test".into(),
            starts_at: "2026-07-18T12:30".into(),
            ends_at: "2026-07-18T11:30".into(),
            campaign_type: CampaignEditorType::FreeClaim,
            update_listing_pointer: true,
        };
        assert_eq!(
            validate_campaign_form(&form),
            Err(CampaignValidationError::EndMustFollowStart)
        );
    }

    #[test]
    fn unsupported_campaign_type_cannot_submit() {
        let form = CampaignForm {
            campaign_id: "promo-test".into(),
            starts_at: "2026-07-18T12:30".into(),
            ends_at: "2026-07-19T12:30".into(),
            campaign_type: CampaignEditorType::DiscountedPrice,
            update_listing_pointer: true,
        };
        assert_eq!(
            validate_campaign_form(&form),
            Err(CampaignValidationError::UnsupportedCampaignType)
        );

        let timed_access = CampaignForm {
            campaign_type: CampaignEditorType::TimedAccessPolicy,
            ..form
        };
        assert_eq!(
            validate_campaign_form(&timed_access),
            Err(CampaignValidationError::UnsupportedCampaignType)
        );
    }

    #[test]
    fn cancel_request_references_current_tip() {
        let request = build_cancel_request(
            "npub1publisher".into(),
            "game".into(),
            "promo-test".into(),
            "event-tip".into(),
            false,
        );
        assert!(request.cancel);
        assert_eq!(request.predecessor_event_id.as_deref(), Some("event-tip"));
        assert!(!request.update_listing_pointer);
    }

    #[test]
    fn new_campaign_defaults_listing_pointer_to_enabled() {
        assert!(CampaignForm::new("promo-test".into()).update_listing_pointer);
    }

    #[test]
    fn upcoming_edit_request_preserves_campaign_id() {
        let mut form = CampaignForm::new("existing-campaign".into());
        form.starts_at = "2026-07-18T12:30".into();
        form.ends_at = "2026-07-19T12:30".into();
        let request = build_campaign_request(
            "npub1publisher".into(),
            "game".into(),
            &form,
            Some("current-tip".into()),
        )
        .expect("upcoming campaign should map to a request");
        assert_eq!(request.campaign_id, "existing-campaign");
        assert_eq!(request.predecessor_event_id.as_deref(), Some("current-tip"));
    }

    #[test]
    fn active_campaign_terms_are_not_editable() {
        assert!(!campaign_terms_editable("active"));
        assert!(campaign_terms_editable("upcoming"));
    }

    #[test]
    fn pointer_update_plan_covers_initial_and_current_checkbox_states() {
        assert_eq!(
            campaign_pointer_update_plan(false, false),
            CampaignPointerUpdatePlan::None
        );
        assert_eq!(
            campaign_pointer_update_plan(false, true),
            CampaignPointerUpdatePlan::AddWithCampaignPublish
        );
        assert_eq!(
            campaign_pointer_update_plan(true, true),
            CampaignPointerUpdatePlan::AddWithCampaignPublish
        );
        assert_eq!(
            campaign_pointer_update_plan(true, false),
            CampaignPointerUpdatePlan::RemoveAfterCampaignPublish
        );
    }

    #[test]
    fn campaign_pointer_add_is_idempotent_and_replaces_listing_event_id() {
        let mut original = listing(20, "old-listing-event");
        original.campaigns.push(crate::models::CampaignPointer {
            root_event_id: "root-a".into(),
            relay_hint: Some("wss://relay.example.com".into()),
        });

        let updated = apply_campaign_pointer_mutation(
            &original,
            "root-a",
            "replacement-listing-event",
            false,
        );

        assert_eq!(updated.campaigns, original.campaigns);
        assert_eq!(
            updated.event_id.as_deref(),
            Some("replacement-listing-event")
        );
    }

    #[test]
    fn campaign_pointer_add_uses_no_relay_hint() {
        let updated =
            apply_campaign_pointer_mutation(&listing(20, "old"), "root-b", "replacement", false);

        assert_eq!(updated.campaigns.len(), 1);
        assert_eq!(updated.campaigns[0].root_event_id, "root-b");
        assert_eq!(updated.campaigns[0].relay_hint, None);
    }

    #[test]
    fn campaign_pointer_remove_deletes_only_target() {
        let mut original = listing(20, "old");
        original.campaigns = ["root-a", "root-b", "root-c"]
            .into_iter()
            .map(|root_event_id| crate::models::CampaignPointer {
                root_event_id: root_event_id.into(),
                relay_hint: None,
            })
            .collect();

        let updated = apply_campaign_pointer_mutation(&original, "root-b", "replacement", true);

        assert_eq!(
            updated
                .campaigns
                .iter()
                .map(|pointer| pointer.root_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root-a", "root-c"]
        );
        assert_eq!(updated.event_id.as_deref(), Some("replacement"));
    }

    #[test]
    fn response_pointer_mutation_requires_confirmed_listing_update() {
        let original = listing(20, "old");
        let failed = PublishCampaignResponse {
            event_id: "campaign-event".into(),
            root_event_id: "root".into(),
            listing_event_id: Some("replacement".into()),
            pointer_update_error: Some("relay rejected listing".into()),
        };
        assert_eq!(
            apply_campaign_response_pointer_mutation(&original, &failed, false, true),
            None
        );

        let successful = PublishCampaignResponse {
            pointer_update_error: None,
            ..failed
        };
        assert_eq!(
            apply_campaign_response_pointer_mutation(&original, &successful, false, false),
            None
        );
        let updated = apply_campaign_response_pointer_mutation(&original, &successful, false, true)
            .expect("confirmed pointer update should produce local listing state");
        assert_eq!(updated.campaigns[0].root_event_id, "root");
        assert_eq!(updated.event_id.as_deref(), Some("replacement"));
    }

    #[test]
    fn pointer_failure_is_partial_success() {
        let response = PublishCampaignResponse {
            event_id: "event".into(),
            root_event_id: "root".into(),
            listing_event_id: None,
            pointer_update_error: Some("relay rejected listing".into()),
        };
        assert_eq!(
            campaign_publish_outcome(&response),
            CampaignPublishOutcome::PartialSuccess("relay rejected listing".into())
        );
    }
}
