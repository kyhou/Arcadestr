//! Tauri commands for ADP publish flow.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arcadestr_core::adp_client::{
    AdpClient, AdpServerInfo, CreateSettlementQuoteRequest as CoreCreateSettlementQuoteRequest,
    EntitlementClaimRequest as CoreEntitlementClaimRequest,
    PurchaseConfirmRequest as CorePurchaseConfirmRequest,
    PurchaseConfirmResponse as CorePurchaseConfirmResponse, SettlementProofRequest, UploadResponse,
};
use arcadestr_core::adp_discovery::{
    discover_adp_servers as discover_adp_servers_core, AdpServerAnnouncement,
};
use arcadestr_core::adp_publish::{
    build_adp_listing_event_builder, build_fulfillment_authorization_event_builder,
    decode_authoritative_launch_tier, validate_adp_listing_unsigned_event, AdpListingInput,
    AdpListingPublishInput, FulfillmentAuthorizationInput,
};
use arcadestr_core::adp_publish_contract::{
    AdpPublishValidationViolation, AuthoritativeAdpEditMetadata, AuthoritativeAdpEditTag,
    LoadAuthoritativeAdpEditMetadataRequest, PublishAdpListingError,
    PublishAdpListingRequestExtension,
};
use arcadestr_core::adp_storage::{
    AdpProvisioning, AdpProvisioningRepository, DownloadToken, DownloadTokensRepository,
};
use arcadestr_core::archive_inspection_contract::BuildSelectionRef;
use arcadestr_core::auth::AuthState;
use arcadestr_core::authorization::{
    AuthorizationTerms, CAPABILITY_ISSUE_GRANT, CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD,
};
use arcadestr_core::distribution::{LaunchProfile, ProfileId, Sha256Digest};
use arcadestr_core::file_hash::{sha256_file, sha256_file_with_progress};
use arcadestr_core::http_client::HttpClient;
use arcadestr_core::is_replaceable_event_newer;
use arcadestr_core::lnurlp::{request_invoice, resolve_lud16_with_policy};
use arcadestr_core::marketplace::confirm_nip99_listing_propagated;
use arcadestr_core::nip46::AppSignerState;
use arcadestr_core::nwc_client::{
    get_nwc_connection, load_nwc_connection, remove_nwc_connection, save_nwc_connection, NwcClient,
    NwcClientError,
};
use arcadestr_core::signers::{ActiveSigner, NostrSigner, SignerError};
use nostr::nips::nip19::FromBech32;
use nostr::nips::nip19::ToBech32;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::archive_selection::{ArchiveSelectionService, LeaseUploadError, PublicationLease};
use crate::AppState;

async fn resolve_lud16(
    http: &dyn HttpClient,
    address: &str,
) -> Result<arcadestr_core::lnurlp::LnurlPayEndpoint, arcadestr_core::lnurlp::LnurlError> {
    // Release builds cannot opt in, even if the development variable is set.
    let allow_local = cfg!(debug_assertions)
        && std::env::var("ARCADESTR_REGTEST_ALLOW_LOOPBACK_HTTP")
            .is_ok_and(|value| value == "1" || value == "true");
    resolve_lud16_with_policy(http, address, allow_local).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentMode {
    None,
    Direct,
    Delegate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishAdpListingRequest {
    pub expected_publisher_npub: String,
    pub existing_event_id: Option<String>,
    /// Identifier of the listing being replaced. Only meaningful for edits, and
    /// even then it is validated against the fetched authoritative listing. New
    /// listings must leave this empty: the backend mints the identifier.
    pub existing_d_tag: Option<String>,
    /// Identifier minted for this publication by an earlier attempt that failed
    /// before the listing existed. Carrying it back makes the retry resume that
    /// publication — reusing its provisioning scope instead of stranding a
    /// fulfillment key on the operator for a coordinate no listing will use.
    /// Ignored once `existing_event_id` names a published listing.
    #[serde(default)]
    pub pending_d_tag: Option<String>,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<String>,
    pub fulfillment_mode: FulfillmentMode,
    pub operator_url: Option<String>,
    pub servers: Vec<String>,
    #[serde(default)]
    pub accepted_terms: Vec<arcadestr_core::adp_settlement::AcceptedTermsRef>,
    /// Legacy transport field retained for request compatibility. Publication
    /// rejects it: only a backend-issued `selected_build` may supply bytes.
    pub file_path: Option<String>,
    /// Opaque backend-selected build. Every new artifact must use it.
    #[serde(default)]
    pub selected_build: Option<BuildSelectionRef>,
    pub existing_file_hash: Option<String>,
    pub existing_fulfillment_pubkey: Option<String>,
    pub version: Option<String>,
    pub acquisition: arcadestr_core::marketplace::AcquisitionPolicy,
    pub platforms: Vec<String>,
    pub campaigns: Vec<CampaignPointerInput>,
    pub nip94_event_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveAdpOperatorRequest {
    pub publisher_npub: String,
    pub fulfillment_pubkey: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishServerUploadResult {
    pub server_url: String,
    pub status: String,
    pub error: Option<String>,
    pub upload: Option<UploadResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishAdpListingResult {
    pub operation_id: String,
    /// Event ID of the published kind:30402 listing.
    pub event_id: String,
    /// Set only when this publication minted a new identifier. Serde renders the
    /// canonical lowercase hyphenated form over IPC.
    pub game_id: Option<Uuid>,
    /// The listing `d` tag, generated or preserved.
    pub d_tag: String,
    /// `30402:<publisher-hex>:<d-tag>`, the stable game coordinate.
    pub game_coordinate: String,
    pub acceptance_event_id: Option<String>,
    pub fulfillment_pubkey: Option<String>,
    pub file_hash: Option<String>,
    pub uploads: Vec<PublishServerUploadResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishProgressPayload {
    pub operation_id: String,
    pub publisher_npub: String,
    pub game_coordinate: String,
    pub step: String,
    pub status: String,
    pub server_url: Option<String>,
    pub message: Option<String>,
    pub bytes_uploaded: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HashProgressPayload {
    pub bytes_hashed: u64,
    pub total_bytes: u64,
}

/// Identity of the game page a publish operation acts on.
///
/// Arcadestr mints a UUID v4 for every first publication and keeps it typed
/// until an event tag or coordinate needs text. Listings that already exist —
/// including externally published NIP-99 listings with human-readable `d` tags —
/// keep their original identifier verbatim, so the two cases stay distinct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GameIdentifier {
    Generated(Uuid),
    Existing(String),
}

impl GameIdentifier {
    /// Mint the identifier for a first publication.
    fn generate() -> Self {
        GameIdentifier::Generated(Uuid::new_v4())
    }

    /// Protocol boundary: the textual `d` tag.
    fn d_tag(&self) -> String {
        match self {
            GameIdentifier::Generated(game_id) => game_id.to_string(),
            GameIdentifier::Existing(d_tag) => d_tag.clone(),
        }
    }

    /// The minted UUID, when this publication created one.
    fn generated_game_id(&self) -> Option<Uuid> {
        match self {
            GameIdentifier::Generated(game_id) => Some(*game_id),
            GameIdentifier::Existing(_) => None,
        }
    }
}

/// A game coordinate held as typed parts, rendered to `30402:<pubkey>:<d>` only
/// where the protocol demands a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GameCoordinate {
    publisher: nostr::PublicKey,
    identifier: GameIdentifier,
}

impl GameCoordinate {
    fn new(publisher: nostr::PublicKey, identifier: GameIdentifier) -> Self {
        Self {
            publisher,
            identifier,
        }
    }

    fn d_tag(&self) -> String {
        self.identifier.d_tag()
    }

    fn generated_game_id(&self) -> Option<Uuid> {
        self.identifier.generated_game_id()
    }

    /// Protocol boundary: the NIP-01 replaceable-event coordinate.
    fn to_coordinate_string(&self) -> String {
        format!("30402:{}:{}", self.publisher.to_hex(), self.d_tag())
    }
}

/// Identity for a first publication. A caller-supplied identifier is rejected
/// rather than honoured: publishers do not choose game coordinates.
///
/// `pending_d_tag` is not such an identifier. It is the identity this same
/// publication minted on an earlier attempt that failed before the listing
/// existed, handed back so the retry resumes that publication instead of
/// starting a second one.
fn resolve_new_game_identifier(
    requested_d_tag: Option<&str>,
    pending_d_tag: Option<&str>,
) -> Result<GameIdentifier, String> {
    if requested_d_tag.is_some() {
        return Err(
            "a new Game page cannot be published with a caller-supplied identifier".to_string(),
        );
    }
    match pending_d_tag.filter(|value| !value.is_empty()) {
        Some(pending) => resume_pending_game_identifier(pending),
        None => Ok(GameIdentifier::generate()),
    }
}

/// Identity for a retry of a publication that never reached its listing.
///
/// Only a canonically rendered UUID v4 is accepted, which is precisely what
/// this command mints. That keeps the identifier out of the publisher's hands
/// just as firmly as minting does: a resumed publication cannot smuggle in a
/// chosen coordinate under the name of a pending one.
fn resume_pending_game_identifier(pending_d_tag: &str) -> Result<GameIdentifier, String> {
    let game_id = Uuid::try_parse(pending_d_tag)
        .ok()
        .filter(|game_id| {
            game_id.get_version() == Some(uuid::Version::Random)
                && game_id.get_variant() == uuid::Variant::RFC4122
                && game_id.to_string() == pending_d_tag
        })
        .ok_or_else(|| {
            "the identifier carried over from the previous attempt is not one this app minted"
                .to_string()
        })?;
    Ok(GameIdentifier::Generated(game_id))
}

/// Refusal shown when a resumed identity turns out to name a live Game page.
///
/// Distinct from the shape refusal in [`resume_pending_game_identifier`]: that
/// one means the carried-over token is not a resume token at all, this one
/// means it is, but the coordinate it names is no longer free.
const RESUMED_COORDINATE_IS_LIVE: &str =
    "the identifier carried over from the previous attempt already belongs to a published Game page; \
     open it from Published games and edit it instead of publishing over it";

/// What the relays report about the coordinate a retry wants to resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoordinateOccupancy {
    /// Every relay that was asked answered, and none holds a listing here.
    ConfirmedAbsent,
    /// A listing is live at this coordinate.
    Occupied,
    /// Relay coverage was incomplete, so absence cannot be concluded.
    Indeterminate,
}

/// The identity a retry may resume, given what the relays report.
///
/// Shape alone does not make a pending identifier safe to resume. A kind:30402
/// coordinate is parameterized replaceable and namespaced by publisher, so a
/// UUID that names one of this publisher's live Game pages would resume *onto*
/// it — republishing that page's coordinate with this form's content, and
/// resolving to that game's fulfillment key, which looks healthy while the
/// published listing is overwritten.
///
/// Only confirmed absence permits a resume. Minting a fresh identity is always
/// safe, so incomplete relay coverage falls back to it rather than guessing.
fn pending_d_tag_to_resume(
    pending_d_tag: &str,
    occupancy: CoordinateOccupancy,
) -> Result<Option<String>, String> {
    match occupancy {
        CoordinateOccupancy::ConfirmedAbsent => Ok(Some(pending_d_tag.to_string())),
        CoordinateOccupancy::Occupied => Err(RESUMED_COORDINATE_IS_LIVE.to_string()),
        CoordinateOccupancy::Indeterminate => Ok(None),
    }
}

/// One relay query for a coordinate, together with how completely it was
/// answered.
struct CoordinateQuery {
    events: Vec<nostr::Event>,
    /// Relays that were connected when the query began but never answered it.
    silent_relays: usize,
}

/// Reads occupancy out of a coordinate query.
///
/// A single relay holding the listing settles the question, so occupancy is
/// decided before coverage: one relay that answers "present" outranks any
/// number that stayed silent.
fn coordinate_occupancy(
    developer_pubkey: nostr::PublicKey,
    d_tag: &str,
    query: Result<CoordinateQuery, String>,
) -> CoordinateOccupancy {
    let Ok(query) = query else {
        return CoordinateOccupancy::Indeterminate;
    };
    let occupied = query.events.iter().any(|event| {
        event.kind == nostr::Kind::Custom(30402)
            && event.pubkey == developer_pubkey
            && listing_event_d_tag(event) == Some(d_tag)
    });
    if occupied {
        return CoordinateOccupancy::Occupied;
    }
    if query.silent_relays > 0 {
        return CoordinateOccupancy::Indeterminate;
    }
    CoordinateOccupancy::ConfirmedAbsent
}

/// Asks the relays whether a pending coordinate is still free, tracking which
/// relays actually answered.
///
/// Absence is only a fact about relays that replied, so the ones that stayed
/// silent are counted and handed on: [`coordinate_occupancy`] turns an
/// incompletely answered query into [`CoordinateOccupancy::Indeterminate`]
/// rather than into a claim of absence.
async fn query_coordinate(
    state: &State<'_, AppState>,
    developer_pubkey: nostr::PublicKey,
    d_tag: &str,
) -> Result<CoordinateQuery, String> {
    use std::collections::HashSet;
    use std::sync::Mutex;

    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = relay_manager.lock().await;
    let asked = manager
        .get_connected_relays()
        .await
        .into_iter()
        .filter(|relay| relay.connected)
        .map(|relay| relay.url)
        .collect::<HashSet<_>>();
    if asked.is_empty() {
        return Err("no relay was connected to answer for the coordinate".to_string());
    }

    let collected = Arc::new(Mutex::new(Vec::new()));
    let answered = Arc::new(Mutex::new(HashSet::new()));
    let events_sink = Arc::clone(&collected);
    let answered_sink = Arc::clone(&answered);
    manager
        .fetch_events_streaming(
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(30402))
                .author(developer_pubkey)
                .identifier(d_tag),
            10,
            3,
            move |relay_url, events| {
                answered_sink
                    .lock()
                    .expect("coordinate query relay set poisoned")
                    .insert(relay_url);
                events_sink
                    .lock()
                    .expect("coordinate query event sink poisoned")
                    .extend(events);
            },
        )
        .await
        .map_err(|error| format!("coordinate query failed: {error}"))?;

    let answered = answered
        .lock()
        .expect("coordinate query relay set poisoned")
        .clone();
    let events = collected
        .lock()
        .expect("coordinate query event sink poisoned")
        .clone();
    Ok(CoordinateQuery {
        events,
        silent_relays: asked.difference(&answered).count(),
    })
}

/// Gate in front of the resume path: shape first, then the coordinate itself.
///
/// Returns the identifier to resume, or `None` when this publication should
/// mint a fresh one instead.
async fn resumable_pending_d_tag(
    state: &State<'_, AppState>,
    developer_pubkey: nostr::PublicKey,
    pending_d_tag: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(pending) = pending_d_tag.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    // A token this app never minted is refused outright, and is not worth a
    // relay round trip.
    resume_pending_game_identifier(pending)?;
    let occupancy = coordinate_occupancy(
        developer_pubkey,
        pending,
        query_coordinate(state, developer_pubkey, pending).await,
    );
    pending_d_tag_to_resume(pending, occupancy)
}

/// Identity for a replacement edit. The authoritative value comes from the
/// fetched listing; any claimed value must agree with it exactly.
fn resolve_existing_game_identifier(
    requested_d_tag: Option<&str>,
    authoritative_d_tag: Option<&str>,
) -> Result<GameIdentifier, String> {
    let authoritative = authoritative_d_tag
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "the listing being replaced has no identifier".to_string())?;
    match requested_d_tag {
        Some(requested) if requested != authoritative => Err(
            "a published Game page keeps its identifier; the coordinate cannot change".to_string(),
        ),
        _ => Ok(GameIdentifier::Existing(authoritative.to_string())),
    }
}

/// Optimistic concurrency: the authoritative listing must still be the one the
/// caller intends to replace.
///
/// A retry after a partial publication uses the recovered published event ID
/// here, so a newer remote replacement fails the same way an ordinary stale
/// edit does rather than being silently overwritten.
fn ensure_expected_listing_event(
    authoritative: &nostr::Event,
    expected_event_id: &str,
    conflict_message: &str,
) -> Result<(), String> {
    if authoritative.id.to_hex() != expected_event_id {
        return Err(conflict_message.to_string());
    }
    Ok(())
}

/// The `d` tag carried by a fetched listing event.
fn listing_event_d_tag(event: &nostr::Event) -> Option<&str> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|values| values.first().map(String::as_str) == Some("d"))
        .and_then(|values| values.get(1))
        .map(String::as_str)
}

fn decode_verified_authoritative_edit_metadata(
    request: &LoadAuthoritativeAdpEditMetadataRequest,
    expected_publisher: nostr::PublicKey,
    event: &nostr::Event,
) -> Result<AuthoritativeAdpEditMetadata, PublishAdpListingError> {
    let event_id = event.id.to_hex();
    if !event.verify_id() {
        return Err(PublishAdpListingError::AuthoritativeEventIdInvalid { event_id });
    }
    if !event.verify_signature() {
        return Err(PublishAdpListingError::AuthoritativeSignatureInvalid { event_id });
    }
    if event.pubkey != expected_publisher {
        return Err(PublishAdpListingError::AuthoritativePublisherMismatch {
            expected_publisher_npub: request.expected_publisher_npub.clone(),
            actual_publisher_hex: event.pubkey.to_hex(),
        });
    }
    if event.kind.as_u16() != 30402 {
        return Err(PublishAdpListingError::AuthoritativeKindMismatch {
            expected: 30402,
            actual: event.kind.as_u16(),
        });
    }

    let actual_d_tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|values| values.first().map(String::as_str) == Some("d"))
        .filter_map(|values| values.get(1).cloned())
        .collect::<Vec<_>>();
    if actual_d_tags.len() != 1
        || actual_d_tags.first().map(String::as_str) != Some(request.d_tag.as_str())
    {
        return Err(PublishAdpListingError::AuthoritativeCoordinateMismatch {
            expected_d_tag: request.d_tag.clone(),
            actual_d_tags,
        });
    }
    if event_id != request.expected_event_id {
        return Err(PublishAdpListingError::StaleExpectedEventId {
            expected_event_id: request.expected_event_id.clone(),
            actual_event_id: event_id,
        });
    }

    Ok(decode_authoritative_launch_tier(event).into())
}

/// Computes removals from the parser's valid IDs in signed-event order.
fn removed_authoritative_profile_ids(
    metadata: &AuthoritativeAdpEditMetadata,
    incoming_profiles: &[LaunchProfile],
) -> Vec<ProfileId> {
    let AuthoritativeAdpEditMetadata::Present(metadata) = metadata else {
        return Vec::new();
    };

    metadata
        .tags
        .iter()
        .filter_map(|tag| match tag {
            AuthoritativeAdpEditTag::Launch {
                valid_profile_id, ..
            } => valid_profile_id.as_ref(),
            AuthoritativeAdpEditTag::Package { .. }
            | AuthoritativeAdpEditTag::Platform { .. }
            | AuthoritativeAdpEditTag::LaunchDefault { .. } => None,
        })
        .filter(|profile_id| {
            !incoming_profiles
                .iter()
                .any(|profile| &profile.id == *profile_id)
        })
        .fold(Vec::new(), |mut removed, profile_id| {
            if !removed.iter().any(|existing| existing == profile_id) {
                removed.push(profile_id.clone());
            }
            removed
        })
}

