use handlebars::{DirectorySourceOptionsBuilder, Handlebars};
use plugin_core::{ActionContext, AppState, Plugin, PluginResult};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tide::{Request, Response};

/// Structure d'une action dans config_actions.json
#[derive(Debug, Deserialize, Clone)]
pub struct ActionConfig {
    pub plugin:      Option<String>,
    // MySQL
    pub sql:         Option<String>,
    // MongoDB
    pub collection:  Option<String>,
    pub filter:      Option<String>,
    pub operation:   Option<String>,
    // Rendu
    pub view:        Option<String>,
    pub return_type: Option<String>,
    pub redirect_to: Option<String>,
}

/// Dispatcher central : résout chaque requête HTTP via config_actions.json
pub struct Dispatcher {
    /// config_actions.json parsé — clé : "METHOD/path"
    config:    HashMap<String, ActionConfig>,
    /// Plugins précachés au démarrage — clé : nom du plugin (ex: "countries")
    plugins:   HashMap<String, Box<dyn Plugin>>,
    /// Moteur de templates Handlebars
    hbs:       Arc<Handlebars<'static>>,
}

impl Dispatcher {
    pub fn new(
        config:       HashMap<String, ActionConfig>,
        plugins:      HashMap<String, Box<dyn Plugin>>,
        templates_dir: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut hbs = Handlebars::new();
        // Charge tous les .hbs du répertoire templates/
        let opts = DirectorySourceOptionsBuilder::default()
            .tpl_extension(".hbs")
            .build()
            .unwrap();
        hbs.register_templates_directory(templates_dir, opts)?;
        hbs.set_strict_mode(false);

        println!("Templates Handlebars chargés depuis : {}", templates_dir);

        Ok(Self {
            config,
            plugins,
            hbs: Arc::new(hbs),
        })
    }

    /// Point d'entrée catch-all : appelé pour chaque requête HTTP
    pub async fn handle(&self, mut req: Request<AppState>) -> tide::Result {
        let method = req.method().to_string().to_uppercase();
        let path   = req.url().path().to_string();

        // ── 1. Construction de la clé de lookup ──────────────────────────────
        // On cherche d'abord une correspondance exacte, puis on essaie
        // de matcher les routes paramétrées (ex: /countries/:code)
        let (action, url_params) = match self.resolve_action(&method, &path) {
            Some(r) => r,
            None => {
                return self.render_error(
                    404,
                    &format!("Route introuvable : {} {}", method, path),
                );
            }
        };

        // ── 2. Collecte de tous les paramètres ───────────────────────────────
        // Paramètres URL (:code), query string (?x=y) et body form/JSON fusionnés
        let mut params: HashMap<String, String> = url_params;

        // Query string : GET /countries?region=Europe
        for (k, v) in req.url().query_pairs() {
            params.insert(k.to_string(), v.to_string());
        }

        // Body JSON ou form-urlencoded pour POST/PUT
        if method == "POST" || method == "PUT" || method == "PATCH" {
            let content_type = req
                .content_type()
                .map(|m| m.essence().to_string())
                .unwrap_or_default();

            if content_type.contains("application/json") {
                if let Ok(body) = req.body_json::<serde_json::Value>().await {
                    if let Some(obj) = body.as_object() {
                        for (k, v) in obj {
                            let val = match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            params.insert(k.clone(), val);
                        }
                    }
                }
            } else {
                // application/x-www-form-urlencoded
                if let Ok(body_str) = req.body_string().await {
                    for pair in body_str.split('&') {
                        let mut parts = pair.splitn(2, '=');
                        if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                            let key = urlencoding_decode(k);
                            let val = urlencoding_decode(v);
                            params.insert(key, val);
                        }
                    }
                }
            }
        }

        // ── Construction du ActionContext ─────────────────────────────────────
        let ctx = ActionContext {
            sql:         action.sql.clone().unwrap_or_default(),
            collection:  action.collection.clone().unwrap_or_default(),
            filter:      action.filter.clone().unwrap_or_else(|| "{}".to_string()),
            operation:   action.operation.clone().unwrap_or_else(|| "find".to_string()),
            params,
            view:        action.view.clone().unwrap_or_default(),
            return_type: action.return_type.clone().unwrap_or_else(|| "json".to_string()),
            redirect_to: action.redirect_to.clone(),
        };

