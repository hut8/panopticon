use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;

pub async fn init_pool() -> Result<PgPool> {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres:///panopticon".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    info!("Connected to PostgreSQL");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations complete");

    Ok(pool)
}
