use langgraph_checkpoint::{Checkpoint, CheckpointError, CheckpointSaver};
use postgres::error::SqlState;
use postgres::{Client, NoTls};

const CREATE_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS checkpoints (
    thread_id TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (thread_id, checkpoint_id)
);
"#;

#[derive(Debug, Clone)]
pub struct PostgresSaver {
    connection_string: String,
}

impl PostgresSaver {
    pub fn new(connection_string: impl Into<String>) -> Result<Self, CheckpointError> {
        let saver = Self { connection_string: connection_string.into() };
        saver.init_schema()?;
        Ok(saver)
    }

    fn init_schema(&self) -> Result<(), CheckpointError> {
        let mut client = self.open()?;
        client.batch_execute(CREATE_TABLE_SQL).map_err(map_pg_error)?;
        Ok(())
    }

    fn open(&self) -> Result<Client, CheckpointError> {
        Client::connect(&self.connection_string, NoTls).map_err(map_pg_error)
    }
}

impl CheckpointSaver for PostgresSaver {
    fn put(&self, checkpoint: Checkpoint) -> Result<(), CheckpointError> {
        let payload = serde_json::to_value(&checkpoint)
            .map_err(|err| CheckpointError::Storage(err.to_string()))?;
        let mut client = self.open()?;
        let result = client.execute(
            "INSERT INTO checkpoints (thread_id, checkpoint_id, payload) VALUES ($1, $2, $3)",
            &[&checkpoint.thread_id, &checkpoint.checkpoint_id, &payload],
        );
        match result {
            Ok(_) => Ok(()),
            Err(err) => {
                if let Some(code) = err.code() {
                    if *code == SqlState::UNIQUE_VIOLATION {
                        return Err(CheckpointError::Conflict(format!(
                            "thread `{}` already has checkpoint `{}`",
                            checkpoint.thread_id, checkpoint.checkpoint_id
                        )));
                    }
                }
                Err(map_pg_error(err))
            }
        }
    }

    fn get(
        &self,
        thread_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<Checkpoint>, CheckpointError> {
        let mut client = self.open()?;
        let row = client
            .query_opt(
                "SELECT payload FROM checkpoints WHERE thread_id = $1 AND checkpoint_id = $2",
                &[&thread_id, &checkpoint_id],
            )
            .map_err(map_pg_error)?;
        match row {
            Some(row) => {
                let payload: serde_json::Value = row.get(0);
                let checkpoint = serde_json::from_value(payload)
                    .map_err(|err| CheckpointError::Storage(err.to_string()))?;
                Ok(Some(checkpoint))
            }
            None => Ok(None),
        }
    }

    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError> {
        let mut client = self.open()?;
        let rows = client
            .query(
                "SELECT payload FROM checkpoints WHERE thread_id = $1 ORDER BY created_at ASC, checkpoint_id ASC",
                &[&thread_id],
            )
            .map_err(map_pg_error)?;
        rows.into_iter()
            .map(|row| {
                let payload: serde_json::Value = row.get(0);
                serde_json::from_value(payload)
                    .map_err(|err| CheckpointError::Storage(err.to_string()))
            })
            .collect()
    }
}

fn map_pg_error(err: postgres::Error) -> CheckpointError {
    CheckpointError::Storage(err.to_string())
}
