use sqlx::MySqlPool;
use tokio::runtime::Handle;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};



/// Informations utilisateur stockées dans le cache de sessions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionUser {
    pub id_users:      i64,
    pub login:         String,
    pub name:          String,
    pub first_name:    String,
    pub function:      String,
    pub office:        String,
    /// Timestamp Unix d'expiration (epoch secondes)
    pub expires_at:    u64,
}

/// État partagé injecté dans chaque requête Tide.
#[derive(Clone)]
pub struct AppState {
    pub pool:   MySqlPool,
    pub handle: Handle,
    pub mongo:  Option<mongodb::Client>,
    /// Cache des sessions actives : session_id → SessionUser
    /// Arc<Mutex> pour partage thread-safe entre handlers Tide
    pub sessions: Arc<Mutex<HashMap<String, SessionUser>>>,

}

/// Contexte d'action injecté par le dispatcher dans chaque plugin.
/// Contient tout ce dont le plugin a besoin pour exécuter son travail
/// sans connaître les détails de la route ou de la config.
#[derive(Debug, Clone)]
pub struct ActionContext {
    // ── SQL ───────────────────────────────────────────────────────────────────
    pub sql:         String,
    // ── MongoDB ───────────────────────────────────────────────────────────────
    pub collection:  String,
    pub filter:      String,
    pub operation:   String,
    // ── Upload ────────────────────────────────────────────────────────────────
    /// Dossier de destination sur le disque (lu depuis UPLOAD_DIR dans .env)
    pub upload_dir:  String,
    /// Types MIME autorisés séparés par virgule (ex: "image/jpeg,image/png,application/pdf")
    pub allowed_mime: String,
    /// Taille maximale en Mo (ex: "10")
    pub max_size_mb:  String,
    // ── Commun ────────────────────────────────────────────────────────────────
    pub params:      std::collections::HashMap<String, String>,
    pub view:        String,
    pub return_type: String,
    pub redirect_to: Option<String>,
    /// Corps brut de la requête (pour multipart/form-data)
    pub body_bytes:  Vec<u8>,
    /// Content-Type complet de la requête (nécessaire pour parser le boundary multipart)
    pub content_type: String,
    pub form_action: Option<String>,   // ← nouveau gère l'action d'un formulaire (ex.: "/countrie")
}

/// Résultat retourné par un plugin au dispatcher.
pub enum PluginResult {
    /// Données JSON à sérialiser ou à passer au template Handlebars
    Data(serde_json::Value),
    /// Erreur générique → HTTP 500
    Error(String),
    /// Login réussi → dispatcher pose les cookies et redirige
    AuthSuccess {
        session_id:  String,
        jwt:         String,
        redirect_to: String,
        user:        serde_json::Value,
    },
    /// Login échoué → dispatcher redirige vers /login?error=1
    AuthError(String),
    /// Logout → dispatcher supprime les cookies et redirige
    AuthLogout {
        redirect_to: String,
    },
}

/// Trait que chaque plugin doit implémenter.
///
/// # Frontière cdylib
/// Les plugins sont des `.so` / `.dll`. Chaque cdylib embarque
/// potentiellement sa propre copie de Tokio/sqlx. Pour éviter la panique
/// "this functionality requires a Tokio context", toute Future sqlx doit
/// être exécutée via `tokio::task::block_in_place` + `handle.block_on(...)`.
pub trait Plugin: Send + Sync {
    /// Identifiant unique du plugin (ex: "countries", "regions").
    fn name(&self) -> &'static str;

    /// Exécute l'action décrite dans le contexte et retourne des données JSON.
    /// Le dispatcher se charge ensuite du rendu (html/json/redirect).
    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult;
}

/// Interface FFI : le plugin appelle register_plugin pour s'enregistrer.
pub trait PluginRegistrar: Send {
    fn register_plugin(&mut self, plugin: Box<dyn Plugin>);
}
