use mongodb::options::{AuthMechanism, ClientOptions, Credential};
use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, System};
use tokio::runtime::Runtime;

// ── Runtime + client MongoDB autonomes ───────────────────────────────────────
// Même pattern que plugin_mongo : le client est créé DANS HEALTH_RT.
// Ses tâches de fond (heartbeat, pool) tournent sur HEALTH_RT, pas sur le
// runtime principal → block_in_place n'affame jamais le heartbeat.
struct HealthContext {
    rt:    Runtime,
    mongo: Option<mongodb::Client>,
}

static HEALTH_CTX: OnceLock<HealthContext> = OnceLock::new();

fn get_health_ctx() -> &'static HealthContext {
    HEALTH_CTX.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("plugin-health")
            .build()
            .expect("Impossible de créer le runtime du plugin_health");

        // Client MongoDB créé DANS HEALTH_RT
        let mongo = rt.block_on(async {
            let uri = match std::env::var("MONGODB_URI") {
                Err(_)  => return None,
                Ok(uri) => uri,
            };
            let mut opts = match ClientOptions::parse(&uri).await {
                Err(_)   => return None,
                Ok(opts) => opts,
            };
            if let Ok(user) = std::env::var("MONGODB_USER") {
                let pass    = std::env::var("MONGODB_PASS").unwrap_or_default();
                let auth_db = std::env::var("MONGODB_AUTH_DB")
                    .unwrap_or_else(|_| "admin".to_string());
                opts.credential = Some(
                    Credential::builder()
                        .username(user)
                        .password(pass)
                        .source(auth_db)
                        .mechanism(AuthMechanism::ScramSha256)
                        .build()
                );
            }
            mongodb::Client::with_options(opts).ok()
        });

        HealthContext { rt, mongo }
    })
}

pub struct PluginHealth;

impl Plugin for PluginHealth {
    fn name(&self) -> &'static str { "health" }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        let metrics = collect_metrics(state);
        match ctx.operation.as_str() {
            "dashboard" => PluginResult::Data(metrics),
            _           => PluginResult::Data(metrics),
        }
    }
}

fn collect_metrics(state: &AppState) -> Value {
    // ── Sessions ──────────────────────────────────────────────────────────────
    let (active_sessions, expired_sessions) = {
        let now     = now_secs();
        let sess    = state.sessions.lock().unwrap();
        let active  = sess.values().filter(|u| u.expires_at > now).count();
        let expired = sess.values().filter(|u| u.expires_at <= now).count();
        (active, expired)
    };

    // ── Pings bases de données ────────────────────────────────────────────────
    // MySQL  : block_in_place + handle.block_on  (pool sqlx lié au runtime principal)
    // MongoDB: block_in_place + HEALTH_RT.block_on (client isolé du runtime principal)
    let db_status = ping_databases(state);

    // ── RAM via sysinfo ───────────────────────────────────────────────────────
    let mut sys = System::new_all();
    sys.refresh_all();

    let total_mem = sys.total_memory();
    let used_mem  = sys.used_memory();
    let free_mem  = total_mem.saturating_sub(used_mem);
    let mem_pct   = if total_mem > 0 { used_mem * 100 / total_mem } else { 0 };

    // ── Disque ────────────────────────────────────────────────────────────────
    let disks     = Disks::new_with_refreshed_list();
    let disk_info = find_main_disk(&disks);
    println!("disks : {:?}", disks);
    // ── Uptime ────────────────────────────────────────────────────────────────
    let uptime_secs = System::uptime();

    // ── Statut global ─────────────────────────────────────────────────────────
    let db_ok = db_status["mysql"]["status"] == "ok"
        && (db_status["mongodb"]["status"] == "ok"
            || db_status["mongodb"]["status"] == "disabled");

    let status = if !db_ok || mem_pct > 90 || disk_info.usage_pct > 90 {
        "warning"
    } else {
        "ok"
    };

    json!({
        "status":    status,
        "timestamp": now_secs(),
        "sessions": {
            "active":  active_sessions,
            "expired": expired_sessions,
            "total":   active_sessions + expired_sessions
        },
        "databases": db_status,
        "memory": {
            "total_mb":      total_mem / 1024 / 1024,
            "used_mb":       used_mem  / 1024 / 1024,
            "free_mb":       free_mem  / 1024 / 1024,
            "usage_percent": mem_pct
        },
        "disk": {
            "mount":         disk_info.mount,
            "total_gb":      disk_info.total_gb,
            "used_gb":       disk_info.used_gb,
            "free_gb":       disk_info.free_gb,
            "usage_percent": disk_info.usage_pct
        },
        "uptime": {
            "seconds":   uptime_secs,
            "formatted": format_uptime(uptime_secs)
        }
    })
}

