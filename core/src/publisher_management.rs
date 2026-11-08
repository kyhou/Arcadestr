//! Publisher management uses existing NIP-103 chains and local recovery evidence.
use crate::authorization::{
    CAPABILITY_ISSUE_GRANT, CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD,
};
use crate::publisher_recovery::PublisherCheckpoint;
use nostr::{Event, EventBuilder, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
#[cfg(feature = "native")]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationAction {
    CancelAuthority,
    RevokeGrant,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokePublisherCredentialRequest {
    pub expected_publisher_npub: String,
    pub coordinate: String,
    pub root_event_id: String,
    pub expected_tip_event_id: String,
    pub action: RevocationAction,
    pub operation_id: String,
    pub confirm: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialReview {
    pub root_event_id: String,
    pub tip_event_id: String,
    pub coordinate: String,
    pub subject: String,
    pub terminal: bool,
}

/// A new UI submission cannot bypass an unresolved signed transition by minting
/// another operation ID. Completed operations remain historical, not replay work.
pub fn validate_revocation_submission(
    request: &RevokePublisherCredentialRequest,
    operations: &[crate::publisher_recovery::PublisherOperation],
) -> Result<bool, String> {
    if !request.confirm {
        return Err("Explicit developer confirmation is required".into());
    }
    if let Some(previous) = operations
        .iter()
        .find(|op| op.operation_id == request.operation_id)
    {
        if previous.kind != "credential_revocation"
            || previous.coordinate != request.coordinate
            || previous.intent
                != serde_json::to_value(request).map_err(|_| "Invalid revocation intent")?
        {
            return Err("Revocation intent is immutable".into());
        }
        return Ok(true);
    }
    if operations.iter().any(|op| {
        op.kind == "credential_revocation"
            && op.state != "complete"
            && op.coordinate == request.coordinate
            && op.intent["root_event_id"].as_str() == Some(request.root_event_id.as_str())
    }) {
        return Err("An incomplete transition already exists for this credential; reconcile its original operation in Publisher recovery".into());
    }
    Ok(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRoleIntent {
    pub server_url: String,
    pub issue_receipt: bool,
    pub issue_grant: bool,
}
impl ServerRoleIntent {
    pub fn capabilities(&self) -> BTreeSet<String> {
        let mut result = BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()]);
        if self.issue_receipt {
            result.insert(CAPABILITY_ISSUE_RECEIPT.into());
        }
        if self.issue_grant {
            result.insert(CAPABILITY_ISSUE_GRANT.into());
        }
        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorPlan {
    pub operator: PublicKey,
    pub urls: Vec<String>,
    pub capabilities: BTreeSet<String>,
}

/// A single held signer per operator/scope, even when it advertises several URLs.
/// Contradictory alias roles are rejected rather than silently escalating a mirror.
#[cfg(feature = "native")]
pub fn operator_plans(
    servers: &[(String, String)],
    roles: &[ServerRoleIntent],
    legacy_issuer: Option<&str>,
) -> Result<Vec<OperatorPlan>, String> {
    let mut plans: BTreeMap<String, OperatorPlan> = BTreeMap::new();
    let mut seen = BTreeMap::new();
    let mut role_urls = BTreeSet::new();
    for role in roles {
        if !role_urls.insert(&role.server_url)
            || !servers.iter().any(|(url, _)| url == &role.server_url)
        {
            return Err("Roles must name each selected URL at most once".into());
        }
    }
    for (url, key) in servers {
        crate::http_client::validate_publication_endpoint(url)
            .map_err(|_| "Unsafe distribution URL")?;
        if let Some(previous) = seen.insert(url, key) {
            if previous != key {
                return Err("A selected URL advertised conflicting operator identities".into());
            }
            continue;
        }
        let operator =
            PublicKey::from_hex(key).map_err(|_| "Invalid distribution operator identity")?;
        let fallback = ServerRoleIntent {
            server_url: url.clone(),
            issue_receipt: legacy_issuer == Some(url.as_str()),
            issue_grant: legacy_issuer == Some(url.as_str()),
        };
        let role = roles
            .iter()
            .find(|role| role.server_url == *url)
            .unwrap_or(&fallback);
        let capabilities = role.capabilities();
        if let Some(plan) = plans.get_mut(&operator.to_hex()) {
            if plan.capabilities != capabilities {
                return Err("URLs for the same operator must have identical issuing roles".into());
            }
            plan.urls.push(url.clone());
        } else {
            plans.insert(
                operator.to_hex(),
                OperatorPlan {
                    operator,
                    urls: vec![url.clone()],
                    capabilities,
                },
            );
        }
    }
    Ok(plans.into_values().collect())
}

/// Cancellation changes future issuance only; immutable authority terms survive.
pub fn cancel_authority_builder(
    authority: &crate::authorization::ResolvedAuthorization,
    publisher: PublicKey,
    expected_tip: nostr::EventId,
    now: u64,
) -> Result<EventBuilder, String> {
    let root = authority.events.first().ok_or("Missing authority root")?;
    if authority.developer_pubkey != publisher
        || authority.root_event_id != expected_tip
        || authority.cancelled_at.is_some()
        || now <= root.event.created_at.as_secs()
    {
        return Err("Authority is stale, already cancelled, or not owned by this publisher".into());
    }
    let tags = root
        .event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) != Some("status"))
        .cloned()
        .chain([
            Tag::parse(["status", "cancelled"]).map_err(|_| "Invalid cancellation status")?,
            Tag::parse(["e", &root.event.id.to_hex()])
                .map_err(|_| "Invalid cancellation predecessor")?,
        ]);
    Ok(EventBuilder::new(Kind::Custom(30406), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(now)))
}

pub fn revoke_grant_builder(
    grant: &crate::entitlements::ResolvedEntitlementGrant,
    publisher: PublicKey,
    expected_tip: nostr::EventId,
    now: u64,
) -> Result<EventBuilder, String> {
    use crate::entitlements::GrantStatus;
    let tip = grant.events.last().ok_or("Missing grant")?;
    if crate::adp_protocol::coordinate_publisher(&tip.coordinate) != Some(publisher)
        || tip.event.id != expected_tip
        || tip.status != GrantStatus::Granted
        || now <= tip.event.created_at.as_secs()
    {
        return Err("Grant is stale, already revoked, or not owned by this publisher".into());
    }
    let tags = tip
        .event
        .tags
        .iter()
        .filter(|tag| {
            matches!(
                tag.as_slice().first().map(String::as_str),
                Some("d" | "a" | "p" | "source_event" | "authorization")
            )
        })
        .cloned()
        .chain([
            Tag::parse(["status", "revoked"]).map_err(|_| "Invalid revocation status")?,
            Tag::parse(["e", &expected_tip.to_hex()])
                .map_err(|_| "Invalid revocation predecessor")?,
        ]);
    Ok(EventBuilder::new(
        Kind::Custom(crate::adp_protocol::ENTITLEMENT_GRANT_KIND),
        "",
    )
    .tags(tags)
    .custom_created_at(Timestamp::from_secs(now)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionState {
    Declared,
    Provisioned,
    UploadPending,
    RegistrationAcknowledged,
    Failed,
    Unknown,
}
impl DistributionState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Declared => "Declared",
            Self::Provisioned => "Provisioned (historical response)",
            Self::UploadPending => "Upload pending",
            Self::RegistrationAcknowledged => "Registration acknowledged (historical upload)",
            Self::Failed => "Failed before upload",
            Self::Unknown => "Unknown remote outcome",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionEvidence {
    pub server_url: String,
    pub operation_id: String,
    pub state: DistributionState,
    pub operator: Option<String>,
    pub capabilities: Vec<String>,
    pub diagnostic: Option<String>,
}

/// Per-operation evidence, never a current-availability assertion. Never merge
/// acknowledgements from a different artifact/revision into a later operation.
pub fn distribution_evidence(
    server: &str,
    operation_id: &str,
    steps: &[PublisherCheckpoint],
) -> DistributionEvidence {
    let relevant = steps
        .iter()
        .filter(|step| step.target == server)
        .collect::<Vec<_>>();
    let upload = relevant.iter().find(|step| step.kind == "upload");
    let declared = relevant.iter().find(|step| step.kind == "distribution");
    let provision_url = declared
        .and_then(|step| step.intent["provision_url"].as_str())
        .unwrap_or(server);
    let provision = steps
        .iter()
        .find(|step| step.kind == "provision" && step.target == provision_url);
    let upload_expected =
        declared.is_some_and(|step| step.intent["upload_expected"].as_bool() == Some(true));
    let listing_acknowledged = steps.iter().any(|step| {
        step.kind == "relay"
            && step.intent["event"]["kind"].as_u64() == Some(30402)
            && step.outcome.is_some()
    });
    let state = if upload.is_some_and(|step| step.outcome.is_some()) {
        DistributionState::RegistrationAcknowledged
    } else if upload.is_some() || provision.is_some_and(|step| step.outcome.is_none()) {
        DistributionState::Unknown
    } else if relevant.iter().any(|step| step.last_error.is_some()) {
        DistributionState::Failed
    } else if upload_expected && listing_acknowledged {
        DistributionState::UploadPending
    } else if provision.is_some_and(|step| step.outcome.is_some()) {
        DistributionState::Provisioned
    } else if upload_expected {
        DistributionState::UploadPending
    } else {
        DistributionState::Declared
    };
    DistributionEvidence {
        server_url: server.into(),
        operation_id: operation_id.into(),
        state,
        operator: declared.and_then(|step| step.intent["operator"].as_str().map(str::to_owned)),
        capabilities: declared
            .and_then(|step| serde_json::from_value(step.intent["capabilities"].clone()).ok())
            .unwrap_or_default(),
        diagnostic: relevant.iter().find_map(|step| step.last_error.clone()),
    }
}

/// Validate a signed transition against the same central chain resolver used by buyers.
pub fn validate_revocation(
    root: &Event,
    transition: &Event,
    authorization: bool,
) -> Result<(), String> {
    if authorization {
        let resolved = crate::authorization::resolve_authorization(
            root.id,
            &[root.clone(), transition.clone()],
        )
        .map_err(|e| e.to_string())?;
        if resolved.events.last().map(|event| event.event.id) != Some(transition.id) {
            return Err("Invalid authority cancellation".into());
        }
    } else {
        let nodes = [root, transition]
            .into_iter()
            .map(crate::entitlements::parse_entitlement_event)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let resolved =
            crate::entitlements::resolve_entitlement_grant(&nodes).map_err(|e| e.to_string())?;
        if resolved.events.last().map(|event| event.event.id) != Some(transition.id) {
            return Err("Invalid grant revocation".into());
        }
    }
    Ok(())
}