fn enforce_authoritative_profile_removals(
    metadata: &AuthoritativeAdpEditMetadata,
    extension: &PublishAdpListingRequestExtension,
) -> Result<(), PublishAdpListingError> {
    let incoming_profiles = extension
        .launch_tier
        .as_ref()
        .map(|tier| tier.launch_profiles.as_slice())
        .unwrap_or_default();
    let removed_profile_ids = removed_authoritative_profile_ids(metadata, incoming_profiles);
    let all_acknowledged = removed_profile_ids.iter().all(|removed| {
        extension
            .acknowledged_removed_profile_ids
            .iter()
            .any(|acknowledged| acknowledged == removed)
    });

    if !all_acknowledged {
        return Err(
            PublishAdpListingError::ProfileRemovalAcknowledgementRequired {
                removed_profile_ids,
            },
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FulfillmentMetadata {
    fulfillment_pubkey: Option<String>,
    should_provision: bool,
}

fn resolve_existing_fulfillment_metadata(
    mode: &FulfillmentMode,
    developer_pubkey: &str,
    existing_pubkey: Option<&str>,
) -> Result<FulfillmentMetadata, String> {
    match mode {
        FulfillmentMode::None => {
            if existing_pubkey.is_some() {
                return Err(
                    "existing fulfillment metadata cannot be cleared by an ordinary edit"
                        .to_string(),
                );
            }
            Ok(FulfillmentMetadata {
                fulfillment_pubkey: None,
                should_provision: false,
            })
        }
        FulfillmentMode::Direct => Ok(FulfillmentMetadata {
            fulfillment_pubkey: Some(developer_pubkey.to_string()),
            should_provision: false,
        }),
        FulfillmentMode::Delegate => match existing_pubkey {
            Some(pubkey) if pubkey == developer_pubkey => Ok(FulfillmentMetadata {
                fulfillment_pubkey: None,
                should_provision: true,
            }),
            Some(pubkey) if !pubkey.is_empty() => Ok(FulfillmentMetadata {
                fulfillment_pubkey: Some(pubkey.to_string()),
                should_provision: false,
            }),
            Some(_) => Err("existing fulfillment key cannot be empty".to_string()),
            None => Ok(FulfillmentMetadata {
                fulfillment_pubkey: None,
                should_provision: true,
            }),
        },
    }
}

struct SdkSignerAdapter(Arc<dyn nostr::signer::NostrSigner>);

/// Remote signing must not authorize a later action in a replaced session.
pub(crate) struct PublicationSigner {
    pub(crate) signer: Arc<dyn NostrSigner>,
    pub(crate) session: arcadestr_core::settlement_recovery::SessionFence,
}
impl PublicationSigner {
    async fn sign_with_timeout(
        &self,
        unsigned: nostr::UnsignedEvent,
        timeout: Duration,
    ) -> Result<nostr::Event, SignerError> {
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        if let Some(event) = crate::publisher_commands::retained_signature(&unsigned)
            .await
            .map_err(SignerError::SigningFailed)?
        {
            self.session
                .ensure_current()
                .map_err(SignerError::SigningFailed)?;
            return Ok(event);
        }
        let mut unsigned = unsigned;
        if let (Some(journal), Some(key)) = (
            crate::publisher_commands::journal(),
            crate::publisher_commands::signing_key(&unsigned),
        ) {
            let key = format!("intent:{key}");
            if let Some(saved) = journal
                .step(&key)
                .await
                .map_err(SignerError::SigningFailed)?
            {
                unsigned = serde_json::from_value(saved.intent).map_err(|_| {
                    SignerError::SigningFailed("Invalid retained signing intent".into())
                })?;
            } else {
                journal
                    .prepare(
                        &key,
                        "signing",
                        &key,
                        serde_json::to_value(&unsigned).map_err(|_| {
                            SignerError::SigningFailed("Invalid signing intent".into())
                        })?,
                    )
                    .await
                    .map_err(SignerError::SigningFailed)?;
            }
        }
        let result = tokio::time::timeout(timeout, self.signer.sign_event(unsigned))
            .await
            .map_err(|_| SignerError::SigningFailed("publication signer timed out".into()))?;
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        let event = result?;
        crate::publisher_commands::retain_signature(&event)
            .await
            .map_err(SignerError::SigningFailed)?;
        Ok(event)
    }
}
#[async_trait::async_trait]
impl NostrSigner for PublicationSigner {
    async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        let result = tokio::time::timeout(Duration::from_secs(30), self.signer.get_public_key())
            .await
            .map_err(|_| SignerError::SigningFailed("publication signer unavailable".into()))?;
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        result
    }
    async fn sign_event(
        &self,
        unsigned: nostr::UnsignedEvent,
    ) -> Result<nostr::Event, SignerError> {
        self.sign_with_timeout(unsigned, Duration::from_secs(120))
            .await
    }
}

#[async_trait::async_trait]
impl NostrSigner for SdkSignerAdapter {
    async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
        self.0
            .get_public_key()
            .await
            .map_err(|err| SignerError::SigningFailed(err.to_string()))
    }

    async fn sign_event(
        &self,
        unsigned: nostr::UnsignedEvent,
    ) -> Result<nostr::Event, SignerError> {
        self.0
            .sign_event(unsigned)
            .await
            .map_err(|err| SignerError::SigningFailed(err.to_string()))
    }
}

/// Validates the exact final listing event and transfers it unchanged to the signer.
async fn validate_and_sign_adp_listing(
    signer: &dyn NostrSigner,
    unsigned: nostr::UnsignedEvent,
) -> Result<nostr::Event, PublishAdpListingError> {
    validate_adp_listing_unsigned_event(&unsigned).map_err(PublishAdpListingError::from)?;
    signer
        .sign_event(unsigned)
        .await
        .map_err(|error| PublishAdpListingError::Signer {
            message: error.to_string(),
        })
}

pub(crate) async fn resolve_active_signer(
    signer_state: &Arc<tokio::sync::Mutex<AppSignerState>>,
    auth: &AuthState,
) -> Result<Arc<dyn NostrSigner>, String> {
    let active_client = { signer_state.lock().await.active_client.clone() };
    if let Some(client) = active_client {
        let signer = client
            .signer()
            .await
            .map_err(|err| format!("failed to access active signer: {err}"))?;
        return Ok(Arc::new(SdkSignerAdapter(signer)));
    }

    auth.signer()
        .cloned()
        .map(|signer: ActiveSigner| Arc::new(signer) as Arc<dyn NostrSigner>)
        .ok_or_else(|| "not authenticated".to_string())
}

pub(crate) async fn ensure_publish_account_current(
    state: &State<'_, AppState>,
    signer_state: &Arc<tokio::sync::Mutex<AppSignerState>>,
    expected: nostr::PublicKey,
    session: &arcadestr_core::settlement_recovery::SessionFence,
) -> Result<(), String> {
    session.ensure_current()?;
    let auth = { state.auth.lock().await.clone() };
    let current = resolve_active_signer(signer_state, &auth)
        .await?
        .get_public_key()
        .await
        .map_err(|_| "active publication account could not be verified".to_string())?;
    if current != expected {
        return Err("active account changed while publication was being prepared".into());
    }
    session.ensure_current()
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequestLnurlInvoiceRequest {
    pub lud16: String,
    pub amount_sats: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLnurlInvoiceResponse {
    pub bolt11: String,
}

#[derive(Clone, Deserialize)]
pub struct ConnectNwcWalletRequest {
    pub expected_account_pubkey: String,
    pub connection_string: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectNwcWalletResponse {
    pub wallet_pubkey: String,
    pub relays: Vec<String>,
    pub lud16: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayNwcInvoiceRequest {
    pub expected_account_pubkey: String,
    pub bolt11: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PayNwcInvoiceResponse {
    pub preimage: String,
    pub fees_paid_msat: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfirmPurchaseRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub server_url: String,
    pub bolt11: String,
    pub preimage: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfirmPurchaseResponse {
    pub receipt: nostr::Event,
    pub download_token: String,
    pub token_expires_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartSplitPurchaseRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub server_url: String,
    pub refund_lud16: String,
    pub purchase_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartSplitPurchaseResponse {
    pub quote_id: String,
    pub quote_event: nostr::Event,
    pub quote: arcadestr_core::adp_settlement::SignedQuote,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SplitQuoteRequest {
    pub server_url: String,
    pub quote_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SplitInvoiceResponse {
    pub quote_id: String,
    pub attempt_id: String,
    pub leg_id: String,
    pub amount_msat: u64,
    pub payment_hash: String,
    pub bolt11: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitSplitPaymentRequest {
    pub server_url: String,
    pub quote_id: String,
    pub attempt_id: String,
    pub preimage: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResumeSplitPaymentRequest {
    pub server_url: String,
    pub quote_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SplitSettlementResponse {
    pub state: String,
    pub settled_leg: Option<String>,
    pub receipt: Option<nostr::Event>,
    pub download_token: Option<String>,
    pub token_expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryHistoryItem {
    pub leg_id: String,
    pub state: String,
    pub principal_msat: u64,
    pub due_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettlementHistoryItem {
    pub quote_id: String,
    pub processor_url: String,
    pub listing_event_id: String,
    pub state: String,
    pub failure_at: Option<u64>,
    pub receipt: Option<nostr::Event>,
    pub recovery: Vec<RecoveryHistoryItem>,
    pub attempts: Vec<arcadestr_core::settlement_repository::RecoveryAttempt>,
    pub recovery_state: Option<String>,
    pub recovery_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplaceSplitRefundDestinationRequest {
    pub server_url: String,
    pub quote_id: String,
    pub refund_lud16: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SplitRefundRequest {
    pub server_url: String,
    pub quote_id: String,
    pub leg_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfirmSplitRefundRequest {
    pub server_url: String,
    pub quote_id: String,
    pub leg_id: String,
    pub attempt_id: String,
    pub preimage: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimEntitlementRequest {
    pub expected_account_pubkey: String,
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_event_id: String,
    pub server_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimEntitlementResponse {
    pub grant: nostr::Event,
    pub download_token: String,
    pub token_expires_at: i64,
    pub already_claimed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverCampaignsRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub pointers: Vec<CampaignPointerInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignPointerInput {
    pub root_event_id: String,
    pub relay_hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverCampaignSummariesRequest {
    pub publisher_npub: String,
    pub listings: Vec<CampaignSummaryListingInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CampaignSummaryListingInput {
    pub listing_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CampaignSummary {
    pub listing_id: String,
    pub active: usize,
    pub upcoming: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredCampaign {
    pub root_event_id: String,
    pub campaign_id: String,
    pub starts_at: u64,
    pub ends_at: u64,
    pub classification: String,
    pub event_id: Option<String>,
    pub predecessor_event_id: Option<String>,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishCampaignRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_id: String,
    pub starts_at: Option<u64>,
    pub ends_at: Option<u64>,
    pub predecessor_event_id: Option<String>,
    pub cancel: bool,
    pub update_listing_pointer: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishCampaignResponse {
    pub event_id: String,
    pub root_event_id: String,
    pub listing_event_id: Option<String>,
    pub pointer_update_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCampaignPointerRequest {
    pub publisher_npub: String,
    pub listing_id: String,
    pub campaign_root_id: String,
    pub remove: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HashBuildFileRequest {
    pub file_path: String,
}

#[tauri::command]
pub async fn check_adp_server(
    server_url: String,
    state: State<'_, AppState>,
) -> Result<AdpServerInfo, String> {
    let client = AdpClient::new(server_url, Arc::clone(&state.http_client));
    client.well_known().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn discover_adp_servers(
    state: State<'_, AppState>,
) -> Result<Vec<AdpServerAnnouncement>, String> {
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    discover_adp_servers_core(&relay_manager)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn discover_campaigns(
    request: DiscoverCampaignsRequest,
    state: State<'_, AppState>,
) -> Result<Vec<DiscoveredCampaign>, String> {
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let publisher = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    let pointers = request
        .pointers
        .into_iter()
        .filter_map(|pointer| {
            nostr::EventId::from_hex(&pointer.root_event_id)
                .ok()
                .map(
                    |root_event_id| arcadestr_core::marketplace::CampaignPointer {
                        root_event_id,
                        relay_hint: pointer.relay_hint,
                    },
                )
        })
        .collect::<Vec<_>>();
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let relay_manager = relay_manager.lock().await;
    let report = arcadestr_core::campaign_discovery::CampaignDiscoveryService::new(&relay_manager)
        .discover(&pointers, publisher, &coordinate, now_unix_i64()? as u64)
        .await
        .map_err(|error| error.to_string())?;

    Ok(report
        .campaigns
        .into_iter()
        .filter_map(|candidate| {
            let state = candidate.campaign.state_at(now_unix_i64().ok()? as u64)?;
            let tip = candidate.campaign.events.last();
            Some(DiscoveredCampaign {
                root_event_id: candidate.campaign.root_event_id.to_hex(),
                campaign_id: candidate.campaign.campaign_id,
                starts_at: state.terms.starts,
                ends_at: state.terms.ends,
                classification: match candidate.classification {
                    arcadestr_core::campaign_discovery::CampaignClassification::Upcoming => {
                        "upcoming"
                    }
                    arcadestr_core::campaign_discovery::CampaignClassification::Active => "active",
                    arcadestr_core::campaign_discovery::CampaignClassification::Ended => "ended",
                    arcadestr_core::campaign_discovery::CampaignClassification::Cancelled => {
                        "cancelled"
                    }
                    arcadestr_core::campaign_discovery::CampaignClassification::Invalid => {
                        "invalid"
                    }
                }
                .to_string(),
                event_id: tip.map(|node| node.event.id.to_hex()),
                predecessor_event_id: tip
                    .and_then(|node| node.predecessor)
                    .map(|event_id| event_id.to_hex()),
                mode: "claim".to_string(),
            })
        })
        .collect())
}

#[tauri::command]
pub async fn discover_campaign_summaries(
    request: DiscoverCampaignSummariesRequest,
    state: State<'_, AppState>,
) -> Result<Vec<CampaignSummary>, String> {
    let publisher = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let relay_manager = relay_manager.lock().await;
    let events = match relay_manager
        .fetch_events_best_effort(
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(
                    arcadestr_core::adp_protocol::ADP_CAMPAIGN_KIND,
                ))
                .author(publisher),
        )
        .await
    {
        Ok(events) => events,
        Err(arcadestr_core::relay_manager::RelayManagerError::QueryTimeout) => Vec::new(),
        Err(error) => return Err(error.to_string()),
    };
    drop(relay_manager);
    let now = now_unix_i64()? as u64;

    request
        .listings
        .into_iter()
        .map(|listing| {
            let coordinate =
                listing_coordinate_from_npub(&request.publisher_npub, &listing.listing_id)?;
            let report = arcadestr_core::campaign_discovery::resolve_campaign_candidates_report(
                &[],
                &[],
                &events,
                &events,
                publisher,
                &coordinate,
                now,
            );
            Ok(CampaignSummary {
                listing_id: listing.listing_id,
                active: report
                    .campaigns
                    .iter()
                    .filter(|item| {
                        item.classification
                            == arcadestr_core::campaign_discovery::CampaignClassification::Active
                    })
                    .count(),
                upcoming: report
                    .campaigns
                    .iter()
                    .filter(|item| {
                        item.classification
                            == arcadestr_core::campaign_discovery::CampaignClassification::Upcoming
                    })
                    .count(),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn publish_campaign(
    request: PublishCampaignRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<PublishCampaignResponse, String> {
    publish_campaign_recoverable(request, state, signer_state, None).await
}

pub(crate) async fn publish_campaign_recoverable(
    request: PublishCampaignRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    resume_id: Option<String>,
) -> Result<PublishCampaignResponse, String> {
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let journal = crate::publisher_commands::begin_simple_operation(
        &state,
        &request.publisher_npub,
        "campaign",
        coordinate,
        serde_json::to_value(&request).map_err(|_| "Invalid campaign intent")?,
        resume_id,
    )
    .await?;
    let result = crate::publisher_commands::JOURNAL
        .scope(
            journal.clone(),
            publish_campaign_inner(request, state, signer_state),
        )
        .await?;
    if result.pointer_update_error.is_none() {
        journal
            .repo
            .complete(
                &journal.publisher,
                &journal.operation_id,
                &serde_json::to_value(&result).map_err(|_| "Invalid campaign result")?,
            )
            .await?;
    }
    Ok(result)
}

async fn publish_campaign_inner(
    request: PublishCampaignRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<PublishCampaignResponse, String> {
    let session = state.settlement_recovery.session_fence()?;
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = PublicationSigner {
        signer: resolve_active_signer(signer_state.inner(), &auth_snapshot).await?,
        session: session.clone(),
    };
    let publisher = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let expected_publisher = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    if publisher != expected_publisher {
        return Err("active signer is not the listing publisher".into());
    }
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let _coordinate_lease = state
        .settlement_recovery
        .claim(&publisher.to_hex(), &format!("publish:{coordinate}"))?;
    // Confirm the managed listing exists before publishing a campaign.
    fetch_listing_event_by_coordinate(&state, &coordinate).await?;
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = relay_manager.lock().await;
    let report = arcadestr_core::campaign_discovery::CampaignDiscoveryService::new(&manager)
        .discover(&[], publisher, &coordinate, now_unix_i64()? as u64)
        .await
        .map_err(|error| error.to_string())?;
    drop(manager);
    let existing = report
        .campaigns
        .into_iter()
        .find(|candidate| candidate.campaign.campaign_id == request.campaign_id);
    let predecessor = request
        .predecessor_event_id
        .as_deref()
        .map(nostr::EventId::from_hex)
        .transpose()
        .map_err(|_| "invalid predecessor event id".to_string())?;

    let params = if request.cancel {
        let predecessor =
            predecessor.ok_or_else(|| "cancellation requires predecessor".to_string())?;
        arcadestr_core::campaign::CampaignBuildParams::cancel(
            request.campaign_id.clone(),
            coordinate.clone(),
            predecessor,
        )
    } else {
        arcadestr_core::campaign::CampaignBuildParams::active(
            request.campaign_id.clone(),
            coordinate.clone(),
            request
                .starts_at
                .ok_or_else(|| "campaign start is required".to_string())?,
            request
                .ends_at
                .ok_or_else(|| "campaign end is required".to_string())?,
            predecessor,
        )
    };
    let retained_campaign = if let Some(journal) = crate::publisher_commands::journal() {
        journal
            .step(&format!("signed:1031:{}", request.campaign_id))
            .await?
            .map(|step| serde_json::from_value::<nostr::Event>(step.intent))
            .transpose()
            .map_err(|_| "Invalid retained campaign")?
    } else {
        None
    };
    let already_seen = retained_campaign.as_ref().is_some_and(|saved| {
        existing.as_ref().is_some_and(|existing| {
            existing
                .campaign
                .events
                .iter()
                .any(|node| node.event.id == saved.id)
        })
    });
    if predecessor.is_none() && existing.is_some() && !already_seen {
        return Err("campaign id already exists".into());
    }
    let builder = arcadestr_core::campaign::build_campaign_event_builder(&params)
        .map_err(|error| error.to_string())?;
    let event = signer
        .sign_event(builder.build(publisher))
        .await
        .map_err(|error| error.to_string())?;

    let (root_event_id, mut prospective) = if let Some(existing) = existing {
        let expected_tip = existing
            .campaign
            .events
            .last()
            .map(|node| node.event.id)
            .ok_or_else(|| "campaign chain is empty".to_string())?;
        if already_seen && expected_tip != event.id {
            return Err("Campaign changed after this operation; recovery does not replay an obsolete transition".into());
        }
        if !already_seen && predecessor != Some(expected_tip) {
            return Err("predecessor is not the current campaign tip".into());
        }
        (existing.campaign.root_event_id, existing.campaign.events)
    } else {
        (event.id, Vec::new())
    };
    if !already_seen {
        prospective.push(
            arcadestr_core::campaign::parse_campaign_event(&event)
                .map_err(|error| error.to_string())?,
        );
    }
    arcadestr_core::campaign::resolve_campaign(&prospective, publisher, &coordinate)
        .map_err(|error| error.to_string())?;
    session.ensure_current()?;
    if !already_seen {
        publish_event(&state, &event, &session).await?;
    }

    let (listing_event_id, pointer_update_error) = if request.update_listing_pointer {
        let pointer_result: Result<String, String> = async {
            // Campaign publication can involve several relay and signing awaits. Merge the
            // pointer into the latest listing instead of overwriting a concurrent edit.
            let listing = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
            if let Some(retained) = crate::publisher_commands::pointer_source(&listing).await? {
                return Ok(retained.id.to_hex());
            }
            let mut tags = listing
                .tags
                .iter()
                .filter(|tag| {
                    let values = (*tag).clone().to_vec();
                    values.first().map(String::as_str) != Some("campaign")
                        || values.get(1).map(String::as_str)
                            != Some(root_event_id.to_hex().as_str())
                })
                .cloned()
                .collect::<Vec<_>>();
            if !request.cancel {
                tags.push(
                    nostr::Tag::parse(["campaign", root_event_id.to_hex().as_str()])
                        .map_err(|error| error.to_string())?,
                );
            }
            let listing_update = validate_and_sign_adp_listing(
                &signer,
                nostr::EventBuilder::new(nostr::Kind::Custom(30402), listing.content.clone())
                    .tags(tags)
                    .custom_created_at(arcadestr_core::marketplace::listing_replacement_timestamp(
                        &listing,
                        nostr::Timestamp::now(),
                    )?)
                    .build(publisher),
            )
            .await
            .map_err(|error| error.to_string())?;
            ensure_listing_revision_current(&state, &coordinate, Some(&listing)).await?;
            session.ensure_current()?;
            publish_event(&state, &listing_update, &session).await?;
            Ok(listing_update.id.to_hex())
        }
        .await;
        match pointer_result {
            Ok(event_id) => (Some(event_id), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, None)
    };

    session.ensure_current()?;
    Ok(PublishCampaignResponse {
        event_id: event.id.to_hex(),
        root_event_id: root_event_id.to_hex(),
        listing_event_id,
        pointer_update_error,
    })
}

#[tauri::command]
pub async fn update_campaign_pointer(
    request: UpdateCampaignPointerRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<String, String> {
    update_campaign_pointer_recoverable(request, state, signer_state, None).await
}

pub(crate) async fn update_campaign_pointer_recoverable(
    request: UpdateCampaignPointerRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    resume_id: Option<String>,
) -> Result<String, String> {
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let journal = crate::publisher_commands::begin_simple_operation(
        &state,
        &request.publisher_npub,
        "campaign_pointer",
        coordinate,
        serde_json::to_value(&request).map_err(|_| "Invalid campaign pointer intent")?,
        resume_id,
    )
    .await?;
    let result = crate::publisher_commands::JOURNAL
        .scope(
            journal.clone(),
            update_campaign_pointer_inner(request, state, signer_state),
        )
        .await?;
    journal
        .repo
        .complete(
            &journal.publisher,
            &journal.operation_id,
            &serde_json::json!(result),
        )
        .await?;
    Ok(result)
}

async fn update_campaign_pointer_inner(
    request: UpdateCampaignPointerRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<String, String> {
    let session = state.settlement_recovery.session_fence()?;
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = PublicationSigner {
        signer: resolve_active_signer(signer_state.inner(), &auth_snapshot).await?,
        session: session.clone(),
    };
    let publisher = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let expected = nostr::PublicKey::from_bech32(&request.publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    if publisher != expected {
        return Err("active signer is not the listing publisher".into());
    }
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let _coordinate_lease = state
        .settlement_recovery
        .claim(&publisher.to_hex(), &format!("publish:{coordinate}"))?;
    let listing = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
    if let Some(retained) = crate::publisher_commands::pointer_source(&listing).await? {
        return Ok(retained.id.to_hex());
    }
    let root = nostr::EventId::from_hex(&request.campaign_root_id)
        .map_err(|_| "invalid campaign root event id".to_string())?;
    let mut tags = listing
        .tags
        .iter()
        .filter(|tag| {
            let values = (*tag).clone().to_vec();
            values.first().map(String::as_str) != Some("campaign")
                || values.get(1).map(String::as_str) != Some(root.to_hex().as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    if !request.remove {
        tags.push(
            nostr::Tag::parse(["campaign", root.to_hex().as_str()])
                .map_err(|error| error.to_string())?,
        );
    }
    let updated = validate_and_sign_adp_listing(
        &signer,
        nostr::EventBuilder::new(nostr::Kind::Custom(30402), listing.content.clone())
            .tags(tags)
            .custom_created_at(arcadestr_core::marketplace::listing_replacement_timestamp(
                &listing,
                nostr::Timestamp::now(),
            )?)
            .build(publisher),
    )
    .await
    .map_err(|error| error.to_string())?;
    ensure_listing_revision_current(&state, &coordinate, Some(&listing)).await?;
    session.ensure_current()?;
    publish_event(&state, &updated, &session).await?;
    session.ensure_current()?;
    Ok(updated.id.to_hex())
}

#[tauri::command]
pub async fn hash_build_file(
    app: tauri::AppHandle,
    request: HashBuildFileRequest,
) -> Result<String, String> {
    sha256_file_with_progress(
        std::path::Path::new(&request.file_path),
        |bytes_hashed, total_bytes| {
            let _ = app.emit(
                "hash-progress",
                HashProgressPayload {
                    bytes_hashed,
                    total_bytes,
                },
            );
        },
    )
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn select_build_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .blocking_pick_file()
        .map(|path| path.to_string());
    Ok(path)
}

#[tauri::command]
pub async fn request_lnurl_invoice(
    request: RequestLnurlInvoiceRequest,
    state: State<'_, AppState>,
) -> Result<RequestLnurlInvoiceResponse, String> {
    arcadestr_core::purchase_contract::require_live_purchase_authorization()?;
    request_lnurl_invoice_with_http(state.http_client.as_ref(), request).await
}

// Resolve from backend authentication, never from an arbitrary frontend account key.
fn nwc_account(
    auth: &arcadestr_core::auth::AuthState,
    expected: &str,
) -> Result<nostr::PublicKey, String> {
    let expected =
        nostr::PublicKey::from_hex(expected).map_err(|_| "Invalid NWC account key.".to_string())?;
    let active = auth
        .public_key()
        .ok_or_else(|| "Sign in before managing a Lightning wallet.".to_string())?;
    if active != expected {
        return Err("Active account changed. Refresh the wallet and retry.".into());
    }
    Ok(active)
}

// Never forward wallet/relay-supplied text: it may echo credentials.
fn nwc_safe_error(error: NwcClientError) -> String {
    match error {
        NwcClientError::NotConfigured => "No NWC wallet configured. Configure one in Settings.",
        NwcClientError::Keyring(_) => {
            "NWC credential store unavailable. Unlock your OS keyring and retry."
        }
        NwcClientError::WalletTimeout => "NWC wallet did not respond before timeout.",
        NwcClientError::Relay(_) => "NWC relay connection failed.",
        NwcClientError::WalletError { code, .. } => match code.as_str() {
            "INSUFFICIENT_BALANCE" => "NWC wallet has insufficient balance.",
            "QUOTA_EXCEEDED" => "NWC connection spending budget exceeded.",
            "RATE_LIMITED" => "NWC wallet rate limit reached. Retry later.",
            "NOT_IMPLEMENTED" => "NWC wallet does not support this operation.",
            _ => "NWC wallet rejected the request. Check wallet permissions, budget, and balance.",
        },
        NwcClientError::InvalidUri
        | NwcClientError::MissingRelay
        | NwcClientError::MissingSecret
        | NwcClientError::InvalidWalletPubkey
        | NwcClientError::InvalidSecret
        | NwcClientError::InvalidRelay => "Invalid NWC connection URI or insecure relay address.",
        _ => "NWC request failed or returned an invalid response.",
    }
    .into()
}

#[tauri::command]
pub async fn get_nwc_wallet(
    expected_account_pubkey: String,
    state: State<'_, AppState>,
) -> Result<Option<ConnectNwcWalletResponse>, String> {
    let auth = state.auth.lock().await;
    let account = nwc_account(&auth, &expected_account_pubkey)?;
    get_nwc_connection(&account)
        .map(|connection| connection.as_ref().map(nwc_connection_response))
        .map_err(nwc_safe_error)
}

#[tauri::command]
pub async fn disconnect_nwc_wallet(
    expected_account_pubkey: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let auth = state.auth.lock().await;
    let account = nwc_account(&auth, &expected_account_pubkey)?;
    remove_nwc_connection(&account).map_err(nwc_safe_error)
}

#[tauri::command]
pub async fn verify_nwc_wallet(
    expected_account_pubkey: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let connection = {
        let auth = state.auth.lock().await;
        let account = nwc_account(&auth, &expected_account_pubkey)?;
        load_nwc_connection(&account).map_err(nwc_safe_error)?
    };
    match NwcClient::new(connection)
        .with_timeout(std::time::Duration::from_secs(10))
        .verify()
        .await
    {
        Ok(()) => Ok("verified".into()),
        Err(NwcClientError::VerificationUnsupported) => Ok("unsupported".into()),
        Err(error) => Err(nwc_safe_error(error)),
    }
}

#[tauri::command]
pub async fn connect_nwc_wallet(
    request: ConnectNwcWalletRequest,
    state: State<'_, AppState>,
) -> Result<ConnectNwcWalletResponse, String> {
    let auth = state.auth.lock().await;
    let account = nwc_account(&auth, &request.expected_account_pubkey)?;
    let connection =
        save_nwc_connection(&account, &request.connection_string).map_err(nwc_safe_error)?;
    Ok(nwc_connection_response(&connection))
}

fn nwc_connection_response(
    connection: &arcadestr_core::nwc_client::NwcConnection,
) -> ConnectNwcWalletResponse {
    ConnectNwcWalletResponse {
        wallet_pubkey: connection.wallet_pubkey_hex().to_string(),
        relays: connection.safe_relay_metadata(),
        lud16: connection.lud16().map(ToOwned::to_owned),
    }
}

#[tauri::command]
pub async fn pay_nwc_invoice(
    request: PayNwcInvoiceRequest,
    state: State<'_, AppState>,
) -> Result<PayNwcInvoiceResponse, String> {
    arcadestr_core::purchase_contract::require_live_purchase_authorization()?;
    let auth = { state.auth.lock().await.clone() };
    let initiating_account = nwc_account(&auth, &request.expected_account_pubkey)?;
    let connection = { load_nwc_connection(&initiating_account).map_err(nwc_safe_error)? };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let account = signer
        .get_public_key()
        .await
        .map_err(|_| "payment account unavailable".to_string())?;
    if account != initiating_account {
        return Err("payment signer/account mismatch".into());
    }
    ensure_purchase_account_current(&state, account).await?;
    // Raw invoices are not purchase authorization. Only an original persisted
    // attempt can enter the app-owned plan executor; there is no legacy bypass.
    let parsed = request
        .bolt11
        .parse::<lightning_invoice::Bolt11Invoice>()
        .map_err(|_| "Invalid payment invoice")?;
    let (quote_id,attempt_id,encrypted):(String,String,Vec<u8>)=sqlx::query_as(
        "SELECT quote_id,attempt_id,encrypted_attempt FROM settlement_invoice_attempts WHERE account_pubkey=? AND payment_hash=?")
        .bind(account.to_hex()).bind(parsed.payment_hash().to_string()).fetch_optional(state.database.pool()).await
        .map_err(|_|"Purchase checkpoint unavailable")?.ok_or("An immutable approved purchase is required; raw-invoice NWC dispatch is disabled")?;
    let text = signer
        .nip44_decrypt(
            &account,
            std::str::from_utf8(&encrypted).map_err(|_| "Invalid invoice checkpoint")?,
        )
        .await
        .map_err(|_| "Invoice checkpoint locked")?;
    let original: arcadestr_core::adp_client::SettlementInvoiceResponse =
        serde_json::from_str(&text).map_err(|_| "Invalid invoice checkpoint")?;
    if original.bolt11 != request.bolt11 {
        return Err("Invoice differs from the original purchase checkpoint".into());
    }
    let stored = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    )
    .load(&account.to_hex(), &quote_id)
    .await
    .map_err(|_| "Purchase checkpoint unavailable")?
    .ok_or("Purchase checkpoint missing")?;
    let lease = crate::settlement_ui::claim(
        &state,
        account,
        &format!("purchase:{}:{}", stored.processor_url, quote_id),
    )
    .await?;
    let result =
        arcadestr_core::purchase_execution::PurchaseExecutor::new(state.database.pool().clone())
            .pay_leg_guarded(
                signer,
                &NwcClient::new(connection),
                &quote_id,
                &attempt_id,
                &|| lease.ensure_current(),
            )
            .await
            .map_err(|error| error.to_string())?;
    // Always persist the initiating account's result before checking an account switch.
    ensure_purchase_account_current(&state, account).await?;
    Ok(PayNwcInvoiceResponse {
        preimage: result.preimage,
        fees_paid_msat: result.fees_paid_msat,
    })
}

/// Non-spending review of the original plan. Optional get_info is the only
/// wallet request here; neither this command nor approval changes permissions.
#[tauri::command]
pub async fn get_purchase_plan(
    request: arcadestr_core::purchase_contract::PurchasePlanRequest,
    state: State<'_, AppState>,
) -> Result<arcadestr_core::purchase_contract::PurchasePlanReview, String> {
    use arcadestr_core::{durable_payments::PaymentWallet, purchase_execution::PurchaseExecutor};
    let auth = { state.auth.lock().await.clone() };
    let buyer = nwc_account(&auth, &request.expected_account_pubkey)?;
    let signer = auth.signer().ok_or("Connect/unlock the original buyer")?;
    let wallet = load_nwc_connection(&buyer).ok().map(|connection| {
        NwcClient::new(connection).with_timeout(std::time::Duration::from_secs(3))
    });
    let executor = PurchaseExecutor::new(state.database.pool().clone());
    let mut review = executor
        .review(
            signer,
            wallet.as_ref().map(|w| w as &dyn PaymentWallet),
            &request.quote_id,
        )
        .await
        .map_err(|e| e.to_string())?;
    if !review.terms_verified {
        if let Ok(listing) =
            fetch_listing_event_by_coordinate(&state, &review.quote.listing_coordinate).await
        {
            if listing.id.to_hex() == review.quote.listing_event_id {
                if let Ok(terms) = fetch_terms_events(&state, &listing).await {
                    if executor
                        .verify_sources(signer, &request.quote_id, &listing, &terms)
                        .await
                        .is_ok()
                    {
                        review = executor
                            .review(
                                signer,
                                wallet.as_ref().map(|w| w as &dyn PaymentWallet),
                                &request.quote_id,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    if let Some(wallet) = &wallet {
        review.wallet_capabilities = wallet.capabilities().await.ok();
    }
    ensure_purchase_account_current(&state, buyer).await?;
    Ok(review)
}

#[tauri::command]
pub async fn approve_purchase_plan(
    request: arcadestr_core::purchase_contract::ApprovePurchasePlanRequest,
    state: State<'_, AppState>,
) -> Result<arcadestr_core::purchase_contract::PurchasePlanReview, String> {
    if !request.confirm_complete_plan {
        return Err("Explicit approval of the complete settlement plan is required".into());
    }
    let auth = { state.auth.lock().await.clone() };
    let buyer = nwc_account(&auth, &request.expected_account_pubkey)?;
    let signer = auth.signer().ok_or("Connect/unlock the original buyer")?;
    let wallet = NwcClient::new(load_nwc_connection(&buyer).map_err(nwc_safe_error)?);
    let lease =
        crate::settlement_ui::claim(&state, buyer, &format!("approve:{}", request.quote_id))
            .await?;
    let executor =
        arcadestr_core::purchase_execution::PurchaseExecutor::new(state.database.pool().clone());
    let review = executor
        .review(signer, Some(&wallet), &request.quote_id)
        .await
        .map_err(|e| e.to_string())?;
    lease.ensure_current()?;
    executor
        .approve_verified_with_fee_policy(
            signer,
            &wallet,
            &request.quote_id,
            &request.quote_event_id,
            &request.commitment,
            arcadestr_core::purchase_execution::PurchaseLimits {
                principal_msat: review.quote.gross_msat,
                fee_reserve_msat: request.fee_reserve_msat,
                total_msat: request.total_msat,
                maximum_dispatches: review.quote.legs.len(),
            },
            request.network,
            request.fee_policy,
            request.accept_unenforced_fee_risk,
            &|| lease.ensure_current(),
        )
        .await
        .map_err(|e| e.to_string())?;
    lease.ensure_current()?;
    ensure_purchase_account_current(&state, buyer).await?;
    get_purchase_plan(
        arcadestr_core::purchase_contract::PurchasePlanRequest {
            expected_account_pubkey: request.expected_account_pubkey,
            quote_id: request.quote_id,
        },
        state,
    )
    .await
}

#[tauri::command]
pub async fn confirm_purchase(
    request: ConfirmPurchaseRequest,
    state: State<'_, AppState>,
) -> Result<ConfirmPurchaseResponse, String> {
    let auth_snapshot = { state.auth.lock().await.clone() };
    let signer = auth_snapshot
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer_pubkey = signer
        .get_public_key()
        .await
        .map_err(|err| err.to_string())?
        .to_hex();

    let game_coordinate =
        listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let listing_event = fetch_listing_event_by_coordinate(&state, &game_coordinate).await?;
    let listing_for_validation = listing_event.clone();
    let bolt11 = request.bolt11.clone();
    let preimage = request.preimage.clone();
    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let response = client
        .purchase_confirm(
            signer,
            CorePurchaseConfirmRequest {
                game_coordinate: game_coordinate.clone(),
                listing_event,
                zap_receipt_event: None,
                bolt11: Some(request.bolt11),
                preimage: Some(request.preimage),
                quote_id: None,
            },
        )
        .await
        .map_err(|err| err.to_string())?;
    persist_purchase_confirmation(
        &state,
        &buyer_pubkey,
        &game_coordinate,
        &request.server_url,
        response,
        &listing_for_validation,
        &bolt11,
        &preimage,
    )
    .await
}

async fn ensure_purchase_account_current(
    state: &State<'_, AppState>,
    expected: nostr::PublicKey,
) -> Result<(), String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let current = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    if current != expected {
        return Err("active account changed while settlement was in progress".into());
    }
    Ok(())
}

/// Read-only publisher review. No invoice, quote, or signing request is made.
#[tauri::command]
pub async fn preview_publisher_terms(
    request: arcadestr_core::adp_settlement::PublisherTermsContext,
    state: State<'_, AppState>,
) -> Result<arcadestr_core::adp_settlement::PublisherTermsReview, String> {
    let session = state.settlement_recovery.session_fence()?;
    let publisher = { state.auth.lock().await.public_key() }
        .ok_or("Authenticate as the publisher before reviewing commercial terms")?;
    if request.publisher_pubkey != publisher.to_hex() {
        return Err("Commercial review belongs to another publisher".into());
    }
    let review = load_publisher_terms_review(&state, &request, None).await?;
    session.ensure_current()?;
    Ok(review)
}

async fn load_publisher_terms_review(
    state: &State<'_, AppState>,
    context: &arcadestr_core::adp_settlement::PublisherTermsContext,
    preserved: Option<&[arcadestr_core::adp_settlement::AcceptedTermsRef]>,
) -> Result<arcadestr_core::adp_settlement::PublisherTermsReview, String> {
    use arcadestr_core::adp_settlement::{AcceptedTermsRef, TERMS_KIND};
    let mut servers = Vec::new();
    let mut advertised = std::collections::BTreeMap::new();
    for url in &context.servers {
        let info = AdpClient::new(url.clone(), Arc::clone(&state.http_client))
            .well_known()
            .await
            .map_err(|_| "Commercial endpoint capabilities are unavailable")?;
        let server = publisher_terms_server(url, info)?;
        if preserved.is_none() {
            let root = server
                .advertised_terms
                .clone()
                .ok_or("Operator terms root is unavailable")?;
            if advertised
                .insert(server.operator_pubkey.clone(), root.clone())
                .is_some_and(|previous| previous != root)
            {
                return Err(
                    "Operator endpoints advertise conflicting terms; review is unavailable".into(),
                );
            }
        }
        servers.push(server);
    }
    let references = preserved.map(<[_]>::to_vec).unwrap_or_else(|| {
        advertised
            .into_iter()
            .map(|(operator_pubkey, terms_event_id)| AcceptedTermsRef {
                operator_pubkey,
                terms_event_id,
            })
            .collect()
    });
    let manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = { manager.lock().await.clone() };
    let mut events = Vec::new();
    let now = nostr::Timestamp::now();
    for reference in &references {
        let id = nostr::EventId::from_hex(&reference.terms_event_id)
            .map_err(|_| "Invalid commercial terms root")?;
        let author = nostr::PublicKey::from_hex(&reference.operator_pubkey)
            .map_err(|_| "Invalid terms operator")?;
        events.extend(
            fetch_publisher_commercial_events(&manager, nostr::Filter::new().id(id)).await?,
        );
        // Withdrawals use JSON content, not an indexed root tag. Query the existing
        // kind/author history; refuse saturated results rather than claiming absence.
        let history = fetch_publisher_commercial_events(
            &manager,
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(TERMS_KIND))
                .author(author)
                .until(now)
                .limit(513),
        )
        .await?;
        if history.len() >= 512 {
            return Err("Terms history query is incomplete; publication is blocked".into());
        }
        events.extend(history);
    }
    arcadestr_core::adp_settlement::review_publisher_terms(
        context,
        &servers,
        &references,
        &events,
        preserved.is_none(),
        now.as_secs(),
    )
    .map_err(|error| error.to_string())
}

fn publisher_terms_server(
    url: &str,
    info: AdpServerInfo,
) -> Result<arcadestr_core::adp_settlement::PublisherTermsServer, String> {
    use arcadestr_core::adp_settlement::{
        PublisherTermsServer, QUOTE_KIND, RECOVERY_KIND, SETTLEMENT_VERSION, TERMS_KIND,
    };
    let settlement = info
        .settlement
        .ok_or("Operator does not advertise settlement support")?;
    let kinds = info
        .event_kinds
        .ok_or("Operator settlement event kinds are unavailable")?;
    if settlement.version != SETTLEMENT_VERSION
        || !settlement.split_enabled
        || kinds.commercial_terms != Some(TERMS_KIND)
        || kinds.private_quote != Some(QUOTE_KIND)
        || kinds.recovery_evidence != Some(RECOVERY_KIND)
    {
        return Err("Operator advertises incompatible settlement capabilities".into());
    }
    if nostr::PublicKey::from_hex(&info.pubkey)
        .map_err(|_| "Invalid operator identity")?
        .to_hex()
        != info.pubkey
    {
        return Err("Noncanonical operator identity".into());
    }
    Ok(PublisherTermsServer {
        url: url.into(),
        operator_pubkey: info.pubkey,
        version: settlement.version,
        split_enabled: settlement.split_enabled,
        advertised_terms: settlement.terms_event_id,
    })
}

async fn fetch_publisher_commercial_events(
    manager: &arcadestr_core::relay_manager::RelayManager,
    filter: nostr::Filter,
) -> Result<Vec<nostr::Event>, String> {
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    // Reuse the existing EOSE-aware transport. Unlike SDK aggregate fetches,
    // incomplete selected-relay replies must not establish withdrawal absence.
    manager
        .fetch_events_streaming(filter, 10, 10, move |_, batch| {
            if let Ok(mut sink) = sink.lock() {
                sink.extend(batch);
            }
        })
        .await
        .map_err(|_| {
            "Commercial evidence query is incomplete or unavailable; retry without changing consent"
        })?;
    let mut events = events
        .lock()
        .map_err(|_| "Commercial evidence collection failed")?;
    Ok(std::mem::take(&mut *events))
}

async fn validate_publisher_commercial_intent(
    state: &State<'_, AppState>,
    request: &PublishAdpListingRequest,
    extension: &PublishAdpListingRequestExtension,
    source: Option<&nostr::Event>,
) -> Result<Vec<arcadestr_core::adp_settlement::AcceptedTermsRef>, String> {
    use arcadestr_core::adp_settlement::{
        publisher_commercial_references, PublisherCommercialIntent, PublisherTermsContext,
    };
    if !request.accepted_terms.is_empty() {
        return Err(
            "Unreviewed commercial acceptance is not supported; review exact signed terms first"
                .into(),
        );
    }
    let context = PublisherTermsContext {
        publisher_pubkey: nostr::PublicKey::from_bech32(&request.expected_publisher_npub)
            .map_err(|_| "Invalid publisher")?
            .to_hex(),
        servers: request.servers.clone(),
        price_sats: request.price_sats,
        developer_destination: request.lud16.clone().unwrap_or_default(),
    };
    match &extension.commercial_intent {
        PublisherCommercialIntent::Preserve => {
            let accepted =
                publisher_commercial_references(&extension.commercial_intent, source, None)
                    .map_err(|e| e.to_string())?;
            if !accepted.is_empty() {
                load_publisher_terms_review(state, &context, Some(&accepted)).await?;
            }
            Ok(accepted)
        }
        PublisherCommercialIntent::ReplaceAfterReview { review } => {
            let fresh = load_publisher_terms_review(state, &context, None).await?;
            let _ = review;
            publisher_commercial_references(&extension.commercial_intent, source, Some(&fresh))
                .map_err(|e| e.to_string())
        }
        PublisherCommercialIntent::Retire => {
            publisher_commercial_references(&extension.commercial_intent, source, None)
                .map_err(|e| e.to_string())
        }
    }
}

async fn fetch_terms_events(
    state: &State<'_, AppState>,
    listing: &nostr::Event,
) -> Result<Vec<nostr::Event>, String> {
    let mut ids = Vec::new();
    let mut operators = BTreeSet::new();
    for tag in listing.tags.iter() {
        let values = tag.clone().to_vec();
        if values.first().is_none_or(|name| name != "adp_terms") {
            continue;
        }
        let [_, operator, event_id] = values.as_slice() else {
            return Err("listing contains a malformed adp_terms reference".into());
        };
        if !operators.insert(operator.clone()) {
            return Err("listing accepts conflicting terms for one operator".into());
        }
        ids.push(
            nostr::EventId::from_hex(event_id).map_err(|_| "invalid terms event id".to_string())?,
        );
    }
    if ids.is_empty() {
        return Err("listing does not opt into split settlement".into());
    }
    let relay_client = {
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let manager = relay_manager.lock().await;
        manager.get_client_arc()
    };
    let timeout = std::time::Duration::from_secs(10);
    let mut events = Vec::new();
    for id in ids {
        let fetched = tokio::time::timeout(
            timeout,
            relay_client.fetch_events(nostr::Filter::new().id(id), timeout),
        )
        .await
        .map_err(|_| "terms lookup timed out".to_string())?
        .map_err(|error| error.to_string())?;
        let event = fetched
            .into_iter()
            .find(|event| event.id == id)
            .ok_or_else(|| format!("accepted terms event {id} is unavailable"))?;
        events.push(event);
    }
    Ok(events)
}

#[tauri::command]
pub async fn start_split_purchase(
    request: StartSplitPurchaseRequest,
    state: State<'_, AppState>,
) -> Result<StartSplitPurchaseResponse, String> {
    arcadestr_core::purchase_contract::require_live_purchase_authorization()?;
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let coordinate = listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let listing = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
    let terms_events = fetch_terms_events(&state, &listing).await?;
    if !listing.tags.iter().any(|tag| {
        let v = tag.as_slice();
        v.first().map(String::as_str) == Some("server")
            && v.get(1).is_some_and(|url| {
                url.trim_end_matches('/') == request.server_url.trim_end_matches('/')
            })
    }) {
        return Err("Processor URL is not authorized by the signed listing".into());
    }
    let repository = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    for existing in repository
        .list_for_account(&buyer.to_hex())
        .await
        .map_err(|_| "Stored settlements unavailable")?
    {
        if existing.state == "settled" && existing.receipt_event_json.is_some() {
            continue;
        }
        if existing.processor_url.trim_end_matches('/') != request.server_url.trim_end_matches('/')
        {
            continue;
        }
        let event: nostr::Event = serde_json::from_str(&existing.quote_event_json)
            .map_err(|_| "Stored quote is invalid; review recovery")?;
        let text = signer
            .nip44_decrypt(&event.pubkey, &event.content)
            .await
            .map_err(|_| "Stored purchase is locked; review recovery")?;
        let quote = arcadestr_core::adp_settlement::verify_quote_event(&event, &text)
            .map_err(|_| "Stored quote failed validation")?;
        if quote.listing_coordinate == coordinate
            && request.purchase_id.as_deref() != Some(&existing.quote_id)
        {
            return Err(format!("Existing purchase {} must be recovered or reviewed first. No new purchase was created.",existing.quote_id));
        }
    }
    let endpoint = resolve_lud16(state.http_client.as_ref(), &request.refund_lud16)
        .await
        .map_err(|error| error.to_string())?;
    let purchase_id = request
        .purchase_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let refund = arcadestr_core::adp_settlement::RefundDestination {
        endpoint: request.refund_lud16.clone(),
        min_sendable_msat: endpoint.min_sendable_msat,
        max_sendable_msat: endpoint.max_sendable_msat,
    };
    let refund_commitment = arcadestr_core::adp_settlement::destination_commitment(&refund)
        .map_err(|e| e.to_string())?;
    let response = client
        .create_settlement_quote(
            signer,
            CoreCreateSettlementQuoteRequest {
                purchase_id: purchase_id.clone(),
                listing_event: listing.clone(),
                terms_events: terms_events.clone(),
                refund_destination: refund,
                zap_requests: std::collections::HashMap::new(),
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    let plaintext = signer
        .nip44_decrypt(&response.quote.pubkey, &response.quote.content)
        .await
        .map_err(|error| format!("quote decryption failed: {error}"))?;
    let quote = arcadestr_core::adp_settlement::verify_quote_event(&response.quote, &plaintext)
        .map_err(|error| error.to_string())?;
    if quote.quote_id != purchase_id
        || quote.buyer_pubkey != buyer.to_hex()
        || quote.listing_event_id != listing.id.to_hex()
        || quote.state != arcadestr_core::adp_settlement::QuoteState::Active
        || quote.refund_destination_commitment != refund_commitment
    {
        return Err("processor quote does not match purchase request".into());
    }
    arcadestr_core::adp_settlement::verify_quote_listing_terms(&quote, &listing, &terms_events)
        .map_err(|_| "Processor quote differs from developer-accepted terms")?;
    ensure_purchase_account_current(&state, buyer).await?;
    let repository = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    repository
        .save_quote(
            &buyer.to_hex(),
            &request.server_url,
            &serde_json::to_string(&response.quote).map_err(|error| error.to_string())?,
            &quote,
            u64::try_from(now_unix_i64()?).map_err(|_| "system clock before epoch".to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    arcadestr_core::purchase_execution::PurchaseExecutor::new(state.database.pool().clone())
        .verify_sources(signer, &purchase_id, &listing, &terms_events)
        .await
        .map_err(|error| error.to_string())?;
    Ok(StartSplitPurchaseResponse {
        quote_id: purchase_id,
        quote_event: response.quote,
        quote,
    })
}

#[tauri::command]
pub async fn next_split_invoice(
    request: SplitQuoteRequest,
    state: State<'_, AppState>,
) -> Result<SplitInvoiceResponse, String> {
    arcadestr_core::purchase_contract::require_live_purchase_authorization()?;
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let repository = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    let stored = repository
        .load(&buyer.to_hex(), &request.quote_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "settlement is not stored for this account".to_string())?;
    if !arcadestr_core::settlement_repository::account_can_continue(
        &stored.account_pubkey,
        Some(&buyer.to_hex()),
    ) {
        return Err("active account cannot continue this settlement".into());
    }
    if stored.processor_url.trim_end_matches('/') != request.server_url.trim_end_matches('/')
        || stored.state != "active"
    {
        return Err(
            "Stored settlement is not payable at this server; use recovery/status instead".into(),
        );
    }
    if repository
        .has_unreconciled_payment(&buyer.to_hex(), &request.quote_id)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("A prior payment must be reconciled and its proof submitted before requesting another invoice".into());
    }
    let client = AdpClient::new(request.server_url, Arc::clone(&state.http_client));
    let invoice = client
        .settlement_next_invoice(signer, &request.quote_id)
        .await
        .map_err(|error| error.to_string())?;
    let event: nostr::Event =
        serde_json::from_str(&stored.quote_event_json).map_err(|_| "Invalid original quote")?;
    let text = signer
        .nip44_decrypt(&event.pubkey, &event.content)
        .await
        .map_err(|_| "Original quote locked")?;
    let quote = arcadestr_core::adp_settlement::verify_quote_event(&event, &text)
        .map_err(|_| "Invalid original quote")?;
    arcadestr_core::adp_settlement::verify_original_invoice(&quote, &invoice)
        .map_err(|_| "Returned invoice does not bind the original plan")?;
    ensure_purchase_account_current(&state, buyer).await?;
    let encrypted = signer
        .nip44_encrypt(
            &buyer,
            &serde_json::to_string(&invoice).map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| format!("invoice persistence encryption failed: {error}"))?;
    repository
        .record_invoice_attempt(
            &buyer.to_hex(),
            &request.quote_id,
            &invoice.leg_id,
            &invoice.attempt_id,
            encrypted.as_bytes(),
            &invoice.payment_hash,
            "disclosed",
            u64::try_from(now_unix_i64()?).map_err(|_| "system clock before epoch".to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(SplitInvoiceResponse {
        quote_id: invoice.quote_id,
        attempt_id: invoice.attempt_id,
        leg_id: invoice.leg_id,
        amount_msat: invoice.amount_msat,
        payment_hash: invoice.payment_hash,
        bolt11: invoice.bolt11,
        expires_at: invoice.expires_at,
    })
}

#[tauri::command]
pub async fn submit_split_payment(
    request: SubmitSplitPaymentRequest,
    state: State<'_, AppState>,
) -> Result<SplitSettlementResponse, String> {
    let buyer = state
        .auth
        .lock()
        .await
        .public_key()
        .ok_or("not authenticated")?;
    let lease = crate::settlement_ui::claim(
        &state,
        buyer,
        &format!("purchase:{}:{}", request.server_url, request.quote_id),
    )
    .await?;
    submit_split_payment_inner(request, state, &lease).await
}

pub(crate) async fn submit_split_payment_inner(
    request: SubmitSplitPaymentRequest,
    state: State<'_, AppState>,
    lease: &arcadestr_core::settlement_recovery::RecoveryLease,
) -> Result<SplitSettlementResponse, String> {
    lease.ensure_current()?;
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let repository = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    let stored = repository
        .load(&buyer.to_hex(), &request.quote_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "settlement is not stored for this account".to_string())?;
    if !arcadestr_core::settlement_repository::account_can_continue(
        &stored.account_pubkey,
        Some(&buyer.to_hex()),
    ) {
        return Err("active account cannot continue this settlement".into());
    }
    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    if request.server_url.trim_end_matches('/') != stored.processor_url.trim_end_matches('/') {
        return Err("settlement server does not match the stored quote".into());
    }
    let attempt = repository
        .invoice_attempt(&buyer.to_hex(), &request.quote_id, &request.attempt_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "invoice attempt is not stored for this quote/account".to_string())?;
    let encrypted_invoice = std::str::from_utf8(&attempt.encrypted_attempt)
        .map_err(|_| "invalid stored invoice".to_string())?;
    let plaintext = signer
        .nip44_decrypt(&buyer, encrypted_invoice)
        .await
        .map_err(|_| "invoice decryption failed".to_string())?;
    let invoice: arcadestr_core::adp_client::SettlementInvoiceResponse =
        serde_json::from_str(&plaintext).map_err(|_| "invalid stored invoice".to_string())?;
    if invoice.quote_id != request.quote_id
        || invoice.attempt_id != request.attempt_id
        || invoice.payment_hash != attempt.payment_hash
    {
        return Err("stored invoice binding mismatch".into());
    }
    arcadestr_core::durable_payments::validate_payment_proof(
        &invoice.bolt11,
        &arcadestr_core::nwc_client::PayInvoiceResult {
            preimage: request.preimage.clone(),
            fees_paid_msat: None,
        },
    )
    .map_err(|e| e.to_string())?;
    let encrypted_proof = signer
        .nip44_encrypt(&buyer, &request.preimage)
        .await
        .map_err(|_| "proof encryption failed".to_string())?;
    if !repository
        .checkpoint_proof(
            &buyer.to_hex(),
            &request.quote_id,
            &request.attempt_id,
            encrypted_proof.as_bytes(),
        )
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("payment proof could not be durably stored".into());
    }
    ensure_purchase_account_current(&state, buyer).await?;
    lease.ensure_current()?;
    let response = client
        .submit_settlement_proof(
            signer,
            &request.quote_id,
            SettlementProofRequest {
                attempt_id: request.attempt_id.clone(),
                preimage: request.preimage,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    lease.ensure_current()?;
    ensure_purchase_account_current(&state, buyer).await?;
    repository
        .update_invoice_state(
            &buyer.to_hex(),
            &request.attempt_id,
            "settled",
            u64::try_from(now_unix_i64()?).map_err(|_| "system clock before epoch".to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    response
        .validate_completion_fields()
        .map_err(|error| error.to_string())?;
    if let Some(receipt) = &response.receipt {
        persist_split_receipt(
            &state,
            signer,
            buyer,
            &stored,
            receipt,
            response
                .download_token
                .as_deref()
                .ok_or_else(|| "settled response omitted download token".to_string())?,
            response
                .token_expires_at
                .ok_or_else(|| "settled response omitted token expiry".to_string())?,
        )
        .await?;
        repository
            .mark_receipt(
                &buyer.to_hex(),
                &request.quote_id,
                &serde_json::to_string(receipt).map_err(|error| error.to_string())?,
                u64::try_from(now_unix_i64()?)
                    .map_err(|_| "system clock before epoch".to_string())?,
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(SplitSettlementResponse {
        state: response.state,
        settled_leg: response.settled_leg,
        receipt: response.receipt,
        download_token: response.download_token,
        token_expires_at: response.token_expires_at,
    })
}

/// Resume an existing attempt only. Never request a replacement invoice or dispatch a payment.
#[tauri::command]
pub async fn resume_split_payment(
    request: ResumeSplitPaymentRequest,
    state: State<'_, AppState>,
) -> Result<SplitSettlementResponse, String> {
    resume_split_payment_for_account(request, state, None).await
}
pub(crate) async fn resume_split_payment_for_account(
    request: ResumeSplitPaymentRequest,
    state: State<'_, AppState>,
    expected: Option<nostr::PublicKey>,
) -> Result<SplitSettlementResponse, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|_| "account unavailable".to_string())?;
    if expected.is_some_and(|p| p != buyer) {
        return Err("Account changed; startup recovery paused".into());
    }
    let lease = crate::settlement_ui::claim(
        &state,
        buyer,
        &format!("purchase:{}:{}", request.server_url, request.quote_id),
    )
    .await?;
    let repository = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    let stored = repository
        .load(&buyer.to_hex(), &request.quote_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "settlement not stored for this account".to_string())?;
    if request.server_url.trim_end_matches('/') != stored.processor_url.trim_end_matches('/') {
        return Err("settlement server does not match the stored quote".into());
    }
    let wallet = load_nwc_connection(&buyer).ok().map(NwcClient::new);
    let admitted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM purchase_execution_admissions WHERE account_pubkey=? AND quote_id=? AND attempt_id=?)")
        .bind(buyer.to_hex()).bind(&request.quote_id).bind(&request.attempt_id).fetch_one(state.database.pool()).await.map_err(|_|"Purchase admission checkpoint unavailable")?;
    if admitted {
        if let Some(wallet) = &wallet {
            arcadestr_core::purchase_execution::PurchaseExecutor::new(
                state.database.pool().clone(),
            )
            .reconcile_leg(signer, wallet, &request.quote_id, &request.attempt_id)
            .await
            .map_err(|e| e.to_string())?;
            lease.ensure_current()?;
        }
    }
    let (_, preimage) = arcadestr_core::settlement_recovery::existing_proof(
        &repository,
        &arcadestr_core::durable_payments::DurableNwcPayments::new(state.database.pool().clone()),
        signer,
        wallet
            .as_ref()
            .map(|w| w as &dyn arcadestr_core::durable_payments::PaymentWallet),
        &request.quote_id,
        &request.attempt_id,
    )
    .await?;
    lease.ensure_current()?;
    submit_split_payment_inner(
        SubmitSplitPaymentRequest {
            server_url: stored.processor_url,
            quote_id: request.quote_id,
            attempt_id: request.attempt_id,
            preimage,
        },
        state,
        &lease,
    )
    .await
}

#[tauri::command]
pub async fn settlement_history(
    state: State<'_, AppState>,
) -> Result<Vec<SettlementHistoryItem>, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let account = buyer.to_hex();
    let repository = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    let settlements = repository
        .list_for_account(&account)
        .await
        .map_err(|error| error.to_string())?;
    let mut history = Vec::with_capacity(settlements.len());
    for item in settlements {
        let attempts = repository
            .attempts_for_quote(&account, &item.quote_id)
            .await
            .map_err(|_| "Stored attempts unavailable")?;
        let progress:Option<(String,String)>=sqlx::query_as("SELECT state,message FROM client_settlement_progress WHERE account_pubkey=? AND processor_url=? AND quote_id=? ORDER BY updated_at DESC LIMIT 1").bind(&account).bind(&item.processor_url).bind(&item.quote_id).fetch_optional(state.database.pool()).await.map_err(|_|"Stored recovery progress unavailable")?;
        let recovery = repository
            .list_recovery_for_quote(&account, &item.quote_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|obligation| RecoveryHistoryItem {
                leg_id: obligation.leg_id,
                state: obligation.state,
                principal_msat: obligation.principal_msat,
                due_at: obligation.due_at,
            })
            .collect();
        history.push(SettlementHistoryItem {
            attempts,
            recovery_state: progress.as_ref().map(|p| p.0.clone()),
            recovery_message: progress.map(|p| p.1),
            quote_id: item.quote_id,
            processor_url: item.processor_url,
            listing_event_id: item.listing_event_id,
            state: item.state,
            failure_at: item.failure_at,
            receipt: item
                .receipt_event_json
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .map_err(|error| error.to_string())?,
            recovery,
        });
    }
    Ok(history)
}

#[tauri::command]
pub async fn refresh_settlement(
    request: SplitQuoteRequest,
    state: State<'_, AppState>,
) -> Result<arcadestr_core::adp_client::SettlementStatusResponse, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let repository = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    let original = repository
        .load(&buyer.to_hex(), &request.quote_id)
        .await
        .map_err(|_| "Settlement storage unavailable")?
        .ok_or("Settlement not stored for active account")?;
    if original.processor_url.trim_end_matches('/') != request.server_url.trim_end_matches('/') {
        return Err("Settlement server differs from original quote".into());
    }
    let client = AdpClient::new(request.server_url, Arc::clone(&state.http_client));
    let status = client
        .settlement_status(signer, &request.quote_id)
        .await
        .map_err(|error| error.to_string())?;
    ensure_purchase_account_current(&state, buyer).await?;
    if status.state == arcadestr_core::adp_settlement::QuoteState::Failed {
        let rows = client
            .refund_obligations(signer, Some(&request.quote_id))
            .await
            .map_err(|_| "Buyer refund status unavailable")?;
        ensure_purchase_account_current(&state, buyer).await?;
        for row in rows.obligations {
            repository
                .save_recovery_obligation(
                    &buyer.to_hex(),
                    &request.quote_id,
                    &row.leg_id,
                    row.principal_msat,
                    row.due_at,
                    None,
                    &row.state,
                )
                .await
                .map_err(|_| "Refund status persistence unavailable")?;
        }
    }
    arcadestr_core::settlement_repository::SettlementRepository::new(state.database.pool().clone())
        .update_status(
            &buyer.to_hex(),
            &request.quote_id,
            status.state,
            status.failure_at,
            u64::try_from(now_unix_i64()?).map_err(|_| "system clock before epoch".to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(status)
}

#[tauri::command]
pub async fn replace_split_refund_destination(
    request: ReplaceSplitRefundDestinationRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let endpoint = resolve_lud16(state.http_client.as_ref(), &request.refund_lud16)
        .await
        .map_err(|error| error.to_string())?;
    let client = AdpClient::new(request.server_url, Arc::clone(&state.http_client));
    let response = client
        .replace_refund_destination(
            signer,
            &request.quote_id,
            arcadestr_core::adp_settlement::RefundDestination {
                endpoint: request.refund_lud16,
                min_sendable_msat: endpoint.min_sendable_msat,
                max_sendable_msat: endpoint.max_sendable_msat,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    ensure_purchase_account_current(&state, buyer).await?;
    Ok(response)
}

#[tauri::command]
pub async fn prepare_split_recovery(
    request: SplitQuoteRequest,
    state: State<'_, AppState>,
) -> Result<arcadestr_core::adp_client::RecoveryPreparationResponse, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let client = AdpClient::new(request.server_url, Arc::clone(&state.http_client));
    let mut response = client
        .prepare_recovery(signer, &request.quote_id)
        .await
        .map_err(|error| error.to_string())?;
    if response.claimant == "buyer" {
        let plaintext = response
            .plaintext
            .clone()
            .ok_or_else(|| "fallback recovery omitted plaintext".to_string())?;
        let processor = nostr::PublicKey::from_hex(&response.evidence.processor_pubkey)
            .map_err(|_| "recovery evidence has invalid processor".to_string())?;
        let encrypted = signer
            .nip44_encrypt(&processor, &plaintext)
            .await
            .map_err(|error| format!("recovery encryption failed: {error}"))?;
        let event = signer
            .sign_event(
                nostr::EventBuilder::new(
                    nostr::Kind::Custom(arcadestr_core::adp_settlement::RECOVERY_KIND),
                    encrypted,
                )
                .build(buyer),
            )
            .await
            .map_err(|error| error.to_string())?;
        client
            .submit_recovery(signer, &request.quote_id, event.clone(), plaintext)
            .await
            .map_err(|error| error.to_string())?;
        response.event = Some(event);
    }
    ensure_purchase_account_current(&state, buyer).await?;
    let event_json = response
        .event
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| error.to_string())?;
    let due_at = response
        .evidence
        .failure_at
        .checked_add(arcadestr_core::adp_settlement::REFUND_DUE_SECS)
        .ok_or_else(|| "refund deadline overflow".to_string())?;
    let stored = arcadestr_core::settlement_repository::SettlementRepository::new(
        state.database.pool().clone(),
    );
    let quote_record = stored
        .load(&buyer.to_hex(), &request.quote_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "settlement is not stored".to_string())?;
    let quote_event: nostr::Event =
        serde_json::from_str(&quote_record.quote_event_json).map_err(|error| error.to_string())?;
    let quote_plaintext = signer
        .nip44_decrypt(&quote_event.pubkey, &quote_event.content)
        .await
        .map_err(|error| error.to_string())?;
    let quote = arcadestr_core::adp_settlement::verify_quote_event(&quote_event, &quote_plaintext)
        .map_err(|error| error.to_string())?;
    for leg_id in &response.evidence.paid_leg_ids {
        let leg = quote
            .legs
            .iter()
            .find(|leg| &leg.leg_id == leg_id)
            .ok_or_else(|| "recovery names an unknown paid leg".to_string())?;
        stored
            .save_recovery_obligation(
                &buyer.to_hex(),
                &request.quote_id,
                leg_id,
                leg.amount_msat,
                due_at,
                event_json.as_deref(),
                "refund_owed",
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(response)
}

#[tauri::command]
pub async fn prepare_split_refund(
    request: SplitRefundRequest,
    state: State<'_, AppState>,
) -> Result<arcadestr_core::adp_client::RefundInvoiceResponse, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let account = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let lease = crate::settlement_ui::claim(
        &state,
        account,
        &format!(
            "refund:{}:{}:{}",
            request.server_url, request.quote_id, request.leg_id
        ),
    )
    .await?;
    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let response = client
        .prepare_refund(signer, &request.quote_id, &request.leg_id)
        .await
        .map_err(|error| error.to_string())?;
    lease.ensure_current()?;
    crate::settlement_ui::store_prepared(&state, signer, account, &request, &response).await?;
    ensure_purchase_account_current(&state, account).await?;
    arcadestr_core::settlement_repository::SettlementRepository::new(state.database.pool().clone())
        .update_recovery_state(
            &account.to_hex(),
            &request.quote_id,
            &request.leg_id,
            "initiated",
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(response)
}

#[tauri::command]
pub async fn confirm_split_refund(
    request: ConfirmSplitRefundRequest,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let auth = { state.auth.lock().await.clone() };
    let signer = auth
        .signer()
        .ok_or_else(|| "not authenticated".to_string())?;
    let account = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    let client = AdpClient::new(request.server_url, Arc::clone(&state.http_client));
    let response = client
        .confirm_refund(
            signer,
            &request.quote_id,
            &request.leg_id,
            &request.attempt_id,
            &request.preimage,
        )
        .await
        .map_err(|error| error.to_string())?;
    ensure_purchase_account_current(&state, account).await?;
    arcadestr_core::settlement_repository::SettlementRepository::new(state.database.pool().clone())
        .update_recovery_state(
            &account.to_hex(),
            &request.quote_id,
            &request.leg_id,
            "verified",
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(response)
}

/// Claim signing is session-bound, but never uses publisher signing checkpoints.
struct ClaimSigner<'a> {
    signer: &'a dyn NostrSigner,
    session: arcadestr_core::settlement_recovery::SessionFence,
}

#[async_trait::async_trait]
impl NostrSigner for ClaimSigner<'_> {
    async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        let result = self.signer.get_public_key().await;
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        result
    }

    async fn sign_event(
        &self,
        unsigned: nostr::UnsignedEvent,
    ) -> Result<nostr::Event, SignerError> {
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        let result = self.signer.sign_event(unsigned).await;
        self.session
            .ensure_current()
            .map_err(SignerError::SigningFailed)?;
        result
    }
}

#[tauri::command]
pub async fn claim_entitlement(
    request: ClaimEntitlementRequest,
    state: State<'_, AppState>,
) -> Result<ClaimEntitlementResponse, String> {
    // Capture the epoch before any signer/relay await, including A -> B -> A.
    let (auth_snapshot, session) = {
        let auth = state.auth.lock().await;
        let account = auth.public_key().ok_or("not authenticated")?;
        let expected = nostr::PublicKey::from_hex(&request.expected_account_pubkey)
            .map_err(|_| "invalid claim account".to_string())?;
        if account != expected {
            return Err("active claim account changed".into());
        }
        state.settlement_recovery.observe(Some(account.to_hex()));
        (auth.clone(), state.settlement_recovery.session_fence()?)
    };
    let signer = ClaimSigner {
        signer: auth_snapshot.signer().ok_or("not authenticated")?,
        session: session.clone(),
    };
    let buyer = signer
        .get_public_key()
        .await
        .map_err(|error| error.to_string())?;
    if auth_snapshot.public_key() != Some(buyer) {
        return Err("claim signer does not match the initiating account".into());
    }
    let game_coordinate =
        listing_coordinate_from_npub(&request.publisher_npub, &request.listing_id)?;
    let campaign_root_id = nostr::EventId::from_hex(&request.campaign_event_id)
        .map_err(|_| "invalid campaign event id".to_string())?;
    let listing = fetch_listing_event_by_coordinate(&state, &game_coordinate).await?;
    session.ensure_current()?;
    let publisher = listing.pubkey;
    let pointer = arcadestr_core::marketplace::CampaignPointer {
        root_event_id: campaign_root_id,
        relay_hint: None,
    };
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let relay_manager = { relay_manager.lock().await.clone() };
    let report = arcadestr_core::campaign_discovery::CampaignDiscoveryService::new(&relay_manager)
        .discover(
            &[pointer],
            publisher,
            &game_coordinate,
            now_unix_i64()? as u64,
        )
        .await
        .map_err(|error| error.to_string())?;
    session.ensure_current()?;
    let campaign = report
        .campaigns
        .into_iter()
        .find(|candidate| candidate.campaign.root_event_id == campaign_root_id)
        .ok_or_else(|| "campaign is invalid or unavailable".to_string())?
        .campaign;

    let client = AdpClient::new(request.server_url.clone(), Arc::clone(&state.http_client));
    let response = client
        .entitlement_claim(
            &signer,
            CoreEntitlementClaimRequest {
                game_coordinate: game_coordinate.clone(),
                campaign_event_id: request.campaign_event_id,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    session.ensure_current()?;

    let parsed = arcadestr_core::entitlements::parse_entitlement_event(&response.grant)
        .map_err(|error| format!("invalid grant response: {error}"))?;
    if parsed.recipient != buyer {
        return Err("invalid grant response: recipient mismatch".into());
    }
    if parsed.coordinate != game_coordinate {
        return Err("invalid grant response: coordinate mismatch".into());
    }
    if parsed.source_event != campaign_root_id {
        return Err("invalid grant response: campaign source mismatch".into());
    }
    let grant = arcadestr_core::entitlements::resolve_entitlement_grant(&[parsed.clone()])
        .map_err(|error| format!("invalid grant response: {error}"))?;
    let authorization = if let Some(root_event_id) = parsed.authorization {
        Some(
            arcadestr_core::authorization::discover_authorization(
                &relay_manager,
                root_event_id,
                publisher,
            )
            .await
            .map_err(|error| format!("invalid grant authorization: {error}"))?,
        )
    } else if parsed.event.pubkey == publisher {
        None
    } else {
        return Err("invalid grant response: delegated grant has no authorization event".into());
    };
    arcadestr_core::entitlements::validate_adp_entitlement(
        &grant,
        &campaign,
        authorization.as_ref(),
    )
    .map_err(|error| format!("invalid grant response: {error}"))?;
    let buyer_hex = buyer.to_hex();
    session.ensure_current()?;
    if state
        .auth
        .lock()
        .await
        .public_key()
        .map(|key| key.to_hex())
        .as_deref()
        != Some(buyer_hex.as_str())
    {
        return Err("active account changed while grant authorization was being verified".into());
    }
    session.ensure_current()?;

    let entitlements = arcadestr_core::entitlements_repository::EntitlementsRepository::new(
        state.database.pool().clone(),
    );
    entitlements
        .ingest_event(&response.grant, &campaign, authorization.as_ref())
        .await
        .map_err(|error| error.to_string())?;
    session.ensure_current()?;
    DownloadTokensRepository::new(state.database.pool().clone())
        .upsert(&DownloadToken {
            buyer_pubkey: buyer_hex,
            game_coordinate: game_coordinate,
            server_url: request.server_url,
            token: response.download_token.clone(),
            expires_at: response.token_expires_at,
        })
        .await
        .map_err(|error| error.to_string())?;
    session.ensure_current()?;

    Ok(ClaimEntitlementResponse {
        grant: response.grant,
        download_token: response.download_token,
        token_expires_at: response.token_expires_at,
        already_claimed: response.already_claimed,
    })
}

fn listing_coordinate_from_npub(publisher_npub: &str, listing_id: &str) -> Result<String, String> {
    let merchant_pubkey = nostr::PublicKey::from_bech32(publisher_npub)
        .map_err(|_| "invalid publisher pubkey".to_string())?;
    Ok(format!("30402:{}:{}", merchant_pubkey.to_hex(), listing_id))
}

async fn request_lnurl_invoice_with_http(
    http: &dyn HttpClient,
    request: RequestLnurlInvoiceRequest,
) -> Result<RequestLnurlInvoiceResponse, String> {
    let endpoint = resolve_lud16(http, &request.lud16)
        .await
        .map_err(|err| err.to_string())?;
    let amount_msat = request
        .amount_sats
        .checked_mul(1_000)
        .ok_or_else(|| "amount_sats overflows millisats".to_string())?;
    let bolt11 = request_invoice(http, &endpoint, amount_msat)
        .await
        .map_err(|err| err.to_string())?;
    Ok(RequestLnurlInvoiceResponse { bolt11 })
}

pub(crate) async fn fetch_listing_event_by_coordinate(
    state: &State<'_, AppState>,
    coordinate: &str,
) -> Result<nostr::Event, String> {
    fetch_replaceable_listing_event_by_coordinate(state, coordinate)
        .await?
        .ok_or_else(|| "listing event not found on relays".to_string())
}

async fn fetch_replaceable_listing_event_by_coordinate(
    state: &State<'_, AppState>,
    coordinate: &str,
) -> Result<Option<nostr::Event>, String> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts
        .next()
        .ok_or_else(|| "missing coordinate kind".to_string())?;
    let developer = parts
        .next()
        .ok_or_else(|| "missing coordinate developer".to_string())?;
    let d_tag = parts
        .next()
        .ok_or_else(|| "missing coordinate d tag".to_string())?;
    if kind != "30402" {
        return Err("purchase coordinate must be a kind:30402 listing".to_string());
    }

    let developer_pubkey = nostr::PublicKey::from_hex(developer)
        .map_err(|err| format!("invalid coordinate developer pubkey: {err}"))?;
    let relay_client = {
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let manager = relay_manager.lock().await;
        manager.get_client_arc()
    };
    let timeout_duration = std::time::Duration::from_secs(10);
    let events = tokio::time::timeout(
        timeout_duration,
        relay_client.fetch_events(
            nostr::Filter::new()
                .kind(nostr::Kind::Custom(30402))
                .author(developer_pubkey)
                .identifier(d_tag),
            timeout_duration,
        ),
    )
    .await
    .map_err(|_| "listing event fetch timed out".to_string())?
    .map_err(|err| err.to_string())?;
    let events = events.into_iter().collect::<Vec<_>>();
    Ok(arcadestr_core::marketplace::current_listing_revision(
        &events,
        developer_pubkey,
        d_tag,
    ))
}

fn map_authoritative_refetch(
    result: Result<Option<nostr::Event>, String>,
    coordinate: &str,
) -> Result<nostr::Event, PublishAdpListingError> {
    result
        .map_err(|message| PublishAdpListingError::AuthoritativeRelayRefetchFailure { message })?
        .ok_or_else(|| PublishAdpListingError::AuthoritativeListingNotFound {
            coordinate: coordinate.to_owned(),
        })
}

fn select_replaceable_event(
    events: impl IntoIterator<Item = nostr::Event>,
) -> Option<nostr::Event> {
    events.into_iter().reduce(|current, candidate| {
        let candidate_id = candidate.id.to_hex();
        let current_id = current.id.to_hex();
        if is_replaceable_event_newer(
            candidate.created_at.as_secs(),
            Some(&candidate_id),
            current.created_at.as_secs(),
            Some(&current_id),
        ) {
            candidate
        } else {
            current
        }
    })
}

pub(crate) async fn recovery_current_listing(
    state: &State<'_, AppState>,
    coordinate: &str,
) -> Result<Option<nostr::Event>, String> {
    let mut parts = coordinate.splitn(3, ':');
    if parts.next() != Some("30402") {
        return Err("Invalid listing coordinate".into());
    }
    let publisher = nostr::PublicKey::from_hex(parts.next().ok_or("Missing publisher")?)
        .map_err(|_| "Invalid publisher")?;
    let d = parts.next().ok_or("Missing identifier")?;
    let query = query_coordinate(state, publisher, d).await?;
    if query.silent_relays != 0 {
        return Err("Recovery lookup is incomplete; no external action was retried".into());
    }
    Ok(select_replaceable_event(query.events.into_iter().filter(
        |event| {
            event.verify().is_ok()
                && event.pubkey == publisher
                && event.kind.as_u16() == 30402
                && listing_event_d_tag(event) == Some(d)
        },
    )))
}

async fn persist_purchase_confirmation(
    state: &State<'_, AppState>,
    buyer_pubkey: &str,
    game_coordinate: &str,
    server_url: &str,
    response: CorePurchaseConfirmResponse,
    listing_event: &nostr::Event,
    bolt11: &str,
    preimage: &str,
) -> Result<ConfirmPurchaseResponse, String> {
    let parsed = arcadestr_core::purchases::parse_receipt_event(&response.receipt)
        .map_err(|error| error.to_string())?;
    arcadestr_core::purchases::validate_receipt_listing_binding(&parsed, listing_event)
        .map_err(|error| error.to_string())?;
    if parsed.coordinate != game_coordinate {
        return Err("Receipt differs from the requested game".into());
    }
    let authorization = if response.receipt.pubkey == listing_event.pubkey {
        None
    } else {
        let root = parsed
            .authorization
            .ok_or_else(|| "delegated receipt is missing authorization".to_string())?;
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let relay_manager = relay_manager.lock().await;
        Some(
            arcadestr_core::authorization::discover_authorization(
                &relay_manager,
                root,
                listing_event.pubkey,
            )
            .await
            .map_err(|error| format!("receipt authorization unavailable or invalid: {error}"))?,
        )
    };
    let _receipt = arcadestr_core::purchases::parse_and_validate_receipt_with_authorization(
        &response.receipt,
        buyer_pubkey,
        authorization.as_ref(),
        arcadestr_core::purchases::ReceiptEvidence {
            bolt11: Some(bolt11),
            preimage: Some(preimage),
            zap_receipts: &[],
            lsp_pubkey: None,
        },
    )
    .map_err(|err| err.to_string())?;
    let current_buyer = state.auth.lock().await.public_key().map(|key| key.to_hex());
    if current_buyer.as_deref() != Some(buyer_pubkey) {
        return Err("active account changed while receipt authorization was being verified".into());
    }
    crate::receipt_commands::archive_confirmation(
        state,
        nostr::PublicKey::from_hex(buyer_pubkey).map_err(|_| "Invalid receipt buyer")?,
        &response.receipt,
        authorization.as_ref(),
    )
    .await?;
    let tokens = DownloadTokensRepository::new(state.database.pool().clone());
    tokens
        .upsert(&DownloadToken {
            buyer_pubkey: buyer_pubkey.to_string(),
            game_coordinate: game_coordinate.to_string(),
            server_url: server_url.to_string(),
            token: response.download_token.clone(),
            expires_at: response.token_expires_at,
        })
        .await
        .map_err(|err| err.to_string())?;

    Ok(ConfirmPurchaseResponse {
        receipt: response.receipt,
        download_token: response.download_token,
        token_expires_at: response.token_expires_at,
    })
}

async fn persist_split_receipt(
    state: &State<'_, AppState>,
    signer: &dyn NostrSigner,
    buyer: nostr::PublicKey,
    stored: &arcadestr_core::settlement_repository::StoredSettlement,
    receipt_event: &nostr::Event,
    download_token: &str,
    token_expires_at: i64,
) -> Result<(), String> {
    let quote_event: nostr::Event = serde_json::from_str(&stored.quote_event_json)
        .map_err(|error| format!("stored quote event is corrupt: {error}"))?;
    let plaintext = signer
        .nip44_decrypt(&quote_event.pubkey, &quote_event.content)
        .await
        .map_err(|error| format!("quote decryption failed: {error}"))?;
    let quote = arcadestr_core::adp_settlement::verify_quote_event(&quote_event, &plaintext)
        .map_err(|error| error.to_string())?;
    let parsed = arcadestr_core::purchases::parse_receipt_event(receipt_event)
        .map_err(|error| error.to_string())?;
    let aggregate = parsed
        .aggregate
        .as_ref()
        .ok_or_else(|| "split settlement returned a legacy receipt".to_string())?;
    if aggregate.quote_event_id != quote_event.id
        || aggregate.listing_event_id.to_hex() != quote.listing_event_id
        || aggregate.processor_pubkey.to_hex() != quote.processor_pubkey
        || aggregate.gross_msat != quote.gross_msat
        || aggregate.legs.len() != quote.legs.len()
    {
        return Err("aggregate receipt does not match the stored quote".into());
    }
    for (proof, leg) in aggregate.legs.iter().zip(&quote.legs) {
        if proof.leg_id != leg.leg_id
            || proof.role != leg.role
            || proof.recipient_pubkey != leg.recipient_pubkey
            || proof.amount_msat != leg.amount_msat
            || proof.accepted_terms_id != leg.accepted_terms_id
        {
            return Err("aggregate receipt leg differs from signed quote".into());
        }
    }
    // Completion uses the original verified listing snapshot. A later listing
    // or term update cannot rewrite an in-progress buyer-approved purchase.
    let original:Option<String>=sqlx::query_scalar("SELECT listing_event_json FROM purchase_quote_verification WHERE account_pubkey=? AND quote_id=?")
        .bind(buyer.to_hex()).bind(&quote.quote_id).fetch_optional(state.database.pool()).await.map_err(|_|"Original listing checkpoint unavailable")?;
    let listing_event = if let Some(json) = original {
        let event: nostr::Event =
            serde_json::from_str(&json).map_err(|_| "Original listing checkpoint invalid")?;
        event
            .verify()
            .map_err(|_| "Original listing signature invalid")?;
        event
    } else {
        fetch_listing_event_by_coordinate(state, &quote.listing_coordinate).await?
    };
    if listing_event.id.to_hex() != quote.listing_event_id {
        return Err("current listing differs from settlement snapshot".into());
    }
    arcadestr_core::purchases::validate_receipt_listing_binding(&parsed, &listing_event)
        .map_err(|error| error.to_string())?;
    if parsed.coordinate != quote.listing_coordinate {
        return Err("Receipt differs from the approved game".into());
    }
    let authorization = if receipt_event.pubkey == listing_event.pubkey {
        None
    } else {
        let root = parsed
            .authorization
            .ok_or_else(|| "delegated receipt is missing authorization".to_string())?;
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let relay_manager = relay_manager.lock().await;
        Some(
            arcadestr_core::authorization::discover_authorization(
                &relay_manager,
                root,
                listing_event.pubkey,
            )
            .await
            .map_err(|error| format!("receipt authorization unavailable or invalid: {error}"))?,
        )
    };
    let _receipt = arcadestr_core::purchases::parse_and_validate_receipt_with_authorization(
        receipt_event,
        &buyer.to_hex(),
        authorization.as_ref(),
        arcadestr_core::purchases::ReceiptEvidence {
            bolt11: None,
            preimage: None,
            zap_receipts: &[],
            lsp_pubkey: None,
        },
    )
    .map_err(|error| error.to_string())?;
    ensure_purchase_account_current(state, buyer).await?;
    crate::receipt_commands::archive_confirmation(
        state,
        buyer,
        receipt_event,
        authorization.as_ref(),
    )
    .await?;
    DownloadTokensRepository::new(state.database.pool().clone())
        .upsert(&DownloadToken {
            buyer_pubkey: buyer.to_hex(),
            game_coordinate: quote.listing_coordinate,
            server_url: stored.processor_url.clone(),
            token: download_token.to_string(),
            expires_at: token_expires_at,
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn publisher_hex_from_npub(publisher_npub: &str) -> Result<String, String> {
    nostr::PublicKey::from_bech32(publisher_npub)
        .map(|publisher| publisher.to_hex())
        .map_err(|err| format!("invalid publisher npub: {err}"))
}

pub(crate) fn verify_expected_publisher(
    expected_publisher_npub: &str,
    signer_pubkey: nostr::PublicKey,
) -> Result<(), String> {
    let expected = nostr::PublicKey::from_bech32(expected_publisher_npub)
        .map_err(|err| format!("invalid expected publisher npub: {err}"))?;
    if expected != signer_pubkey {
        return Err(
            "Active account changed before publication. Nothing was published; review the form and publish again with the intended account. (expected publisher does not match signer pubkey)"
                .to_string(),
        );
    }
    Ok(())
}

/// Listing tags the publish form owns end to end.
///
/// One definition, used by every replacement publication — ordinary edits and
/// retries after a partial publication alike. A tag named here is dropped from
/// the tags preserved off the authoritative listing, because the form's current
/// values are republished in its place. Anything absent from this list is
/// unmanaged and survives untouched, including tags Arcadestr does not know.
const FORM_MANAGED_LISTING_TAGS: &[&str] = &[
    "d",
    "title",
    "price",
    "t",
    "image",
    "acquisition",
    "server",
    "file_hash",
    "version",
    "fulfillment_pubkey",
    "lud16",
    "package",
    "launch",
    "launch_default",
    "platform",
    "campaign",
    "nip94",
];

/// Tag naming a delegated fulfillment authorization.
///
/// Deliberately absent from [`FORM_MANAGED_LISTING_TAGS`]: metadata edits preserve
/// every reference verbatim. Adding a new root never removes sibling/history tags.
#[cfg(test)]
const FULFILLMENT_AUTHORIZATION_TAG: &str = "fulfillment_authorization";

fn is_publish_form_tag(values: &[String]) -> bool {
    values
        .first()
        .is_some_and(|name| FORM_MANAGED_LISTING_TAGS.contains(&name.as_str()))
}

#[cfg(test)]
fn is_fulfillment_authorization_tag(values: &[String]) -> bool {
    values.first().map(String::as_str) == Some(FULFILLMENT_AUTHORIZATION_TAG)
}

/// Tags carried forward from the authoritative listing into its replacement.
///
/// The core builder appends these verbatim, so every tag the form republishes
/// must already be gone: otherwise the replacement carries both the stale value
/// and the current one.
fn preserved_listing_tags(existing_tags: &[Vec<String>]) -> Vec<Vec<String>> {
    existing_tags
        .iter()
        .filter(|values| !is_publish_form_tag(values))
        .cloned()
        .collect()
}

fn unique_operator_url(matches: &[AdpProvisioning]) -> Option<String> {
    match matches {
        [entry] => Some(entry.server_url.clone()),
        _ => None,
    }
}

/// Refetches and decodes the signed listing revision named by edit intent.
///
/// The application cache is never consulted. Phase A still has a concurrent-
/// publisher window between this refetch and later signing; a future revision
/// check can narrow it, but selected-relay observations never provide distributed CAS.
#[tauri::command]
pub async fn load_authoritative_adp_edit_metadata(
    request: LoadAuthoritativeAdpEditMetadataRequest,
    state: State<'_, AppState>,
) -> Result<AuthoritativeAdpEditMetadata, PublishAdpListingError> {
    let expected_publisher = nostr::PublicKey::from_bech32(&request.expected_publisher_npub)
        .map_err(|error| PublishAdpListingError::Authority {
            message: format!("invalid expected publisher npub: {error}"),
        })?;
    let coordinate = format!("30402:{}:{}", expected_publisher.to_hex(), request.d_tag);
    let event = map_authoritative_refetch(
        fetch_replaceable_listing_event_by_coordinate(&state, &coordinate).await,
        &coordinate,
    )?;
    decode_verified_authoritative_edit_metadata(&request, expected_publisher, &event)
}

#[tauri::command]
pub async fn resolve_adp_operator(
    request: ResolveAdpOperatorRequest,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let publisher_hex = publisher_hex_from_npub(&request.publisher_npub)?;
    let provisioning_repo = AdpProvisioningRepository::new(state.database.pool().clone());
    let matches = provisioning_repo
        .for_fulfillment_scope(&publisher_hex, &request.fulfillment_pubkey, &request.scope)
        .await
        .map_err(|err| err.to_string())?;

    Ok(unique_operator_url(&matches))
}

#[tauri::command]
pub async fn publish_adp_listing<R: tauri::Runtime>(
    request: PublishAdpListingRequest,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    archive_service: State<'_, ArchiveSelectionService>,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    publish_adp_listing_pipeline(
        request,
        PublishAdpListingRequestExtension::default(),
        app,
        state,
        signer_state,
        archive_service,
        false,
        false,
    )
    .await
}

#[tauri::command]
pub async fn publish_adp_listing_with_contract<R: tauri::Runtime>(
    request: PublishAdpListingRequest,
    extension: PublishAdpListingRequestExtension,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    archive_service: State<'_, ArchiveSelectionService>,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    publish_adp_listing_pipeline(
        request,
        extension,
        app,
        state,
        signer_state,
        archive_service,
        false,
        false,
    )
    .await
}

pub(crate) async fn publish_adp_listing_pipeline<R: tauri::Runtime>(
    mut request: PublishAdpListingRequest,
    mut extension: PublishAdpListingRequestExtension,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    archive_service: State<'_, ArchiveSelectionService>,
    resume: bool,
    allow_uncertain: bool,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    let session = state.settlement_recovery.session_fence()?;
    session.ensure_current()?;
    let expected = nostr::PublicKey::from_bech32(&request.expected_publisher_npub)
        .map_err(|_| "invalid expected publisher".to_string())?;
    if resume {
        let auth = { state.auth.lock().await.clone() };
        let signer = PublicationSigner {
            signer: resolve_active_signer(signer_state.inner(), &auth).await?,
            session: session.clone(),
        };
        let active = signer
            .get_public_key()
            .await
            .map_err(|_| "Publisher signer is unavailable for recovery".to_string())?;
        verify_expected_publisher(&request.expected_publisher_npub, active)?;
    }
    let operation_id = extension
        .operation_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| "invalid publication operation ID".to_string())?
        .unwrap_or_else(Uuid::new_v4);
    extension.operation_id = Some(operation_id.to_string());
    let _operation_lease = state.settlement_recovery.claim(
        &expected.to_hex(),
        &format!("publish-operation:{operation_id}"),
    )?;
    if request.existing_event_id.is_none() && request.pending_d_tag.is_none() {
        request.pending_d_tag = Some(Uuid::new_v4().to_string());
    }
    validate_selected_build_requirement(&request, &extension)?;
    for endpoint in request.servers.iter().chain(request.operator_url.iter()) {
        arcadestr_core::http_client::validate_publication_endpoint(endpoint).map_err(|_| {
            PublishAdpListingError::Upload {
                message: "unsafe publication endpoint".into(),
            }
        })?;
    }
    let lease = if let Some(reference) = request.selected_build.as_ref() {
        let auth = { state.auth.lock().await.clone() };
        let active_signer = resolve_active_signer(signer_state.inner(), &auth)
            .await
            .map_err(|_| PublishAdpListingError::Archive { error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::Account { failure: arcadestr_core::archive_inspection_contract::ArchiveAccountFailure::NotAuthenticated } })?;
        let active_npub = active_signer.get_public_key().await
            .map_err(|_| PublishAdpListingError::Archive { error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::Account { failure: arcadestr_core::archive_inspection_contract::ArchiveAccountFailure::NotAuthenticated } })?
            .to_bech32()
            .map_err(|_| PublishAdpListingError::Archive { error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::Account { failure: arcadestr_core::archive_inspection_contract::ArchiveAccountFailure::InvalidHandle } })?;
        let lease = archive_service
            .publication_lease(
                reference,
                &request.expected_publisher_npub,
                &active_npub,
                crate::archive_commands::account_check(&state, &signer_state),
            )
            .await
            .map_err(|error| PublishAdpListingError::Archive { error })?;
        request.existing_file_hash = Some(lease.digest.as_str().to_owned());
        Some(lease)
    } else {
        None
    };
    let repo = arcadestr_core::publisher_recovery::repository::PublisherRepository::new(
        state.database.pool().clone(),
    );
    let publisher = expected.to_hex();
    let journal = crate::publisher_commands::PublisherJournal {
        repo: repo.clone(),
        publisher: publisher.clone(),
        operation_id: operation_id.to_string(),
        allow_uncertain,
    };
    if !resume {
        repo.begin(&publisher, &arcadestr_core::publisher_recovery::PublisherOperation {
            operation_id: operation_id.to_string(), kind: "listing".into(),
            coordinate: format!("30402:{}:{}", publisher, request.existing_d_tag.as_deref().or(request.pending_d_tag.as_deref()).unwrap_or_default()),
            intent: serde_json::json!({"request":crate::publisher_commands::durable_request(&request)?, "extension":&extension, "requires_archive":lease.is_some()}),
            artifact_hash: lease.as_ref().map(|lease| lease.digest.as_str().to_owned()), state: "incomplete".into(), result: None,
        }).await?;
    } else {
        let original = repo
            .operation(&publisher, &operation_id.to_string())
            .await?;
        if let Some(expected_hash) = &original.artifact_hash {
            let retained_steps = repo.steps(&publisher, &operation_id.to_string()).await?;
            if let Some(saved) = retained_steps.iter().find(|step| {
                step.kind == "signed_event" && step.step_id.starts_with("signed:30402:")
            }) {
                let signed: nostr::Event = serde_json::from_value(saved.intent.clone())
                    .map_err(|_| "Invalid retained signed artifact".to_string())?;
                signed
                    .verify()
                    .map_err(|_| "Invalid retained artifact signature".to_string())?;
                let hashes = signed
                    .tags
                    .iter()
                    .filter_map(|tag| match tag.as_slice() {
                        [name, hash] if name == "file_hash" => Some(hash.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if signed.pubkey != expected || hashes.as_slice() != [expected_hash.as_str()] {
                    return Err("Retained signed listing and original artifact pin disagree; no external recovery action was taken".to_string().into());
                }
            }
            if let Some(lease) = &lease {
                arcadestr_core::publisher_recovery::verify_reselected_artifact(
                    expected_hash,
                    lease.digest.as_str(),
                )?;
            } else {
                let steps = repo.steps(&publisher, &operation_id.to_string()).await?;
                if request.servers.iter().any(|server| {
                    !steps.iter().any(|step| {
                        step.step_id == format!("upload:{server}") && step.state == "acknowledged"
                    })
                }) {
                    return Err("Reselect the original archive before resuming; expired handles are never restored".to_string().into());
                }
            }
        } else if lease.is_some() {
            return Err(
                "Recovery cannot introduce an archive into a metadata-only operation"
                    .to_string()
                    .into(),
            );
        }
    }
    let result = crate::publisher_commands::JOURNAL
        .scope(journal.clone(), async {
            if resume {
                crate::publisher_commands::reconcile_authorizations(&state, &journal, &session)
                    .await?;
            }
            publish_adp_listing_pipeline_inner(
                request,
                extension,
                app,
                state,
                signer_state,
                lease.as_ref(),
                &session,
            )
            .await
        })
        .await;
    if let Ok(result) = &result {
        repo.complete(
            &publisher,
            &operation_id.to_string(),
            &serde_json::to_value(result).map_err(|_| "Invalid publisher result".to_string())?,
        )
        .await?;
    }
    session.ensure_current()?;
    match result {
        Ok(result) => {
            if let Some(lease) = lease {
                lease.finish().await;
            }
            session.ensure_current()?;
            Ok(result)
        }
        Err(error) => Err(error),
    }
}

fn validate_selected_build_requirement(
    request: &PublishAdpListingRequest,
    extension: &PublishAdpListingRequestExtension,
) -> Result<(), PublishAdpListingError> {
    if request.selected_build.is_some() && request.file_path.is_some() {
        return Err(PublishAdpListingError::Other {
            message: "selected_build and file_path are ambiguous; publish only the backend-selected build"
                .to_string(),
        });
    }
    if request.file_path.is_some() {
        return Err(PublishAdpListingError::Other {
            message: "file_path is not a valid publication source; select the build through the archive workflow"
                .to_string(),
        });
    }
    if request.selected_build.is_some() && matches!(request.fulfillment_mode, FulfillmentMode::None)
    {
        return Err(PublishAdpListingError::Other {
            message: "a selected build requires direct or delegated fulfillment".to_string(),
        });
    }
    if request.existing_event_id.is_none() && request.existing_file_hash.is_some() {
        return Err(PublishAdpListingError::Other {
            message: "a new listing cannot bootstrap an artifact from a claimed file hash"
                .to_string(),
        });
    }

    let creates_artifact = request.existing_event_id.is_none()
        && !matches!(request.fulfillment_mode, FulfillmentMode::None);
    if request.selected_build.is_none()
        && (creates_artifact
            || (request.existing_event_id.is_none() && extension.launch_tier.is_some()))
    {
        return Err(PublishAdpListingError::Archive {
            error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::SelectedBuildRequired,
        });
    }
    Ok(())
}

fn authoritative_reused_file_hash(
    request: &PublishAdpListingRequest,
    authoritative: &nostr::Event,
) -> Result<Option<String>, PublishAdpListingError> {
    if request.selected_build.is_some() || matches!(request.fulfillment_mode, FulfillmentMode::None)
    {
        return Ok(None);
    }

    let file_hash_tags = authoritative
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|values| values.first().map(String::as_str) == Some("file_hash"))
        .collect::<Vec<_>>();
    let [values] = file_hash_tags.as_slice() else {
        return Err(PublishAdpListingError::Other {
            message: "the authoritative listing must contain exactly one file_hash tag; select a replacement build"
                .to_string(),
        });
    };
    let [_, signed_hash] = *values else {
        return Err(PublishAdpListingError::Other {
            message:
                "the authoritative listing file_hash tag is malformed; select a replacement build"
                    .to_string(),
        });
    };
    if Sha256Digest::parse(signed_hash).is_err() {
        return Err(PublishAdpListingError::Other {
            message: "the authoritative listing file_hash is invalid; select a replacement build"
                .to_string(),
        });
    }
    if request.existing_file_hash.as_deref() != Some(signed_hash.as_str()) {
        return Err(PublishAdpListingError::Other {
            message: "the requested file hash does not exactly match the signed authoritative listing; select a replacement build"
                .to_string(),
        });
    }

    let authoritative_servers = authoritative
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|values| values.first().map(String::as_str) == Some("server"))
        .filter_map(|values| match values {
            [_, server] => Some(server.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if request
        .servers
        .iter()
        .any(|server| !authoritative_servers.contains(server.as_str()))
    {
        return Err(PublishAdpListingError::Archive {
            error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::SelectedBuildRequired,
        });
    }

    Ok(Some(signed_hash.clone()))
}

fn require_launch_tier_file_hash(
    extension: &PublishAdpListingRequestExtension,
    resolved_file_hash: Option<&str>,
) -> Result<(), PublishAdpListingError> {
    if extension.launch_tier.is_some() && resolved_file_hash.is_none() {
        return Err(PublishAdpListingError::CoreValidation {
            violation: AdpPublishValidationViolation::InvalidFileHash,
            message: "a launch tier requires a resolved file_hash".to_string(),
        });
    }
    Ok(())
}

/// Read-only capacity checks must complete for every server before publication side effects.
async fn preflight_upload_capacity(
    servers: &[String],
    file_size: u64,
    http: &Arc<dyn HttpClient>,
) -> Result<(), PublishAdpListingError> {
    for server in servers {
        arcadestr_core::http_client::validate_publication_endpoint(server).map_err(|_| {
            PublishAdpListingError::Upload {
                message: "unsafe publication endpoint".into(),
            }
        })?;
        let info = AdpClient::new(server.clone(), Arc::clone(http))
            .well_known()
            .await
            .map_err(|error| PublishAdpListingError::Upload {
                message: format!("could not check upload capacity for {server}: {error}"),
            })?;
        if let Some(limit) = info.max_upload_bytes {
            if file_size > limit {
                return Err(PublishAdpListingError::Upload {
                    message: format!("build is {file_size} bytes, but {server} accepts at most {limit} bytes; choose a smaller build or increase the server upload limit"),
                });
            }
        }
    }
    Ok(())
}

async fn discover_listing_authorizations(
    state: &State<'_, AppState>,
    listing: &nostr::Event,
) -> Result<Vec<arcadestr_core::authorization::ResolvedAuthorization>, String> {
    let parsed = arcadestr_core::marketplace::parse_listing(listing.clone())?;
    if !parsed.malformed_fulfillment_authorization_tags.is_empty() {
        return Err("Malformed signed authority references require explicit review; metadata edits cannot discard them.".into());
    }
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = relay_manager.lock().await.clone();
    let mut resolved = Vec::new();
    for reference in parsed.fulfillment_authorizations {
        if resolved.iter().any(
            |authorization: &arcadestr_core::authorization::ResolvedAuthorization| {
                authorization.root_event_id == reference.root_event_id
            },
        ) {
            continue;
        }
        resolved.push(arcadestr_core::authorization::discover_authorization(&manager, reference.root_event_id, listing.pubkey).await
            .map_err(|_| "Authorization evidence is unavailable, incomplete or invalid. Restore relay visibility and reload; no authority was renewed.".to_string())?);
    }
    arcadestr_core::authorization::validate_listing_authorization_references(listing, &resolved)?;
    Ok(resolved)
}

async fn ensure_listing_revision_current(
    state: &State<'_, AppState>,
    coordinate: &str,
    expected: Option<&nostr::Event>,
) -> Result<(), String> {
    let current = fetch_replaceable_listing_event_by_coordinate(state, coordinate).await?;
    if current.is_none() && expected.is_some() {
        return Err("The expected revision is not visible on the selected relays. Visibility is incomplete; restore relay access and reload before retrying.".into());
    }
    if current.as_ref().map(|event| event.id) != expected.map(|event| event.id) {
        return Err("The selected relays show a concurrent Game page replacement. Reload before retrying; publication uses optimistic checks, not distributed CAS.".into());
    }
    Ok(())
}

async fn publish_adp_listing_pipeline_inner<R: tauri::Runtime>(
    request: PublishAdpListingRequest,
    extension: PublishAdpListingRequestExtension,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
    publication_lease: Option<&PublicationLease>,
    session: &arcadestr_core::settlement_recovery::SessionFence,
) -> Result<PublishAdpListingResult, PublishAdpListingError> {
    session.ensure_current()?;
    let expected_publisher = nostr::PublicKey::from_bech32(&request.expected_publisher_npub)
        .map_err(|error| PublishAdpListingError::Authority {
            message: format!("invalid expected publisher npub: {error}"),
        })?;
    let authoritative_edit = if let Some(expected_event_id) = request.existing_event_id.as_deref() {
        let claimed_d_tag = request.existing_d_tag.as_deref().ok_or_else(|| {
            "the identifier of the Game page being replaced is required".to_string()
        })?;
        let coordinate = format!("30402:{}:{claimed_d_tag}", expected_publisher.to_hex());
        let event = map_authoritative_refetch(
            fetch_replaceable_listing_event_by_coordinate(&state, &coordinate).await,
            &coordinate,
        )?;
        let authoritative_request = LoadAuthoritativeAdpEditMetadataRequest {
            expected_publisher_npub: request.expected_publisher_npub.clone(),
            d_tag: claimed_d_tag.to_owned(),
            expected_event_id: expected_event_id.to_owned(),
        };
        let metadata = decode_verified_authoritative_edit_metadata(
            &authoritative_request,
            expected_publisher,
            &event,
        )?;
        enforce_authoritative_profile_removals(&metadata, &extension)?;
        Some((event, metadata))
    } else {
        None
    };
    let authoritative_reused_hash = match authoritative_edit.as_ref() {
        Some((event, _)) => authoritative_reused_file_hash(&request, event)?,
        None => None,
    };
    if let (Some(journal), Some((event, _))) = (
        crate::publisher_commands::journal(),
        authoritative_edit.as_ref(),
    ) {
        let key = "original-listing-revision";
        if journal.step(key).await?.is_none() {
            journal
                .prepare(
                    key,
                    "source_listing",
                    &event.id.to_hex(),
                    serde_json::to_value(event)
                        .map_err(|_| "Invalid source listing".to_string())?,
                )
                .await?;
            journal
                .ack(key, serde_json::json!({"event_id":event.id}))
                .await?;
        }
    }
    let existing_authorizations = if let Some((event, _)) = authoritative_edit.as_ref() {
        discover_listing_authorizations(&state, event).await?
    } else {
        Vec::new()
    };
    let campaign_references = arcadestr_core::marketplace::merge_campaign_references(
        authoritative_edit.as_ref().map(|(event, _)| event),
        &request
            .campaigns
            .iter()
            .map(|pointer| (pointer.root_event_id.clone(), pointer.relay_hint.clone()))
            .collect::<Vec<_>>(),
    )?;
    let commercial_refs = validate_publisher_commercial_intent(
        &state,
        &request,
        &extension,
        authoritative_edit.as_ref().map(|(event, _)| event),
    )
    .await?;
    session.ensure_current()?;
    let resolved_file_hash = publication_lease
        .map(|lease| lease.digest.as_str().to_owned())
        .or(authoritative_reused_hash);
    require_launch_tier_file_hash(&extension, resolved_file_hash.as_deref())?;
    for server in &request.servers {
        arcadestr_core::http_client::validate_publication_endpoint(server)
            .map_err(|_| "Unsafe distribution endpoint".to_string())?;
    }
    if let Some(lease) = publication_lease {
        preflight_upload_capacity(&request.servers, lease.file_size, &state.http_client).await?;
    }

    // This authoritative Phase A check deliberately precedes every publication-
    // side signer call, provisioning request and upload. The expected event ID
    // detects changes before this refetch; the later relay-to-sign window remains
    // optimistic until a separate CAS/revision-locking phase exists.
    let auth_snapshot = { state.auth.lock().await.clone() };
    let active_signer = resolve_active_signer(signer_state.inner(), &auth_snapshot)
        .await
        .map_err(|message| PublishAdpListingError::Authority { message })?;
    let signer: Arc<dyn NostrSigner> = Arc::new(PublicationSigner {
        signer: active_signer,
        session: session.clone(),
    });
    let developer_pubkey =
        signer
            .get_public_key()
            .await
            .map_err(|error| PublishAdpListingError::Signer {
                message: error.to_string(),
            })?;
    verify_expected_publisher(&request.expected_publisher_npub, developer_pubkey)?;
    let developer_npub = developer_pubkey.to_hex();
    // The identity of the game page is settled once, here, before anything that
    // depends on the coordinate is built. Every later stage reads it from
    // `game`; nothing downstream mints a second one.
    let (game, existing_listing_tags) = if let Some(expected_event_id) =
        request.existing_event_id.as_deref()
    {
        let claimed_d_tag = request.existing_d_tag.as_deref().ok_or_else(|| {
            "the identifier of the Game page being replaced is required".to_string()
        })?;
        let (existing, _authoritative_launch_metadata) =
            authoritative_edit
                .as_ref()
                .ok_or_else(|| PublishAdpListingError::Other {
                    message: "authoritative edit state was not retained".to_string(),
                })?;
        ensure_expected_listing_event(
            existing,
            expected_event_id,
            "This Game page changed since it was opened. Reload it before publishing so newer metadata is not overwritten.",
        )?;
        if existing.pubkey != developer_pubkey {
            return Err(PublishAdpListingError::Authority {
                message: "the Game page being replaced belongs to another publisher".to_string(),
            });
        }
        let identifier =
            resolve_existing_game_identifier(Some(claimed_d_tag), listing_event_d_tag(existing))?;
        // Kept whole here: which tags the form replaces is not settled until
        // fulfillment authorization is resolved, further down.
        let existing_tags = existing
            .tags
            .iter()
            .map(|tag| tag.clone().to_vec())
            .collect::<Vec<_>>();
        (
            GameCoordinate::new(developer_pubkey, identifier),
            existing_tags,
        )
    } else {
        // A carried-over identity is only resumable onto a coordinate the
        // relays confirm is free; anything else falls back to a fresh one.
        let resumable =
            resumable_pending_d_tag(&state, developer_pubkey, request.pending_d_tag.as_deref())
                .await?;
        let identifier =
            resolve_new_game_identifier(request.existing_d_tag.as_deref(), resumable.as_deref())?;
        (
            GameCoordinate::new(developer_pubkey, identifier),
            Vec::new(),
        )
    };
    let game_d_tag = game.d_tag();
    session.ensure_current()?;
    let _coordinate_lease = state.settlement_recovery.claim(
        &developer_npub,
        &format!("publish:{}", game.to_coordinate_string()),
    )?;
    let app = Arc::new(PublicationProgress {
        app,
        operation_id: extension
            .operation_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        publisher_npub: request.expected_publisher_npub.clone(),
        game_coordinate: game.to_coordinate_string(),
        session: session.clone(),
    });
    if game.generated_game_id().is_some() {
        // The identity of a first publication is settled but nothing has been
        // published under it yet. Hand it to the caller before any step that
        // can fail, so a retry resumes this publication — same coordinate, same
        // provisioning scope — instead of minting a second one and leaving this
        // one's fulfillment key stranded on the operator.
        emit_progress(
            &app,
            "pending-coordinate",
            "ok",
            Some(game.to_coordinate_string()),
        )?;
    }
    let FulfillmentMetadata {
        fulfillment_pubkey: mut fulfillment_pubkey,
        should_provision,
    } = if authoritative_edit.is_some() {
        if matches!(request.fulfillment_mode, FulfillmentMode::Delegate)
            && existing_authorizations.is_empty()
            && !extension.authorize_operators
        {
            return Err("This listing has no verified delegated authority. Set up authority explicitly before selecting delegated fulfillment; metadata edits never issue it.".to_string().into());
        }
        FulfillmentMetadata {
            fulfillment_pubkey: None,
            should_provision: extension.authorize_operators,
        }
    } else {
        if request.existing_fulfillment_pubkey.is_some() {
            return Err(
                "A claimed fulfillment key cannot bootstrap a new listing's authority."
                    .to_string()
                    .into(),
            );
        }
        resolve_existing_fulfillment_metadata(&request.fulfillment_mode, &developer_npub, None)?
    };
    let mut resolved_servers = Vec::new();
    for server in &request.servers {
        session.ensure_current()?;
        let info = AdpClient::new(server.clone(), Arc::clone(&state.http_client))
            .well_known()
            .await
            .map_err(|e| e.to_string())?;
        resolved_servers.push((server.clone(), info.pubkey));
    }
    let operator_plans = arcadestr_core::publisher_management::operator_plans(
        &resolved_servers,
        &extension.server_roles,
        request.operator_url.as_deref(),
    )?;
    if let Some(journal) = crate::publisher_commands::journal() {
        let upload_expected = journal
            .repo
            .operation(&journal.publisher, &journal.operation_id)
            .await?
            .artifact_hash
            .is_some();
        for plan in &operator_plans {
            for url in &plan.urls {
                journal.prepare(&format!("distribution:{url}"), "distribution", url, serde_json::json!({"operator":plan.operator.to_hex(),"capabilities":plan.capabilities,"upload_expected":upload_expected,"provision_url":plan.urls.first()})).await?;
            }
        }
    }
    if publication_lease.is_some()
        && !existing_authorizations.is_empty()
        && !extension.authorize_operators
    {
        let (listing, _) = authoritative_edit
            .as_ref()
            .ok_or_else(|| "Missing authoritative listing".to_string())?;
        let references = arcadestr_core::authorization::validate_listing_authorization_references(
            listing,
            &existing_authorizations,
        )?;
        let listed_candidates = references
            .iter()
            .map(|reference| reference.fulfillment_pubkey)
            .collect();
        let required = BTreeSet::from([CAPABILITY_UPLOAD_BUILD.to_string()]);
        for server in &request.servers {
            let info = AdpClient::new(server.clone(), Arc::clone(&state.http_client))
                .well_known()
                .await
                .map_err(|e| e.to_string())?;
            let operator = nostr::PublicKey::from_hex(&info.pubkey)
                .map_err(|_| "Invalid operator key".to_string())?;
            // Selection establishes a listed, currently authorized candidate only.
            // Key availability remains the server's responsibility; an acknowledgement
            // is not cryptographic evidence or a new source of distribution authority.
            if arcadestr_core::authorization::select_reusable_authorization(
                &existing_authorizations,
                &references,
                &listed_candidates,
                operator,
                &game.to_coordinate_string(),
                &required,
                nostr::Timestamp::now().as_secs(),
            )
            .is_none()
            {
                return Err("No existing current upload-build capability matches a selected operator. Review authority explicitly; no root was renewed or capability expanded.".to_string().into());
            }
        }
    }

    let mut file_hash = None;
    let mut acceptance_event_id = None;
    let mut new_authorizations = Vec::new();

    match request.fulfillment_mode {
        FulfillmentMode::None => {}
        FulfillmentMode::Direct => {
            if request.servers.is_empty() {
                return Err(
                    "at least one distribution server is required for fulfillment"
                        .to_string()
                        .into(),
                );
            }
            let version = request
                .version
                .as_deref()
                .ok_or_else(|| "version is required for fulfillment".to_string())?;
            if version.trim().is_empty() {
                return Err("version is required for fulfillment".to_string().into());
            }

            let hash = if let Some(lease) = publication_lease {
                lease
                    .verify_digest()
                    .await
                    .map_err(|error| PublishAdpListingError::Archive { error })?;
                resolved_file_hash.clone().ok_or_else(|| {
                    PublishAdpListingError::CoreValidation {
                        violation: AdpPublishValidationViolation::InvalidFileHash,
                        message: "fulfillment requires a resolved file_hash".to_string(),
                    }
                })?
            } else {
                resolved_file_hash
                    .clone()
                    .ok_or_else(|| PublishAdpListingError::Other {
                        message: "an authoritative file hash is required when no build is selected"
                            .to_string(),
                    })?
            };
            file_hash = Some(hash);
        }
        FulfillmentMode::Delegate => {
            if request.servers.is_empty() {
                return Err(
                    "at least one distribution server is required for fulfillment"
                        .to_string()
                        .into(),
                );
            }
            let version = request
                .version
                .as_deref()
                .ok_or_else(|| "version is required for fulfillment".to_string())?;
            if version.trim().is_empty() {
                return Err("version is required for fulfillment".to_string().into());
            }

            let hash = if let Some(lease) = publication_lease {
                lease
                    .verify_digest()
                    .await
                    .map_err(|error| PublishAdpListingError::Archive { error })?;
                resolved_file_hash.clone().ok_or_else(|| {
                    PublishAdpListingError::CoreValidation {
                        violation: AdpPublishValidationViolation::InvalidFileHash,
                        message: "fulfillment requires a resolved file_hash".to_string(),
                    }
                })?
            } else {
                resolved_file_hash
                    .clone()
                    .ok_or_else(|| PublishAdpListingError::Other {
                        message: "an authoritative file hash is required when no build is selected"
                            .to_string(),
                    })?
            };
            file_hash = Some(hash);

            if should_provision {
                for plan in &operator_plans {
                    let operator_url = plan
                        .urls
                        .first()
                        .ok_or("Missing operator endpoint".to_string())?
                        .as_str();
                    let matching = existing_authorizations
                        .iter()
                        .filter(|authority| authority.terms.operator_pubkey == plan.operator)
                        .collect::<Vec<_>>();
                    if !matching.is_empty() {
                        if matching.iter().any(|authority| {
                            authority.cancelled_at.is_none()
                                && plan.capabilities.is_subset(&authority.terms.capabilities)
                                && authority.terms.valid_from <= nostr::Timestamp::now().as_secs()
                        }) {
                            continue; // Preserve the already-listed authority verbatim; no POST or new root.
                        }
                        return Err("Existing operator authority is cancelled, stale or lacks the requested capabilities. Setup never silently renews or expands it".to_string().into());
                    }
                    emit_progress(
                        &app,
                        "check-operator",
                        "pending",
                        Some(operator_url.to_string()),
                    )?;
                    let adp_client =
                        AdpClient::new(operator_url.to_string(), Arc::clone(&state.http_client));
                    let server_info = adp_client
                        .well_known()
                        .await
                        .map_err(|err| progress_error(&app, "check-operator", err))?;
                    if server_info.pubkey != plan.operator.to_hex() {
                        return Err("Operator identity changed during provisioning"
                            .to_string()
                            .into());
                    }
                    emit_progress(
                        &app,
                        "check-operator",
                        "ok",
                        Some(server_info.pubkey.clone()),
                    )?;

                    let provisioning_repo =
                        AdpProvisioningRepository::new(state.database.pool().clone());
                    let scope = game_d_tag.as_str();

                    emit_progress(&app, "provision", "pending", None)?;
                    let pending_row = provisioning_repo
                        .active_for_scope(&developer_npub, operator_url, Some(scope))
                        .await
                        .map_err(|e| e.to_string())?;
                    let reusable_authorization = if let Some(row) = &pending_row {
                        let root = row.authorization_root_event_id.as_deref().ok_or_else(|| "Pending authority has no exact root; review it explicitly before retrying.".to_string())?;
                        let root = nostr::EventId::from_hex(root)
                            .map_err(|_| "Invalid pending authority root".to_string())?;
                        let relay_manager =
                            { state.nostr.lock().await.get_relay_manager().clone() };
                        let manager = relay_manager.lock().await.clone();
                        Some(arcadestr_core::authorization::discover_authorization(&manager, root, developer_pubkey).await
                        .map_err(|_| "Pending authority evidence is incomplete; restore relay visibility before retrying.".to_string())?)
                    } else {
                        None
                    };
                    let provisioning = resolve_provisioning(ResolveProvisioningInput {
                        provisioning_repo: &provisioning_repo,
                        adp_client: &adp_client,
                        signer: signer.as_ref(),
                        developer_pubkey,
                        developer_npub: &developer_npub,
                        server_url: operator_url,
                        scope,
                        server_info: &server_info,
                        reusable_authorization: reusable_authorization.as_ref(),
                        required_capabilities: plan.capabilities.clone(),
                    })
                    .await
                    .inspect_err(|error| {
                        let _ = emit_progress(&app, "provision", "error", Some(error.to_string()));
                    })?;

                    match provisioning {
                        ProvisioningDecision::Reused {
                            fulfillment_pubkey: reused_pubkey,
                            authorization_event_id,
                            valid_from: _,
                            authorization_event,
                            attestation,
                            row,
                        } => {
                            validate_current_attestation(&state, &row, attestation.as_deref())
                                .await?;
                            ensure_publish_account_current(
                                &state,
                                signer_state.inner(),
                                developer_pubkey,
                                session,
                            )
                            .await?;
                            provisioning_repo
                                .upsert(&row)
                                .await
                                .map_err(|err| progress_error(&app, "provision", err))?;
                            session.ensure_current()?;
                            publish_authorization_once(&state, &authorization_event, session)
                                .await
                                .map_err(|err| progress_error(&app, "provision", err))?;
                            emit_progress(
                                &app,
                                "provision",
                                "ok",
                                Some("reused existing provisioning".into()),
                            )?;
                            fulfillment_pubkey = Some(reused_pubkey);
                            acceptance_event_id = Some(authorization_event_id);
                        }
                        ProvisioningDecision::Created {
                            fulfillment_pubkey: created_pubkey,
                            authorization_event_id,
                            valid_from: _,
                            authorization_event,
                            attestation,
                            row,
                        } => {
                            validate_current_attestation(&state, &row, attestation.as_deref())
                                .await?;
                            ensure_publish_account_current(
                                &state,
                                signer_state.inner(),
                                developer_pubkey,
                                session,
                            )
                            .await?;
                            provisioning_repo
                                .upsert(&row)
                                .await
                                .map_err(|err| progress_error(&app, "provision", err))?;
                            session.ensure_current()?;
                            publish_authorization_once(&state, &authorization_event, session)
                                .await
                                .map_err(|err| progress_error(&app, "provision", err))?;
                            emit_progress(
                                &app,
                                "provision",
                                "ok",
                                Some("created provisioning".into()),
                            )?;
                            fulfillment_pubkey = Some(created_pubkey);
                            acceptance_event_id = Some(authorization_event_id);
                        }
                    }
                    new_authorizations.push(FulfillmentAuthorizationInput {
                        root_event_id: acceptance_event_id
                            .clone()
                            .ok_or("Missing authorization root".to_string())?,
                        fulfillment_pubkey: fulfillment_pubkey
                            .clone()
                            .ok_or("Missing provisioned key".to_string())?,
                        relay_hint: None,
                    });
                }
            }
        }
    }

    if let Some(expected_event_id) = request.existing_event_id.as_deref() {
        let coordinate = game.to_coordinate_string();
        let current = fetch_listing_event_by_coordinate(&state, &coordinate).await?;
        ensure_expected_listing_event(
            &current,
            expected_event_id,
            "This Game page changed while publishing authorization was prepared. Reload it before publishing so newer metadata is not overwritten.",
        )?;
    }

    emit_progress(&app, "publish-listing", "pending", None)?;
    ensure_publish_account_current(&state, signer_state.inner(), developer_pubkey, session).await?;
    let fulfillment_authorizations = new_authorizations;
    // The authoritative listing supplies identity, concurrency and unmanaged
    // metadata only. Every field the form owns is republished from the form's
    // current values, so a retry after a partial publication carries the
    // publisher's latest intent rather than the half-published state.
    let preserved_tags = preserved_listing_tags(&existing_listing_tags);
    let mut listing_input = AdpListingInput {
        d_tag: game_d_tag.clone(),
        title: request.title.clone(),
        description: request.description.clone(),
        price_sats: request.price_sats,
        lud16: request.lud16.clone(),
        tags: request.tags.clone(),
        images: request.images.clone(),
        servers: request.servers.clone(),
        file_hash: file_hash.clone(),
        version: request.version.clone(),
        fulfillment_authorizations,
        acquisition: request.acquisition.clone(),
        platforms: request.platforms.clone(),
        campaigns: campaign_references,
        nip94_event_id: request.nip94_event_id.clone(),
        preserved_tags,
    };
    if matches!(
        extension.commercial_intent,
        arcadestr_core::adp_settlement::PublisherCommercialIntent::ReplaceAfterReview { .. }
    ) {
        let mut operators = Vec::new();
        for server in &request.servers {
            let info = AdpClient::new(server.clone(), Arc::clone(&state.http_client))
                .well_known()
                .await
                .map_err(|error| PublishAdpListingError::Other {
                    message: error.to_string(),
                })?;
            operators.push(arcadestr_core::adp_settlement::ServerOperator {
                url: server.clone(),
                operator_pubkey: info.pubkey,
            });
        }
        arcadestr_core::adp_publish::apply_managed_adp_terms(
            &mut listing_input,
            &operators,
            &commercial_refs,
        )
        .map_err(|error| PublishAdpListingError::Other {
            message: error.to_string(),
        })?;
    }
    let mut listing_builder = build_adp_listing_event_builder(AdpListingPublishInput {
        listing: &listing_input,
        launch_tier: extension.launch_tier.as_ref(),
    })
    .map_err(PublishAdpListingError::from)?;
    ensure_publish_account_current(&state, signer_state.inner(), developer_pubkey, session).await?;
    if let Some(lease) = publication_lease {
        lease
            .verify_digest()
            .await
            .map_err(|error| PublishAdpListingError::Archive { error })?;
    }
    let predecessor = authoritative_edit.as_ref().map(|(event, _)| event);
    ensure_listing_revision_current(&state, &game.to_coordinate_string(), predecessor).await?;
    session.ensure_current()?;
    let retained_listing = if let Some(journal) = crate::publisher_commands::journal() {
        journal
            .step(&format!("signed:30402:{}", game_d_tag))
            .await?
            .is_some()
    } else {
        false
    };
    if let Some(predecessor) = predecessor.filter(|_| !retained_listing) {
        listing_builder = listing_builder.custom_created_at(
            arcadestr_core::marketplace::listing_replacement_timestamp(
                predecessor,
                nostr::Timestamp::now(),
            )?,
        );
    }
    let listing_event =
        validate_and_sign_adp_listing(signer.as_ref(), listing_builder.build(developer_pubkey))
            .await
            .inspect_err(|error| {
                let _ = emit_progress(&app, "publish-listing", "error", Some(error.to_string()));
            })?;
    // Signing can involve a remote signer. Revalidate after it returns and before
    // making the signed listing irreversible on relays.
    validate_publisher_commercial_intent(
        &state,
        &request,
        &extension,
        authoritative_edit.as_ref().map(|(event, _)| event),
    )
    .await?;
    ensure_listing_revision_current(&state, &game.to_coordinate_string(), predecessor).await?;
    ensure_publish_account_current(&state, signer_state.inner(), developer_pubkey, session)
        .await
        .map_err(|message| PublishAdpListingError::Authority { message })?;
    session.ensure_current()?;
    if predecessor.is_some_and(|event| event.id == listing_event.id) {
        if let Some(journal) = crate::publisher_commands::before_publish(&listing_event).await? {
            journal
                .ack(
                    &format!("relay:{}", listing_event.id),
                    serde_json::json!({"accepted":true}),
                )
                .await?;
        }
    } else {
        publish_event(&state, &listing_event, session)
            .await
            .map_err(|error| PublishAdpListingError::Relay {
                message: progress_error(&app, "publish-listing", error),
            })?;
    }
    emit_progress(
        &app,
        "publish-listing",
        "ok",
        Some(listing_event.id.to_hex()),
    )?;
    let coordinate = game.to_coordinate_string();
    // Publication is not transactional: from here on a failure leaves a real
    // listing behind. Hand the coordinate to the caller immediately so a retry
    // edits that listing instead of minting a second one.
    emit_progress(&app, "listing-coordinate", "ok", Some(coordinate.clone()))?;

    emit_progress(&app, "confirm-propagation", "pending", None)?;
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let propagated = confirm_nip99_listing_propagated(&relay_manager, &listing_event, 2)
        .await
        .map_err(|err| PublishAdpListingError::PartialPublication {
            cause: Box::new(PublishAdpListingError::Relay {
                message: progress_error(&app, "confirm-propagation", err),
            }),
            successful_servers: Vec::new(),
        })?;
    if !propagated {
        emit_progress(
            &app,
            "confirm-propagation",
            "error",
            Some("exact current listing revision not confirmed on two selected relays; visibility may be incomplete".into()),
        )?;
        return Err(PublishAdpListingError::PartialPublication { cause: Box::new(PublishAdpListingError::Relay {
            message: "Exact current listing revision not confirmed on two selected relays; refresh before retrying (this is not global consensus).".to_string(),
        }), successful_servers: Vec::new() });
    }
    emit_progress(&app, "confirm-propagation", "ok", Some(coordinate.clone()))?;

    let mut uploads: Vec<PublishServerUploadResult> = Vec::new();
    if !matches!(request.fulfillment_mode, FulfillmentMode::None) && publication_lease.is_some() {
        let lease = publication_lease.ok_or_else(|| PublishAdpListingError::Other {
            message: "selected build lease disappeared before upload".to_string(),
        })?;
        let mut upload_errors = Vec::new();
        for server_url in &request.servers {
            if matches!(request.fulfillment_mode, FulfillmentMode::Delegate)
                || !existing_authorizations.is_empty()
            {
                let authority_check: Result<(), String> = async {
                    let current_authorizations = discover_listing_authorizations(&state, &listing_event).await?;
                    let references = arcadestr_core::authorization::validate_listing_authorization_references(&listing_event, &current_authorizations)?;
                    let candidates = references.iter().map(|reference| reference.fulfillment_pubkey).collect();
                    let info = AdpClient::new(server_url.clone(), Arc::clone(&state.http_client)).well_known().await.map_err(|e| e.to_string())?;
                    let operator = nostr::PublicKey::from_hex(&info.pubkey).map_err(|_| "Invalid operator".to_string())?;
                    if arcadestr_core::authorization::select_reusable_authorization(&current_authorizations, &references, &candidates, operator, &coordinate, &BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()]), nostr::Timestamp::now().as_secs()).is_none() {
                        return Err("Upload authority changed or is unavailable; review it explicitly. No root was renewed.".into());
                    }
                    Ok(())
                }.await;
                if authority_check.is_err() {
                    if let Some(journal) = crate::publisher_commands::journal() {
                        journal.failed(&format!("distribution:{server_url}"),"Current upload authority could not be verified; this upload attempt did not start").await?;
                    }
                }
                authority_check.map_err(|message| PublishAdpListingError::PartialPublication {
                    cause: Box::new(PublishAdpListingError::Authority { message }),
                    successful_servers: uploads
                        .iter()
                        .filter(|upload| upload.status == "ok")
                        .map(|upload| upload.server_url.clone())
                        .collect(),
                })?;
            }
            ensure_listing_revision_current(&state, &coordinate, Some(&listing_event))
                .await
                .map_err(|message| PublishAdpListingError::PartialPublication {
                    cause: Box::new(PublishAdpListingError::Relay { message }),
                    successful_servers: uploads
                        .iter()
                        .filter(|upload| upload.status == "ok")
                        .map(|upload| upload.server_url.clone())
                        .collect(),
                })?;
            session.ensure_current()?;
            ensure_publish_account_current(&state, signer_state.inner(), developer_pubkey, session)
                .await
                .map_err(|message| PublishAdpListingError::PartialPublication {
                    cause: Box::new(PublishAdpListingError::Authority { message }),
                    successful_servers: uploads
                        .iter()
                        .filter(|upload| upload.status == "ok")
                        .map(|upload| upload.server_url.clone())
                        .collect(),
                })?;
            emit_server_progress(&app, "upload", "pending", Some(server_url.clone()), None)?;
            let adp_client = AdpClient::new(server_url.clone(), Arc::clone(&state.http_client));
            let progress_app = app.clone();
            let progress_server_url = server_url.clone();
            let last_reported_percent = Arc::new(AtomicU64::new(u64::MAX));
            let progress = move |bytes_uploaded: u64, total_bytes: u64| {
                let percent = if total_bytes == 0 {
                    100
                } else {
                    bytes_uploaded.saturating_mul(100) / total_bytes
                };
                if last_reported_percent.swap(percent, Ordering::Relaxed) != percent {
                    let _ = emit_upload_progress(
                        &progress_app,
                        &progress_server_url,
                        bytes_uploaded,
                        total_bytes,
                    );
                }
            };
            let upload_step = format!("upload:{server_url}");
            let journal = crate::publisher_commands::journal();
            let retained = if let Some(journal) = &journal {
                journal.external_checkpoint(&upload_step, "upload", server_url, serde_json::json!({"event_id":listing_event.id,"file_hash":lease.digest.as_str(),"coordinate":coordinate})).await?
            } else {
                None
            };
            if let Some(outcome) = retained {
                let upload: UploadResponse = serde_json::from_value(outcome)
                    .map_err(|_| "Invalid retained upload acknowledgement".to_string())?;
                arcadestr_core::adp_client::validate_upload_acknowledgement(
                    &listing_event,
                    &lease.digest,
                    &upload,
                )
                .map_err(|_| "Retained upload does not match the pinned artifact".to_string())?;
                uploads.push(PublishServerUploadResult {
                    server_url: server_url.clone(),
                    status: "ok".into(),
                    error: None,
                    upload: Some(upload),
                });
                continue;
            }
            let upload = lease
                .upload_with_progress(
                    &adp_client,
                    signer.as_ref(),
                    &listing_event,
                    &request.expected_publisher_npub,
                    crate::archive_commands::account_check(&state, &signer_state),
                    session,
                    progress,
                )
                .await;
            if let Some(journal) = &journal {
                if upload.is_err() {
                    journal.failed(&upload_step, "Upload response failed or was interrupted; the remote outcome remains unacknowledged").await?;
                }
            }
            if let (Some(journal), Ok(response)) = (&journal, &upload) {
                journal
                    .ack(
                        &upload_step,
                        serde_json::to_value(response)
                            .map_err(|_| "Invalid upload acknowledgement".to_string())?,
                    )
                    .await?;
            }
            if let Err(message) = ensure_publish_account_current(
                &state,
                signer_state.inner(),
                developer_pubkey,
                session,
            )
            .await
            {
                let mut successful_servers = uploads
                    .iter()
                    .filter(|upload| upload.status == "ok")
                    .map(|upload| upload.server_url.clone())
                    .collect::<Vec<_>>();
                if upload.is_ok() {
                    successful_servers.push(server_url.clone());
                }
                return Err(PublishAdpListingError::PartialPublication {
                    cause: Box::new(PublishAdpListingError::Authority { message }),
                    successful_servers,
                });
            }
            match upload {
                Ok(upload) => {
                    let download_url = upload.download_url.clone();
                    uploads.push(PublishServerUploadResult {
                        server_url: server_url.clone(),
                        status: "ok".to_string(),
                        error: None,
                        upload: Some(upload),
                    });
                    emit_server_progress(
                        &app,
                        "upload",
                        "ok",
                        Some(server_url.clone()),
                        Some(download_url),
                    )
                    .map_err(|message| {
                        PublishAdpListingError::PartialPublication {
                            cause: Box::new(PublishAdpListingError::Other { message }),
                            successful_servers: uploads
                                .iter()
                                .filter(|upload| upload.status == "ok")
                                .map(|upload| upload.server_url.clone())
                                .collect(),
                        }
                    })?;
                }
                Err(LeaseUploadError::Archive(error)) => {
                    let error = PublishAdpListingError::Archive { error };
                    return Err(PublishAdpListingError::PartialPublication {
                        cause: Box::new(error),
                        successful_servers: uploads
                            .iter()
                            .filter(|upload| upload.status == "ok")
                            .map(|upload| upload.server_url.clone())
                            .collect(),
                    });
                }
                Err(LeaseUploadError::Upload(err)) => {
                    let message = err.to_string();
                    emit_server_progress(
                        &app,
                        "upload",
                        "error",
                        Some(server_url.clone()),
                        Some(message.clone()),
                    )?;
                    upload_errors.push(format!("{server_url}: {message}"));
                    uploads.push(PublishServerUploadResult {
                        server_url: server_url.clone(),
                        status: "error".to_string(),
                        error: Some(message),
                        upload: None,
                    });
                }
            }
        }

        if !upload_errors.is_empty() {
            return Err(PublishAdpListingError::PartialPublication {
                cause: Box::new(PublishAdpListingError::Upload {
                    message: format!("upload failed for: {}", upload_errors.join("; ")),
                }),
                successful_servers: uploads
                    .iter()
                    .filter(|upload| upload.status == "ok")
                    .map(|upload| upload.server_url.clone())
                    .collect(),
            });
        }
    }

    if publication_lease.is_none() {
        if let Some(journal) = crate::publisher_commands::journal() {
            for step in journal
                .repo
                .steps(&journal.publisher, &journal.operation_id)
                .await?
                .into_iter()
                .filter(|step| step.kind == "upload" && step.state == "acknowledged")
            {
                let response: UploadResponse = serde_json::from_value(
                    step.outcome
                        .ok_or_else(|| "Missing retained upload acknowledgement".to_string())?,
                )
                .map_err(|_| "Invalid retained upload acknowledgement".to_string())?;
                let digest = arcadestr_core::distribution::Sha256Digest::parse(
                    file_hash
                        .as_deref()
                        .ok_or_else(|| "Missing pinned artifact hash".to_string())?,
                )
                .map_err(|_| "Invalid pinned artifact hash".to_string())?;
                arcadestr_core::adp_client::validate_upload_acknowledgement(
                    &listing_event,
                    &digest,
                    &response,
                )
                .map_err(|_| "Retained upload acknowledgement mismatch".to_string())?;
                uploads.push(PublishServerUploadResult {
                    server_url: step.target,
                    status: "ok".into(),
                    error: None,
                    upload: Some(response),
                });
            }
        }
    }
    Ok(PublishAdpListingResult {
        operation_id: app.operation_id.clone(),
        event_id: listing_event.id.to_hex(),
        game_id: game.generated_game_id(),
        d_tag: game_d_tag,
        game_coordinate: coordinate,
        acceptance_event_id,
        fulfillment_pubkey,
        file_hash,
        uploads,
    })
}

async fn validate_current_attestation(
    state: &State<'_, AppState>,
    row: &AdpProvisioning,
    provided: Option<&nostr::Event>,
) -> Result<(), String> {
    let operator = nostr::PublicKey::from_hex(&row.operator_pubkey)
        .map_err(|_| "operator pubkey is invalid".to_string())?;
    let d = format!("{}:{}", row.developer_npub, row.fulfillment_pubkey);
    let attestation = if let Some(attestation) = provided {
        if attestation.id.to_hex() != row.attestation_event_id {
            return Err("operator attestation evidence ID mismatch".into());
        }
        attestation.clone()
    } else {
        let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let relay_manager = relay_manager.lock().await.clone();
        let events = relay_manager
            .fetch_events_best_effort(
                nostr::Filter::new()
                    .kind(nostr::Kind::Custom(30404))
                    .author(operator),
            )
            .await
            .map_err(|error| format!("operator attestation evidence unavailable: {error}"))?;
        select_replaceable_event(events.into_iter().filter(|event| {
            event
                .tags
                .iter()
                .any(|tag| matches!(tag.as_slice(), [name, value] if name == "d" && value == &d))
        }))
        .ok_or_else(|| "operator attestation evidence unavailable".to_string())?
    };
    // The clock is read here rather than when publishing began. The operator
    // mints this attestation during provisioning, several steps after the
    // command starts, so a timestamp captured up front predates the
    // `valid_from` it would be compared against.
    let now: u64 = now_unix_i64()?
        .try_into()
        .map_err(|_| "current time is negative".to_string())?;
    ensure_attestation_covers_provisioning(row, operator, &attestation, now)
}

fn ensure_attestation_covers_provisioning(
    row: &AdpProvisioning,
    operator: nostr::PublicKey,
    attestation: &nostr::Event,
    now: u64,
) -> Result<(), String> {
    let parsed = arcadestr_core::authorization::parse_attestation_event(attestation)
        .map_err(|error| format!("invalid operator attestation: {error}"))?;
    if attestation.pubkey != operator
        || parsed.developer_pubkey.to_hex() != row.developer_npub
        || parsed.fulfillment_pubkey.to_hex() != row.fulfillment_pubkey
        || parsed.scope.as_deref() != row.scope.as_deref()
    {
        return Err("operator attestation does not cover this fulfillment key".into());
    }
    if !parsed.allows_new_operations_observed_at(now) {
        return Err("operator no longer attests possession of this fulfillment key".into());
    }
    Ok(())
}

enum ProvisioningDecision {
    Reused {
        fulfillment_pubkey: String,
        authorization_event_id: String,
        valid_from: i64,
        authorization_event: Box<nostr::Event>,
        attestation: Option<Box<nostr::Event>>,
        row: Box<AdpProvisioning>,
    },
    Created {
        fulfillment_pubkey: String,
        authorization_event_id: String,
        valid_from: i64,
        authorization_event: Box<nostr::Event>,
        attestation: Option<Box<nostr::Event>>,
        row: Box<AdpProvisioning>,
    },
}

struct ResolveProvisioningInput<'a> {
    provisioning_repo: &'a AdpProvisioningRepository,
    adp_client: &'a AdpClient,
    signer: &'a dyn NostrSigner,
    developer_pubkey: nostr::PublicKey,
    developer_npub: &'a str,
    server_url: &'a str,
    scope: &'a str,
    server_info: &'a AdpServerInfo,
    reusable_authorization: Option<&'a arcadestr_core::authorization::ResolvedAuthorization>,
    required_capabilities: BTreeSet<String>,
}

async fn resolve_provisioning(
    input: ResolveProvisioningInput<'_>,
) -> Result<ProvisioningDecision, PublishAdpListingError> {
    let listing_coordinate = format!("30402:{}:{}", input.developer_pubkey.to_hex(), input.scope);
    let existing = input
        .provisioning_repo
        .active_for_scope(input.developer_npub, input.server_url, Some(input.scope))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(existing) = &existing {
        let authorization = input.reusable_authorization.ok_or_else(|| "Existing pending authority requires exact chain evidence; no replacement root was issued.".to_string())?;
        let operator = nostr::PublicKey::from_hex(&input.server_info.pubkey)
            .map_err(|_| "Invalid operator".to_string())?;
        let key = nostr::PublicKey::from_hex(&existing.fulfillment_pubkey)
            .map_err(|_| "Invalid pending key".to_string())?;
        let references = [
            arcadestr_core::marketplace::FulfillmentAuthorizationReference {
                root_event_id: authorization.root_event_id,
                fulfillment_pubkey: key,
                relay_hint: None,
            },
        ];
        if authorization.developer_pubkey != input.developer_pubkey
            || existing.authorization_root_event_id.as_deref()
                != Some(authorization.root_event_id.to_hex().as_str())
            || arcadestr_core::authorization::select_reusable_authorization(
                std::slice::from_ref(authorization),
                &references,
                &[key].into_iter().collect(),
                operator,
                &listing_coordinate,
                &input.required_capabilities,
                nostr::Timestamp::now().as_secs(),
            )
            .is_none()
        {
            return Err("Pending authority is cancelled, incomplete, misbound or lacks upload-build. Review authority explicitly; retry does not renew or expand it.".to_string().into());
        }
    }
    let provision_step = format!("provision:{}:{}", input.server_url, input.scope);
    let journal = crate::publisher_commands::journal();
    let retained = if let Some(journal) = &journal {
        journal
            .external_checkpoint(
                &provision_step,
                "provision",
                input.server_url,
                serde_json::json!({"scope":input.scope,"operator":input.server_info.pubkey}),
            )
            .await?
    } else {
        None
    };
    let provision = if let Some(outcome) = retained {
        let mut response =
            serde_json::from_value::<arcadestr_core::adp_client::ProvisionResponse>(outcome)
                .map_err(|_| "Invalid retained provisioning response".to_string())?;
        // The POST acknowledgement is historical. Never mistake its embedded
        // attestation for the current operator state during recovery.
        response.attestation = None;
        response
    } else {
        let provision_result = input
            .adp_client
            .provision(input.signer, Some(input.scope))
            .await;
        let provision = match provision_result {
            Ok(provision) => provision,
            Err(error) => {
                if let Some(journal) = &journal {
                    journal.failed(&provision_step, "Provisioning response failed; the remote outcome remains unacknowledged").await?;
                }
                return Err(error.to_string().into());
            }
        };
        if let Some(journal) = &journal {
            journal
                .ack(
                    &provision_step,
                    serde_json::to_value(&provision)
                        .map_err(|_| "Invalid provisioning response".to_string())?,
                )
                .await?;
        }
        provision
    };

    if provision.scope.as_deref() != Some(input.scope) {
        return Err(
            "Provisioning response does not match the original game scope"
                .to_string()
                .into(),
        );
    }

    if let Some(existing) = existing {
        if existing.fulfillment_pubkey != provision.fulfillment_pubkey {
            return Err("Operator changed the pending fulfillment key. Review authority explicitly before retrying.".to_string().into());
        }
        let authorization = input
            .reusable_authorization
            .ok_or_else(|| "Missing pending authority evidence".to_string())?;
        let authorization_event = authorization
            .events
            .first()
            .ok_or_else(|| "Missing authorization root".to_string())?
            .event
            .clone();
        let authorization_event_id = authorization_event.id.to_hex();
        let mut row = existing;
        row.attestation_event_id = provision.attestation_event_id.clone();

        return Ok(ProvisioningDecision::Reused {
            fulfillment_pubkey: row.fulfillment_pubkey.clone(),
            authorization_event_id,
            valid_from: row.valid_from,
            authorization_event: Box::new(authorization_event),
            attestation: provision.attestation.clone().map(Box::new),
            row: Box::new(row),
        });
    }
    let now = now_unix_i64()?;
    let valid_from: u64 = now
        .try_into()
        .map_err(|_| "current time is negative".to_string())?;
    let terms = AuthorizationTerms {
        authorization_id: uuid::Uuid::new_v4().to_string(),
        coordinate: listing_coordinate,
        operator_pubkey: nostr::PublicKey::from_hex(&input.server_info.pubkey)
            .map_err(|_| "operator pubkey is invalid".to_string())?,
        fulfillment_pubkey: nostr::PublicKey::from_hex(&provision.fulfillment_pubkey)
            .map_err(|_| "provisioned fulfillment pubkey is invalid".to_string())?,
        capabilities: input.required_capabilities.clone(),
        valid_from,
    };
    let authorization_builder = build_fulfillment_authorization_event_builder(&terms)
        .map_err(PublishAdpListingError::from)?;
    let authorization_event = input
        .signer
        .sign_event(authorization_builder.build(input.developer_pubkey))
        .await
        .map_err(|error| PublishAdpListingError::Signer {
            message: error.to_string(),
        })?;
    let authorization_event_id = authorization_event.id.to_hex();
    let signed_authority =
        arcadestr_core::authorization::parse_authorization_event(&authorization_event)
            .map_err(|_| "Invalid retained authorization".to_string())?;
    if signed_authority.terms.coordinate != terms.coordinate
        || signed_authority.terms.operator_pubkey != terms.operator_pubkey
        || signed_authority.terms.fulfillment_pubkey != terms.fulfillment_pubkey
        || signed_authority.terms.capabilities != terms.capabilities
        || !matches!(
            signed_authority.transition,
            arcadestr_core::authorization::AuthorizationTransition::ActiveRoot
        )
    {
        return Err("Retained authorization does not match the original operator, scope, key and capabilities; recovery did not renew it".to_string().into());
    }
    let row = AdpProvisioning {
        id: format!(
            "{}:{}:{}",
            input.developer_npub, input.server_url, input.scope
        ),
        developer_npub: input.developer_npub.to_string(),
        server_url: input.server_url.to_string(),
        operator_pubkey: input.server_info.pubkey.clone(),
        scope: Some(input.scope.to_string()),
        fulfillment_pubkey: provision.fulfillment_pubkey.clone(),
        attestation_event_id: provision.attestation_event_id.clone(),
        acceptance_event_id: authorization_event_id.clone(),
        authorization_root_event_id: Some(authorization_event_id.clone()),
        authorization_capabilities: terms.capabilities.into_iter().collect(),
        authorization_profile_version: 2,
        valid_from: arcadestr_core::authorization::parse_authorization_event(&authorization_event)
            .map_err(|_| "Invalid retained authorization".to_string())?
            .terms
            .valid_from
            .try_into()
            .map_err(|_| "Authorization time overflow".to_string())?,
        revoked_at: None,
        created_at: now,
    };

    Ok(ProvisioningDecision::Created {
        fulfillment_pubkey: provision.fulfillment_pubkey,
        authorization_event_id,
        valid_from: row.valid_from,
        authorization_event: Box::new(authorization_event),
        attestation: provision.attestation.map(Box::new),
        row: Box::new(row),
    })
}

async fn publish_authorization_once(
    state: &State<'_, AppState>,
    event: &nostr::Event,
    session: &arcadestr_core::settlement_recovery::SessionFence,
) -> Result<(), String> {
    if let Some(journal) = crate::publisher_commands::journal() {
        if journal
            .step(&format!("relay:{}", event.id))
            .await?
            .is_some_and(|step| step.state == "acknowledged")
        {
            session.ensure_current()?;
            return Ok(());
        }
    }
    publish_event(state, event, session).await
}

pub(crate) async fn publish_event(
    state: &State<'_, AppState>,
    event: &nostr::Event,
    session: &arcadestr_core::settlement_recovery::SessionFence,
) -> Result<(), String> {
    let relay_manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = relay_manager.lock().await.clone();
    let journal = crate::publisher_commands::before_publish(event).await?;
    session.ensure_current()?;
    let result = match manager.send_event(event).await {
        Ok(result) => result,
        Err(error) => {
            if let Some(journal) = &journal {
                journal.failed(&format!("relay:{}", event.id), "Relay publication response unavailable; reconcile this exact event before retry").await?;
            }
            return Err(error.to_string());
        }
    };
    if result.success_count == 0 {
        if let Some(journal) = &journal {
            journal
                .failed(
                    &format!("relay:{}", event.id),
                    "No selected relay acknowledged this event",
                )
                .await?;
        }
        return Err("event publish failed on all relays".to_string());
    }
    if let Some(journal) = journal {
        journal
            .ack(
                &format!("relay:{}", event.id),
                serde_json::json!({"accepted":true}),
            )
            .await?;
    }
    Ok(())
}

struct PublicationProgress<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    operation_id: String,
    publisher_npub: String,
    game_coordinate: String,
    session: arcadestr_core::settlement_recovery::SessionFence,
}

fn emit_progress<R: tauri::Runtime>(
    app: &PublicationProgress<R>,
    step: &str,
    status: &str,
    message: Option<String>,
) -> Result<(), String> {
    emit_server_progress(app, step, status, None, message)
}

fn emit_server_progress<R: tauri::Runtime>(
    app: &PublicationProgress<R>,
    step: &str,
    status: &str,
    server_url: Option<String>,
    message: Option<String>,
) -> Result<(), String> {
    app.session.ensure_current()?;
    app.app
        .emit(
            "publish-progress",
            PublishProgressPayload {
                operation_id: app.operation_id.clone(),
                publisher_npub: app.publisher_npub.clone(),
                game_coordinate: app.game_coordinate.clone(),
                step: step.to_string(),
                status: status.to_string(),
                server_url,
                message,
                bytes_uploaded: None,
                total_bytes: None,
            },
        )
        .map_err(|err| err.to_string())
}

fn emit_upload_progress<R: tauri::Runtime>(
    app: &PublicationProgress<R>,
    server_url: &str,
    bytes_uploaded: u64,
    total_bytes: u64,
) -> Result<(), String> {
    app.session.ensure_current()?;
    app.app
        .emit(
            "publish-progress",
            PublishProgressPayload {
                operation_id: app.operation_id.clone(),
                publisher_npub: app.publisher_npub.clone(),
                game_coordinate: app.game_coordinate.clone(),
                step: "upload".to_string(),
                status: "progress".to_string(),
                server_url: Some(server_url.to_string()),
                message: None,
                bytes_uploaded: Some(bytes_uploaded),
                total_bytes: Some(total_bytes),
            },
        )
        .map_err(|err| err.to_string())
}

fn progress_error<R: tauri::Runtime, E: std::fmt::Display>(
    app: &PublicationProgress<R>,
    step: &str,
    err: E,
) -> String {
    let message = err.to_string();
    let _ = emit_progress(app, step, "error", Some(message.clone()));
    message
}

fn now_unix_i64() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    include!("publisher_lifecycle_tests.rs");
    use super::*;

    #[test]
    fn publisher_commercial_capabilities_reject_missing_incompatible_and_wrong_identity_metadata() {
        let operator = nostr::Keys::generate().public_key().to_hex();
        let fixture = serde_json::json!({
            "adp_version":"0.2", "pubkey": operator,
            "event_kinds": {"entitlement_grant":1030,"adp_campaign":1031,
                "commercial_terms":1032,"private_quote":1033,"recovery_evidence":1034},
            "settlement": {"version":1,"split_enabled":true,"terms_event_id":"11".repeat(32),
                "fee":{"type":"none"},"legacy_paid_sales":"allow"}
        });
        let decode = |json| serde_json::from_value::<AdpServerInfo>(json).expect("fixture");
        let server = publisher_terms_server("https://operator.test", decode(fixture.clone()))
            .expect("supported");
        assert_eq!(server.operator_pubkey, operator);
        for (path, value) in [
            ("/settlement/version", serde_json::json!(2)),
            ("/settlement/split_enabled", serde_json::json!(false)),
            ("/event_kinds/commercial_terms", serde_json::json!(1021)),
            ("/event_kinds/private_quote", serde_json::Value::Null),
            ("/event_kinds/recovery_evidence", serde_json::json!(9999)),
            ("/pubkey", serde_json::json!("not-an-operator")),
        ] {
            let mut changed = fixture.clone();
            *changed.pointer_mut(path).expect("path") = value;
            assert!(publisher_terms_server("https://operator.test", decode(changed)).is_err());
        }
        let mut absent = fixture;
        absent["settlement"] = serde_json::Value::Null;
        assert!(publisher_terms_server("https://operator.test", decode(absent)).is_err());
    }
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    use std::path::PathBuf;
    use std::time::Duration;

    use arcadestr_core::adp_client::{AdpClient, AdpServerInfo};
    use arcadestr_core::adp_storage::AdpProvisioningRepository;
    use arcadestr_core::auth::AuthState;
    use arcadestr_core::authorization::{parse_authorization_event, AuthorizationTransition};
    use arcadestr_core::distribution::{
        resolve_install_plan, LaunchType, PackageFormat, PlatformTarget, RelativePath, WorkingDir,
    };
    use arcadestr_core::http_client::{HttpClient, ReqwestHttpClient};
    use arcadestr_core::marketplace_cache::MarketplaceCache;
    use arcadestr_core::nip05_validator::Nip05Validator;
    use arcadestr_core::nip46::AppSignerState;
    use arcadestr_core::nostr::{EventDeduplicator, NostrClient};
    use arcadestr_core::profile_fetcher::ProfileFetcher;
    use arcadestr_core::relay_cache::RelayCache;
    use arcadestr_core::relay_hints::RelayHints;
    use arcadestr_core::relay_manager::RelayManagerConfig;
    use arcadestr_core::signers::{LocalSigner, NostrSigner};

    use arcadestr_core::storage::Database;
    use arcadestr_core::subscriptions::SubscriptionRegistry;
    use arcadestr_core::user_cache::UserCache;
    use nostr::{nips::nip19::ToBech32, EventBuilder, Keys, Kind, Tag, TagKind, Timestamp};

    struct CountingSigner {
        keys: Keys,
        calls: AtomicUsize,
    }

    struct DelayedPublicationSigner {
        keys: Keys,
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
        reject: bool,
    }
    #[async_trait::async_trait]
    impl NostrSigner for DelayedPublicationSigner {
        async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
            Ok(self.keys.public_key())
        }
        async fn sign_event(
            &self,
            unsigned: nostr::UnsignedEvent,
        ) -> Result<nostr::Event, SignerError> {
            self.started.notify_one();
            self.release.notified().await;
            if self.reject {
                return Err(SignerError::SigningFailed("rejected".into()));
            }
            unsigned
                .sign(&self.keys)
                .await
                .map_err(|e| SignerError::SigningFailed(e.to_string()))
        }
    }

    #[tokio::test]
    async fn publication_signer_fences_delayed_results_and_no_later_side_effect() {
        use arcadestr_core::settlement_recovery::RecoveryCoordinator;
        for transition in ["switch", "roundtrip", "logout", "reconnect", "reject"] {
            let coordinator = RecoveryCoordinator::default();
            let delayed = Arc::new(DelayedPublicationSigner {
                keys: Keys::generate(),
                started: Default::default(),
                release: Default::default(),
                reject: transition == "reject",
            });
            let account = delayed.keys.public_key().to_hex();
            coordinator.activate(Some(account.clone()));
            let fenced = PublicationSigner {
                signer: delayed.clone(),
                session: coordinator.session_fence().expect("session"),
            };
            let unsigned = EventBuilder::new(Kind::Custom(30402), "offline fixture")
                .build(delayed.keys.public_key());
            let task = tokio::spawn(async move { fenced.sign_event(unsigned).await });
            delayed.started.notified().await;
            match transition {
                "switch" => coordinator.activate(Some("bob".into())),
                "roundtrip" => {
                    coordinator.activate(Some("bob".into()));
                    coordinator.activate(Some(account));
                }
                "logout" => coordinator.activate(None),
                "reconnect" => coordinator.activate(Some(account)),
                _ => {}
            }
            delayed.release.notify_one();
            assert!(
                task.await.expect("task").is_err(),
                "{transition} must not expose a signed result to publication/provision/upload"
            );
        }
    }

    #[tokio::test]
    async fn publication_signer_timeout_exposes_no_signed_result() {
        let coordinator = arcadestr_core::settlement_recovery::RecoveryCoordinator::default();
        let delayed = Arc::new(DelayedPublicationSigner {
            keys: Keys::generate(),
            started: Default::default(),
            release: Default::default(),
            reject: false,
        });
        coordinator.activate(Some(delayed.keys.public_key().to_hex()));
        let signer = PublicationSigner {
            signer: delayed.clone(),
            session: coordinator.session_fence().expect("session"),
        };
        let unsigned = EventBuilder::new(Kind::Custom(30402), "offline fixture")
            .build(delayed.keys.public_key());
        let error = signer
            .sign_with_timeout(unsigned, Duration::from_millis(10))
            .await
            .expect_err("timeout");
        assert!(error.to_string().contains("timed out"));
    }

    impl CountingSigner {
        fn new() -> Self {
            Self {
                keys: Keys::generate(),
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(AtomicOrdering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl NostrSigner for CountingSigner {
        async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
            Ok(self.keys.public_key())
        }

        async fn sign_event(
            &self,
            unsigned: nostr::UnsignedEvent,
        ) -> Result<nostr::Event, SignerError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            unsigned
                .sign(&self.keys)
                .await
                .map_err(|error| SignerError::SigningFailed(error.to_string()))
        }
    }

    fn final_listing_unsigned(
        signer: &CountingSigner,
        tags: Vec<Vec<&str>>,
    ) -> nostr::UnsignedEvent {
        let tags = tags
            .into_iter()
            .map(|tag| Tag::parse(tag).expect("fixture tag should parse"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(30402), "listing")
            .tags(tags)
            .build(signer.keys.public_key())
    }

    fn valid_final_launch(profile_id: &'static str) -> Vec<&'static str> {
        vec!["launch", profile_id, "bin/game", ".", "[]", "game", "Play"]
    }

    async fn assert_rejected_before_signing(
        tags: Vec<Vec<&str>>,
        expected: arcadestr_core::adp_publish_contract::AdpPublishValidationViolation,
    ) {
        let signer = CountingSigner::new();
        let error = validate_and_sign_adp_listing(&signer, final_listing_unsigned(&signer, tags))
            .await
            .expect_err("malformed final listing must be rejected");

        assert!(matches!(
            error,
            PublishAdpListingError::CoreValidation { violation, .. } if violation == expected
        ));
        assert_eq!(signer.calls(), 0);
    }

    #[tokio::test]
    async fn malformed_final_listing_invariants_never_reach_the_signer() {
        use arcadestr_core::adp_publish_contract::AdpPublishValidationViolation as Violation;

        assert_rejected_before_signing(
            vec![vec!["package", "zip"]],
            Violation::MissingGameLaunchProfile,
        )
        .await;
        assert_rejected_before_signing(
            vec![valid_final_launch("play")],
            Violation::LaunchPackageCount { actual: 0 },
        )
        .await;
        assert_rejected_before_signing(
            vec![
                vec!["package", "zip"],
                valid_final_launch("play"),
                vec!["launch_default", "play"],
            ],
            Violation::MissingLaunchPlatform,
        )
        .await;
        assert_rejected_before_signing(
            vec![
                vec!["package", "zip"],
                vec!["platform", "linux-x86_64"],
                valid_final_launch("play"),
                valid_final_launch("play"),
            ],
            Violation::DuplicateLaunchProfileId {
                profile_id: ProfileId::parse("play").expect("valid profile ID"),
            },
        )
        .await;
    }

    #[tokio::test]
    async fn valid_three_profile_and_download_only_listings_sign_once_each() {
        let signer = CountingSigner::new();
        let valid = final_listing_unsigned(
            &signer,
            vec![
                vec!["package", "zip"],
                vec!["platform", "linux-x86_64"],
                valid_final_launch("play"),
                valid_final_launch("safe"),
                valid_final_launch("server"),
                vec!["launch_default", "play"],
            ],
        );
        validate_and_sign_adp_listing(&signer, valid)
            .await
            .expect("valid launch listing should sign");
        assert_eq!(signer.calls(), 1);

        let download_only = final_listing_unsigned(
            &signer,
            vec![vec!["d", "download-only"], vec!["platform", "linux-x86_64"]],
        );
        validate_and_sign_adp_listing(&signer, download_only)
            .await
            .expect("download-only listing should sign");
        assert_eq!(signer.calls(), 2);
    }

    #[test]
    fn new_edit_and_retry_listing_publications_share_one_signing_pipeline() {
        let source = include_str!("adp_commands.rs");
        let pipeline = source
            .split("async fn publish_adp_listing_pipeline_inner")
            .nth(1)
            .and_then(|source| source.split("async fn validate_current_attestation").next())
            .expect("listing pipeline should have stable structural boundaries");

        assert_eq!(
            pipeline.matches("validate_and_sign_adp_listing(").count(),
            1
        );
        assert!(!pipeline.contains(".sign_event("));
        assert!(pipeline.contains("request.existing_event_id.as_deref()"));
        assert!(pipeline.contains("request.pending_d_tag.as_deref()"));
    }

    #[test]
    fn expected_publisher_must_match_resolved_signer() {
        let expected = nostr::Keys::generate();
        let other = nostr::Keys::generate();
        let expected_npub = expected
            .public_key()
            .to_bech32()
            .expect("test npub should encode");

        assert_eq!(
            verify_expected_publisher(&expected_npub, expected.public_key()),
            Ok(())
        );
        let error = verify_expected_publisher(&expected_npub, other.public_key())
            .expect_err("a changed account must be rejected");
        assert!(error.contains("Active account changed"));
        assert!(error.contains("expected publisher does not match signer pubkey"));
    }

    #[test]
    fn new_publication_generates_a_version_4_uuid_identifier() {
        let identifier =
            resolve_new_game_identifier(None, None).expect("a first publication should mint an id");

        let game_id = identifier
            .generated_game_id()
            .expect("a first publication is a generated identity");
        assert_eq!(game_id.get_version(), Some(uuid::Version::Random));
        assert_eq!(game_id.get_variant(), uuid::Variant::RFC4122);
    }

    #[test]
    fn generated_identifier_renders_canonical_lowercase_hyphenated_text() {
        let identifier =
            resolve_new_game_identifier(None, None).expect("a first publication should mint an id");

        let d_tag = identifier.d_tag();
        assert_eq!(d_tag.len(), 36);
        assert_eq!(
            d_tag
                .match_indices('-')
                .map(|(index, _)| index)
                .collect::<Vec<_>>(),
            vec![8, 13, 18, 23]
        );
        assert!(d_tag.chars().all(|character| character == '-'
            || character.is_ascii_lowercase()
            || character.is_ascii_digit()));
        assert_eq!(
            d_tag,
            identifier
                .generated_game_id()
                .expect("generated identity")
                .to_string()
        );
    }

    #[test]
    fn independent_new_publications_generate_different_identifiers() {
        let first =
            resolve_new_game_identifier(None, None).expect("first publication should mint an id");
        let second =
            resolve_new_game_identifier(None, None).expect("second publication should mint an id");

        assert_ne!(first.d_tag(), second.d_tag());
    }

    #[test]
    fn new_publication_rejects_a_caller_supplied_identifier() {
        let error = resolve_new_game_identifier(Some("publisher-chosen-slug"), None)
            .expect_err("a caller-supplied identifier must not be honoured");

        assert!(error.contains("caller-supplied identifier"));
    }

    #[test]
    fn a_retry_resumes_the_identifier_the_failed_attempt_minted() {
        let minted = resolve_new_game_identifier(None, None)
            .expect("the first attempt should mint an id")
            .d_tag();

        let resumed = resolve_new_game_identifier(None, Some(&minted))
            .expect("a retry should resume the pending publication");

        assert_eq!(resumed.d_tag(), minted);
        assert_eq!(
            resumed.generated_game_id().map(|id| id.to_string()),
            Some(minted),
            "a resumed publication still owns the identity it minted"
        );
    }

    #[test]
    fn a_pending_identifier_this_app_did_not_mint_is_refused() {
        // The pending channel must not become a way to choose a coordinate.
        for smuggled in [
            "publisher-chosen-slug",
            "not-a-uuid-at-all",
            // A valid UUID, but not the v4 shape this command mints.
            "000001f5-5cb4-21ef-9a00-325096b39f47",
            // Canonical rendering only: no uppercase, braces or urn prefix.
            "F47AC10B-58CC-4372-A567-0E02B2C3D479",
            "{f47ac10b-58cc-4372-a567-0e02b2c3d479}",
            "urn:uuid:f47ac10b-58cc-4372-a567-0e02b2c3d479",
        ] {
            let error = resolve_new_game_identifier(None, Some(smuggled))
                .expect_err("a pending identifier this app did not mint must be refused");

            assert!(
                error.contains("not one this app minted"),
                "{smuggled} was refused for the wrong reason: {error}"
            );
        }
    }

    #[test]
    fn an_empty_pending_identifier_mints_a_fresh_one() {
        let identifier = resolve_new_game_identifier(None, Some(""))
            .expect("an empty carry-over should behave like a first attempt");

        assert!(identifier.generated_game_id().is_some());
    }

    fn listing_event(publisher: &Keys, d_tag: &str) -> nostr::Event {
        EventBuilder::new(Kind::Custom(30402), "")
            .tags([Tag::identifier(d_tag)])
            .sign_with_keys(publisher)
            .expect("listing event should sign")
    }

    #[test]
    fn a_pending_coordinate_the_relays_confirm_is_free_is_resumed() {
        let publisher = Keys::generate();
        let pending = resolve_new_game_identifier(None, None)
            .expect("the first attempt should mint an id")
            .d_tag();
        // Another game of the same publisher answers the query, but not this
        // coordinate; every relay that was asked replied.
        let query = Ok(CoordinateQuery {
            events: vec![listing_event(&publisher, "a-different-game")],
            silent_relays: 0,
        });

        let occupancy = coordinate_occupancy(publisher.public_key(), &pending, query);

        assert_eq!(occupancy, CoordinateOccupancy::ConfirmedAbsent);
        assert_eq!(
            pending_d_tag_to_resume(&pending, occupancy),
            Ok(Some(pending.clone()))
        );
    }

    #[test]
    fn a_pending_coordinate_that_is_already_live_is_refused() {
        let publisher = Keys::generate();
        let pending = resolve_new_game_identifier(None, None)
            .expect("an id should mint")
            .d_tag();
        // The publisher's own published game sits at this coordinate. Resuming
        // would replace it: kind:30402 is parameterized replaceable.
        let query = Ok(CoordinateQuery {
            events: vec![listing_event(&publisher, &pending)],
            silent_relays: 0,
        });

        let occupancy = coordinate_occupancy(publisher.public_key(), &pending, query);

        assert_eq!(occupancy, CoordinateOccupancy::Occupied);
        let error = pending_d_tag_to_resume(&pending, occupancy)
            .expect_err("a live coordinate must not be published over");
        assert_eq!(error, RESUMED_COORDINATE_IS_LIVE);
        assert_ne!(
            error,
            resume_pending_game_identifier("not-a-resume-token")
                .expect_err("the shape refusal should still exist"),
            "the frontend must be able to tell the two refusals apart"
        );
    }

    #[test]
    fn an_unanswered_coordinate_query_mints_instead_of_resuming() {
        let publisher = Keys::generate();
        let pending = resolve_new_game_identifier(None, None)
            .expect("an id should mint")
            .d_tag();

        // Nothing came back at all.
        assert_eq!(
            coordinate_occupancy(
                publisher.public_key(),
                &pending,
                Err("coordinate query failed".to_string()),
            ),
            CoordinateOccupancy::Indeterminate
        );
        // Some relays answered and none held the coordinate, but others never
        // did, so absence is not established.
        let partial = coordinate_occupancy(
            publisher.public_key(),
            &pending,
            Ok(CoordinateQuery {
                events: Vec::new(),
                silent_relays: 1,
            }),
        );
        assert_eq!(partial, CoordinateOccupancy::Indeterminate);

        // Falling back to a fresh identity is the safe direction, so this is
        // not an error: the publication proceeds under a new coordinate.
        assert_eq!(pending_d_tag_to_resume(&pending, partial), Ok(None));
        let identifier = resolve_new_game_identifier(None, None)
            .expect("the fallback should mint a fresh identity");
        assert_ne!(identifier.d_tag(), pending);
    }

    #[test]
    fn one_relay_holding_the_coordinate_outranks_the_silent_ones() {
        let publisher = Keys::generate();
        let pending = resolve_new_game_identifier(None, None)
            .expect("an id should mint")
            .d_tag();

        // Presence is positive evidence: it settles the question even though
        // coverage was incomplete.
        assert_eq!(
            coordinate_occupancy(
                publisher.public_key(),
                &pending,
                Ok(CoordinateQuery {
                    events: vec![listing_event(&publisher, &pending)],
                    silent_relays: 4,
                }),
            ),
            CoordinateOccupancy::Occupied
        );
    }

    #[test]
    fn another_publishers_listing_never_occupies_this_publishers_coordinate() {
        let publisher = Keys::generate();
        let stranger = Keys::generate();
        let pending = resolve_new_game_identifier(None, None)
            .expect("an id should mint")
            .d_tag();

        // Coordinates are namespaced by publisher, so a relay answering with
        // someone else's listing at the same `d` tag proves nothing about this
        // publisher's coordinate.
        assert_eq!(
            coordinate_occupancy(
                publisher.public_key(),
                &pending,
                Ok(CoordinateQuery {
                    events: vec![listing_event(&stranger, &pending)],
                    silent_relays: 0,
                }),
            ),
            CoordinateOccupancy::ConfirmedAbsent
        );
    }

    #[test]
    fn listing_edits_preserve_a_legacy_non_uuid_identifier() {
        let identifier = resolve_existing_game_identifier(Some("my-game-v1"), Some("my-game-v1"))
            .expect("a legacy identifier must remain publishable");

        assert_eq!(identifier, GameIdentifier::Existing("my-game-v1".into()));
        assert_eq!(identifier.d_tag(), "my-game-v1");
        assert_eq!(identifier.generated_game_id(), None);
    }

    #[test]
    fn listing_edits_preserve_a_uuid_identifier() {
        let existing = Uuid::new_v4().to_string();

        let identifier = resolve_existing_game_identifier(Some(&existing), Some(&existing))
            .expect("a UUID identifier must be preserved verbatim");

        assert_eq!(identifier.d_tag(), existing);
        assert_eq!(
            identifier.generated_game_id(),
            None,
            "an edit must never mint a replacement identifier"
        );
    }

    #[test]
    fn listing_edits_reject_an_attempted_identifier_change() {
        let error = resolve_existing_game_identifier(Some("renamed-game"), Some("my-game-v1"))
            .expect_err("changing the coordinate of a published listing must fail");

        assert!(error.contains("cannot change"));
    }

    #[test]
    fn listing_edits_require_an_authoritative_identifier() {
        let error = resolve_existing_game_identifier(Some("my-game-v1"), None)
            .expect_err("an edit without an authoritative d tag must fail");

        assert!(error.contains("no identifier"));
    }

    #[test]
    fn authoritative_identifier_is_read_from_the_fetched_listing() {
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(30402), "")
            .tags([nostr::Tag::identifier("my-game-v1")])
            .sign_with_keys(&keys)
            .expect("test listing event should sign");

        assert_eq!(listing_event_d_tag(&event), Some("my-game-v1"));
    }

    #[test]
    fn every_coordinate_dependent_input_shares_one_generated_identifier() {
        let publisher = nostr::Keys::generate().public_key();
        let game = GameCoordinate::new(
            publisher,
            resolve_new_game_identifier(None, None).expect("first publication should mint an id"),
        );

        // The scope handed to provisioning and authorization, the listing `d`
        // tag, the propagation/upload coordinate and the returned result are all
        // rendered from this one value.
        let d_tag = game.d_tag();
        let coordinate = game.to_coordinate_string();
        assert_eq!(
            coordinate,
            format!("30402:{}:{}", publisher.to_hex(), d_tag)
        );
        assert_eq!(game.to_coordinate_string(), coordinate);
        assert_eq!(game.d_tag(), d_tag);
        assert_eq!(
            game.generated_game_id().map(|id| id.to_string()),
            Some(d_tag)
        );

        // PublishAdpListingRequest deliberately has no caller-supplied `d_tag`.
        // Existing and pending identities cross separate, behaviorally tested gates
        // before this single resolved coordinate reaches downstream publication.
    }

    #[test]
    fn a_recovered_coordinate_reuses_the_published_identifier_on_retry() {
        let publisher = nostr::Keys::generate().public_key();
        let published = GameCoordinate::new(
            publisher,
            resolve_new_game_identifier(None, None).expect("first publication should mint an id"),
        );
        let published_d_tag = published.d_tag();

        // A retry after a failure that followed listing publication arrives as
        // an edit carrying the recovered identifier.
        let retried = GameCoordinate::new(
            publisher,
            resolve_existing_game_identifier(Some(&published_d_tag), Some(&published_d_tag))
                .expect("the recovered identifier should be accepted"),
        );

        assert_eq!(retried.d_tag(), published_d_tag);
        assert_eq!(
            retried.to_coordinate_string(),
            published.to_coordinate_string()
        );
        assert_eq!(
            retried.generated_game_id(),
            None,
            "a retry must not mint a second coordinate"
        );
    }

    #[test]
    fn the_coordinate_is_published_before_any_step_that_can_fail_after_it() {
        let source = include_str!("adp_commands.rs");
        let coordinate_handoff = source
            .find(r#"emit_progress(&app, "listing-coordinate", "ok""#)
            .expect("the published coordinate must be handed to the caller");
        let propagation = source
            .find(r#"emit_progress(&app, "confirm-propagation", "pending""#)
            .expect("propagation confirmation should follow publication");
        let upload = source
            .find(r#"emit_server_progress(&app, "upload", "pending""#)
            .expect("uploads should follow publication");

        assert!(coordinate_handoff < propagation);
        assert!(coordinate_handoff < upload);
    }

    #[test]
    fn the_minted_identity_is_handed_over_before_any_step_that_can_fail() {
        let source = include_str!("adp_commands.rs");
        let pending_handoff = source
            .find(r#""pending-coordinate","#)
            .expect("the minted identity must be handed to the caller");
        // Every step that provisions, hashes or publishes under this identity
        // can fail, and each failure sends the publisher back to a retry.
        for later in [
            r#"emit_progress(&app, "hash-file", "pending""#,
            r#"emit_progress(&app, "check-operator", "pending""#,
            r#"emit_progress(&app, "provision", "pending""#,
            r#"emit_progress(&app, "publish-listing", "pending""#,
        ] {
            let step = source
                .find(later)
                .unwrap_or_else(|| panic!("{later} should exist"));
            assert!(
                pending_handoff < step,
                "{later} runs before the caller is told what identity it runs under"
            );
        }
    }

    #[test]
    fn ordinary_listing_edits_preserve_unmanaged_tags() {
        assert!(is_publish_form_tag(&["title".into(), "Game".into()]));
        assert!(is_publish_form_tag(&[
            "fulfillment_pubkey".into(),
            "key".into()
        ]));
        assert!(!is_publish_form_tag(&[
            "summary".into(),
            "Short description".into()
        ]));
        assert!(!is_publish_form_tag(&["status".into(), "active".into()]));
    }

    #[test]
    fn campaign_pointers_are_owned_by_the_publish_form() {
        // Campaign pointers are republished from the form on every replacement,
        // so they must be filtered out of the preserved tags. Dropping them from
        // the managed set would resurrect stale pointers alongside current ones.
        assert!(FORM_MANAGED_LISTING_TAGS.contains(&"campaign"));
        assert!(is_publish_form_tag(&["campaign".into(), "root".into()]));
    }

    #[test]
    fn launch_default_is_replaced_instead_of_preserved() {
        let existing = vec![
            vec![
                "launch_default".to_string(),
                "stale-linux".to_string(),
                "linux-x86_64".to_string(),
            ],
            vec![
                "launch_default".to_string(),
                "stale-windows".to_string(),
                "windows-x86_64".to_string(),
            ],
            vec!["summary".to_string(), "Keep me".to_string()],
        ];

        assert!(FORM_MANAGED_LISTING_TAGS.contains(&"launch_default"));
        assert_eq!(
            preserved_listing_tags(&existing),
            vec![vec!["summary".to_string(), "Keep me".to_string()]]
        );
    }

    #[test]
    fn one_managed_tag_definition_covers_every_tag_the_form_republishes() {
        // Every tag name the core listing builder emits from form input, paired
        // with the field that drives it. If the builder gains a form-driven tag,
        // this list and FORM_MANAGED_LISTING_TAGS must gain it too, or the
        // replacement listing will carry the stale value as well.
        let builder_emits_from_form = [
            "d",
            "title",
            "price",
            "t",
            "image",
            "acquisition",
            "server",
            "file_hash",
            "version",
            "lud16",
            "package",
            "launch",
            "launch_default",
            "platform",
            "campaign",
            "nip94",
        ];
        for tag in builder_emits_from_form {
            assert!(
                FORM_MANAGED_LISTING_TAGS.contains(&tag),
                "`{tag}` is republished from form state and must be managed"
            );
        }
        assert!(
            !FORM_MANAGED_LISTING_TAGS.contains(&FULFILLMENT_AUTHORIZATION_TAG),
            "authorizations are owned only by publications that issue one"
        );

        let source = include_str!("adp_commands.rs");
        assert_eq!(
            source
                .matches(concat!("FORM_MANAGED_", "LISTING_TAGS: &[&str]"))
                .count(),
            1,
            "edits and recovered retries must share one managed-tag definition"
        );
    }

    const RETRY_TEST_SHA256: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn listing_event_tags(event: &nostr::Event, name: &str) -> Vec<Vec<String>> {
        event
            .tags
            .iter()
            .map(|tag| tag.clone().to_vec())
            .filter(|values| values.first().map(String::as_str) == Some(name))
            .collect()
    }

    /// Tags of a listing that was published, as the replacement path reads them
    /// back off the relay.
    fn published_listing_tags(campaigns: &[&str]) -> Vec<Vec<String>> {
        let mut tags = vec![
            vec!["d".into(), "2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into()],
            vec!["title".into(), "Half-published Game".into()],
            vec!["price".into(), "5000".into(), "sat".into()],
            vec!["t".into(), "game".into()],
            vec!["acquisition".into(), "gated".into()],
            vec!["platform".into(), "windows-x86_64".into()],
            vec!["lud16".into(), "stale@example.com".into()],
            vec!["nip94".into(), "a".repeat(64)],
            // Not managed by the publish form: must survive untouched.
            vec!["summary".into(), "Written elsewhere".into()],
            vec!["status".into(), "active".into()],
        ];
        for campaign in campaigns {
            tags.push(vec!["campaign".into(), (*campaign).to_string()]);
        }
        tags
    }

    fn retry_listing_input(
        existing_tags: &[Vec<String>],
        campaigns: Vec<(String, Option<String>)>,
    ) -> AdpListingInput {
        AdpListingInput {
            d_tag: "2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b".into(),
            title: "Renamed Game".into(),
            description: "Current form description".into(),
            price_sats: 9000,
            lud16: Some("current@example.com".into()),
            tags: vec!["roguelike".into()],
            images: vec!["https://example.com/cover.png".into()],
            servers: Vec::new(),
            file_hash: None,
            version: None,
            fulfillment_authorizations: Vec::new(),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Public,
            platforms: vec!["linux-x86_64".into()],
            campaigns,
            nip94_event_id: None,
            preserved_tags: preserved_listing_tags(existing_tags),
        }
    }

    fn retry_listing_event(input: &AdpListingInput) -> nostr::Event {
        build_adp_listing_event_builder(input)
            .expect("the replacement listing should build")
            .sign_with_keys(&Keys::generate())
            .expect("the replacement listing should sign")
    }

    #[test]
    fn a_campaign_pointer_added_after_partial_failure_is_published() {
        let existing = published_listing_tags(&[]);
        let added = "b".repeat(64);

        let event = retry_listing_event(&retry_listing_input(
            &existing,
            vec![(added.clone(), Some("wss://relay.example.com".into()))],
        ));

        assert_eq!(
            listing_event_tags(&event, "campaign"),
            vec![vec![
                "campaign".to_string(),
                added,
                "wss://relay.example.com".to_string()
            ]]
        );
    }

    #[test]
    fn a_campaign_pointer_removed_after_partial_failure_is_dropped() {
        let existing = published_listing_tags(&["b".repeat(64).as_str()]);

        let event = retry_listing_event(&retry_listing_input(&existing, Vec::new()));

        assert!(
            listing_event_tags(&event, "campaign").is_empty(),
            "a pointer removed in the form must not return through preserved tags"
        );
    }

    #[test]
    fn replacing_campaign_pointers_retains_no_stale_pointer() {
        let stale = "b".repeat(64);
        let replacement = "c".repeat(64);
        let existing = published_listing_tags(&[stale.as_str()]);

        let event = retry_listing_event(&retry_listing_input(
            &existing,
            vec![(replacement.clone(), None)],
        ));

        assert_eq!(
            listing_event_tags(&event, "campaign"),
            vec![vec!["campaign".to_string(), replacement]]
        );
    }

    #[test]
    fn retrying_without_campaign_changes_keeps_the_intended_pointers() {
        let pointer = "b".repeat(64);
        let existing = published_listing_tags(&[pointer.as_str()]);

        let event = retry_listing_event(&retry_listing_input(
            &existing,
            vec![(pointer.clone(), None)],
        ));

        assert_eq!(
            listing_event_tags(&event, "campaign"),
            vec![vec!["campaign".to_string(), pointer]],
            "an unchanged pointer must appear exactly once"
        );
    }

    #[test]
    fn unknown_unmanaged_tags_survive_a_retry() {
        let existing = published_listing_tags(&[]);

        let event = retry_listing_event(&retry_listing_input(&existing, Vec::new()));

        assert_eq!(
            listing_event_tags(&event, "summary"),
            vec![vec!["summary".to_string(), "Written elsewhere".to_string()]]
        );
        assert_eq!(
            listing_event_tags(&event, "status"),
            vec![vec!["status".to_string(), "active".to_string()]]
        );
    }

    #[test]
    fn form_managed_tags_are_never_duplicated_through_preserved_tags() {
        let existing = published_listing_tags(&["b".repeat(64).as_str()]);

        let event = retry_listing_event(&retry_listing_input(
            &existing,
            vec![("c".repeat(64), None)],
        ));

        for tag in FORM_MANAGED_LISTING_TAGS {
            let occurrences = listing_event_tags(&event, tag);
            match *tag {
                // `t` legitimately repeats: the mandatory game topic plus the
                // form's own topics.
                "t" => assert!(occurrences.len() <= 2),
                // Target-scoped defaults legitimately repeat once per profile.
                "launch_default" => {}
                _ => assert!(
                    occurrences.len() <= 1,
                    "`{tag}` appeared {} times in the replacement listing",
                    occurrences.len()
                ),
            }
        }
        assert!(listing_event_tags(&event, "nip94").is_empty());
    }

    #[test]
    fn current_form_pricing_overrides_the_partially_published_listing() {
        let existing = published_listing_tags(&[]);
        let mut input = retry_listing_input(&existing, Vec::new());
        input.acquisition = arcadestr_core::marketplace::AcquisitionPolicy::Gated;

        let event = retry_listing_event(&input);

        assert_eq!(
            listing_event_tags(&event, "price"),
            vec![vec![
                "price".to_string(),
                "9000".to_string(),
                "sat".to_string()
            ]]
        );
        assert_eq!(
            listing_event_tags(&event, "lud16"),
            vec![vec!["lud16".to_string(), "current@example.com".to_string()]]
        );
        assert_eq!(
            listing_event_tags(&event, "title"),
            vec![vec!["title".to_string(), "Renamed Game".to_string()]]
        );
    }

    #[test]
    fn current_form_acquisition_policy_overrides_the_partially_published_listing() {
        let existing = published_listing_tags(&[]);

        let event = retry_listing_event(&retry_listing_input(&existing, Vec::new()));

        assert_eq!(
            listing_event_tags(&event, "acquisition"),
            vec![vec!["acquisition".to_string(), "public".to_string()]]
        );
    }

    #[test]
    fn current_form_platform_and_fulfillment_fields_override_preserved_counterparts() {
        let existing = published_listing_tags(&[]);
        let mut input = retry_listing_input(&existing, Vec::new());
        input.servers = vec!["https://dist.example.com".into()];
        input.file_hash = Some(RETRY_TEST_SHA256.into());
        input.version = Some("1.2.3".into());

        let event = retry_listing_event(&input);

        assert_eq!(
            listing_event_tags(&event, "platform"),
            vec![vec!["platform".to_string(), "linux-x86_64".to_string()]],
            "the stale windows platform must not survive"
        );
        assert_eq!(
            listing_event_tags(&event, "server"),
            vec![vec![
                "server".to_string(),
                "https://dist.example.com".to_string()
            ]]
        );
        assert_eq!(
            listing_event_tags(&event, "file_hash"),
            vec![vec!["file_hash".to_string(), RETRY_TEST_SHA256.to_string()]]
        );
        assert_eq!(
            listing_event_tags(&event, "version"),
            vec![vec!["version".to_string(), "1.2.3".to_string()]]
        );
    }

    #[test]
    fn an_existing_authorization_survives_an_edit_that_issues_none() {
        let mut existing = published_listing_tags(&[]);
        existing.push(vec![
            FULFILLMENT_AUTHORIZATION_TAG.to_string(),
            "root-event".into(),
            "delegate-key".into(),
        ]);

        let preserved = preserved_listing_tags(&existing);
        assert!(preserved
            .iter()
            .any(|values| is_fulfillment_authorization_tag(values)));

        // Adding authority can never erase unrelated or historical siblings.
        assert_eq!(
            preserved
                .iter()
                .filter(|values| is_fulfillment_authorization_tag(values))
                .count(),
            1
        );
    }

    #[test]
    fn a_retry_republishes_the_same_generated_identity() {
        let existing = published_listing_tags(&[]);
        let published_d_tag = "2f9a1c34-5b6d-4e7f-8a9b-0c1d2e3f4a5b";

        let identifier =
            resolve_existing_game_identifier(Some(published_d_tag), Some(published_d_tag))
                .expect("the recovered identity should be accepted");
        let event = retry_listing_event(&retry_listing_input(&existing, Vec::new()));

        assert_eq!(identifier.d_tag(), published_d_tag);
        assert_eq!(identifier.generated_game_id(), None);
        assert_eq!(
            listing_event_tags(&event, "d"),
            vec![vec!["d".to_string(), published_d_tag.to_string()]]
        );
    }

    #[test]
    fn a_newer_authoritative_listing_still_conflicts_on_retry() {
        let keys = Keys::generate();
        let published = signed_listing_event(&keys, "first attempt", 1_000);
        let newer_remote = signed_listing_event(&keys, "someone else replaced it", 2_000);

        assert_eq!(
            ensure_expected_listing_event(&published, &published.id.to_hex(), "conflict"),
            Ok(())
        );
        assert_eq!(
            ensure_expected_listing_event(&newer_remote, &published.id.to_hex(), "conflict"),
            Err("conflict".to_string()),
            "a newer remote replacement must not be silently overwritten"
        );
    }

    #[test]
    fn a_legacy_identifier_still_edits_normally() {
        let mut existing = published_listing_tags(&[]);
        existing[0] = vec!["d".into(), "my-game-v1".into()];

        let identifier = resolve_existing_game_identifier(Some("my-game-v1"), Some("my-game-v1"))
            .expect("legacy identifiers remain editable");
        let mut input = retry_listing_input(&existing, Vec::new());
        input.d_tag = identifier.d_tag();

        let event = retry_listing_event(&input);

        assert_eq!(
            listing_event_tags(&event, "d"),
            vec![vec!["d".to_string(), "my-game-v1".to_string()]]
        );
    }

    #[test]
    fn timed_acquisition_wire_format_deserializes_into_core_policy() {
        let policy: arcadestr_core::marketplace::AcquisitionPolicy = serde_json::from_value(
            serde_json::json!({ "TimedAccess": { "starts_at": 100, "ends_at": 200 } }),
        )
        .expect("app timed policy payload should deserialize");

        assert_eq!(
            policy,
            arcadestr_core::marketplace::AcquisitionPolicy::TimedAccess {
                starts_at: 100,
                ends_at: 200,
            }
        );
    }
    use serde_json::json;
    use tauri::Manager;
    use tokio::sync::{Mutex as AsyncMutex, RwLock};

    #[tokio::test]
    async fn restored_nip46_client_signer_is_used_when_legacy_auth_is_empty() {
        let keys = Keys::generate();
        let expected_pubkey = keys.public_key();
        let client = nostr_sdk::Client::new(keys);
        let signer_state = Arc::new(AsyncMutex::new(AppSignerState {
            active_client: Some(client),
            ..AppSignerState::new()
        }));

        let resolved = resolve_active_signer(&signer_state, &AuthState::new())
            .await
            .expect("active NIP-46 signer should be resolved");

        assert_eq!(
            resolved
                .get_public_key()
                .await
                .expect("resolved signer should expose public key"),
            expected_pubkey
        );
    }

    fn signed_listing_event(keys: &Keys, content: &str, created_at: u64) -> nostr::Event {
        EventBuilder::new(Kind::Custom(30402), content)
            .tags([Tag::custom(TagKind::d(), ["same-coordinate"])])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("listing event should sign")
    }

    fn launch_tag(profile_id: &str, executable: &str, display_name: &str) -> Tag {
        Tag::parse([
            "launch",
            profile_id,
            executable,
            ".",
            "[]",
            "game",
            display_name,
        ])
        .expect("launch tag should parse")
    }

    fn authoritative_listing_event(
        keys: &Keys,
        kind: Kind,
        d_tag: &str,
        mut metadata_tags: Vec<Tag>,
    ) -> nostr::Event {
        let mut tags = vec![Tag::parse(["d", d_tag]).expect("d tag should parse")];
        tags.append(&mut metadata_tags);
        EventBuilder::new(kind, "authoritative listing")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("authoritative listing should sign")
    }

    fn authoritative_request(
        keys: &Keys,
        d_tag: &str,
        expected_event_id: String,
    ) -> LoadAuthoritativeAdpEditMetadataRequest {
        LoadAuthoritativeAdpEditMetadataRequest {
            expected_publisher_npub: keys
                .public_key()
                .to_bech32()
                .expect("publisher npub should encode"),
            d_tag: d_tag.to_owned(),
            expected_event_id,
        }
    }

    fn launch_profile(profile_id: &str, executable: &str) -> LaunchProfile {
        LaunchProfile {
            id: ProfileId::parse(profile_id).expect("profile ID should parse"),
            display_name: profile_id.to_owned(),
            executable: RelativePath::parse(executable).expect("executable should parse"),
            working_dir: WorkingDir::parse(".").expect("working directory should parse"),
            arguments: Vec::new(),
            launch_type: LaunchType::Game,
        }
    }

    #[test]
    fn managed_family_filter_removes_every_package_and_launch_occurrence() {
        let unknown_tags = vec![
            vec!["summary".to_owned(), "unchanged\0bytes".to_owned()],
            vec!["x-future".to_owned(), "alpha".to_owned(), "beta".to_owned()],
        ];
        let authoritative_tags = vec![
            vec!["package".to_owned(), "zip".to_owned()],
            vec!["package".to_owned(), "tar.gz".to_owned()],
            vec!["platform".to_owned(), "linux-x86_64".to_owned()],
            vec!["platform".to_owned()],
            vec![
                "launch".to_owned(),
                "play".to_owned(),
                "bin/game".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "game".to_owned(),
                "Play".to_owned(),
            ],
            vec![
                "launch".to_owned(),
                "safe".to_owned(),
                "bin/safe".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "game".to_owned(),
                "Safe".to_owned(),
            ],
            vec![
                "launch".to_owned(),
                "server".to_owned(),
                "bin/server".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "server".to_owned(),
                "Server".to_owned(),
            ],
            vec![
                "launch".to_owned(),
                "play".to_owned(),
                "bin/duplicate".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "game".to_owned(),
                "Duplicate play".to_owned(),
            ],
            vec!["launch".to_owned()],
            unknown_tags[0].clone(),
            unknown_tags[1].clone(),
        ];

        let preserved = preserved_listing_tags(&authoritative_tags);

        assert_eq!(preserved, unknown_tags);
        assert!(preserved.iter().all(|tag| {
            !matches!(
                tag.first().map(String::as_str),
                Some("package" | "launch" | "platform")
            )
        }));
    }

    #[test]
    fn target_scoped_defaults_replace_stale_form_managed_claims() {
        let existing = vec![
            vec!["launch_default".to_owned(), "stale".to_owned()],
            vec!["summary".to_owned(), "preserved".to_owned()],
        ];
        let mut listing = retry_listing_input(&existing, Vec::new());
        listing.platforms.clear();
        let linux = PlatformTarget {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
        };
        let windows = PlatformTarget {
            os: "windows".to_owned(),
            arch: "x86_64".to_owned(),
        };
        let profiles = vec![
            launch_profile("linux-play", "bin/linux-game"),
            launch_profile("windows-play", "bin/windows-game.exe"),
        ];
        let tier = arcadestr_core::adp_publish_contract::AdpLaunchTierIntent {
            package_format: PackageFormat::Zip,
            platforms: vec![linux.clone(), windows.clone()],
            launch_profiles: profiles.clone(),
            publisher_defaults: vec![
                arcadestr_core::adp_publish_contract::PublisherLaunchDefault {
                    profile_id: profiles[0].id.clone(),
                    targets: vec![linux],
                },
                arcadestr_core::adp_publish_contract::PublisherLaunchDefault {
                    profile_id: profiles[1].id.clone(),
                    targets: vec![windows],
                },
            ],
        };

        let event = build_adp_listing_event_builder(AdpListingPublishInput {
            listing: &listing,
            launch_tier: Some(&tier),
        })
        .expect("scoped publication should build")
        .sign_with_keys(&Keys::generate())
        .expect("listing should sign");

        assert_eq!(
            listing_event_tags(&event, "launch_default"),
            vec![
                vec![
                    "launch_default".to_owned(),
                    "linux-play".to_owned(),
                    "linux-x86_64".to_owned(),
                ],
                vec![
                    "launch_default".to_owned(),
                    "windows-play".to_owned(),
                    "windows-x86_64".to_owned(),
                ],
            ]
        );
        assert_eq!(
            listing_event_tags(&event, "summary"),
            vec![vec!["summary".to_owned(), "preserved".to_owned()]]
        );
    }

    #[test]
    fn download_only_filter_is_harmless_to_unmanaged_tags_and_emits_no_launch_tier() {
        let unmanaged_tags = vec![
            vec!["summary".to_owned(), "Download only".to_owned()],
            vec!["x-preserve".to_owned(), "one".to_owned(), "two".to_owned()],
        ];
        let authoritative_tags = vec![
            vec!["d".to_owned(), "download-only".to_owned()],
            vec!["server".to_owned(), "https://old.example.com".to_owned()],
            vec!["file_hash".to_owned(), RETRY_TEST_SHA256.to_owned()],
            vec!["version".to_owned(), "old".to_owned()],
            vec!["platform".to_owned(), "linux-x86_64".to_owned()],
            unmanaged_tags[0].clone(),
            unmanaged_tags[1].clone(),
        ];
        let preserved = preserved_listing_tags(&authoritative_tags);
        let mut input = retry_listing_input(&authoritative_tags, Vec::new());
        input.preserved_tags = preserved.clone();

        let event = retry_listing_event(&input);

        assert_eq!(preserved, unmanaged_tags);
        assert!(listing_event_tags(&event, "package").is_empty());
        assert!(listing_event_tags(&event, "launch").is_empty());
        assert_eq!(
            listing_event_tags(&event, "summary"),
            vec![unmanaged_tags[0].clone()]
        );
        assert_eq!(
            listing_event_tags(&event, "x-preserve"),
            vec![unmanaged_tags[1].clone()]
        );
    }

    #[tokio::test]
    async fn editing_three_profiles_replaces_stale_launch_tags_and_resolves_exactly() {
        let authoritative_keys = Keys::generate();
        let authoritative = authoritative_listing_event(
            &authoritative_keys,
            Kind::Custom(30402),
            "managed-edit",
            vec![
                Tag::parse(["server", "https://old.example.com"]).expect("server tag should parse"),
                Tag::parse(["file_hash", RETRY_TEST_SHA256]).expect("file hash tag should parse"),
                Tag::parse(["version", "old-version"]).expect("version tag should parse"),
                Tag::parse(["platform", "linux-x86_64"]).expect("platform tag should parse"),
                Tag::parse(["package", "tar.gz"]).expect("package tag should parse"),
                launch_tag("play", "bin/old-game", "play"),
                launch_tag("safe", "bin/safe", "safe"),
                launch_tag("server", "bin/server", "server"),
                Tag::parse(["launch_default", "safe"]).expect("launch_default tag should parse"),
                Tag::parse(["x-preserve", "opaque", "bytes"]).expect("unmanaged tag should parse"),
            ],
        );
        let request = authoritative_request(
            &authoritative_keys,
            "managed-edit",
            authoritative.id.to_hex(),
        );
        let metadata = decode_verified_authoritative_edit_metadata(
            &request,
            authoritative_keys.public_key(),
            &authoritative,
        )
        .expect("authoritative edit metadata should decode");
        let intended_profiles = vec![
            launch_profile("play", "bin/new-game"),
            launch_profile("safe", "bin/safe"),
            launch_profile("server", "bin/server"),
        ];
        let extension = publish_extension(intended_profiles.clone(), &[]);

        assert_eq!(
            enforce_authoritative_profile_removals(&metadata, &extension),
            Ok(())
        );

        let authoritative_tags = authoritative
            .tags
            .iter()
            .map(|tag| tag.clone().to_vec())
            .collect::<Vec<_>>();
        let preserved_tags = preserved_listing_tags(&authoritative_tags);
        let listing = AdpListingInput {
            d_tag: "managed-edit".to_owned(),
            title: "Edited Game".to_owned(),
            description: "Current description".to_owned(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            servers: vec!["https://dist.example.com".to_owned()],
            file_hash: Some(RETRY_TEST_SHA256.to_owned()),
            version: Some("current-version".to_owned()),
            fulfillment_authorizations: Vec::new(),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Public,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags,
        };
        let signer = CountingSigner::new();
        let unsigned = build_adp_listing_event_builder(AdpListingPublishInput {
            listing: &listing,
            launch_tier: extension.launch_tier.as_ref(),
        })
        .expect("edited listing should build")
        .build(signer.keys.public_key());
        assert_eq!(validate_adp_listing_unsigned_event(&unsigned), Ok(()));

        let edited = validate_and_sign_adp_listing(&signer, unsigned)
            .await
            .expect("validated edit should sign");
        let expected_launch_tags = vec![
            vec![
                "launch".to_owned(),
                "play".to_owned(),
                "bin/new-game".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "game".to_owned(),
                "play".to_owned(),
            ],
            vec![
                "launch".to_owned(),
                "safe".to_owned(),
                "bin/safe".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "game".to_owned(),
                "safe".to_owned(),
            ],
            vec![
                "launch".to_owned(),
                "server".to_owned(),
                "bin/server".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "game".to_owned(),
                "server".to_owned(),
            ],
        ];

        assert_eq!(signer.calls(), 1);
        assert_eq!(
            listing_event_tags(&edited, "package"),
            vec![vec!["package".to_owned(), "zip".to_owned()]]
        );
        assert_eq!(listing_event_tags(&edited, "launch"), expected_launch_tags);
        assert_eq!(
            listing_event_tags(&edited, "launch_default"),
            vec![vec!["launch_default".to_owned(), "play".to_owned()]]
        );
        assert_eq!(
            listing_event_tags(&edited, "x-preserve"),
            vec![vec![
                "x-preserve".to_owned(),
                "opaque".to_owned(),
                "bytes".to_owned(),
            ]]
        );
        assert!(!edited.tags.iter().map(Tag::as_slice).any(|tag| {
            tag.get(1).is_some_and(|value| {
                value == "tar.gz" || value == "bin/old-game" || value == "old-version"
            })
        }));
        assert_eq!(
            listing_event_tags(&edited, "launch")[0],
            vec![
                "launch".to_owned(),
                "play".to_owned(),
                "bin/new-game".to_owned(),
                ".".to_owned(),
                "[]".to_owned(),
                "game".to_owned(),
                "play".to_owned(),
            ]
        );

        let resolved = resolve_install_plan(
            &edited,
            &PlatformTarget {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            },
        )
        .expect("signed edited listing should resolve");
        assert_eq!(resolved.plan.artifact.package_format, PackageFormat::Zip);
        assert_eq!(resolved.plan.launch_profiles, intended_profiles);
        assert_eq!(
            resolved
                .plan
                .publisher_default_profile_id
                .as_ref()
                .map(ProfileId::as_str),
            Some("play")
        );
        assert!(resolved.discarded_profiles.is_empty());
    }

    #[test]
    fn authoritative_three_profile_event_is_present_without_loss() {
        let keys = Keys::generate();
        let raw_launch_tags = [
            vec!["launch", "play", "bin/game", ".", "[]", "game", "Play"],
            vec![
                "launch",
                "safe",
                "bin/game-safe",
                ".",
                "[]",
                "game",
                "Safe mode",
            ],
            vec![
                "launch",
                "server",
                "bin/server",
                ".",
                "[]",
                "game",
                "Server",
            ],
        ];
        let event = authoritative_listing_event(
            &keys,
            Kind::Custom(30402),
            "game",
            vec![
                Tag::parse(["package", "zip"]).expect("package tag should parse"),
                Tag::parse(["platform", "linux-x86_64"]).expect("platform tag should parse"),
                launch_tag("play", "bin/game", "Play"),
                launch_tag("safe", "bin/game-safe", "Safe mode"),
                launch_tag("server", "bin/server", "Server"),
                Tag::parse(["launch_default", "play"]).expect("launch_default tag should parse"),
            ],
        );
        let request = authoritative_request(&keys, "game", event.id.to_hex());

        let metadata =
            decode_verified_authoritative_edit_metadata(&request, keys.public_key(), &event)
                .expect("valid authoritative metadata should decode");
        let AuthoritativeAdpEditMetadata::Present(metadata) = metadata else {
            panic!("three-profile launch tier should be present");
        };
        let launches = metadata
            .tags
            .iter()
            .filter_map(|tag| match tag {
                arcadestr_core::adp_publish_contract::AuthoritativeAdpEditTag::Launch {
                    raw,
                    profile,
                    ..
                } => Some((raw, profile)),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(launches.len(), 3);
        for (index, expected_raw) in raw_launch_tags.iter().enumerate() {
            let expected_raw = expected_raw
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            assert_eq!(launches[index].0, &expected_raw);
            let profile = launches[index]
                .1
                .as_ref()
                .expect("valid launch tag should expose a typed profile");
            assert_eq!(profile.id.as_str(), expected_raw[1]);
            assert_eq!(profile.executable.as_str(), expected_raw[2]);
            assert_eq!(profile.working_dir.as_str(), expected_raw[3]);
            assert!(profile.arguments.is_empty());
            assert_eq!(profile.launch_type, LaunchType::Game);
            assert_eq!(profile.display_name, expected_raw[6]);
        }
        assert_eq!(
            metadata
                .publisher_defaults
                .first()
                .map(|default| default.profile_id.as_str()),
            Some("play")
        );
        assert_eq!(
            metadata.publisher_defaults[0].targets,
            vec![PlatformTarget {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            }]
        );
        assert!(metadata.defects.is_empty());
    }

    #[test]
    fn authoritative_download_only_event_is_successful_absent() {
        let keys = Keys::generate();
        let file_hash = "11".repeat(32);
        let event = authoritative_listing_event(
            &keys,
            Kind::Custom(30402),
            "download",
            vec![Tag::parse(["file_hash", file_hash.as_str()]).expect("file hash tag should parse")],
        );
        let request = authoritative_request(&keys, "download", event.id.to_hex());

        assert_eq!(
            decode_verified_authoritative_edit_metadata(&request, keys.public_key(), &event),
            Ok(AuthoritativeAdpEditMetadata::Absent)
        );
    }

    #[test]
    fn malformed_launch_tier_is_successful_present_with_defects() {
        let keys = Keys::generate();
        let event = authoritative_listing_event(
            &keys,
            Kind::Custom(30402),
            "malformed",
            vec![Tag::parse(["package", "unknown"]).expect("package tag should parse")],
        );
        let request = authoritative_request(&keys, "malformed", event.id.to_hex());

        let result =
            decode_verified_authoritative_edit_metadata(&request, keys.public_key(), &event);

        assert!(matches!(
            result,
            Ok(AuthoritativeAdpEditMetadata::Present(metadata)) if !metadata.defects.is_empty()
        ));
    }

    #[test]
    fn refetch_failure_and_not_found_are_typed_and_never_absent() {
        let coordinate = "30402:publisher:missing";

        assert_eq!(
            map_authoritative_refetch(Err("relay offline".to_owned()), coordinate),
            Err(PublishAdpListingError::AuthoritativeRelayRefetchFailure {
                message: "relay offline".to_owned(),
            })
        );
        assert_eq!(
            map_authoritative_refetch(Ok(None), coordinate),
            Err(PublishAdpListingError::AuthoritativeListingNotFound {
                coordinate: coordinate.to_owned(),
            })
        );
    }

    #[test]
    fn authoritative_identity_and_coordinate_mismatches_are_exact_typed_errors() {
        let keys = Keys::generate();
        let other = Keys::generate();
        let event = authoritative_listing_event(&keys, Kind::Custom(30402), "game", Vec::new());
        let request = authoritative_request(&keys, "game", event.id.to_hex());

        let other_request = authoritative_request(&other, "game", event.id.to_hex());
        assert_eq!(
            decode_verified_authoritative_edit_metadata(&other_request, other.public_key(), &event,),
            Err(PublishAdpListingError::AuthoritativePublisherMismatch {
                expected_publisher_npub: other_request.expected_publisher_npub,
                actual_publisher_hex: keys.public_key().to_hex(),
            })
        );

        let wrong_d_tag = authoritative_request(&keys, "other-game", event.id.to_hex());
        assert_eq!(
            decode_verified_authoritative_edit_metadata(&wrong_d_tag, keys.public_key(), &event),
            Err(PublishAdpListingError::AuthoritativeCoordinateMismatch {
                expected_d_tag: "other-game".to_owned(),
                actual_d_tags: vec!["game".to_owned()],
            })
        );

        let wrong_kind = authoritative_listing_event(&keys, Kind::TextNote, "game", Vec::new());
        let wrong_kind_request = authoritative_request(&keys, "game", wrong_kind.id.to_hex());
        assert_eq!(
            decode_verified_authoritative_edit_metadata(
                &wrong_kind_request,
                keys.public_key(),
                &wrong_kind,
            ),
            Err(PublishAdpListingError::AuthoritativeKindMismatch {
                expected: 30402,
                actual: Kind::TextNote.as_u16(),
            })
        );

        let stale_request = authoritative_request(&keys, "game", "00".repeat(32));
        assert_eq!(
            decode_verified_authoritative_edit_metadata(&stale_request, keys.public_key(), &event),
            Err(PublishAdpListingError::StaleExpectedEventId {
                expected_event_id: "00".repeat(32),
                actual_event_id: event.id.to_hex(),
            })
        );

        let invalid_id = nostr::Event::new(
            nostr::EventId::all_zeros(),
            event.pubkey,
            event.created_at,
            event.kind,
            event.tags.iter().cloned().collect::<Vec<_>>(),
            event.content.clone(),
            event.sig,
        );
        let invalid_id_request =
            authoritative_request(&keys, "game", nostr::EventId::all_zeros().to_hex());
        assert_eq!(
            decode_verified_authoritative_edit_metadata(
                &invalid_id_request,
                keys.public_key(),
                &invalid_id,
            ),
            Err(PublishAdpListingError::AuthoritativeEventIdInvalid {
                event_id: nostr::EventId::all_zeros().to_hex(),
            })
        );

        let unrelated =
            authoritative_listing_event(&other, Kind::Custom(30402), "game", Vec::new());
        let invalid_signature = nostr::Event::new(
            event.id,
            event.pubkey,
            event.created_at,
            event.kind,
            event.tags.iter().cloned().collect::<Vec<_>>(),
            event.content.clone(),
            unrelated.sig,
        );
        assert_eq!(
            decode_verified_authoritative_edit_metadata(
                &request,
                keys.public_key(),
                &invalid_signature,
            ),
            Err(PublishAdpListingError::AuthoritativeSignatureInvalid {
                event_id: event.id.to_hex(),
            })
        );
    }

    fn publish_extension(
        incoming_profiles: Vec<LaunchProfile>,
        acknowledged: &[&str],
    ) -> PublishAdpListingRequestExtension {
        let publisher_default_profile_id = incoming_profiles
            .iter()
            .find(|profile| profile.launch_type == LaunchType::Game)
            .map(|profile| profile.id.clone())
            .unwrap_or_else(|| ProfileId::parse("play").expect("test default profile ID"));
        PublishAdpListingRequestExtension {
            authorize_operators: false,
            server_roles: Vec::new(),
            commercial_intent: Default::default(),
            operation_id: None,
            launch_tier: Some(arcadestr_core::adp_publish_contract::AdpLaunchTierIntent {
                package_format: arcadestr_core::distribution::PackageFormat::Zip,
                platforms: vec![arcadestr_core::distribution::PlatformTarget {
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                }],
                launch_profiles: incoming_profiles,
                publisher_defaults: vec![
                    arcadestr_core::adp_publish_contract::PublisherLaunchDefault {
                        profile_id: publisher_default_profile_id,
                        targets: vec![arcadestr_core::distribution::PlatformTarget {
                            os: "linux".to_owned(),
                            arch: "x86_64".to_owned(),
                        }],
                    },
                ],
            }),
            acknowledged_removed_profile_ids: acknowledged
                .iter()
                .map(|profile_id| ProfileId::parse(profile_id).expect("profile ID should parse"))
                .collect(),
        }
    }

    #[test]
    fn selected_build_preflight_enforces_artifact_source_boundaries() {
        let mut request = PublishAdpListingRequest {
            expected_publisher_npub: "npub1publisher".to_owned(),
            existing_event_id: None,
            existing_d_tag: None,
            pending_d_tag: None,
            title: "Game".to_owned(),
            description: "Description".to_owned(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            fulfillment_mode: FulfillmentMode::None,
            operator_url: None,
            servers: Vec::new(),
            accepted_terms: Vec::new(),
            file_path: None,
            selected_build: None,
            existing_file_hash: None,
            existing_fulfillment_pubkey: None,
            version: None,
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Public,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
        };

        assert_eq!(
            validate_selected_build_requirement(
                &request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Ok(())
        );

        request.fulfillment_mode = FulfillmentMode::Direct;
        assert!(matches!(
            validate_selected_build_requirement(
                &request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Err(PublishAdpListingError::Archive {
                error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::SelectedBuildRequired,
            })
        ));

        request.fulfillment_mode = FulfillmentMode::None;
        assert!(matches!(
            validate_selected_build_requirement(&request, &publish_extension(Vec::new(), &[])),
            Err(PublishAdpListingError::Archive {
                error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::SelectedBuildRequired,
            })
        ));

        request.existing_file_hash = Some(RETRY_TEST_SHA256.to_string());
        assert!(matches!(
            validate_selected_build_requirement(
                &request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Err(PublishAdpListingError::Other { .. })
        ));
        request.existing_file_hash = None;

        request.selected_build = Some(
            serde_json::from_value(json!({
                "selection_id": "123e4567-e89b-42d3-a456-426614174000",
                "sha256": RETRY_TEST_SHA256,
            }))
            .expect("selected build fixture should deserialize"),
        );
        assert!(matches!(
            validate_selected_build_requirement(
                &request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Err(PublishAdpListingError::Other { .. })
        ));

        request.fulfillment_mode = FulfillmentMode::Direct;
        assert_eq!(
            validate_selected_build_requirement(
                &request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Ok(())
        );
        request.file_path = Some("/caller/controlled.zip".to_string());
        assert!(matches!(
            validate_selected_build_requirement(
                &request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Err(PublishAdpListingError::Other { message })
                if message.contains("ambiguous")
        ));

        request.selected_build = None;
        assert!(matches!(
            validate_selected_build_requirement(
                &request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Err(PublishAdpListingError::Other { message })
                if message.contains("file_path")
        ));
    }

    #[test]
    fn launch_tier_requires_a_resolved_file_hash_before_side_effects() {
        let extension = publish_extension(Vec::new(), &[]);

        assert_eq!(
            require_launch_tier_file_hash(&extension, None),
            Err(PublishAdpListingError::CoreValidation {
                violation: AdpPublishValidationViolation::InvalidFileHash,
                message: "a launch tier requires a resolved file_hash".to_string(),
            })
        );
        assert_eq!(
            require_launch_tier_file_hash(&extension, Some(RETRY_TEST_SHA256)),
            Ok(())
        );
        assert_eq!(
            require_launch_tier_file_hash(&PublishAdpListingRequestExtension::default(), None,),
            Ok(())
        );

        let source = include_str!("adp_commands.rs");
        let pipeline = source
            .split("async fn publish_adp_listing_pipeline_inner")
            .nth(1)
            .and_then(|source| source.split("async fn validate_current_attestation").next())
            .expect("publication pipeline boundaries");
        let guard = pipeline
            .find("require_launch_tier_file_hash(")
            .expect("resolved file-hash invariant guard");
        for side_effect in [
            "resolve_active_signer(",
            ".get_public_key()",
            "resolve_provisioning(",
            "publish_event(",
            ".upload_with_progress(",
        ] {
            let side_effect = pipeline
                .find(side_effect)
                .unwrap_or_else(|| panic!("pipeline should contain {side_effect}"));
            assert!(
                guard < side_effect,
                "file-hash guard must precede {side_effect}"
            );
        }
    }

    fn artifact_reuse_request(hash: Option<&str>, servers: &[&str]) -> PublishAdpListingRequest {
        PublishAdpListingRequest {
            expected_publisher_npub: "npub1publisher".to_owned(),
            existing_event_id: Some("event-id".to_owned()),
            existing_d_tag: Some("game".to_owned()),
            pending_d_tag: None,
            title: "Game".to_owned(),
            description: "Description".to_owned(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            fulfillment_mode: FulfillmentMode::Direct,
            operator_url: None,
            servers: servers.iter().map(|server| (*server).to_owned()).collect(),
            accepted_terms: Vec::new(),
            file_path: None,
            selected_build: None,
            existing_file_hash: hash.map(str::to_owned),
            existing_fulfillment_pubkey: None,
            version: Some("1.0.0".to_owned()),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Public,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
        }
    }

    #[test]
    fn authoritative_artifact_reuse_requires_one_exact_hash_and_no_added_server() {
        let keys = Keys::generate();
        let reuse_request =
            artifact_reuse_request(Some(RETRY_TEST_SHA256), &["https://one.example"]);
        assert_eq!(
            validate_selected_build_requirement(
                &reuse_request,
                &PublishAdpListingRequestExtension::default(),
            ),
            Ok(())
        );
        assert_eq!(
            validate_selected_build_requirement(
                &reuse_request,
                &publish_extension(Vec::new(), &[]),
            ),
            Ok(())
        );

        let authoritative = authoritative_listing_event(
            &keys,
            Kind::Custom(30402),
            "game",
            vec![
                Tag::parse(["file_hash", RETRY_TEST_SHA256]).expect("file hash tag should parse"),
                Tag::parse(["server", "https://one.example"]).expect("server tag should parse"),
            ],
        );
        assert_eq!(
            authoritative_reused_file_hash(
                &artifact_reuse_request(Some(RETRY_TEST_SHA256), &["https://one.example"]),
                &authoritative,
            ),
            Ok(Some(RETRY_TEST_SHA256.to_owned()))
        );
        let resolved = authoritative_reused_file_hash(
            &artifact_reuse_request(Some(RETRY_TEST_SHA256), &["https://one.example"]),
            &authoritative,
        )
        .expect("legitimate metadata-only edit should preserve artifact reuse");
        assert_eq!(
            require_launch_tier_file_hash(&publish_extension(Vec::new(), &[]), resolved.as_deref()),
            Ok(())
        );
        assert_eq!(
            authoritative_reused_file_hash(
                &artifact_reuse_request(Some(RETRY_TEST_SHA256), &[]),
                &authoritative,
            ),
            Ok(Some(RETRY_TEST_SHA256.to_owned()))
        );

        let changed_hash = "11".repeat(32);
        assert!(matches!(
            authoritative_reused_file_hash(
                &artifact_reuse_request(Some(&changed_hash), &["https://one.example"]),
                &authoritative,
            ),
            Err(PublishAdpListingError::Other { .. })
        ));
        assert!(matches!(
            authoritative_reused_file_hash(
                &artifact_reuse_request(None, &["https://one.example"]),
                &authoritative,
            ),
            Err(PublishAdpListingError::Other { .. })
        ));
        assert!(matches!(
            authoritative_reused_file_hash(
                &artifact_reuse_request(Some(RETRY_TEST_SHA256), &["https://two.example"]),
                &authoritative,
            ),
            Err(PublishAdpListingError::Archive {
                error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError::SelectedBuildRequired,
            })
        ));

        for tags in [
            Vec::new(),
            vec![Tag::parse(["file_hash"]).expect("short tag should parse")],
            vec![
                Tag::parse(["file_hash", RETRY_TEST_SHA256]).expect("file hash tag should parse"),
                Tag::parse(["file_hash", RETRY_TEST_SHA256])
                    .expect("duplicate file hash tag should parse"),
            ],
            vec![Tag::parse(["file_hash", "not-a-digest"])
                .expect("malformed digest tag should still parse")],
        ] {
            let malformed = authoritative_listing_event(&keys, Kind::Custom(30402), "game", tags);
            assert!(matches!(
                authoritative_reused_file_hash(
                    &artifact_reuse_request(Some(RETRY_TEST_SHA256), &["https://one.example"],),
                    &malformed,
                ),
                Err(PublishAdpListingError::Other { .. })
            ));
        }
    }

    fn authoritative_launch_metadata(
        keys: &Keys,
        launch_tags: Vec<Tag>,
    ) -> AuthoritativeAdpEditMetadata {
        let mut tags = vec![
            Tag::parse(["package", "zip"]).expect("package tag should parse"),
            Tag::parse(["platform", "linux-x86_64"]).expect("platform tag should parse"),
        ];
        tags.extend(launch_tags);
        let event = authoritative_listing_event(keys, Kind::Custom(30402), "game", tags);
        let request = authoritative_request(keys, "game", event.id.to_hex());
        decode_verified_authoritative_edit_metadata(&request, keys.public_key(), &event)
            .expect("authoritative event should verify and decode")
    }

    #[test]
    fn one_removed_profile_without_acknowledgement_returns_exact_typed_id() {
        let keys = Keys::generate();
        let metadata = authoritative_launch_metadata(
            &keys,
            vec![launch_tag("legacy", "bin/legacy", "Legacy")],
        );

        assert_eq!(
            enforce_authoritative_profile_removals(&metadata, &publish_extension(Vec::new(), &[]),),
            Err(
                PublishAdpListingError::ProfileRemovalAcknowledgementRequired {
                    removed_profile_ids: vec![
                        ProfileId::parse("legacy").expect("profile ID should parse")
                    ],
                }
            )
        );
    }

    #[test]
    fn partial_acknowledgement_returns_the_full_current_removed_set() {
        let keys = Keys::generate();
        let metadata = authoritative_launch_metadata(
            &keys,
            vec![
                launch_tag("first", "bin/first", "First"),
                launch_tag("second", "bin/second", "Second"),
            ],
        );

        assert_eq!(
            enforce_authoritative_profile_removals(
                &metadata,
                &publish_extension(Vec::new(), &["first"]),
            ),
            Err(
                PublishAdpListingError::ProfileRemovalAcknowledgementRequired {
                    removed_profile_ids: vec![
                        ProfileId::parse("first").expect("profile ID should parse"),
                        ProfileId::parse("second").expect("profile ID should parse"),
                    ],
                }
            )
        );
    }

    #[test]
    fn acknowledging_all_current_removals_permits_publication() {
        let keys = Keys::generate();
        let metadata = authoritative_launch_metadata(
            &keys,
            vec![
                launch_tag("first", "bin/first", "First"),
                launch_tag("second", "bin/second", "Second"),
            ],
        );

        assert_eq!(
            enforce_authoritative_profile_removals(
                &metadata,
                &publish_extension(Vec::new(), &["second", "first", "first"]),
            ),
            Ok(())
        );
    }

    #[test]
    fn stale_acknowledgement_does_not_conceal_a_new_current_removal() {
        let keys = Keys::generate();
        let metadata = authoritative_launch_metadata(
            &keys,
            vec![launch_tag("newly-removed", "bin/old", "Old")],
        );

        assert_eq!(
            enforce_authoritative_profile_removals(
                &metadata,
                &publish_extension(Vec::new(), &["stale"]),
            ),
            Err(
                PublishAdpListingError::ProfileRemovalAcknowledgementRequired {
                    removed_profile_ids: vec![
                        ProfileId::parse("newly-removed").expect("profile ID should parse")
                    ],
                }
            )
        );
    }

    #[test]
    fn changed_executable_with_the_same_id_passes_live_enforcement() {
        let keys = Keys::generate();
        let metadata =
            authoritative_launch_metadata(&keys, vec![launch_tag("play", "old/game", "Play")]);

        assert_eq!(
            enforce_authoritative_profile_removals(
                &metadata,
                &publish_extension(vec![launch_profile("play", "new/game")], &[]),
            ),
            Ok(())
        );
    }

    #[test]
    fn removed_ids_use_only_valid_authoritative_ids_in_first_occurrence_order() {
        let keys = Keys::generate();
        let metadata = authoritative_launch_metadata(
            &keys,
            vec![
                launch_tag("legacy-b", "bin/b", "B"),
                launch_tag("INVALID", "bin/ignored", "Ignored"),
                launch_tag("legacy-a", "bin/a", "A"),
                launch_tag("legacy-b", "bin/b2", "B duplicate"),
                launch_tag("kept", "bin/kept", "Kept"),
            ],
        );

        let removed =
            removed_authoritative_profile_ids(&metadata, &[launch_profile("kept", "bin/new-kept")]);

        assert_eq!(
            removed,
            vec![
                ProfileId::parse("legacy-b").expect("profile ID should parse"),
                ProfileId::parse("legacy-a").expect("profile ID should parse"),
            ]
        );
    }

    #[test]
    fn absent_authoritative_tier_has_no_removals() {
        let keys = Keys::generate();
        let event = authoritative_listing_event(&keys, Kind::Custom(30402), "game", Vec::new());
        let request = authoritative_request(&keys, "game", event.id.to_hex());
        let metadata =
            decode_verified_authoritative_edit_metadata(&request, keys.public_key(), &event)
                .expect("authoritative event should decode as absent");

        assert_eq!(
            enforce_authoritative_profile_removals(
                &metadata,
                &PublishAdpListingRequestExtension::default(),
            ),
            Ok(())
        );
    }

    #[test]
    fn legacy_edit_with_default_extension_cannot_strip_authoritative_profiles() {
        let keys = Keys::generate();
        let metadata =
            authoritative_launch_metadata(&keys, vec![launch_tag("play", "bin/game", "Play")]);

        assert_eq!(
            enforce_authoritative_profile_removals(
                &metadata,
                &PublishAdpListingRequestExtension::default(),
            ),
            Err(
                PublishAdpListingError::ProfileRemovalAcknowledgementRequired {
                    removed_profile_ids: vec![
                        ProfileId::parse("play").expect("profile ID should parse")
                    ],
                }
            )
        );
    }

    #[test]
    fn removal_enforcement_precedes_all_publication_side_effect_paths() {
        let source = include_str!("adp_commands.rs");
        let pipeline = source
            .split("async fn publish_adp_listing_pipeline_inner")
            .nth(1)
            .and_then(|source| source.split("async fn validate_current_attestation").next())
            .expect("publication pipeline should have stable structural boundaries");
        let enforcement = pipeline
            .find("enforce_authoritative_profile_removals(")
            .expect("shared pipeline must enforce removals");

        for side_effect in [
            "resolve_active_signer(",
            "resolve_provisioning(",
            "validate_and_sign_adp_listing(",
            ".upload",
        ] {
            let side_effect = pipeline
                .find(side_effect)
                .unwrap_or_else(|| panic!("pipeline should contain {side_effect}"));
            assert!(
                enforcement < side_effect,
                "enforcement must precede {side_effect}"
            );
        }
        assert!(!pipeline.contains("repair_existing_authorization("));
    }

    #[test]
    fn replaceable_selector_uses_lower_id_for_equal_timestamp_in_any_arrival_order() {
        let keys = Keys::generate();
        let first = signed_listing_event(&keys, "first", 100);
        let second = signed_listing_event(&keys, "second", 100);
        let expected_id = std::cmp::min(first.id.to_hex(), second.id.to_hex());

        let forward = select_replaceable_event([first.clone(), second.clone()])
            .expect("forward events should select a winner");
        let reverse = select_replaceable_event([second, first])
            .expect("reverse events should select a winner");

        assert_eq!(forward.id.to_hex(), expected_id);
        assert_eq!(reverse.id.to_hex(), expected_id);
    }

    #[test]
    fn publication_has_optimistic_revision_checks_on_both_sides_of_remote_signing() {
        let source = include_str!("adp_commands.rs");
        let pipeline = source
            .split("async fn publish_adp_listing_pipeline_inner")
            .nth(1)
            .and_then(|s| s.split("async fn validate_current_attestation").next())
            .expect("pipeline");
        let signing = pipeline
            .find("validate_and_sign_adp_listing(")
            .expect("signing");
        let publication = pipeline
            .find("publish_event(&state, &listing_event")
            .expect("publication");
        let checks = pipeline
            .match_indices("ensure_listing_revision_current(")
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert!(checks.iter().any(|index| *index < signing));
        assert!(checks
            .iter()
            .any(|index| *index > signing && *index < publication));
        assert!(pipeline.contains("listing_replacement_timestamp("));
    }

    #[test]
    fn replaceable_selector_prefers_newer_timestamp() {
        let keys = Keys::generate();
        let older = signed_listing_event(&keys, "older", 100);
        let newer = signed_listing_event(&keys, "newer", 101);

        let selected = select_replaceable_event([newer.clone(), older])
            .expect("events should select a winner");

        assert_eq!(selected.id, newer.id);
    }

    async fn test_db() -> Database {
        let path = std::env::temp_dir().join(format!(
            "arcadestr-adp-command-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ));
        Database::new(&path)
            .await
            .expect("test database should open")
    }

    #[tokio::test]
    async fn request_lnurl_invoice_resolves_lud16_and_requests_fixed_amount_invoice() {
        let http = Arc::new(
            LocalMockHttpClient::default()
                .with_json_response(
                    "https://example.com/.well-known/lnurlp/buyer",
                    json!({
                        "callback": "https://example.com/lnurl/callback",
                        "minSendable": 1_000,
                        "maxSendable": 10_000,
                        "nostrPubkey": "lsp-pubkey"
                    }),
                )
                .with_json_response(
                    "https://example.com/lnurl/callback?amount=5000",
                    json!({ "pr": "lnbc5u1fixed" }),
                ),
        );

        let response = request_lnurl_invoice_with_http(
            http.as_ref(),
            RequestLnurlInvoiceRequest {
                lud16: "buyer@example.com".to_string(),
                amount_sats: 5,
            },
        )
        .await
        .expect("invoice request succeeds");

        assert_eq!(response.bolt11, "lnbc5u1fixed");
    }

    #[test]
    fn nwc_connection_response_exposes_safe_metadata_without_secret() {
        let uri = concat!(
            "nostr+walletconnect://",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "?relay=wss%3A%2F%2Frelay.example.com",
            "&secret=0000000000000000000000000000000000000000000000000000000000000002",
            "&lud16=buyer%40example.com"
        );
        let connection = arcadestr_core::nwc_client::NwcConnection::parse(uri)
            .expect("valid NWC connection should parse");

        let response = nwc_connection_response(&connection);

        assert_eq!(
            response.wallet_pubkey,
            "0000000000000000000000000000000000000000000000000000000000000001"
        );
        assert_eq!(response.relays, vec!["wss://relay.example.com".to_string()]);
        assert_eq!(response.lud16.as_deref(), Some("buyer@example.com"));
        assert!(!format!("{response:?}")
            .contains("0000000000000000000000000000000000000000000000000000000000000002"));
    }

    #[test]
    fn nwc_commands_require_matching_active_account() {
        let a = nostr::Keys::generate().public_key();
        let b = nostr::Keys::generate().public_key();
        let mut auth = arcadestr_core::auth::AuthState::new();
        assert!(nwc_account(&auth, &a.to_hex()).is_err());
        auth.set_public_key(a);
        assert_eq!(nwc_account(&auth, &a.to_hex()).expect("active account"), a);
        assert_eq!(
            nwc_account(&auth, &a.to_hex().to_uppercase()).expect("canonical hex"),
            a
        );
        assert!(nwc_account(&auth, &b.to_hex()).is_err());
        assert!(nwc_account(&auth, "malformed").is_err());
        auth.set_public_key(b);
        assert!(nwc_account(&auth, &a.to_hex()).is_err());
        assert_eq!(
            nwc_account(&auth, &b.to_hex()).expect("switched account"),
            b
        );
        auth.disconnect();
        assert!(nwc_account(&auth, &b.to_hex()).is_err());
    }

    #[test]
    fn nwc_metadata_serialization_and_errors_never_echo_credentials() {
        let secret = "0000000000000000000000000000000000000000000000000000000000000002";
        let wallet = nostr::Keys::generate().public_key();
        let uri = format!("nostr+walletconnect://{wallet}?relay=wss%3A%2F%2Frelay.example.com%2F{secret}%3Ftoken%3D{secret}&secret={secret}&lud16={secret}%40example.com");
        let connection =
            arcadestr_core::nwc_client::NwcConnection::parse(&uri).expect("fixture URI");
        let response = nwc_connection_response(&connection);
        let json = serde_json::to_string(&response).expect("metadata serializes");
        assert!(!json.contains(secret));
        assert!(!json.contains("nostr+walletconnect"));
        assert_eq!(response.relays, ["wss://relay.example.com"]);
        for error in [
            NwcClientError::Relay(uri.clone()),
            NwcClientError::Keyring(uri.clone()),
            NwcClientError::Nostr(uri.clone()),
            NwcClientError::UnexpectedResponseType(uri.clone()),
            NwcClientError::WalletError {
                code: uri.clone(),
                message: uri,
            },
        ] {
            assert!(!nwc_safe_error(error).contains(secret));
        }
        assert_ne!(
            nwc_safe_error(NwcClientError::NotConfigured),
            nwc_safe_error(NwcClientError::Keyring("locked".into()))
        );
    }

    fn unique_test_path(prefix: &str, extension: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}.{extension}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ))
    }

    fn live_relays_from_env() -> Vec<String> {
        std::env::var("ARCADESTR_RELAYS")
            .expect("ARCADESTR_RELAYS must be set for live ADP publish test")
            .split(',')
            .map(str::trim)
            .filter(|relay| !relay.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    async fn live_test_app(relays: Vec<String>) -> tauri::App<tauri::test::MockRuntime> {
        let db_path = unique_test_path("arcadestr-adp-live-command", "db");
        let http_client: Arc<dyn HttpClient> =
            Arc::new(ReqwestHttpClient::new(Duration::from_secs(10)).expect("fixture HTTP client"));
        test_app_with_http(relays, db_path, http_client).await
    }

    async fn test_app_with_http(
        relays: Vec<String>,
        db_path: PathBuf,
        http_client: Arc<dyn HttpClient>,
    ) -> tauri::App<tauri::test::MockRuntime> {
        let database = Database::new(&db_path)
            .await
            .expect("live test database should open");
        let user_cache = Arc::new(UserCache::new(database.pool().clone()));
        let relay_config = RelayManagerConfig {
            debug_relays: Some(relays),
            block_discovery: true,
            ..RelayManagerConfig::default()
        };
        let nostr = NostrClient::new_with_cache(
            "adp-live-command".to_string(),
            vec![],
            user_cache.clone(),
            Some(relay_config.clone()),
        )
        .await
        .expect("live test nostr client should initialize");
        nostr.connect().await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let validator_client = NostrClient::new_with_cache(
            "adp-live-validator".to_string(),
            vec![],
            user_cache.clone(),
            Some(relay_config),
        )
        .await
        .expect("live test validator client should initialize");
        let nip05_validator = Arc::new(std::sync::Mutex::new(Nip05Validator::spawn(
            Arc::new(validator_client),
            user_cache.clone(),
        )));
        let mut auth = AuthState::new();
        auth.connect_with_key("0000000000000000000000000000000000000000000000000000000000000001")
            .expect("live test signer should initialize");
        let relay_cache = Arc::new(
            RelayCache::new(db_path.with_extension("relay-cache.db"))
                .expect("relay cache should open"),
        );
        let relay_hints = Arc::new(
            RelayHints::new(db_path.with_extension("relay-hints.db"))
                .expect("relay hints should open"),
        );
        let profile_fetcher = Arc::new({
            let mut fetcher = ProfileFetcher::with_persistent_cache(user_cache.clone());
            fetcher.with_nip05_validator(nip05_validator.clone());
            fetcher
        });
        let marketplace_cache = Arc::new(MarketplaceCache::new(database.pool().clone()));
        let purchases = Arc::new(arcadestr_core::purchases::PurchasesRepository::new(
            database.pool().clone(),
        ));

        let settlement_recovery =
            arcadestr_core::settlement_recovery::RecoveryCoordinator::default();
        settlement_recovery.activate(auth.public_key().map(|key| key.to_hex()));
        tauri::test::mock_builder()
            .manage(AppState {
                settlement_recovery,
                auth: Arc::new(AsyncMutex::new(auth)),
                nostr: Arc::new(AsyncMutex::new(nostr)),
                database: Arc::new(database),
                relay_cache,
                deduplicator: Arc::new(AsyncMutex::new(EventDeduplicator::new(10_000))),
                subscription_registry: Arc::new(SubscriptionRegistry::new()),
                profile_fetcher,
                user_cache,
                marketplace_cache,
                purchases,
                extended_network: Arc::new(RwLock::new(None)),
                extended_network_follows: Arc::new(RwLock::new(Vec::new())),
                relay_hints: Some(relay_hints),
                nip05_validator,
                http_client,
            })
            .manage(Arc::new(AsyncMutex::new(AppSignerState::new())))
            .manage(ArchiveSelectionService::new().expect("archive selection service"))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock Tauri app should build")
    }

    async fn write_live_test_file() -> PathBuf {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = u32::MAX;
            for byte in data {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
                }
            }
            !crc
        }

        let file_path = unique_test_path("arcadestr-adp-live-build", "zip");
        let bytes: Vec<u8> = (0..4096).map(|index| (index % 251) as u8).collect();
        let name = "game.bin";
        let crc = crc32(&bytes);
        let mut archive = Vec::new();
        archive.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        archive.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        archive.extend_from_slice(&crc.to_le_bytes());
        archive.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        archive.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        archive.extend_from_slice(&(name.len() as u16).to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(&bytes);
        let central_offset = archive.len() as u32;
        let mut central = Vec::new();
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        central.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        let central_size = central.len() as u32;
        archive.extend_from_slice(&central);
        archive.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        archive.extend_from_slice(&[0; 4]);
        archive.extend_from_slice(&1_u16.to_le_bytes());
        archive.extend_from_slice(&1_u16.to_le_bytes());
        archive.extend_from_slice(&central_size.to_le_bytes());
        archive.extend_from_slice(&central_offset.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        tokio::fs::write(&file_path, archive)
            .await
            .expect("live test binary should be written");
        file_path
    }

    fn tag_values(event: &nostr::Event, tag_name: &str) -> Vec<Vec<String>> {
        event
            .tags
            .iter()
            .map(|tag| tag.clone().to_vec())
            .filter(|values| values.first().map(String::as_str) == Some(tag_name))
            .collect()
    }

    async fn fetch_live_listing_event(
        app: &tauri::App<tauri::test::MockRuntime>,
        event_id: &str,
    ) -> nostr::Event {
        let event_id = nostr::EventId::from_hex(event_id).expect("listing event id should parse");
        let relay_manager = {
            let state = app.state::<AppState>();
            let relay_manager = state.nostr.lock().await.get_relay_manager().clone();
            relay_manager
        };
        let manager = relay_manager.lock().await;
        let events = manager
            .fetch_events_with_timeout(nostr::Filter::new().id(event_id), 5)
            .await
            .expect("listing event should fetch from live relays");
        events
            .into_iter()
            .find(|event| event.id == event_id)
            .expect("published listing event should be returned by a relay")
    }

    fn assert_live_listing_tags(
        event: &nostr::Event,
        result: &PublishAdpListingResult,
        lud16: &str,
    ) {
        let file_hash = result.file_hash.as_ref().expect("file hash should be set");
        let fulfillment_pubkey = result
            .fulfillment_pubkey
            .as_ref()
            .expect("fulfillment pubkey should be set");

        assert!(
            !tag_values(event, "server").is_empty(),
            "missing server tag"
        );
        assert_eq!(
            tag_values(event, "file_hash")[0],
            vec!["file_hash", file_hash.as_str()]
        );
        assert_eq!(
            tag_values(event, "version")[0],
            vec!["version", "0.0.1-live"]
        );
        assert_eq!(tag_values(event, "lud16")[0], vec!["lud16", lud16]);
        assert_eq!(
            tag_values(event, "platform")[0],
            vec!["platform", "linux-x86_64"]
        );

        // Delegated fulfillment is carried by a pointer to the authorization
        // root, not by the legacy inline `fulfillment_pubkey` tag.
        let acceptance_event_id = result
            .acceptance_event_id
            .as_ref()
            .expect("acceptance event id should be set");
        assert!(
            tag_values(event, "fulfillment_pubkey").is_empty(),
            "a delegated listing must not carry a legacy fulfillment_pubkey tag"
        );
        let authorization_tags = tag_values(event, "fulfillment_authorization");
        assert_eq!(authorization_tags.len(), 1);
        let authorization_tag = &authorization_tags[0];
        assert_eq!(authorization_tag[0], "fulfillment_authorization");
        assert_eq!(&authorization_tag[1], acceptance_event_id);
        assert_eq!(&authorization_tag[2], fulfillment_pubkey);
    }

    #[derive(Default)]
    struct LocalMockHttpClient {
        get_responses: Mutex<HashMap<String, serde_json::Value>>,
        post_responses: Mutex<HashMap<String, serde_json::Value>>,
        post_counts: Mutex<HashMap<String, usize>>,
    }

    impl LocalMockHttpClient {
        fn with_json_response(self, url: &str, body: serde_json::Value) -> Self {
            self.get_responses
                .lock()
                .expect("get responses mutex poisoned")
                .insert(url.to_string(), body);
            self
        }

        fn with_json_post_response(self, url: &str, body: serde_json::Value) -> Self {
            self.post_responses
                .lock()
                .expect("post responses mutex poisoned")
                .insert(url.to_string(), body);
            self
        }

        fn post_call_count(&self, url: &str) -> usize {
            self.post_counts
                .lock()
                .expect("post counts mutex poisoned")
                .get(url)
                .copied()
                .unwrap_or(0)
        }
    }

    #[tokio::test]
    async fn upload_capacity_checks_all_servers_before_side_effects() {
        let mock = Arc::new(
            LocalMockHttpClient::default()
                .with_json_response(
                    "https://large/.well-known/adp",
                    json!({"adp_version":"0.2.0", "pubkey":"operator", "max_upload_bytes":2048}),
                )
                .with_json_response(
                    "https://small/.well-known/adp",
                    json!({"adp_version":"0.2.0", "pubkey":"operator", "max_upload_bytes":1024}),
                )
                .with_json_response(
                    "https://legacy/.well-known/adp",
                    json!({"adp_version":"0.2.0", "pubkey":"operator"}),
                ),
        );
        let http: Arc<dyn HttpClient> = mock.clone();
        let servers = vec!["https://large".into(), "https://small".into()];
        let error = preflight_upload_capacity(&servers, 1025, &http)
            .await
            .expect_err("small server");
        assert!(error.to_string().contains("https://small"));
        assert!(error.to_string().contains("1025"));
        assert!(error.to_string().contains("1024"));
        preflight_upload_capacity(&servers, 1024, &http)
            .await
            .expect("exact cap");
        preflight_upload_capacity(&["https://legacy".into()], u64::MAX, &http)
            .await
            .expect("unknown legacy cap");
        assert_eq!(mock.post_call_count("https://small/provision"), 0);
        assert_eq!(mock.post_call_count("https://small/upload"), 0);
    }

    #[async_trait::async_trait]
    impl HttpClient for LocalMockHttpClient {
        async fn get_json(
            &self,
            url: &str,
        ) -> Result<serde_json::Value, arcadestr_core::http_client::HttpClientError> {
            self.get_responses
                .lock()
                .expect("get responses mutex poisoned")
                .get(url)
                .cloned()
                .ok_or_else(|| {
                    arcadestr_core::http_client::HttpClientError::Request(format!(
                        "No local desktop mock GET response for {url}"
                    ))
                })
        }

        async fn get_json_no_redirects(
            &self,
            url: &str,
        ) -> Result<serde_json::Value, arcadestr_core::http_client::HttpClientError> {
            self.get_json(url).await
        }

        async fn post_json(
            &self,
            url: &str,
            _body: serde_json::Value,
            _headers: Vec<(String, String)>,
        ) -> Result<serde_json::Value, arcadestr_core::http_client::HttpClientError> {
            *self
                .post_counts
                .lock()
                .expect("post counts mutex poisoned")
                .entry(url.to_string())
                .or_insert(0) += 1;
            self.post_responses
                .lock()
                .expect("post responses mutex poisoned")
                .get(url)
                .cloned()
                .ok_or_else(|| {
                    arcadestr_core::http_client::HttpClientError::Request(format!(
                        "No local desktop mock POST response for {url}"
                    ))
                })
        }

        async fn download_to_path(
            &self,
            _url: &str,
            _headers: Vec<(String, String)>,
            _dest: &std::path::Path,
            _on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
        ) -> Result<
            arcadestr_core::http_client::HttpDownloadOutcome,
            arcadestr_core::http_client::HttpClientError,
        > {
            Err(arcadestr_core::http_client::HttpClientError::Request(
                "download_to_path path should not be used in local desktop mock tests".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn multiple_operator_provisioning_retains_partial_results_and_requires_uncertain_retry() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("signer");
        let publisher = signer.get_public_key().await.expect("publisher");
        let publisher_hex = publisher.to_hex();
        let info = |url: &str| AdpServerInfo {
            max_upload_bytes: None,
            adp_version: "0.2.0".into(),
            pubkey: Keys::generate().public_key().to_hex(),
            name: None,
            url: Some(url.into()),
            event_kinds: None,
            event_kind_status: None,
            settlement: None,
        };
        let one = info("https://one.test");
        let two = info("https://two.test");
        let response = |key: nostr::PublicKey| json!({"fulfillment_pubkey":key.to_hex(),"attestation_event_id":"fixture","scope":"game"});
        let first_http = Arc::new(LocalMockHttpClient::default().with_json_post_response(
            "https://one.test/provision",
            response(Keys::generate().public_key()),
        ));
        let second_http = Arc::new(LocalMockHttpClient::default());
        let first = AdpClient::new("https://one.test", first_http.clone());
        let second = AdpClient::new("https://two.test", second_http.clone());
        let journal_repo = arcadestr_core::publisher_recovery::repository::PublisherRepository::new(
            db.pool().clone(),
        );
        journal_repo
            .begin(
                &publisher_hex,
                &arcadestr_core::publisher_recovery::PublisherOperation {
                    operation_id: "multi".into(),
                    kind: "listing".into(),
                    coordinate: format!("30402:{publisher}:game"),
                    intent: json!({"operators":[one.pubkey,two.pubkey]}),
                    artifact_hash: None,
                    state: "incomplete".into(),
                    result: None,
                },
            )
            .await
            .expect("operation");
        let mut journal = crate::publisher_commands::PublisherJournal {
            repo: journal_repo,
            publisher: publisher_hex.clone(),
            operation_id: "multi".into(),
            allow_uncertain: false,
        };
        let input = |client, info, url, issuing| ResolveProvisioningInput {
            provisioning_repo: &repo,
            adp_client: client,
            signer: &signer,
            developer_pubkey: publisher,
            developer_npub: &publisher_hex,
            server_url: url,
            scope: "game",
            server_info: info,
            reusable_authorization: None,
            required_capabilities: if issuing {
                BTreeSet::from([
                    CAPABILITY_UPLOAD_BUILD.into(),
                    CAPABILITY_ISSUE_RECEIPT.into(),
                ])
            } else {
                BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()])
            },
        };
        let first_result = crate::publisher_commands::JOURNAL
            .scope(
                journal.clone(),
                resolve_provisioning(input(&first, &one, "https://one.test", true)),
            )
            .await
            .expect("first operator succeeds");
        let ProvisioningDecision::Created {
            authorization_event,
            ..
        } = first_result
        else {
            panic!("new operator")
        };
        let authority =
            arcadestr_core::authorization::parse_authorization_event(&authorization_event)
                .expect("authority");
        assert!(authority
            .terms
            .capabilities
            .contains(CAPABILITY_ISSUE_RECEIPT));
        assert!(!authority
            .terms
            .capabilities
            .contains(CAPABILITY_ISSUE_GRANT));
        assert!(crate::publisher_commands::JOURNAL
            .scope(
                journal.clone(),
                resolve_provisioning(input(&second, &two, "https://two.test", false))
            )
            .await
            .is_err());
        let steps = journal
            .repo
            .steps(&publisher_hex, "multi")
            .await
            .expect("steps");
        assert!(steps
            .iter()
            .find(|step| step.target == "https://one.test")
            .expect("first")
            .outcome
            .is_some());
        assert!(steps
            .iter()
            .find(|step| step.target == "https://two.test")
            .expect("second")
            .outcome
            .is_none());
        let retry_http = Arc::new(LocalMockHttpClient::default().with_json_post_response(
            "https://two.test/provision",
            response(Keys::generate().public_key()),
        ));
        let retry = AdpClient::new("https://two.test", retry_http.clone());
        assert!(crate::publisher_commands::JOURNAL
            .scope(
                journal.clone(),
                resolve_provisioning(input(&retry, &two, "https://two.test", false))
            )
            .await
            .is_err());
        assert_eq!(
            retry_http.post_call_count("https://two.test/provision"),
            0,
            "restart must not silently repeat uncertain POST"
        );
        journal.allow_uncertain = true;
        let retry_result = crate::publisher_commands::JOURNAL
            .scope(
                journal,
                resolve_provisioning(input(&retry, &two, "https://two.test", false)),
            )
            .await
            .expect("explicit retry");
        let ProvisioningDecision::Created {
            authorization_event,
            ..
        } = retry_result
        else {
            panic!("new second operator")
        };
        assert_eq!(
            arcadestr_core::authorization::parse_authorization_event(&authorization_event)
                .expect("authority")
                .terms
                .capabilities,
            BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()])
        );
        assert_eq!(first_http.post_call_count("https://one.test/provision"), 1);
        assert_eq!(retry_http.post_call_count("https://two.test/provision"), 1);
    }

    #[tokio::test]
    async fn publish_same_scope_reuses_provisioning_after_revalidating_with_operator() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let provision_url = "https://dist.example.com/provision";
        let mock = Arc::new(LocalMockHttpClient::default().with_json_post_response(
            provision_url,
            json!({
                "fulfillment_pubkey": "b0822d6340862b961e88b983b9c3b434e1fc750a36e796ee10825e8778badacf",
                "attestation_event_id": "attestation-id",
                "scope": "game"
            }),
        ));
        let http: Arc<dyn HttpClient> = mock.clone();
        let client = AdpClient::new("https://dist.example.com", http);
        let signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test key should be valid");
        let developer_pubkey = signer
            .get_public_key()
            .await
            .expect("public key should load");
        let developer_npub = developer_pubkey.to_hex();
        let server_info = AdpServerInfo {
            max_upload_bytes: None,
            adp_version: "0.2.0".to_string(),
            pubkey: Keys::generate().public_key().to_hex(),
            name: Some("Test ADP".to_string()),
            url: Some("https://dist.example.com".to_string()),
            event_kinds: None,
            event_kind_status: None,
            settlement: None,
        };

        let first = resolve_provisioning(ResolveProvisioningInput {
            provisioning_repo: &repo,
            adp_client: &client,
            signer: &signer,
            developer_pubkey,
            developer_npub: &developer_npub,
            server_url: "https://dist.example.com",
            scope: "game",
            server_info: &server_info,
            reusable_authorization: None,
            required_capabilities: BTreeSet::from([
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ]),
        })
        .await
        .expect("first provisioning should succeed");
        let ProvisioningDecision::Created {
            row,
            authorization_event: original_authorization,
            ..
        } = first
        else {
            panic!("first call should provision");
        };
        repo.upsert(&row).await.expect("row should persist");
        let original_root = original_authorization.id;
        let resolved = arcadestr_core::authorization::resolve_authorization(
            original_root,
            &[(*original_authorization).clone()],
        )
        .expect("original authority");

        let second = resolve_provisioning(ResolveProvisioningInput {
            provisioning_repo: &repo,
            adp_client: &client,
            signer: &signer,
            developer_pubkey,
            developer_npub: &developer_npub,
            server_url: "https://dist.example.com",
            scope: "game",
            server_info: &server_info,
            reusable_authorization: Some(&resolved),
            required_capabilities: BTreeSet::from([
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ]),
        })
        .await
        .expect("second provisioning should succeed");

        let ProvisioningDecision::Reused {
            fulfillment_pubkey,
            valid_from,
            authorization_event,
            row,
            ..
        } = second
        else {
            panic!("second call should reuse provisioning");
        };
        let parsed = parse_authorization_event(&authorization_event).unwrap_or_else(|error| {
            panic!(
                "reused authorization should parse: {error:?}; tags={:?}",
                authorization_event.tags
            )
        });
        assert_eq!(
            authorization_event.id, original_root,
            "retry must not renew the root"
        );
        let AuthorizationTransition::ActiveRoot = parsed.transition else {
            panic!("reused authorization should be an active root");
        };
        let terms = parsed.terms;
        assert_eq!(terms.coordinate, format!("30402:{developer_npub}:game"));
        assert_eq!(terms.fulfillment_pubkey.to_hex(), fulfillment_pubkey);
        assert_eq!(
            terms.valid_from,
            u64::try_from(valid_from).expect("valid time")
        );
        assert_eq!(row.fulfillment_pubkey, fulfillment_pubkey);
        assert_eq!(row.valid_from, valid_from);
        assert_eq!(row.acceptance_event_id, authorization_event.id.to_hex());
        assert_eq!(mock.post_call_count(provision_url), 2);
        let mut cancelled = resolved.clone();
        cancelled.cancelled_at = Some(0);
        let rejected = resolve_provisioning(ResolveProvisioningInput {
            provisioning_repo: &repo,
            adp_client: &client,
            signer: &signer,
            developer_pubkey,
            developer_npub: &developer_npub,
            server_url: "https://dist.example.com",
            scope: "game",
            server_info: &server_info,
            reusable_authorization: Some(&cancelled),
            required_capabilities: BTreeSet::from([
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ]),
        })
        .await;
        assert!(
            rejected.is_err(),
            "cancelled pending authority cannot be silently renewed"
        );
        assert_eq!(
            mock.post_call_count(provision_url),
            2,
            "reject before another provisioning request"
        );
        let journal_repo = arcadestr_core::publisher_recovery::repository::PublisherRepository::new(
            db.pool().clone(),
        );
        journal_repo
            .begin(
                &developer_npub,
                &arcadestr_core::publisher_recovery::PublisherOperation {
                    operation_id: "uncertain-provision".into(),
                    kind: "listing".into(),
                    coordinate: format!("30402:{developer_npub}:game"),
                    intent: json!({"original":"immutable"}),
                    artifact_hash: None,
                    state: "incomplete".into(),
                    result: None,
                },
            )
            .await
            .expect("durable operation");
        let mut journal = crate::publisher_commands::PublisherJournal {
            repo: journal_repo,
            publisher: developer_npub.clone(),
            operation_id: "uncertain-provision".into(),
            allow_uncertain: false,
        };
        journal
            .prepare(
                "provision:https://dist.example.com:game",
                "provision",
                "https://dist.example.com",
                json!({"scope":"game","operator":server_info.pubkey}),
            )
            .await
            .expect("checkpoint before uncertain POST");
        let input = || ResolveProvisioningInput {
            provisioning_repo: &repo,
            adp_client: &client,
            signer: &signer,
            developer_pubkey,
            developer_npub: &developer_npub,
            server_url: "https://dist.example.com",
            scope: "game",
            server_info: &server_info,
            reusable_authorization: Some(&resolved),
            required_capabilities: BTreeSet::from([
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ]),
        };
        assert!(crate::publisher_commands::JOURNAL
            .scope(journal.clone(), resolve_provisioning(input()))
            .await
            .is_err());
        assert_eq!(
            mock.post_call_count(provision_url),
            2,
            "no automatic uncertain POST"
        );
        journal.allow_uncertain = true;
        let retry = crate::publisher_commands::JOURNAL
            .scope(journal.clone(), resolve_provisioning(input()))
            .await
            .expect("explicit same-scope reconciliation retry");
        assert!(matches!(retry, ProvisioningDecision::Reused { .. }));
        assert_eq!(mock.post_call_count(provision_url), 3);
        journal.allow_uncertain = false;
        let cached = crate::publisher_commands::JOURNAL
            .scope(journal, resolve_provisioning(input()))
            .await
            .expect("reuse acknowledged response");
        let ProvisioningDecision::Reused {
            authorization_event,
            ..
        } = cached
        else {
            panic!("exact root reuse");
        };
        assert_eq!(authorization_event.id, original_root);
        assert_eq!(
            mock.post_call_count(provision_url),
            3,
            "acknowledged provisioning is not repeated"
        );
    }

    #[tokio::test]
    async fn imported_delegated_edit_preserves_exact_root_without_local_history() {
        let db = test_db().await;
        let repo = AdpProvisioningRepository::new(db.pool().clone());
        let developer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("test key should be valid");
        let developer_pubkey = developer
            .get_public_key()
            .await
            .expect("developer key should load");
        let developer_hex = developer_pubkey.to_hex();
        let fulfillment_pubkey = Keys::generate().public_key().to_hex();
        let original_valid_from = 1_700_000_000;
        let row = AdpProvisioning {
            id: "existing-game-provisioning".to_string(),
            developer_npub: developer_hex.clone(),
            server_url: "https://operator.example.com".to_string(),
            operator_pubkey: Keys::generate().public_key().to_hex(),
            scope: Some("game".to_string()),
            fulfillment_pubkey: fulfillment_pubkey.clone(),
            attestation_event_id: "stable-authorization-id".to_string(),
            acceptance_event_id: "legacy-acceptance-id".to_string(),
            authorization_root_event_id: Some("11".repeat(32)),
            authorization_capabilities: vec![
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ],
            authorization_profile_version: 2,
            valid_from: original_valid_from,
            revoked_at: None,
            created_at: original_valid_from,
        };
        assert!(repo
            .active_for_scope(&developer_hex, &row.server_url, Some("game"))
            .await
            .expect("lookup")
            .is_none());
        let terms = AuthorizationTerms {
            authorization_id: "imported-root".into(),
            coordinate: format!("30402:{developer_hex}:game"),
            operator_pubkey: nostr::PublicKey::from_hex(&row.operator_pubkey).expect("operator"),
            fulfillment_pubkey: nostr::PublicKey::from_hex(&fulfillment_pubkey).expect("key"),
            capabilities: BTreeSet::from([CAPABILITY_ISSUE_RECEIPT.into()]),
            valid_from: original_valid_from as u64,
        };
        let authorization = developer
            .sign_event(
                build_fulfillment_authorization_event_builder(&terms)
                    .expect("builder")
                    .build(developer_pubkey),
            )
            .await
            .expect("sign offline fixture");
        let resolved = arcadestr_core::authorization::resolve_authorization(
            authorization.id,
            &[authorization.clone()],
        )
        .expect("chain");
        let listing = developer
            .sign_event(
                EventBuilder::new(Kind::Custom(30402), "imported")
                    .tags([
                        Tag::parse(["d", "game"]).expect("d"),
                        Tag::parse(["title", "Imported"]).expect("title"),
                        Tag::parse([
                            "fulfillment_authorization",
                            &authorization.id.to_hex(),
                            &fulfillment_pubkey,
                            "wss://imported.example",
                        ])
                        .expect("reference"),
                    ])
                    .build(developer_pubkey),
            )
            .await
            .expect("listing");
        let references = arcadestr_core::authorization::validate_listing_authorization_references(
            &listing,
            &[resolved],
        )
        .expect("no local row required");
        assert_eq!(references[0].root_event_id, authorization.id);
        assert_eq!(
            references[0].relay_hint.as_deref(),
            Some("wss://imported.example")
        );
        let tags = listing
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>();
        assert_eq!(
            preserved_listing_tags(&tags),
            tags.into_iter()
                .filter(|tag| is_fulfillment_authorization_tag(tag))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn delegated_edit_reuses_the_referenced_key_without_listing_timestamps() {
        let metadata = resolve_existing_fulfillment_metadata(
            &FulfillmentMode::Delegate,
            "developer-key",
            Some("delegated-key"),
        )
        .expect("existing delegated key is accepted");
        assert_eq!(
            metadata.fulfillment_pubkey.as_deref(),
            Some("delegated-key")
        );
        assert!(!metadata.should_provision);
    }

    #[test]
    fn direct_to_delegate_conversion_provisions_a_new_key() {
        let metadata = resolve_existing_fulfillment_metadata(
            &FulfillmentMode::Delegate,
            "developer-key",
            Some("developer-key"),
        )
        .expect("conversion is accepted");
        assert!(metadata.fulfillment_pubkey.is_none());
        assert!(metadata.should_provision);
    }

    #[test]
    fn ordinary_edit_cannot_silently_clear_existing_fulfillment() {
        let error = resolve_existing_fulfillment_metadata(
            &FulfillmentMode::None,
            "developer-key",
            Some("delegated-key"),
        )
        .expect_err("existing fulfillment reference must not be silently cleared");
        assert!(error.contains("cannot be cleared"));
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL"]
    async fn live_resuming_a_pending_scope_reuses_its_fulfillment_key() {
        // What makes a resumed publication worth resuming: the operator keys
        // provisioning by scope, so the retry is handed back the key the failed
        // attempt was already given instead of stranding it and minting another.
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for the live provisioning test");
        let http: Arc<dyn HttpClient> = Arc::new(
            ReqwestHttpClient::new(Duration::from_secs(10))
                .expect("reqwest http client should initialize"),
        );
        let client = AdpClient::new(server_url, http);
        let signer = LocalSigner::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("live test key should be valid");

        let minted = resolve_new_game_identifier(None, None)
            .expect("the first attempt should mint an id")
            .d_tag();
        let first = client
            .provision(&signer, Some(&minted))
            .await
            .expect("the first attempt should be provisioned");

        // The attempt fails here, before any listing exists. The retry resumes
        // the same identity, so it asks for the same scope.
        let resumed = resolve_new_game_identifier(None, Some(&minted))
            .expect("the retry should resume the pending publication")
            .d_tag();
        assert_eq!(resumed, minted);
        let second = client
            .provision(&signer, Some(&resumed))
            .await
            .expect("the retry should be provisioned");

        assert_eq!(
            second.fulfillment_pubkey, first.fulfillment_pubkey,
            "a resumed scope must not strand the key the failed attempt was given"
        );
        assert_eq!(second.attestation_event_id, first.attestation_event_id);
        assert_eq!(second.scope.as_deref(), Some(minted.as_str()));
    }

    #[test]
    fn publisher_npub_contract_converts_to_hex() {
        let keys = Keys::generate();
        let npub = keys
            .public_key()
            .to_bech32()
            .expect("publisher npub should encode");

        assert_eq!(
            publisher_hex_from_npub(&npub).expect("publisher npub should parse"),
            keys.public_key().to_hex()
        );
    }

    #[test]
    fn unique_operator_contract_requires_exactly_one_row() {
        let row = AdpProvisioning {
            id: "one".to_string(),
            developer_npub: "developer".to_string(),
            server_url: "https://operator.example.com".to_string(),
            operator_pubkey: "operator".to_string(),
            scope: Some("game".to_string()),
            fulfillment_pubkey: "fulfillment".to_string(),
            attestation_event_id: "attestation".to_string(),
            acceptance_event_id: "acceptance".to_string(),
            authorization_root_event_id: None,
            authorization_capabilities: Vec::new(),
            authorization_profile_version: 1,
            valid_from: 123,
            revoked_at: Some(456),
            created_at: 123,
        };
        let mut second = row.clone();
        second.id = "two".to_string();
        second.server_url = "https://other.example.com".to_string();

        assert_eq!(unique_operator_url(&[]), None);
        assert_eq!(
            unique_operator_url(std::slice::from_ref(&row)).as_deref(),
            Some("https://operator.example.com")
        );
        assert_eq!(unique_operator_url(&[row, second]), None);
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL and live local relays"]
    async fn live_publish_adp_listing_uploads_and_propagates_to_two_relays() {
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for live ADP publish test");
        let lud16 =
            std::env::var("ADP_TEST_LUD16").unwrap_or_else(|_| "seller@example.com".to_string());
        let price_sats = std::env::var("ADP_TEST_PRICE_SATS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let relays = live_relays_from_env();
        assert_eq!(
            relays,
            vec![
                "ws://localhost:10547".to_string(),
                "ws://localhost:10548".to_string()
            ],
            "live ADP publish test must use the two local Gate 3 relays"
        );
        assert_eq!(
            std::env::var("ARCADESTR_BLOCK_DISCOVERY").as_deref(),
            Ok("1"),
            "live ADP publish test must block public relay discovery"
        );

        let app = live_test_app(relays).await;
        let file_path = write_live_test_file().await;
        let expected_publisher_npub =
            nostr::Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
                .expect("live test key should parse")
                .public_key()
                .to_bech32()
                .expect("live test npub should encode");
        let app_state = app.state::<AppState>();
        let signer_state = app.state::<Arc<AsyncMutex<AppSignerState>>>();
        let archive_service = app.state::<ArchiveSelectionService>();
        let selection = archive_service
            .stage_source(
                file_path,
                &expected_publisher_npub,
                crate::archive_commands::account_check(&app_state, &signer_state),
            )
            .await
            .expect("live archive selection should stage");
        let request_id = || {
            arcadestr_core::archive_inspection_contract::ArchiveRequestId::parse(
                &uuid::Uuid::new_v4().to_string(),
            )
            .expect("generated request ID")
        };
        archive_service
            .inspect(
                arcadestr_core::archive_inspection_contract::InspectArchiveRequest {
                    request_id: request_id(),
                    selection_id: selection.selection_id.clone(),
                    expected_publisher_npub: expected_publisher_npub.clone(),
                    expected_format: None,
                },
                &expected_publisher_npub,
                crate::archive_commands::account_check(&app_state, &signer_state),
            )
            .await
            .expect("live archive inspection should complete");
        let hashed = archive_service
            .hash(
                arcadestr_core::archive_inspection_contract::HashSelectedArchiveRequest {
                    request_id: request_id(),
                    selection_id: selection.selection_id.clone(),
                    expected_publisher_npub: expected_publisher_npub.clone(),
                },
                &expected_publisher_npub,
                crate::archive_commands::account_check(&app_state, &signer_state),
            )
            .await
            .expect("live archive hash should complete");
        let request = PublishAdpListingRequest {
            expected_publisher_npub,
            existing_event_id: None,
            existing_d_tag: None,
            pending_d_tag: None,
            title: "Gate 3 Live Test Binary".to_string(),
            description: "Small live ADP command-level publish test fixture".to_string(),
            price_sats,
            lud16: Some(lud16.clone()),
            tags: vec![],
            images: vec![],
            fulfillment_mode: FulfillmentMode::Delegate,
            operator_url: Some(server_url.clone()),
            servers: vec![server_url.clone()],
            accepted_terms: Vec::new(),
            file_path: None,
            selected_build: Some(BuildSelectionRef {
                selection_id: hashed.selection_id,
                sha256: hashed.sha256,
            }),
            existing_file_hash: None,
            existing_fulfillment_pubkey: None,
            version: Some("0.0.1-live".to_string()),
            acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
            platforms: vec!["linux-x86_64".to_string()],
            campaigns: Vec::new(),
            nip94_event_id: None,
        };

        let result = publish_adp_listing(
            request,
            app.handle().clone(),
            app.state::<AppState>(),
            app.state::<Arc<AsyncMutex<AppSignerState>>>(),
            app.state::<ArchiveSelectionService>(),
        )
        .await
        .expect("live ADP publish command should complete");

        let file_hash = result.file_hash.as_ref().expect("file hash should be set");
        let acceptance_event_id = result
            .acceptance_event_id
            .as_ref()
            .expect("acceptance event id should be set");
        let fulfillment_pubkey = result
            .fulfillment_pubkey
            .as_ref()
            .expect("fulfillment pubkey should be set");
        let upload = result
            .uploads
            .first()
            .and_then(|upload| upload.upload.as_ref())
            .expect("upload should succeed");

        assert_eq!(file_hash.len(), 64);
        assert!(!result.event_id.is_empty());
        assert!(!acceptance_event_id.is_empty());
        assert!(!fulfillment_pubkey.is_empty());
        assert!(!upload.download_url.is_empty());
        assert_eq!(&upload.file_hash, file_hash);
        let game_id = result
            .game_id
            .expect("a first publication should mint a game id");
        assert_eq!(game_id.get_version_num(), 4);
        assert_eq!(result.d_tag, game_id.to_string());
        assert_eq!(upload.game_coordinate, result.game_coordinate);
        assert!(
            upload.game_coordinate.ends_with(&result.d_tag),
            "upload response should reference the published listing coordinate"
        );
        println!("ADP_LIVE_EVENT_ID={}", result.event_id);
        println!("ADP_LIVE_GAME_COORDINATE={}", upload.game_coordinate);
        println!("ADP_LIVE_DOWNLOAD_URL={}", upload.download_url);

        let listing_event = fetch_live_listing_event(&app, &result.event_id).await;
        assert_live_listing_tags(&listing_event, &result, &lud16);
    }

    fn listing_parts_from_coordinate(coordinate: &str) -> (String, String) {
        let mut parts = coordinate.splitn(3, ':');
        assert_eq!(parts.next(), Some("30402"));
        let publisher_hex = parts
            .next()
            .expect("coordinate publisher should exist")
            .to_string();
        let listing_id = parts
            .next()
            .expect("coordinate d tag should exist")
            .to_string();
        let publisher_npub = nostr::PublicKey::from_hex(&publisher_hex)
            .expect("coordinate publisher should parse")
            .to_bech32()
            .expect("coordinate publisher should encode as npub");
        (publisher_npub, listing_id)
    }

    fn live_app_listing_from_coordinate(coordinate: &str) -> arcadestr_app::models::GameListing {
        let mut parts = coordinate.splitn(3, ':');
        assert_eq!(parts.next(), Some("30402"));
        let publisher_hex = parts.next().expect("coordinate publisher should exist");
        let listing_id = parts.next().expect("coordinate d tag should exist");
        let publisher_npub = nostr::PublicKey::from_hex(publisher_hex)
            .expect("coordinate publisher should parse")
            .to_bech32()
            .expect("coordinate publisher should encode as npub");

        arcadestr_app::models::GameListing {
            id: listing_id.to_string(),
            source: arcadestr_app::models::ListingSource::Nip99Listing,
            title: format!("ADP live fixture {listing_id}"),
            description: "ADP Gate 5 live install fixture".to_string(),
            images: Vec::new(),
            download_url: String::new(),
            price: 1.0,
            currency: "SATS".to_string(),
            price_sats: 1,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub,
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at: 1_700_000_000,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: arcadestr_app::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: true,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    fn live_listing_file_hash(event: &nostr::Event) -> String {
        tag_values(event, "file_hash")
            .into_iter()
            .find_map(|values| values.get(1).cloned())
            .expect("live listing should include file_hash tag")
    }

    fn live_listing_server_url(event: &nostr::Event) -> String {
        tag_values(event, "server")
            .into_iter()
            .find_map(|values| values.get(1).cloned())
            .expect("live listing should include server tag")
    }

    struct LiveConfirmPurchaseInput<'a> {
        app: &'a tauri::App<tauri::test::MockRuntime>,
        server_url: &'a str,
        coordinate: &'a str,
        lud16: &'a str,
        nwc_connection: &'a arcadestr_core::nwc_client::NwcConnection,
        use_nwc_payment: bool,
    }

    async fn live_confirm_purchase_pass(
        input: LiveConfirmPurchaseInput<'_>,
    ) -> ConfirmPurchaseResponse {
        eprintln!(
            "ADP_GATE4_STAGE=confirm_pass_start nwc={}",
            input.use_nwc_payment
        );
        let (publisher_npub, listing_id) = listing_parts_from_coordinate(input.coordinate);
        eprintln!(
            "ADP_GATE4_STAGE=request_invoice_start nwc={}",
            input.use_nwc_payment
        );
        let invoice = request_lnurl_invoice(
            RequestLnurlInvoiceRequest {
                lud16: input.lud16.to_string(),
                amount_sats: 1,
            },
            input.app.state::<AppState>(),
        )
        .await
        .expect("LNURL invoice should resolve and request");
        eprintln!(
            "ADP_GATE4_STAGE=request_invoice_ok nwc={}",
            input.use_nwc_payment
        );
        let preimage = if input.use_nwc_payment {
            eprintln!("ADP_GATE4_STAGE=pay_nwc_start primary");
            arcadestr_core::nwc_client::NwcClient::new(input.nwc_connection.clone())
                .pay_invoice(&invoice.bolt11)
                .await
                .expect("NWC pay_invoice should return preimage")
                .preimage
        } else {
            eprintln!("ADP_GATE4_STAGE=pay_nwc_start manual_preimage_fixture");
            arcadestr_core::nwc_client::NwcClient::new(input.nwc_connection.clone())
                .pay_invoice(&invoice.bolt11)
                .await
                .expect("test backend should pay second invoice for manual-path preimage")
                .preimage
        };
        eprintln!(
            "ADP_GATE4_STAGE=pay_preimage_ok nwc={}",
            input.use_nwc_payment
        );

        eprintln!(
            "ADP_GATE4_STAGE=confirm_purchase_start nwc={}",
            input.use_nwc_payment
        );
        confirm_purchase(
            ConfirmPurchaseRequest {
                publisher_npub,
                listing_id,
                server_url: input.server_url.to_string(),
                bolt11: invoice.bolt11,
                preimage,
            },
            input.app.state::<AppState>(),
        )
        .await
        .expect("purchase confirmation should persist receipt and token")
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL, ARCADESTR_RELAYS, ADP_TEST_LUD16, ADP_TEST_GAME_COORDINATE, and ADP_TEST_BUYER_NWC"]
    async fn live_nwc_purchase_confirm_and_manual_preimage_paths() {
        eprintln!("ADP_GATE4_STAGE=test_start");
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for live Gate 4 test");
        let coordinate = std::env::var("ADP_TEST_GAME_COORDINATE")
            .expect("ADP_TEST_GAME_COORDINATE must be set for live Gate 4 test");
        let lud16 = std::env::var("ADP_TEST_LUD16")
            .expect("ADP_TEST_LUD16 must be set for live Gate 4 test");
        let nwc_connection_string = std::env::var("ADP_TEST_BUYER_NWC")
            .expect("ADP_TEST_BUYER_NWC must be set for live Gate 4 test");
        let relays = live_relays_from_env();

        eprintln!("ADP_GATE4_STAGE=app_init_start");
        let app = live_test_app(relays).await;
        eprintln!("ADP_GATE4_STAGE=app_init_ok");
        eprintln!("ADP_GATE4_STAGE=parse_nwc_start");
        let nwc_connection =
            arcadestr_core::nwc_client::NwcConnection::parse(&nwc_connection_string)
                .expect("buyer NWC connection string should parse");
        let connect_result = nwc_connection_response(&nwc_connection);
        eprintln!("ADP_GATE4_STAGE=parse_nwc_ok");
        assert!(!connect_result.wallet_pubkey.is_empty());
        assert!(!connect_result.relays.is_empty());

        let primary = live_confirm_purchase_pass(LiveConfirmPurchaseInput {
            app: &app,
            server_url: &server_url,
            coordinate: &coordinate,
            lud16: &lud16,
            nwc_connection: &nwc_connection,
            use_nwc_payment: true,
        })
        .await;
        eprintln!("ADP_GATE4_STAGE=primary_confirm_ok");
        assert!(!primary.download_token.is_empty());
        assert!(primary.token_expires_at > 0);

        let buyer_pubkey = {
            let state = app.state::<AppState>();
            let auth = state.auth.lock().await;
            auth.public_key()
                .expect("live buyer should be authenticated")
                .to_hex()
        };
        let owned = app
            .state::<AppState>()
            .purchases
            .is_owned(&buyer_pubkey, &coordinate)
            .await
            .expect("ownership lookup should succeed");
        assert!(owned, "receipt persistence should flip is_owned");

        let token_repo =
            DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let token = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup should succeed")
            .expect("download token should be stored");
        assert_eq!(token.token, primary.download_token);
        println!("ADP_GATE4_DOWNLOAD_TOKEN={}", primary.download_token);

        let manual = live_confirm_purchase_pass(LiveConfirmPurchaseInput {
            app: &app,
            server_url: &server_url,
            coordinate: &coordinate,
            lud16: &lud16,
            nwc_connection: &nwc_connection,
            use_nwc_payment: false,
        })
        .await;
        eprintln!("ADP_GATE4_STAGE=manual_confirm_ok");
        assert!(!manual.download_token.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL, ARCADESTR_RELAYS, ADP_TEST_GAME_COORDINATE, ADP_TEST_MANUAL_BOLT11, and ADP_TEST_MANUAL_PREIMAGE"]
    async fn live_manual_preimage_purchase_confirm_persists_receipt_and_token() {
        eprintln!("ADP_GATE4_STAGE=manual_test_start");
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for manual Gate 4 test");
        let coordinate = std::env::var("ADP_TEST_GAME_COORDINATE")
            .expect("ADP_TEST_GAME_COORDINATE must be set for manual Gate 4 test");
        let bolt11 = std::env::var("ADP_TEST_MANUAL_BOLT11")
            .expect("ADP_TEST_MANUAL_BOLT11 must be set for manual Gate 4 test");
        let preimage = std::env::var("ADP_TEST_MANUAL_PREIMAGE")
            .expect("ADP_TEST_MANUAL_PREIMAGE must be set for manual Gate 4 test");
        let relays = live_relays_from_env();

        eprintln!("ADP_GATE4_STAGE=manual_app_init_start");
        let app = live_test_app(relays).await;
        eprintln!("ADP_GATE4_STAGE=manual_app_init_ok");
        let (publisher_npub, listing_id) = listing_parts_from_coordinate(&coordinate);
        eprintln!("ADP_GATE4_STAGE=manual_confirm_purchase_start");
        let response = confirm_purchase(
            ConfirmPurchaseRequest {
                publisher_npub,
                listing_id,
                server_url: server_url.clone(),
                bolt11,
                preimage,
            },
            app.state::<AppState>(),
        )
        .await
        .expect("manual preimage purchase confirmation should persist receipt and token");
        eprintln!("ADP_GATE4_STAGE=manual_confirm_purchase_ok");
        assert!(!response.download_token.is_empty());
        assert!(response.token_expires_at > 0);

        let buyer_pubkey = {
            let state = app.state::<AppState>();
            let auth = state.auth.lock().await;
            auth.public_key()
                .expect("live buyer should be authenticated")
                .to_hex()
        };
        let owned = app
            .state::<AppState>()
            .purchases
            .is_owned(&buyer_pubkey, &coordinate)
            .await
            .expect("ownership lookup should succeed");
        assert!(owned, "manual receipt persistence should flip is_owned");

        let token_repo =
            DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let token = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup should succeed")
            .expect("download token should be stored");
        assert_eq!(token.token, response.download_token);
        println!(
            "ADP_GATE4_MANUAL_DOWNLOAD_TOKEN={}",
            response.download_token
        );
    }

    #[tokio::test]
    #[ignore = "requires ADP_TEST_SERVER_URL, ARCADESTR_RELAYS, ADP_TEST_GAME_COORDINATE, ADP_TEST_MANUAL_BOLT11, and ADP_TEST_MANUAL_PREIMAGE"]
    async fn live_install_game_path_a_and_path_b() {
        eprintln!("ADP_GATE5_STAGE=test_start");
        let server_url = std::env::var("ADP_TEST_SERVER_URL")
            .expect("ADP_TEST_SERVER_URL must be set for live Gate 5 test");
        let coordinate = std::env::var("ADP_TEST_GAME_COORDINATE")
            .expect("ADP_TEST_GAME_COORDINATE must be set for live Gate 5 test");
        let bolt11 = std::env::var("ADP_TEST_MANUAL_BOLT11")
            .expect("ADP_TEST_MANUAL_BOLT11 must be set for live Gate 5 test");
        let preimage = std::env::var("ADP_TEST_MANUAL_PREIMAGE")
            .expect("ADP_TEST_MANUAL_PREIMAGE must be set for live Gate 5 test");
        let relays = live_relays_from_env();

        eprintln!("ADP_GATE5_STAGE=app_init_start");
        let app = live_test_app(relays).await;
        eprintln!("ADP_GATE5_STAGE=app_init_ok");
        let state = app.state::<AppState>();
        let buyer_pubkey = {
            let auth = state.auth.lock().await;
            auth.public_key()
                .expect("live buyer should be authenticated")
                .to_hex()
        };
        let listing_event = fetch_listing_event_by_coordinate(&state, &coordinate)
            .await
            .expect("live listing event should fetch before install");
        let expected_file_hash = live_listing_file_hash(&listing_event);
        let listing_server_url = live_listing_server_url(&listing_event);
        assert_eq!(
            listing_server_url, server_url,
            "ADP_TEST_SERVER_URL must match the listing server tag used by install_game"
        );
        let listing = live_app_listing_from_coordinate(&coordinate);
        let (publisher_npub, listing_id) = listing_parts_from_coordinate(&coordinate);
        let fresh_listing_fetcher = crate::RelayFreshListingFetcher {
            state: state.clone(),
        };
        let app_data_dir = app
            .path()
            .app_local_data_dir()
            .expect("live test app data dir should resolve");
        let artifact_path = crate::deterministic_artifact_path(&app_data_dir, &coordinate);

        eprintln!("ADP_GATE5_STAGE=manual_confirm_purchase_start");
        let response = confirm_purchase(
            ConfirmPurchaseRequest {
                publisher_npub,
                listing_id,
                server_url: server_url.clone(),
                bolt11,
                preimage,
            },
            app.state::<AppState>(),
        )
        .await
        .expect("manual preimage purchase confirmation should persist receipt and token");
        eprintln!("ADP_GATE5_STAGE=manual_confirm_purchase_ok");
        assert!(!response.download_token.is_empty());
        assert!(response.token_expires_at > 0);

        let token_repo =
            DownloadTokensRepository::new(app.state::<AppState>().database.pool().clone());
        let cached_token = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup should succeed")
            .expect("download token should be stored before Path A");
        assert_eq!(cached_token.token, response.download_token);
        println!("ADP_GATE5_PATH_A_TOKEN_PRESENT=true");

        eprintln!("ADP_GATE5_STAGE=path_a_install_start");
        crate::install_game_with_fetcher(
            listing.clone(),
            &state,
            Some(app.handle()),
            app_data_dir.clone(),
            &fresh_listing_fetcher,
        )
        .await
        .expect("Path A install with cached token should succeed");
        let installations = arcadestr_core::adp_storage::InstallationsRepository::new(
            app.state::<AppState>().database.pool().clone(),
        );
        assert!(
            installations
                .get_by_coordinate(&coordinate)
                .await
                .expect("Path A installation lookup should succeed")
                .is_none(),
            "a verified download is not an extracted installation"
        );
        assert!(
            artifact_path.exists(),
            "Path A artifact should exist at {}",
            artifact_path.display()
        );
        let path_a_hash = sha256_file(&artifact_path)
            .await
            .expect("Path A artifact hash should compute");
        assert_eq!(path_a_hash, expected_file_hash);
        let installed_games = crate::get_installed_games(app.state::<AppState>())
            .await
            .expect("get_installed_games should be callable in live test");
        assert!(!installed_games
            .iter()
            .any(|game| game.game_coordinate == coordinate));
        println!(
            "ADP_GATE5_PATH_A_DOWNLOAD_OK=true artifact_exists=true sha256_match=true \
             installation_row=false get_installed_games_excludes_coordinate=true"
        );

        token_repo
            .delete(&buyer_pubkey, &coordinate, &server_url)
            .await
            .expect("Path B should delete local token row");
        let token_after_delete = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup after delete should succeed");
        assert!(token_after_delete.is_none());
        println!("ADP_GATE5_PATH_B_LOCAL_TOKEN_DELETED=true");
        tokio::fs::remove_file(&artifact_path)
            .await
            .expect("Path B should remove Path A artifact before reinstall");
        tokio::time::sleep(Duration::from_secs(1)).await;

        eprintln!("ADP_GATE5_STAGE=path_b_install_start");
        crate::install_game_with_fetcher(
            listing,
            &state,
            Some(app.handle()),
            app_data_dir,
            &fresh_listing_fetcher,
        )
        .await
        .expect("Path B install without local token should succeed");
        assert!(
            artifact_path.exists(),
            "Path B artifact should exist at {}",
            artifact_path.display()
        );
        let path_b_hash = sha256_file(&artifact_path)
            .await
            .expect("Path B artifact hash should compute");
        assert_eq!(path_b_hash, expected_file_hash);
        assert!(
            installations
                .get_by_coordinate(&coordinate)
                .await
                .expect("Path B installation lookup should succeed")
                .is_none(),
            "a repeated verified download is still not an extracted installation"
        );
        let token_after_path_b = token_repo
            .valid_token(
                &buyer_pubkey,
                &coordinate,
                &server_url,
                now_unix_i64().expect("clock should work"),
            )
            .await
            .expect("download token lookup after Path B should succeed");
        assert!(token_after_path_b.is_none());
        println!(
            "ADP_GATE5_PATH_B_NIP98_DOWNLOAD_OK=true artifact_exists=true sha256_match=true \
             installation_row=false local_token_deleted_before_download=true \
             local_token_absent_after_download=true"
        );
    }

    struct ProvisionedAttestation {
        row: AdpProvisioning,
        operator: nostr::PublicKey,
        event: nostr::Event,
    }

    fn provisioned_attestation(scope: &str, valid_from: u64) -> ProvisionedAttestation {
        let operator = Keys::generate();
        let developer = Keys::generate();
        let fulfillment = Keys::generate();
        let developer_hex = developer.public_key().to_hex();
        let fulfillment_hex = fulfillment.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(30404), "")
            .tags([
                Tag::parse(["d", &format!("{developer_hex}:{fulfillment_hex}")])
                    .expect("d tag should parse"),
                Tag::parse(["p", &developer_hex]).expect("p tag should parse"),
                Tag::parse([
                    "fulfillment_pubkey",
                    &fulfillment_hex,
                    &valid_from.to_string(),
                    "",
                ])
                .expect("fulfillment tag should parse"),
                Tag::parse(["scope", scope]).expect("scope tag should parse"),
            ])
            .custom_created_at(Timestamp::from_secs(valid_from))
            .sign_with_keys(&operator)
            .expect("attestation should sign");
        let row = AdpProvisioning {
            id: format!("{developer_hex}:operator:{scope}"),
            developer_npub: developer_hex,
            server_url: "http://localhost:9099".to_string(),
            operator_pubkey: operator.public_key().to_hex(),
            scope: Some(scope.to_string()),
            fulfillment_pubkey: fulfillment_hex,
            attestation_event_id: event.id.to_hex(),
            acceptance_event_id: "acceptance".to_string(),
            authorization_root_event_id: Some("acceptance".to_string()),
            authorization_capabilities: Vec::new(),
            authorization_profile_version: 2,
            valid_from: valid_from as i64,
            revoked_at: None,
            created_at: valid_from as i64,
        };
        ProvisionedAttestation {
            row,
            operator: operator.public_key(),
            event,
        }
    }

    #[test]
    fn an_attestation_minted_during_this_publish_is_not_read_as_stale() {
        // The operator stamps `valid_from` when it provisions the key, which is
        // after publishing began. Validating against a clock read before the
        // build was hashed rejected every first-time provisioning.
        let provisioned = provisioned_attestation("f47ac10b-58cc-4372-a567-0e02b2c3d479", 1_000);

        assert_eq!(
            ensure_attestation_covers_provisioning(
                &provisioned.row,
                provisioned.operator,
                &provisioned.event,
                999,
            ),
            Ok(())
        );
    }

    #[test]
    fn an_attestation_for_another_scope_is_rejected_as_uncovered() {
        let provisioned = provisioned_attestation("some-other-game", 1_000);
        let mut row = provisioned.row.clone();
        row.scope = Some("this-game".to_string());

        let error = ensure_attestation_covers_provisioning(
            &row,
            provisioned.operator,
            &provisioned.event,
            2_000,
        )
        .expect_err("an attestation for a different scope must not be accepted");

        assert!(error.contains("does not cover"));
    }

    #[test]
    fn an_attestation_from_the_distant_future_is_still_rejected() {
        let provisioned = provisioned_attestation("this-game", 10_000);

        let error = ensure_attestation_covers_provisioning(
            &provisioned.row,
            provisioned.operator,
            &provisioned.event,
            1_000,
        )
        .expect_err("an attestation that is not yet in force must not be accepted");

        assert!(error.contains("no longer attests possession"));
    }
}
