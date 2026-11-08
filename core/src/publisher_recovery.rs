//! Durable intent/checkpoints for the existing publisher pipeline, never an executor.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublisherDraft {
    pub draft_id: String,
    pub kind: String,
    pub payload: Value,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherOperation {
    pub operation_id: String,
    pub kind: String,
    pub coordinate: String,
    pub intent: Value,
    pub artifact_hash: Option<String>,
    pub state: String,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherCheckpoint {
    pub step_id: String,
    pub kind: String,
    pub target: String,
    pub intent: Value,
    pub state: String,
    pub outcome: Option<Value>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherRecoveryState {
    pub drafts: Vec<PublisherDraft>,
    pub operations: Vec<PublisherOperation>,
    pub checkpoints: Vec<(String, Vec<PublisherCheckpoint>)>,
}

pub fn verify_reselected_artifact(expected: &str, verified_actual: &str) -> Result<(), String> {
    if expected != verified_actual || crate::distribution::Sha256Digest::parse(expected).is_err() {
        return Err("Reselected archive does not match the original pinned digest".into());
    }
    Ok(())
}

/// No native capability or credential can be restored from a draft/operation.
pub fn validate_durable_payload(value: &Value) -> Result<(), String> {
    if serde_json::to_vec(value)
        .map_err(|_| "Invalid publisher data")?
        .len()
        > 2 * 1024 * 1024
    {
        return Err("Publisher draft exceeds 2 MiB".into());
    }
    fn walk(value: &Value, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("Publisher data is too deeply nested".into());
        }
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if matches!(
                        key.to_ascii_lowercase().as_str(),
                        "nsec"
                            | "private_key"
                            | "secret_key"
                            | "password"
                            | "download_token"
                            | "authorization_header"
                            | "selection_id"
                            | "selected_build"
                            | "file_path"
                    ) {
                        return Err(
                            "Publisher data contains a credential or ephemeral archive capability"
                                .into(),
                        );
                    }
                    walk(value, depth + 1)?;
                }
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, depth + 1)?;
                }
            }
            Value::String(value)
                if url::Url::parse(value)
                    .is_ok_and(|url| !url.username().is_empty() || url.password().is_some()) =>
            {
                return Err("Publisher data contains a credential-bearing URL".into());
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, 0)
}

#[cfg(feature = "native")]
pub mod repository {
    use super::*;
    use sqlx::{Row, SqlitePool};

    #[derive(Clone)]
    pub struct PublisherRepository {
        pool: SqlitePool,
    }
    impl PublisherRepository {
        pub fn new(pool: SqlitePool) -> Self {
            Self { pool }
        }

        pub async fn save_draft(
            &self,
            publisher: &str,
            draft: &PublisherDraft,
            expected: Option<u64>,
        ) -> Result<u64, String> {
            validate_durable_payload(&draft.payload)?;
            if draft.draft_id.is_empty()
                || draft.draft_id.len() > 256
                || !matches!(draft.kind.as_str(), "listing" | "store_page")
            {
                return Err("Invalid publisher draft identity".into());
            }
            let next = expected
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("Draft revision overflow")?;
            let next = i64::try_from(next).map_err(|_| "Draft revision overflow")?;
            let payload = serde_json::to_string(&draft.payload).map_err(|_| "Invalid draft")?;
            let changed = if let Some(expected) = expected {
                sqlx::query("UPDATE publisher_drafts SET payload_json=?, revision=?, updated_at=unixepoch() WHERE publisher=? AND draft_id=? AND kind=? AND revision=?")
                    .bind(payload).bind(next).bind(publisher).bind(&draft.draft_id).bind(&draft.kind).bind(i64::try_from(expected).map_err(|_| "Invalid draft revision")?)
                    .execute(&self.pool).await.map_err(db)?.rows_affected()
            } else {
                sqlx::query("INSERT INTO publisher_drafts(publisher,draft_id,kind,payload_json,revision,updated_at) VALUES(?,?,?,?,?,unixepoch()) ON CONFLICT DO NOTHING")
                    .bind(publisher).bind(&draft.draft_id).bind(&draft.kind).bind(payload).bind(next).execute(&self.pool).await.map_err(db)?.rows_affected()
            };
            if changed != 1 {
                return Err("Draft changed on this device; restore it before saving again".into());
            }
            Ok(next as u64)
        }