// ── Pings ─────────────────────────────────────────────────────────────────────

fn ping_databases(state: &AppState) -> Value {
    let ctx = get_health_ctx();

    // MySQL : utilise le pool du runtime principal (block_in_place requis)
    let mysql = tokio::task::block_in_place(|| {
        state.handle.block_on(async {
            let t0 = Instant::now();
            match tokio::time::timeout(
                Duration::from_secs(3),
                sqlx::query("SELECT 1").fetch_one(&state.pool)
            ).await {
                Ok(Ok(_))  => json!({ "status": "ok",    "latency_ms": t0.elapsed().as_millis() as u64 }),
                Ok(Err(e)) => json!({ "status": "error", "error": e.to_string() }),
                Err(_)     => json!({ "status": "error", "error": "timeout (>3s)" }),
            }
        })
    });

    // MongoDB : utilise HEALTH_RT → heartbeat isolé du runtime principal
    let mongodb = match &ctx.mongo {
        None => json!({ "status": "disabled", "error": "MONGODB_URI absent du .env" }),
        Some(client) => {
            let client  = client.clone();
            let db_name = std::env::var("MONGODB_DB")
                .unwrap_or_else(|_| "admin".to_string());

            // Pas besoin de block_in_place ici car HEALTH_RT est autonome
            // On lance le ping depuis le thread courant en bloquant sur HEALTH_RT
            tokio::task::block_in_place(|| {
                ctx.rt.block_on(async move {
                    let t0 = Instant::now();
                    match tokio::time::timeout(
                        Duration::from_secs(3),
                        client.database(&db_name)
                              .run_command(mongodb::bson::doc! { "ping": 1 })
                    ).await {
                        Ok(Ok(_))  => json!({ "status": "ok",    "latency_ms": t0.elapsed().as_millis() as u64 }),
                        Ok(Err(e)) => json!({ "status": "error", "error": e.to_string() }),
                        Err(_)     => json!({ "status": "error", "error": "timeout (>3s)" }),
                    }
                })
            })
        }
    };

    json!({ "mysql": mysql, "mongodb": mongodb })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct DiskInfo { mount: String, total_gb: u64, used_gb: u64, free_gb: u64, usage_pct: u64 }

fn find_main_disk(disks: &Disks) -> DiskInfo {
    match disks.iter().max_by_key(|d| d.total_space()) {
        None => DiskInfo { mount: "/".into(), total_gb: 0, used_gb: 0, free_gb: 0, usage_pct: 0 },
        Some(d) => {
            let total = d.total_space();
            let free  = d.available_space();
            let used  = total.saturating_sub(free);
            let pct   = if total > 0 { used * 100 / total } else { 0 };
            DiskInfo {
                mount:     d.mount_point().to_string_lossy().to_string(),
                total_gb:  total / 1024 / 1024 / 1024,
                used_gb:   used  / 1024 / 1024 / 1024,
                free_gb:   free  / 1024 / 1024 / 1024,
                usage_pct: pct,
            }
        }
    }
}

fn format_uptime(secs: u64) -> String {
    let days  = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins  = (secs % 3600)  / 60;
    if days > 0       { format!("{}j {}h {}min", days, hours, mins) }
    else if hours > 0 { format!("{}h {}min", hours, mins) }
    else              { format!("{}min", mins) }
}

#[no_mangle]
pub fn plugin_entry(registrar: &mut dyn PluginRegistrar) {
    registrar.register_plugin(Box::new(PluginHealth));
}