        // ── 4. Exécution du plugin (si défini) ───────────────────────────────
        let data = if let Some(plugin_path) = &action.plugin {
            if plugin_path.is_empty() {
                // Route sans plugin (ex: page d'erreur statique)
                serde_json::Value::Null
            } else {
                // Résolution du plugin par son chemin → nom
                let plugin_name = self.plugin_name_from_path(plugin_path);
                match self.plugins.get(&plugin_name) {
                    Some(plugin) => {
                        let state = req.state().clone();
                        match plugin.execute(&ctx, &state) {
                            PluginResult::Data(v)  => {
                                // on affiche le retour du plugin dans la console.
                                println!("Retour requête plugin : {:?}", v);
                                v    
                            },
                            PluginResult::Error(e) => {
                                eprintln!("[dispatcher] Plugin error: {}", e);
                                return self.render_error(500, &e);
                            }
                        }
                    }
                    None => {
                        return self.render_error(
                            500,
                            &format!("Plugin '{}' non chargé", plugin_name),
                        );
                    }
                }
            }
        } else {
            serde_json::Value::Null
        };
        
        // ── 5. Rendu selon return_type ────────────────────────────────────────
        match ctx.return_type.as_str() {
            "json" => {
                let body = serde_json::to_string(&data)?;
                Ok(Response::builder(200)
                    .body(body)
                    .content_type("application/json")
                    .build())
            }

            "html" => {
                println!("data : {}", data);
                let view_name = ctx.view.trim_end_matches(".hbs");
                match self.hbs.render(view_name, &data) {
                    Ok(html) => Ok(Response::builder(200)
                        .body(html)
                        .content_type("text/html;charset=utf-8")
                        .build()),
                    Err(e) => self.render_error(
                        500,
                        &format!("Erreur template '{}' : {}", view_name, e),
                    ),
                }
            }

            "redirect" => {
                let target = ctx.redirect_to
                    .as_deref()
                    .unwrap_or("/");
                Ok(Response::builder(303)
                    .header("Location", target)
                    .build())
            }

            other => self.render_error(
                500,
                &format!("return_type inconnu : '{}'", other),
            ),
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Résout la clé "METHOD/path" dans la config.
    /// Supporte les routes paramétrées ex: "GET/countries/:code"
    /// Retourne (ActionConfig, HashMap<param_name, param_value>)
    fn resolve_action(
        &self,
        method: &str,
        path: &str,
    ) -> Option<(ActionConfig, HashMap<String, String>)> {
        // 1. Correspondance exacte
        let key = format!("{}{}", method, path);
        if let Some(action) = self.config.get(&key) {
            return Some((action.clone(), HashMap::new()));
        }

        // 2. Correspondance paramétrée
        // On parcourt toutes les clés de la config pour trouver un patron
        for (config_key, action) in &self.config {
            // La clé doit commencer par la même méthode
            if !config_key.starts_with(method) {
                continue;
            }
            let config_path = &config_key[method.len()..];
            if let Some(params) = match_path_params(config_path, path) {
                return Some((action.clone(), params));
            }
        }

        None
    }

    /// Extrait le nom du plugin depuis son chemin (dernier segment sans lib/extension)
    /// "./target/debug/libplugin_countries.so" → "countries"
    fn plugin_name_from_path(&self, path: &str) -> String {
        let filename = path
            .split('/')
            .last()
            .unwrap_or(path);
        // Supprime "lib" au début et l'extension (.so, .dll, .dylib)
        let no_ext = filename
            .split('.')
            .next()
            .unwrap_or(filename);
        let no_lib = no_ext.strip_prefix("lib").unwrap_or(no_ext);
        // Supprime le préfixe "plugin_" ou "plugin-" s'il existe
        let name = no_lib
            .strip_prefix("plugin_")
            .or_else(|| no_lib.strip_prefix("plugin-"))
            .unwrap_or(no_lib);
        name.to_string()
    }

    /// Génère une réponse d'erreur JSON simple
    fn render_error(&self, status: u16, msg: &str) -> tide::Result {
        Ok(Response::builder(status)
            .body(serde_json::json!({ "error": msg }).to_string())
            .content_type("application/json")
            .build())
    }
}

/// Compare un patron de route ("/countries/:code") avec un chemin réel ("/countries/FR")
/// et retourne les paramètres extraits, ou None si pas de correspondance.
fn match_path_params(
    pattern: &str,
    path: &str,
) -> Option<HashMap<String, String>> {
    let p_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let r_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if p_segs.len() != r_segs.len() {
        return None;
    }

    let mut params = HashMap::new();
    for (p, r) in p_segs.iter().zip(r_segs.iter()) {
        if let Some(param_name) = p.strip_prefix(':') {
            params.insert(param_name.to_string(), r.to_string());
        } else if p != r {
            return None;
        }
    }
    Some(params)
}

/// Décode un segment URL-encodé (%20 → espace, + → espace, etc.)
fn urlencoding_decode(s: &str) -> String {
    let with_spaces = s.replace('+', " ");
    let mut result  = String::new();
    let mut chars   = with_spaces.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next().unwrap_or('0');
            let h2 = chars.next().unwrap_or('0');
            if let Ok(byte) = u8::from_str_radix(&format!("{}{}", h1, h2), 16) {
                result.push(byte as char);
            }
        } else {
            result.push(c);
        }
    }
    result
}
