-- Account-scoped publisher intent. No credentials or native archive handles.
CREATE TABLE IF NOT EXISTS publisher_drafts (
    publisher TEXT NOT NULL,
    draft_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (publisher, draft_id)
);
CREATE TABLE IF NOT EXISTS publisher_operations (
    publisher TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    coordinate TEXT NOT NULL,
    intent_json TEXT NOT NULL,
    artifact_hash TEXT,
    state TEXT NOT NULL DEFAULT 'incomplete',
    result_json TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (publisher, operation_id)
);
CREATE TABLE IF NOT EXISTS publisher_checkpoints (
    publisher TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    intent_json TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'prepared',
    outcome_json TEXT,
    last_error TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (publisher, operation_id, step_id),
    FOREIGN KEY (publisher, operation_id)
        REFERENCES publisher_operations(publisher, operation_id) ON DELETE RESTRICT
);
