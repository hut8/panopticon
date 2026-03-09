use sqlx::PgPool;
use uuid::Uuid;

/// Record a lock state change to the database.
pub async fn record(
    db: &PgPool,
    device_id: &str,
    lock_state: &str,
    source: &str,
    user_id: Option<Uuid>,
) {
    if let Err(e) = sqlx::query(
        "INSERT INTO lock_state_log (device_id, lock_state, source, user_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(device_id)
    .bind(lock_state)
    .bind(source)
    .bind(user_id)
    .execute(db)
    .await
    {
        tracing::error!(
            device_id,
            lock_state,
            source,
            ?user_id,
            "Failed to log lock state change: {e:#}"
        );
    }
}