        pub async fn drafts(&self, publisher: &str) -> Result<Vec<PublisherDraft>, String> {
            sqlx::query("SELECT draft_id,kind,payload_json,revision FROM publisher_drafts WHERE publisher=? ORDER BY updated_at DESC")
                .bind(publisher).fetch_all(&self.pool).await.map_err(db)?.into_iter().map(|row| Ok(PublisherDraft {
                    draft_id: row.get("draft_id"), kind: row.get("kind"),
                    payload: serde_json::from_str(&row.get::<String,_>("payload_json")).map_err(|_| "Stored draft is invalid")?,
                    revision: row.get::<i64,_>("revision") as u64,
                })).collect()
        }

        pub async fn begin(
            &self,
            publisher: &str,
            operation: &PublisherOperation,
        ) -> Result<(), String> {
            validate_durable_payload(&operation.intent)?;
            let changed = sqlx::query("INSERT INTO publisher_operations(publisher,operation_id,kind,coordinate,intent_json,artifact_hash,state,updated_at) VALUES(?,?,?,?,?,?,'incomplete',unixepoch()) ON CONFLICT DO NOTHING")
                .bind(publisher).bind(&operation.operation_id).bind(&operation.kind).bind(&operation.coordinate)
                .bind(serde_json::to_string(&operation.intent).map_err(|_| "Invalid publisher intent")?).bind(&operation.artifact_hash)
                .execute(&self.pool).await.map_err(db)?.rows_affected();
            if changed != 1 {
                return Err("Operation already exists; reconcile it from Publisher recovery instead of starting it again".into());
            }
            Ok(())
        }

        pub async fn operations(&self, publisher: &str) -> Result<Vec<PublisherOperation>, String> {
            sqlx::query(
                "SELECT * FROM publisher_operations WHERE publisher=? ORDER BY updated_at DESC",
            )
            .bind(publisher)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?
            .into_iter()
            .map(|row| {
                Ok(PublisherOperation {
                    operation_id: row.get("operation_id"),
                    kind: row.get("kind"),
                    coordinate: row.get("coordinate"),
                    intent: serde_json::from_str(&row.get::<String, _>("intent_json"))
                        .map_err(|_| "Invalid stored intent")?,
                    artifact_hash: row.get("artifact_hash"),
                    state: row.get("state"),
                    result: row
                        .get::<Option<String>, _>("result_json")
                        .map(|s| serde_json::from_str(&s))
                        .transpose()
                        .map_err(|_| "Invalid stored result")?,
                })
            })
            .collect()
        }

        pub async fn operation(
            &self,
            publisher: &str,
            id: &str,
        ) -> Result<PublisherOperation, String> {
            self.operations(publisher)
                .await?
                .into_iter()
                .find(|op| op.operation_id == id)
                .ok_or("Publisher operation unavailable for this account".into())
        }

        pub async fn checkpoint(
            &self,
            publisher: &str,
            operation: &str,
            step: &PublisherCheckpoint,
        ) -> Result<(), String> {
            let mut tx = self.pool.begin().await.map_err(db)?;
            // Immutable step intent and operation ownership are checked in the same transaction.
            let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM publisher_operations WHERE publisher=? AND operation_id=? AND state='incomplete'")
                .bind(publisher).bind(operation).fetch_one(&mut *tx).await.map_err(db)?;
            if exists != 1 {
                return Err("Publisher operation is missing or already completed".into());
            }
            let intent = serde_json::to_string(&step.intent).map_err(|_| "Invalid checkpoint")?;
            sqlx::query("INSERT INTO publisher_checkpoints(publisher,operation_id,step_id,kind,target,intent_json,state,updated_at) VALUES(?,?,?,?,?,?,'prepared',unixepoch()) ON CONFLICT DO NOTHING")
                .bind(publisher).bind(operation).bind(&step.step_id).bind(&step.kind).bind(&step.target).bind(&intent)
                .execute(&mut *tx).await.map_err(db)?;
            let row = sqlx::query("SELECT kind,target,intent_json FROM publisher_checkpoints WHERE publisher=? AND operation_id=? AND step_id=?")
                .bind(publisher).bind(operation).bind(&step.step_id).fetch_one(&mut *tx).await.map_err(db)?;
            if row.get::<String, _>("kind") != step.kind
                || row.get::<String, _>("target") != step.target
                || serde_json::from_str::<Value>(&row.get::<String, _>("intent_json"))
                    .map_err(|_| "Invalid checkpoint")?
                    != step.intent
            {
                return Err(
                    "Checkpoint intent changed; the original signed publication is immutable"
                        .into(),
                );
            }
            tx.commit().await.map_err(db)
        }

