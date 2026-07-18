use thiserror::Error;

use crate::entitlements_repository::{EntitlementsRepository, EntitlementsRepositoryError};
use crate::purchases::{PurchaseError, PurchasesRepository};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipSource {
    None,
    PurchaseReceipt,
    EntitlementGrant,
}

#[derive(Debug, Error)]
pub enum OwnershipError {
    #[error(transparent)]
    Purchase(#[from] PurchaseError),
    #[error(transparent)]
    Entitlement(#[from] EntitlementsRepositoryError),
}

pub struct OwnershipService {
    purchases: PurchasesRepository,
    entitlements: EntitlementsRepository,
}

impl OwnershipService {
    pub fn new(purchases: PurchasesRepository, entitlements: EntitlementsRepository) -> Self {
        Self {
            purchases,
            entitlements,
        }
    }

    pub async fn source_for(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
    ) -> Result<OwnershipSource, OwnershipError> {
        if self
            .purchases
            .is_owned(buyer_pubkey, game_coordinate)
            .await?
        {
            return Ok(OwnershipSource::PurchaseReceipt);
        }
        if self
            .entitlements
            .is_owned(buyer_pubkey, game_coordinate)
            .await?
        {
            return Ok(OwnershipSource::EntitlementGrant);
        }
        Ok(OwnershipSource::None)
    }

    pub async fn is_owned(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
    ) -> Result<bool, OwnershipError> {
        Ok(self.source_for(buyer_pubkey, game_coordinate).await? != OwnershipSource::None)
    }
}
