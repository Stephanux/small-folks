use sqlx::MySqlPool;
use tokio::runtime::Handle;

/// État partagé injecté dans chaque requête Tide.
#[derive(Clone)]
pub struct AppState {
    pub pool:   MySqlPool,
    pub handle: Handle,
    pub mongo:  Option<mongodb::Client>,

}

/// Contexte d'action injecté par le dispatcher dans chaque plugin.
/// Contient tout ce dont le plugin a besoin pour exécuter son travail
/// sans connaître les détails de la route ou de la config.
#[derive(Debug, Clone)]
pub struct ActionContext {
    /// Requête SQL lue depuis config_actions.json (avec params nommés :param)
    pub sql:         String,
    pub collection:  String,
    pub filter:      String,
    pub operation:   String,
    /// Paramètres nommés extraits de l'URL ou du body (:code, :name_us, ...)
    pub params:      std::collections::HashMap<String, String>,
    /// Nom du template Handlebars à utiliser (vide si return_type = json)
    pub view:        String,
    /// Type de retour attendu : "json", "html", "redirect"
    pub return_type: String,
    /// URL de redirection (uniquement si return_type = "redirect")
    pub redirect_to: Option<String>,
}

/// Résultat retourné par un plugin au dispatcher.
pub enum PluginResult {
    /// Données JSON à sérialiser ou à passer au template Handlebars
    Data(serde_json::Value),
    /// Message d'erreur à afficher
    Error(String),
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