        pub async fn acknowledge(
            &self,
            publisher: &str,
            operation: &str,
            step: &str,
            outcome: &Value,
        ) -> Result<(), String> {
            let value = serde_json::to_string(outcome).map_err(|_| "Invalid publisher result")?;
            let changed = sqlx::query("UPDATE publisher_checkpoints SET state='acknowledged',outcome_json=?,updated_at=unixepoch() WHERE publisher=? AND operation_id=? AND step_id=? AND (outcome_json IS NULL OR outcome_json=?)")
                .bind(&value).bind(publisher).bind(operation).bind(step).bind(&value).execute(&self.pool).await.map_err(db)?.rows_affected();
            if changed != 1 {
                return Err("Checkpoint result conflicts with retained evidence".into());
            }
            Ok(())
        }

        pub async fn record_failure(
            &self,
            publisher: &str,
            operation: &str,
            step: &str,
            safe_message: &str,
        ) -> Result<(), String> {
            let changed = sqlx::query("UPDATE publisher_checkpoints SET last_error=?,updated_at=unixepoch() WHERE publisher=? AND operation_id=? AND step_id=?")
                .bind(safe_message).bind(publisher).bind(operation).bind(step).execute(&self.pool).await.map_err(db)?.rows_affected();
            if changed != 1 {
                return Err("Publisher checkpoint is unavailable for this account".into());
            }
            Ok(())
        }

        pub async fn steps(
            &self,
            publisher: &str,
            operation: &str,
        ) -> Result<Vec<PublisherCheckpoint>, String> {
            sqlx::query("SELECT * FROM publisher_checkpoints WHERE publisher=? AND operation_id=? ORDER BY updated_at,step_id")
                .bind(publisher).bind(operation).fetch_all(&self.pool).await.map_err(db)?.into_iter().map(|row| Ok(PublisherCheckpoint {
                    step_id: row.get("step_id"), kind: row.get("kind"), target: row.get("target"), state: row.get("state"),
                    intent: serde_json::from_str(&row.get::<String,_>("intent_json")).map_err(|_| "Invalid checkpoint")?,
                    outcome: row.get::<Option<String>,_>("outcome_json").map(|s| serde_json::from_str(&s)).transpose().map_err(|_| "Invalid checkpoint result")?,
                    last_error: row.get("last_error"),
                })).collect()
        }

        pub async fn complete(
            &self,
            publisher: &str,
            operation: &str,
            result: &Value,
        ) -> Result<(), String> {
            let result = serde_json::to_string(result).map_err(|_| "Invalid result")?;
            let changed = sqlx::query("UPDATE publisher_operations SET state='complete',result_json=?,updated_at=unixepoch() WHERE publisher=? AND operation_id=? AND (result_json IS NULL OR result_json=?)")
                .bind(&result).bind(publisher).bind(operation).bind(&result).execute(&self.pool).await.map_err(db)?.rows_affected();
            if changed != 1 {
                return Err("Completed publisher result conflicts with retained evidence".into());
            }
            Ok(())
        }
    }
    fn db(_: sqlx::Error) -> String {
        "Publisher database operation failed; no further external action was authorized".into()
    }
}
