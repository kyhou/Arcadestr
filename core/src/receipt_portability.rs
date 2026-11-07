//! Read-only receipt acquisition. All cryptographic decisions remain in the
//! existing purchases/authorization modules. No signer secrets or wallet access.
use crate::{
    adp_protocol::coordinate_publisher,
    authorization::{
        parse_authorization_event, resolve_authorization, FULFILLMENT_AUTHORIZATION_KIND,
    },
    purchases::{
        parse_receipt_event, resolve_receipt_chain, validate_receipt_evidence, ReceiptEvidence,
        ReceiptProof,
    },
    receipt_sync_contract::{ReceiptChainResult, ReceiptSyncReport, ReceiptVerification},
};
use async_trait::async_trait;
use nostr::{Event, EventId, Filter, JsonUtil, Kind, PublicKey, Timestamp};
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub const MAX_IMPORT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVENTS: usize = 2048;
pub const PAGE_SIZE: usize = 256;
const MAX_PAGES: usize = 4;
type Guard<'a> = &'a (dyn Fn() -> Result<(), String> + Send + Sync);

pub struct AggregateLspEvidence {
    pub key: PublicKey,
    pub signed_events: Vec<Event>,
}

/// Resolve public aggregate Zap provenance using the immutable listing and
/// developer-accepted signed terms. Never infer a Lightning key from a Nostr key.
pub fn aggregate_zap_destination(
    receipt: &crate::purchases::ParsedReceipt,
    leg: &crate::adp_settlement::AggregateProofLeg,
    listing: &Event,
    terms: Option<&Event>,
) -> Result<String, String> {
    use crate::adp_settlement::{parse_terms_event, PaymentRole, TermsPayload};
    crate::purchases::validate_receipt_listing_binding(receipt, listing)
        .map_err(|_| "Aggregate listing binding invalid")?;
    if receipt
        .aggregate
        .as_ref()
        .is_none_or(|a| a.listing_event_id != listing.id)
    {
        return Err("Aggregate immutable listing differs".into());
    }
    match leg.role {
        PaymentRole::Developer => {
            if leg.recipient_pubkey != listing.pubkey.to_hex() || leg.accepted_terms_id.is_some() {
                return Err("Aggregate developer binding invalid".into());
            }
            let addresses = listing
                .tags
                .iter()
                .filter_map(|t| match t.as_slice() {
                    [name, value] if name == "lud16" => Some(value.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if addresses.len() != 1 {
                return Err("Aggregate listing payment destination unavailable".into());
            }
            Ok(addresses[0].clone())
        }
        PaymentRole::Operator => {
            let terms = terms.ok_or("Aggregate operator terms unavailable")?;
            if leg.accepted_terms_id.as_deref() != Some(terms.id.to_hex().as_str()) {
                return Err("Aggregate terms root differs".into());
            }
            let accepted=listing.tags.iter().filter(|t|matches!(t.as_slice(),[name,recipient,id] if name=="adp_terms"&&recipient==&leg.recipient_pubkey&&id==&terms.id.to_hex())).count();
            let TermsPayload::Offer { terms } =
                parse_terms_event(terms).map_err(|_| "Invalid aggregate operator terms")?
            else {
                return Err("Aggregate terms are not an offer".into());
            };
            if accepted != 1 || terms.operator_pubkey != leg.recipient_pubkey {
                return Err("Aggregate operator terms not developer-accepted".into());
            }
            Ok(terms.payment_destination)
        }
    }
}

#[async_trait]
pub trait ReceiptSource: Send + Sync {
    fn relay_urls(&self) -> Vec<String>;
    async fn query(&self, relay: &str, filter: Filter) -> Result<Vec<Event>, String>;
    // nostr 0.44 Filter supports only single-letter tags. A future existing
    // transport may implement this optimization; #p fallback is always retained.
    fn supports_order_filter(&self, _relay: &str) -> bool {
        false
    }
    async fn query_order(
        &self,
        _relay: &str,
        _buyer: PublicKey,
        _order: &str,
    ) -> Result<Vec<Event>, String> {
        Err("Order indexing unavailable".into())
    }
    /// Existing LNURL metadata resolution only; never request an invoice.
    async fn lsp_pubkey(&self, _coordinate: &str) -> Result<Option<PublicKey>, String> {
        Ok(None)
    }
    async fn aggregate_lsp(
        &self,
        _receipt: &crate::purchases::ParsedReceipt,
        _leg: &crate::adp_settlement::AggregateProofLeg,
    ) -> Result<Option<AggregateLspEvidence>, String> {
        Ok(None)
    }
}

pub struct ReceiptPortability {
    pool: SqlitePool,
}
impl ReceiptPortability {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn import_json(
        &self,
        buyer: PublicKey,
        json: &str,
        source: Option<&dyn ReceiptSource>,
        guard: Guard<'_>,
    ) -> Result<ReceiptSyncReport, String> {
        if json.len() > MAX_IMPORT_BYTES {
            return Err("Receipt import exceeds 4 MiB".into());
        }
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|_| "Expected a signed Nostr event or JSON array of events")?;
        let values = if let serde_json::Value::Array(values) = value {
            values
        } else {
            vec![value]
        };
        if values.is_empty() || values.len() > MAX_EVENTS {
            return Err("Import must contain 1–2048 signed events".into());
        }
        let mut rejected = 0;
        let events = values
            .into_iter()
            .filter_map(|value| match serde_json::from_value::<Event>(value) {
                Ok(e) => Some(e),
                Err(_) => {
                    rejected += 1;
                    None
                }
            })
            .collect();
        let mut report = self.ingest(buyer, events, source, guard).await?;
        report.rejected_events += rejected;
        Ok(report)
    }

    async fn evidence(&self, buyer: PublicKey) -> Result<BTreeMap<EventId, Event>, String> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT raw_event FROM receipt_sync_events WHERE buyer_pubkey=? UNION SELECT raw_event FROM purchases WHERE buyer_pubkey=?")
            .bind(buyer.to_hex()).bind(buyer.to_hex()).fetch_all(&self.pool).await.map_err(|_|"Receipt evidence unavailable")?;
        Ok(rows
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<Event>(&raw).ok())
            .map(|e| (e.id, e))
            .collect())
    }

    fn accept(buyer: PublicKey, event: &Event) -> bool {
        if event.verify().is_err() {
            return false;
        }
        match event.kind.as_u16() {
            1020 => parse_receipt_event(event).is_ok_and(|r| r.buyer_pubkey == buyer),
            FULFILLMENT_AUTHORIZATION_KIND => parse_authorization_event(event).is_ok(),
            9735 => true,
            30402 => true,
            crate::adp_settlement::TERMS_KIND => {
                crate::adp_settlement::parse_terms_event(event).is_ok()
            }
            _ => false,
        }
    }

    pub async fn ingest(
        &self,
        buyer: PublicKey,
        incoming: Vec<Event>,
        source: Option<&dyn ReceiptSource>,
        guard: Guard<'_>,
    ) -> Result<ReceiptSyncReport, String> {
        guard()?;
        if incoming.len() > MAX_EVENTS {
            return Err("Too many receipt events".into());
        }
        let mut report = ReceiptSyncReport::default();
        let mut events = self.evidence(buyer).await?;
        let mut acquired = BTreeMap::new();
        for event in incoming {
            if Self::accept(buyer, &event) {
                events.insert(event.id, event.clone());
                acquired.insert(event.id, event);
            } else {
                report.rejected_events += 1;
            }
        }
        let mut lsp = HashMap::new();
        let mut leg_lsps = HashMap::new();
        let cached=sqlx::query("SELECT receipt_root_id,leg_id,lsp_pubkey FROM receipt_sync_leg_lsps WHERE buyer_pubkey=?").bind(buyer.to_hex()).fetch_all(&self.pool).await.map_err(|_|"Public proof evidence cache unavailable")?;
        for row in cached {
            if let (Ok(id), Ok(key)) = (
                EventId::from_hex(&row.get::<String, _>("receipt_root_id")),
                PublicKey::from_hex(&row.get::<String, _>("lsp_pubkey")),
            ) {
                leg_lsps.insert((id, row.get::<String, _>("leg_id")), key);
            }
        }
        if let Some(source) = source {
            // Bounded dependency closure. Unresolved predecessors/auth evidence
            // remain indeterminate, and are retried on the next synchronization.
            let mut requested = HashSet::new();
            let mut requested_auth = HashSet::new();
            let mut requested_orders = HashSet::new();
            for _ in 0..MAX_PAGES {
                guard()?;
                let receipts = events
                    .values()
                    .filter_map(|e| parse_receipt_event(e).ok())
                    .filter(|r| r.buyer_pubkey == buyer)
                    .collect::<Vec<_>>();
                let mut ids = BTreeSet::new();
                let mut authors = BTreeSet::new();
                let mut orders = BTreeSet::new();
                for r in &receipts {
                    if let Some(id) = r.predecessor {
                        if !events.contains_key(&id) && requested.insert(id) {
                            ids.insert(id);
                        }
                    }
                    if let Some(id) = r.authorization {
                        if !events.contains_key(&id) && requested.insert(id) {
                            ids.insert(id);
                        }
                        if let Some(auth) = events
                            .get(&id)
                            .and_then(|e| parse_authorization_event(e).ok())
                        {
                            if requested_auth.insert(id) {
                                if let Some(author) = coordinate_publisher(&r.coordinate) {
                                    authors.insert((author, auth.terms.authorization_id));
                                }
                            }
                        }
                    }
                    for proof in &r.proofs {
                        if let ReceiptProof::Zap(id) = proof {
                            if !events.contains_key(id) && requested.insert(*id) {
                                ids.insert(*id);
                            }
                        }
                    }
                    if let Some(aggregate) = &r.aggregate {
                        for leg in &aggregate.legs {
                            if leg.proof_type == "zap" {
                                if let Ok(id) = EventId::from_hex(&leg.proof_reference) {
                                    if !events.contains_key(&id) && requested.insert(id) {
                                        ids.insert(id);
                                    }
                                }
                            }
                        }
                    }
                    if requested_orders.insert(r.order_id.clone()) {
                        orders.insert(r.order_id.clone());
                    }
                }
                let mut found = Vec::new();
                for relay in source.relay_urls().into_iter().take(16) {
                    if !ids.is_empty() {
                        match source
                            .query(
                                &relay,
                                Filter::new()
                                    .ids(ids.iter().copied().take(PAGE_SIZE))
                                    .limit(PAGE_SIZE),
                            )
                            .await
                        {
                            Ok(batch) => found.extend(batch),
                            Err(_) => report.relay_failures += 1,
                        }
                    }
                    let author_roots = authors.iter().collect::<Vec<_>>();
                    for batch in author_roots.chunks(PAGE_SIZE / 2) {
                        let filter = Filter::new()
                            .kind(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND))
                            .authors(batch.iter().map(|(author, _)| *author))
                            .identifiers(batch.iter().map(|(_, id)| id.as_str()))
                            .limit(PAGE_SIZE);
                        match source.query(&relay, filter).await {
                            Ok(batch) => {
                                if batch.len() >= PAGE_SIZE {
                                    report.incomplete = true;
                                }
                                found.extend(batch);
                            }
                            Err(_) => report.relay_failures += 1,
                        }
                    }
                    if !orders.is_empty() {
                        if source.supports_order_filter(&relay) {
                            for order in orders.iter().take(32) {
                                if let Ok(batch) = source.query_order(&relay, buyer, order).await {
                                    found.extend(batch);
                                }
                            }
                        }
                        // Fallback also covers relays silently ignoring #order.
                        match source
                            .query(
                                &relay,
                                Filter::new()
                                    .kind(Kind::Custom(1020))
                                    .pubkey(buyer)
                                    .limit(PAGE_SIZE),
                            )
                            .await
                        {
                            Ok(batch) => {
                                if batch.len() >= PAGE_SIZE {
                                    report.incomplete = true;
                                }
                                found.extend(batch.into_iter().filter(|e| {
                                    parse_receipt_event(e)
                                        .is_ok_and(|r| orders.contains(&r.order_id))
                                }));
                            }
                            Err(_) => report.relay_failures += 1,
                        }
                    }
                }
                let mut changed = false;
                for event in found.into_iter().take(MAX_EVENTS) {
                    if Self::accept(buyer, &event) && !events.contains_key(&event.id) {
                        changed = true;
                        acquired.insert(event.id, event.clone());
                        events.insert(event.id, event);
                    }
                }
                if !changed {
                    break;
                }
            }
            let coordinates = events
                .values()
                .filter_map(|e| parse_receipt_event(e).ok())
                .filter(|r| r.proofs.iter().any(|p| matches!(p, ReceiptProof::Zap(_))))
                .map(|r| r.coordinate)
                .collect::<BTreeSet<_>>();
            for coordinate in coordinates.into_iter().take(32) {
                if let Ok(Some(key)) = source.lsp_pubkey(&coordinate).await {
                    lsp.insert(coordinate, key);
                }
            }
            let aggregates = events
                .values()
                .filter_map(|e| parse_receipt_event(e).ok())
                .filter(|r| r.aggregate.is_some())
                .take(128)
                .collect::<Vec<_>>();
            for receipt in aggregates {
                if let Some(aggregate) = &receipt.aggregate {
                    for leg in aggregate
                        .legs
                        .iter()
                        .filter(|l| l.proof_type == "zap")
                        .take(32)
                    {
                        let pair = (receipt.event.id, leg.leg_id.clone());
                        if !leg_lsps.contains_key(&pair) {
                            if let Ok(Some(proof)) = source.aggregate_lsp(&receipt, leg).await {
                                leg_lsps.insert(pair, proof.key);
                                for event in proof.signed_events {
                                    if Self::accept(buyer, &event) {
                                        events.insert(event.id, event.clone());
                                        acquired.insert(event.id, event);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        guard()?;
        // Serialize commits, then re-read current evidence to avoid losing a
        // concurrent importer/fork. No relay calls are made under this lock.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| "Receipt transaction unavailable")?;
        sqlx::query("UPDATE receipt_sync_events SET event_id=event_id WHERE buyer_pubkey=? AND 0")
            .bind(buyer.to_hex())
            .execute(&mut *tx)
            .await
            .map_err(|_| "Receipt transaction unavailable")?;
        let current:Vec<String>=sqlx::query_scalar("SELECT raw_event FROM receipt_sync_events WHERE buyer_pubkey=? UNION SELECT raw_event FROM purchases WHERE buyer_pubkey=?")
            .bind(buyer.to_hex()).bind(buyer.to_hex()).fetch_all(&mut *tx).await.map_err(|_|"Receipt evidence unavailable")?;
        for raw in current {
            if let Ok(event) = serde_json::from_str::<Event>(&raw) {
                events.insert(event.id, event);
            }
        }
        for event in acquired.values() {
            sqlx::query("INSERT INTO receipt_sync_events(buyer_pubkey,event_id,kind,raw_event) VALUES(?,?,?,?) ON CONFLICT DO NOTHING")
                .bind(buyer.to_hex()).bind(event.id.to_hex()).bind(i64::from(event.kind.as_u16())).bind(event.as_json()).execute(&mut *tx).await.map_err(|_|"Receipt evidence checkpoint failed")?;
        }
        let cached=sqlx::query("SELECT receipt_root_id,leg_id,lsp_pubkey FROM receipt_sync_leg_lsps WHERE buyer_pubkey=?")
            .bind(buyer.to_hex()).fetch_all(&mut *tx).await.map_err(|_|"Public proof evidence cache unavailable")?;
        for row in cached {
            if let (Ok(id), Ok(key)) = (
                EventId::from_hex(&row.get::<String, _>("receipt_root_id")),
                PublicKey::from_hex(&row.get::<String, _>("lsp_pubkey")),
            ) {
                leg_lsps.insert((id, row.get::<String, _>("leg_id")), key);
            }
        }
        let mut groups: BTreeMap<(String, String), Vec<Event>> = BTreeMap::new();
        let mut successors: HashMap<EventId, Vec<Event>> = HashMap::new();
        for event in events.values() {
            if let Ok(r) = parse_receipt_event(event) {
                if r.buyer_pubkey == buyer {
                    if let Some(parent) = r.predecessor {
                        successors.entry(parent).or_default().push(event.clone());
                    }
                    groups
                        .entry((r.coordinate, r.order_id))
                        .or_default()
                        .push(event.clone());
                }
            }
        }
        for ((coordinate, order), mut chain) in groups {
            // Follow referenced successors even when they mutate order/a. They
            // must be rejected, not split into another group while the old root
            // silently continues granting access.
            let mut visited = chain.iter().map(|e| e.id).collect::<HashSet<_>>();
            let mut pending = chain.iter().map(|e| e.id).collect::<Vec<_>>();
            while let Some(parent) = pending.pop() {
                for event in successors.get(&parent).into_iter().flatten() {
                    if visited.insert(event.id) {
                        chain.push(event.clone());
                        pending.push(event.id);
                    }
                }
            }
            let cached_lsp:Option<String>=sqlx::query_scalar("SELECT lsp_pubkey FROM receipt_chain_verifications WHERE buyer_pubkey=? AND listing_coordinate=? AND order_id=?")
                .bind(buyer.to_hex()).bind(&coordinate).bind(&order).fetch_optional(&mut *tx).await.map_err(|_|"Receipt verification checkpoint unavailable")?.flatten();
            let cached_key = cached_lsp.and_then(|s| PublicKey::from_hex(&s).ok());
            let key = cached_key.or_else(|| lsp.get(&coordinate).copied());
            let (mut verification, mut reason, mut tip) =
                verify_chain(buyer, &chain, &events, key, &leg_lsps);
            let root = chain
                .iter()
                .filter_map(|e| parse_receipt_event(e).ok())
                .find(|r| r.predecessor.is_none() && r.status == "paid");
            // Pre-portability ADP confirmations verified delegated authority
            // before storing the receipt, but did not archive its signed chain.
            // An outage must not invent a revocation of that local trust anchor.
            // This never applies to a newly imported receipt or a new extension.
            const LOCAL_ANCHOR:&str="Previously verified local receipt retained; verification evidence refresh unavailable";
            if verification == ReceiptVerification::Indeterminate
                && matches!(
                    reason,
                    "Missing fulfillment authorization"
                        | "Missing public Zap proof or listing LSP evidence"
                )
            {
                let previous:Option<(String,String)>=sqlx::query_as("SELECT verification,reason FROM receipt_chain_verifications WHERE buyer_pubkey=? AND listing_coordinate=? AND order_id=?")
                    .bind(buyer.to_hex()).bind(&coordinate).bind(&order).fetch_optional(&mut *tx).await.map_err(|_|"Local receipt trust checkpoint unavailable")?;
                let legacy = previous
                    .as_ref()
                    .is_none_or(|(status, reason)| status == "verified" && reason == LOCAL_ANCHOR);
                let local:Vec<String>=sqlx::query_scalar("SELECT event_id FROM purchases WHERE buyer_pubkey=? AND listing_coordinate=? AND order_id=?")
                    .bind(buyer.to_hex()).bind(&coordinate).bind(&order).fetch_all(&mut *tx).await.map_err(|_|"Local receipt trust checkpoint unavailable")?;
                let new_authority = root
                    .as_ref()
                    .and_then(|r| r.authorization)
                    .is_some_and(|id| {
                        acquired.values().any(|e| {
                            e.id == id
                                || parse_authorization_event(e)
                                    .is_ok_and(|a| a.predecessor == Some(id))
                        })
                    });
                if legacy && !new_authority && chain.iter().all(|e| local.contains(&e.id.to_hex()))
                {
                    if let Some(t) = crate::purchases::resolve_persisted_receipt_tip(&chain) {
                        verification = ReceiptVerification::Verified;
                        reason = LOCAL_ANCHOR;
                        tip = Some(t);
                        report.incomplete = true;
                    }
                }
            }
            if verification == ReceiptVerification::Verified {
                if let Some(root) = &root {
                    if let Some(aggregate) = &root.aggregate {
                        for leg in &aggregate.legs {
                            if let Some(key) = leg_lsps.get(&(root.event.id, leg.leg_id.clone())) {
                                sqlx::query("INSERT INTO receipt_sync_leg_lsps(buyer_pubkey,receipt_root_id,leg_id,lsp_pubkey) VALUES(?,?,?,?) ON CONFLICT DO NOTHING")
                            .bind(buyer.to_hex()).bind(root.event.id.to_hex()).bind(&leg.leg_id).bind(key.to_hex()).execute(&mut *tx).await.map_err(|_|"Public proof evidence checkpoint failed")?;
                            }
                        }
                    }
                }
                for event in &chain {
                    let r = parse_receipt_event(event)
                        .map_err(|_| "Receipt changed during verification")?;
                    sqlx::query("INSERT INTO purchases(event_id,order_id,listing_coordinate,buyer_pubkey,merchant_pubkey,payment_hash,status,created_at,raw_event) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(event_id) DO NOTHING")
                        .bind(event.id.to_hex()).bind(&r.order_id).bind(&r.coordinate).bind(buyer.to_hex()).bind(event.pubkey.to_hex()).bind(r.payment_hash).bind(r.status)
                        .bind(i64::try_from(event.created_at.as_secs()).map_err(|_|"Receipt timestamp out of range")?).bind(event.as_json()).execute(&mut *tx).await.map_err(|_|"Verified receipt checkpoint failed")?;
                }
            }
            let acquired_at = root
                .as_ref()
                .map(|r| r.event.created_at.as_secs())
                .unwrap_or(0)
                .min(i64::MAX as u64) as i64;
            sqlx::query("INSERT INTO receipt_chain_verifications(buyer_pubkey,listing_coordinate,order_id,verification,reason,tip_event_id,root_event_id,acquired_at,lsp_pubkey) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(buyer_pubkey,listing_coordinate,order_id) DO UPDATE SET verification=excluded.verification,reason=excluded.reason,tip_event_id=excluded.tip_event_id,root_event_id=excluded.root_event_id,acquired_at=excluded.acquired_at,lsp_pubkey=COALESCE(excluded.lsp_pubkey,receipt_chain_verifications.lsp_pubkey) WHERE receipt_chain_verifications.verification!=excluded.verification OR receipt_chain_verifications.reason!=excluded.reason OR receipt_chain_verifications.tip_event_id IS NOT excluded.tip_event_id OR receipt_chain_verifications.root_event_id IS NOT excluded.root_event_id OR receipt_chain_verifications.acquired_at!=excluded.acquired_at OR receipt_chain_verifications.lsp_pubkey IS NOT excluded.lsp_pubkey")
                .bind(buyer.to_hex()).bind(&coordinate).bind(&order).bind(verification.as_str()).bind(reason).bind(tip.map(|e|e.id.to_hex())).bind(root.as_ref().map(|r|r.event.id.to_hex())).bind(acquired_at).bind(if verification==ReceiptVerification::Verified {key}else{cached_key}.map(|k|k.to_hex()))
                .execute(&mut *tx).await.map_err(|_|"Receipt verification checkpoint failed")?;
            report.chains.push(ReceiptChainResult {
                order_id: order,
                listing_coordinate: coordinate,
                verification,
                reason: reason.into(),
                tip_event_id: tip.map(|e| e.id.to_hex()),
            });
        }
        guard()?;
        tx.commit().await.map_err(|_| "Receipt commit failed")?;
        report.incomplete |= report.relay_failures > 0
            || report
                .chains
                .iter()
                .any(|c| c.verification == ReceiptVerification::Indeterminate);
        report.message="Receipt verification finished. Relay availability and retention limit discovery; manual import is an independent recovery path. No payment or receipt publication occurred.".into();
        Ok(report)
    }

    pub async fn synchronize(
        &self,
        buyer: PublicKey,
        source: &dyn ReceiptSource,
        full: bool,
        now: u64,
        guard: Guard<'_>,
    ) -> Result<ReceiptSyncReport, String> {
        guard()?;
        let mut incoming = Vec::new();
        let mut failures = 0;
        let mut rejected = 0;
        let mut incomplete = false;
        let mut cursors = Vec::new();
        for relay in source.relay_urls().into_iter().take(16) {
            let row=sqlx::query("SELECT newest_seen,backfill_until,backfill_done,last_backfill FROM receipt_sync_cursors WHERE buyer_pubkey=? AND relay_url=?")
                .bind(buyer.to_hex()).bind(&relay).fetch_optional(&self.pool).await.map_err(|_|"Receipt synchronization state unavailable")?;
            let newest = row
                .as_ref()
                .map(|r| r.get::<i64, _>("newest_seen").max(0) as u64)
                .unwrap_or(0);
            let old_until = row
                .as_ref()
                .and_then(|r| r.get::<Option<i64>, _>("backfill_until"))
                .map(|v| v.max(0) as u64);
            let done = row
                .as_ref()
                .is_some_and(|r| r.get::<i64, _>("backfill_done") != 0);
            let last = row
                .as_ref()
                .map(|r| r.get::<i64, _>("last_backfill").max(0) as u64)
                .unwrap_or(0);
            let backfill = full || !done || now.saturating_sub(last) > 86400;
            let since = if backfill {
                0
            } else {
                newest.saturating_sub(300)
            };
            let mut until = if full || done { None } else { old_until };
            let mut observed = newest;
            let mut finished = false;
            let mut failed = false;
            // Always refresh the head while an older backfill is in progress.
            if backfill && until.is_some() {
                match source
                    .query(
                        &relay,
                        Filter::new()
                            .kind(Kind::Custom(1020))
                            .pubkey(buyer)
                            .since(Timestamp::from_secs(newest.saturating_sub(300)))
                            .limit(PAGE_SIZE),
                    )
                    .await
                {
                    Ok(batch) => {
                        if batch.len() >= PAGE_SIZE {
                            incomplete = true;
                        }
                        incoming.extend(batch);
                    }
                    Err(_) => {
                        failures += 1;
                    }
                }
            }
            for _ in 0..MAX_PAGES {
                guard()?;
                let mut filter = Filter::new()
                    .kind(Kind::Custom(1020))
                    .pubkey(buyer)
                    .since(Timestamp::from_secs(since))
                    .limit(PAGE_SIZE);
                if let Some(until) = until {
                    filter = filter.until(Timestamp::from_secs(until));
                }
                match source.query(&relay, filter).await {
                    Ok(batch) => {
                        let full_page = batch.len() >= PAGE_SIZE;
                        let received = batch.len();
                        let valid = batch
                            .into_iter()
                            .filter(|e| e.kind.as_u16() == 1020 && Self::accept(buyer, e))
                            .collect::<Vec<_>>();
                        rejected += received - valid.len();
                        let oldest = valid.iter().map(|e| e.created_at.as_secs()).min();
                        observed = observed.max(
                            valid
                                .iter()
                                .map(|e| e.created_at.as_secs().min(now))
                                .max()
                                .unwrap_or(0),
                        );
                        // Never let malformed/foreign relay results advance a
                        // buyer's cursor or skip an entire saturated page.
                        if full_page && valid.is_empty() {
                            incomplete = true;
                            break;
                        }
                        let batch = valid;
                        incoming.extend(batch);
                        if !full_page {
                            finished = true;
                            break;
                        }
                        if oldest == until || oldest.is_none() {
                            incomplete = true;
                            break;
                        }
                        until = oldest; // overlap boundary; never skip equal-timestamp events
                    }
                    Err(_) => {
                        failures += 1;
                        failed = true;
                        break;
                    }
                }
            }
            incomplete |= !finished;
            if !failed {
                cursors.push((
                    relay,
                    if finished { observed } else { newest },
                    if backfill { until } else { old_until },
                    if backfill { finished } else { done },
                    if backfill && finished { now } else { last },
                ));
            }
        }
        if incoming.len() > MAX_EVENTS {
            incoming.truncate(MAX_EVENTS);
            incomplete = true;
            cursors.clear();
        }
        let mut report = self.ingest(buyer, incoming, Some(source), guard).await?;
        guard()?;
        for (relay, newest, until, done, last) in cursors {
            sqlx::query("INSERT INTO receipt_sync_cursors(buyer_pubkey,relay_url,newest_seen,backfill_until,backfill_done,last_backfill) VALUES(?,?,?,?,?,?) ON CONFLICT(buyer_pubkey,relay_url) DO UPDATE SET newest_seen=excluded.newest_seen,backfill_until=excluded.backfill_until,backfill_done=excluded.backfill_done,last_backfill=excluded.last_backfill")
                .bind(buyer.to_hex()).bind(relay).bind(newest.min(i64::MAX as u64) as i64).bind(until.map(|v|v.min(i64::MAX as u64) as i64)).bind(done).bind(last.min(i64::MAX as u64) as i64)
                .execute(&self.pool).await.map_err(|_|"Receipt synchronization cursor checkpoint failed")?;
        }
        report.rejected_events += rejected;
        report.relay_failures += failures;
        report.incomplete |=
            incomplete || rejected > 0 || failures > 0 || source.relay_urls().is_empty();
        Ok(report)
    }
}

fn verify_chain<'a>(
    buyer: PublicKey,
    chain: &'a [Event],
    evidence: &BTreeMap<EventId, Event>,
    lsp: Option<PublicKey>,
    leg_lsps: &HashMap<(EventId, String), PublicKey>,
) -> (ReceiptVerification, &'static str, Option<&'a Event>) {
    use ReceiptVerification::*;
    let parsed = chain
        .iter()
        .filter_map(|e| parse_receipt_event(e).ok())
        .collect::<Vec<_>>();
    if parsed.len() != chain.len() || parsed.iter().any(|r| r.buyer_pubkey != buyer) {
        return (
            Invalid,
            "Invalid signature, structure or buyer binding",
            None,
        );
    }
    let ids = chain.iter().map(|e| e.id).collect::<HashSet<_>>();
    if parsed.iter().any(|r| {
        r.predecessor
            .is_some_and(|p| !ids.contains(&p) && evidence.contains_key(&p))
    }) {
        return (
            Invalid,
            "Receipt predecessor belongs to a different purchase context",
            None,
        );
    }
    if parsed
        .iter()
        .any(|r| r.predecessor.is_some_and(|p| !ids.contains(&p)))
    {
        return (Indeterminate, "Missing receipt predecessor", None);
    }
    let roots = parsed
        .iter()
        .filter(|r| r.predecessor.is_none() && r.status == "paid")
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return (Invalid, "Receipt chain has no valid paid root", None);
    }
    if roots.len() != 1 {
        return (Invalid, "Conflicting receipt roots", None);
    }
    let root = roots[0];
    let Some(developer) = coordinate_publisher(&root.coordinate) else {
        return (Invalid, "Invalid listing coordinate", None);
    };
    let authorization = if let Some(id) = root.authorization {
        if !evidence.contains_key(&id) {
            return (Indeterminate, "Missing fulfillment authorization", None);
        }
        let Ok(auth_root) = parse_authorization_event(&evidence[&id]) else {
            return (Invalid, "Invalid fulfillment authorization root", None);
        };
        let auth_events = evidence
            .values()
            .filter(|e| e.kind.as_u16() == FULFILLMENT_AUTHORIZATION_KIND)
            .filter(|e| {
                parse_authorization_event(e).is_ok_and(|a| {
                    a.terms.authorization_id == auth_root.terms.authorization_id
                        && a.event.pubkey == auth_root.event.pubkey
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let auth_ids = auth_events.iter().map(|e| e.id).collect::<HashSet<_>>();
        if auth_events
            .iter()
            .filter_map(|e| parse_authorization_event(e).ok())
            .any(|a| a.predecessor.is_some_and(|p| !auth_ids.contains(&p)))
        {
            return (Indeterminate, "Incomplete authorization chain", None);
        }
        match resolve_authorization(id, &auth_events) {
            Ok(a) => Some(a),
            Err(_) => return (Invalid, "Invalid fulfillment authorization chain", None),
        }
    } else {
        None
    };
    let tip = match resolve_receipt_chain(chain, developer, authorization.as_ref()) {
        Ok(t) => t,
        Err(_) => {
            return (
                Invalid,
                "Invalid receipt chain, signer authority or public payment binding",
                None,
            )
        }
    };
    if let Some(aggregate) = &root.aggregate {
        for leg in aggregate.legs.iter().filter(|l| l.proof_type == "zap") {
            let Some(key) = leg_lsps.get(&(root.event.id, leg.leg_id.clone())) else {
                return (
                    Indeterminate,
                    "Aggregate Zap recipient/LSP provenance evidence is unavailable",
                    None,
                );
            };
            let Some(zap) = EventId::from_hex(&leg.proof_reference)
                .ok()
                .and_then(|id| evidence.get(&id))
            else {
                return (Indeterminate, "Missing aggregate public Zap proof", None);
            };
            let mut bound = (*root).clone();
            bound.aggregate = None;
            bound.payment_hash = Some(leg.payment_hash.clone());
            bound.amount_msat = Some(leg.amount_msat);
            bound.proofs = vec![ReceiptProof::Zap(zap.id)];
            if validate_receipt_evidence(
                &bound,
                ReceiptEvidence {
                    zap_receipts: &[zap.clone()],
                    lsp_pubkey: Some(*key),
                    ..Default::default()
                },
            )
            .is_err()
            {
                return (Invalid, "Invalid aggregate public Zap evidence", None);
            }
        }
    }
    let zaps = root
        .proofs
        .iter()
        .filter_map(|p| {
            if let ReceiptProof::Zap(id) = p {
                Some(*id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !zaps.is_empty() {
        if lsp.is_none() || zaps.iter().any(|id| !evidence.contains_key(id)) {
            return (
                Indeterminate,
                "Missing public Zap proof or listing LSP evidence",
                None,
            );
        }
        let events = zaps
            .iter()
            .filter_map(|id| evidence.get(id).cloned())
            .collect::<Vec<_>>();
        if validate_receipt_evidence(
            root,
            ReceiptEvidence {
                zap_receipts: &events,
                lsp_pubkey: lsp,
                ..Default::default()
            },
        )
        .is_err()
        {
            return (Invalid, "Invalid public Zap payment evidence", None);
        }
    }
    (
        Verified,
        "Verified signed receipt chain; relay history completeness is not guaranteed",
        Some(tip),
    )
}
