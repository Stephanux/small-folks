mod dispatcher;

use dispatcher::Dispatcher;
use libloading::{Library, Symbol};
use plugin_core::{AppState, Plugin, PluginRegistrar};
use sqlx::mysql::MySqlPoolOptions;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::runtime::Handle;

type PluginEntryFn = unsafe fn(&mut dyn PluginRegistrar);

struct PluginRegistry {
    plugins: HashMap<String, Box<dyn Plugin>>,
}

impl PluginRegistry {
    fn new() -> Self {
        Self { plugins: HashMap::new() }
    }
}

impl PluginRegistrar for PluginRegistry {
    fn register_plugin(&mut self, plugin: Box<dyn Plugin>) {
        println!("  → Plugin enregistré : {}", plugin.name());
        self.plugins.insert(plugin.name().to_string(), plugin);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://admin:azerty@localhost:3306/R504TP".to_string());
    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let config_path = env::var("CONFIG_ACTIONS")
        .unwrap_or_else(|_| "./config_actions.json".to_string());
    let templates_dir = env::var("TEMPLATES_DIR")
        .unwrap_or_else(|_| "./templates".to_string());
    let bind_addr = format!("{}:{}", host, port);

    // ── Lecture de config_actions.json ────────────────────────────────────────
    println!("Lecture de la configuration : {}", config_path);
    let config_str = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("Impossible de lire {} : {}", config_path, e));
    let config: HashMap<String, dispatcher::ActionConfig> =
        serde_json::from_str(&config_str)
        .unwrap_or_else(|e| panic!("JSON invalide dans {} : {}", config_path, e));
    println!("  → {} actions chargées\n", config.len());

    // ── Pool MySQL ────────────────────────────────────────────────────────────
    println!("Connexion à la base de données...");
    let pool = MySqlPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&database_url)
        .await?;
    println!("Pool MySQL prêt\n");

    let handle = Handle::current();
    let state  = AppState { pool, handle };

    // ── Précache des plugins au démarrage ────────────────────────────────────
    // On collecte les chemins uniques de plugins référencés dans le JSON
    let plugin_paths: std::collections::HashSet<String> = config.values()
        .filter_map(|a| {
            if a.plugin.as_deref().unwrap_or("").is_empty() { None }
            else { a.plugin.clone() }
        })
        .collect();

    let mut _libraries: Vec<Library> = Vec::new();
    let mut registry = PluginRegistry::new();

    println!("Chargement des plugins :");
    for path in &plugin_paths {
        unsafe {
            match Library::new(path) {
                Ok(lib) => {
                    match lib.get::<Symbol<PluginEntryFn>>(b"plugin_entry") {
                        Ok(func) => {
                            func(&mut registry);
                            _libraries.push(lib);
                        }
                        Err(e) => eprintln!("  ✗ {} — symbole introuvable: {}", path, e),
                    }
                }
                Err(e) => eprintln!("  ✗ {} — chargement échoué: {}", path, e),
            }
        }
    }

    // ── Dispatcher ───────────────────────────────────────────────────────────
    let dispatcher = Arc::new(Dispatcher::new(
        config,
        registry.plugins,
        &templates_dir,
    )?);

    // ── Serveur Tide ──────────────────────────────────────────────────────────
    let mut app = tide::with_state(state);
    tide::log::start();
    app.with(tide::log::LogMiddleware::new());

    // Route de santé (hors dispatcher)
    app.at("/health").get(|_| async move {
        Ok(tide::Response::builder(200)
            .body("{\"status\":\"ok\"}")
            .content_type("application/json")
            .build())
    });

    // ── Catch-all : toutes les routes passent par le dispatcher ───────────────
    {
        let d = Arc::clone(&dispatcher);
        app.at("/*").all(move |req| {
            let d = Arc::clone(&d);
            async move { d.handle(req).await }
        });
    }
    {
        let d = Arc::clone(&dispatcher);
        app.at("/").all(move |req| {
            let d = Arc::clone(&d);
            async move { d.handle(req).await }
        });
    }

    println!("\nServeur Tide démarré sur http://{}", bind_addr);
    println!("Configuration : {}", config_path);
    println!("Templates     : {}", templates_dir);
    println!("Route de santé: GET /health\n");

    app.listen(bind_addr).await?;
    Ok(())
}
