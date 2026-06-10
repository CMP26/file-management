use crate::AppResult;
use sqlx::{postgres::PgPoolOptions, PgPool};

pub type Pool = PgPool;

pub async fn create_pool(database_url: &str) -> AppResult<Pool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;

    Ok(pool)
}

pub async fn run_migrations(pool: &Pool) -> AppResult<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|error| crate::AppError::other(error.to_string()))?;
    Ok(())
}
