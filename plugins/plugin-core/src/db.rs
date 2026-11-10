//! Runtime tokio + pool MySQL PROPRES À CHAQUE PLUGIN.
//!
//! Pourquoi : chaque `.so` embarque sa propre copie statique de tokio et de
//! parking_lot. Utiliser `state.handle.block_on` / `state.pool` (créés par le
//! binaire principal) mélange deux copies sur les mêmes verrous (timer, IO driver)
//! → perte de réveil → blocage dès qu'il y a plusieurs requêtes en parallèle.
//!
//! Ce module est compilé DANS chaque plugin : chacun obtient donc son runtime
//! et son pool, entièrement isolés du binaire principal (même principe que
//! plugin_mongo / plugin_health).

use sqlx::mysql::{MySqlPool, MySqlPoolOptions};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

pub struct SqlContext {
    pub rt:   Runtime,
    pub pool: MySqlPool,
}

static CTX: OnceLock<Result<SqlContext, String>> = OnceLock::new();

/// Renvoie le contexte SQL du plugin (créé au premier appel, thread-safe).
pub fn sql_ctx() -> Result<&'static SqlContext, String> {
    CTX.get_or_init(build).as_ref().map_err(|e| e.clone())
}

fn build() -> Result<SqlContext, String> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL absent de l'environnement (.env)".to_string())?;

    let max_conn: u32 = std::env::var("PLUGIN_DB_MAX_CONNECTIONS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(5);

    let rt = Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("plugin-sql-rt")
        .enable_all()
        .build()
        .map_err(|e| format!("Runtime tokio du plugin : {e}"))?;

    let pool = rt.block_on(async {
        MySqlPoolOptions::new()
            .max_connections(max_conn)
            .acquire_timeout(Duration::from_secs(5))
            .after_connect(|conn, _meta| Box::pin(async move {
                sqlx::query("SET NAMES utf8mb4").execute(&mut *conn).await?;
                Ok(())
            }))
            .connect(&url)
            .await
    }).map_err(|e| format!("Connexion MySQL du plugin : {e}"))?;

    Ok(SqlContext { rt, pool })
}
