use std::path::{Path, PathBuf};

use langgraph_checkpoint::{Checkpoint, CheckpointError, CheckpointSaver};
use rusqlite::{params, Connection, Error as SqlError, ErrorCode};

const CREATE_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS checkpoints (
    thread_id TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (thread_id, checkpoint_id)
);
"#;

#[derive(Debug, Clone)]
pub struct SqliteSaver {
    db_path: PathBuf,
}

impl SqliteSaver {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, CheckpointError> {
        let saver = Self { db_path: path.as_ref().to_path_buf() };
        saver.init_schema()?;
        Ok(saver)
    }

    fn init_schema(&self) -> Result<(), CheckpointError> {
        let conn = self.open()?;
        conn.execute_batch(CREATE_TABLE_SQL).map_err(map_sql_error)?;
        Ok(())
    }

    fn open(&self) -> Result<Connection, CheckpointError> {
        Connection::open(&self.db_path).map_err(map_sql_error)
    }
}

impl CheckpointSaver for SqliteSaver {
    fn put(&self, checkpoint: Checkpoint) -> Result<(), CheckpointError> {
        let payload = serde_json::to_string(&checkpoint)
            .map_err(|err| CheckpointError::Storage(err.to_string()))?;

        let conn = self.open()?;
        let result = conn.execute(
            "INSERT INTO checkpoints (thread_id, checkpoint_id, payload) VALUES (?1, ?2, ?3)",
            params![checkpoint.thread_id, checkpoint.checkpoint_id, payload],
        );

        match result {
            Ok(_) => Ok(()),
            Err(err) => {
                if is_unique_violation(&err) {
                    Err(CheckpointError::Conflict(format!(
                        "thread `{}` already has checkpoint `{}`",
                        checkpoint.thread_id, checkpoint.checkpoint_id
                    )))
                } else {
                    Err(map_sql_error(err))
                }
            }
        }
    }

    fn get(
        &self,
        thread_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<Checkpoint>, CheckpointError> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT payload FROM checkpoints WHERE thread_id = ?1 AND checkpoint_id = ?2")
            .map_err(map_sql_error)?;
        let mut rows = stmt.query(params![thread_id, checkpoint_id]).map_err(map_sql_error)?;

        if let Some(row) = rows.next().map_err(map_sql_error)? {
            let payload: String = row.get(0).map_err(map_sql_error)?;
            let checkpoint = serde_json::from_str(&payload)
                .map_err(|err| CheckpointError::Storage(err.to_string()))?;
            Ok(Some(checkpoint))
        } else {
            Ok(None)
        }
    }

    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT payload FROM checkpoints WHERE thread_id = ?1 ORDER BY created_at ASC, checkpoint_id ASC",
            )
            .map_err(map_sql_error)?;
        let mut rows = stmt.query(params![thread_id]).map_err(map_sql_error)?;
        let mut checkpoints = Vec::new();

        while let Some(row) = rows.next().map_err(map_sql_error)? {
            let payload: String = row.get(0).map_err(map_sql_error)?;
            let checkpoint = serde_json::from_str(&payload)
                .map_err(|err| CheckpointError::Storage(err.to_string()))?;
            checkpoints.push(checkpoint);
        }

        Ok(checkpoints)
    }
}

fn map_sql_error(err: SqlError) -> CheckpointError {
    CheckpointError::Storage(err.to_string())
}

fn is_unique_violation(err: &SqlError) -> bool {
    match err {
        SqlError::SqliteFailure(info, _) => info.code == ErrorCode::ConstraintViolation,
        _ => false,
    }
}
