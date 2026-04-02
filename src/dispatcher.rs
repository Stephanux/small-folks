use handlebars::{DirectorySourceOptionsBuilder, Handlebars};
use plugin_core::{ActionContext, AppState, Plugin, PluginResult};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tide::{Request, Response};

/// Structure d'une action dans config_actions.json
#[derive(Debug, Deserialize, Clone)]
pub struct ActionConfig {
    pub plugin:       Option<String>,
    // SQL
    pub sql:          Option<String>,
    // MongoDB
    pub collection:   Option<String>,
    pub filter:       Option<String>,
    pub operation:    Option<String>,
    // Upload
    pub upload_dir:   Option<String>,
    pub allowed_mime: Option<String>,
    pub max_size_mb:  Option<String>,
    // Rendu
    pub view:         Option<String>,
    pub return_type:  Option<String>,
    pub redirect_to:  Option<String>,
}

pub struct Dispatcher {
    config:  HashMap<String, ActionConfig>,
    plugins: HashMap<String, Box<dyn Plugin>>,
    hbs:     Arc<Handlebars<'static>>,
}

impl Dispatcher {
    pub fn new(
        config:        HashMap<String, ActionConfig>,
        plugins:       HashMap<String, Box<dyn Plugin>>,
        templates_dir: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut hbs = Handlebars::new();
        let opts = DirectorySourceOptionsBuilder::default()
            .tpl_extension(".hbs")
            .build()
            .unwrap();
        hbs.register_templates_directory(templates_dir, opts)?;
        hbs.set_strict_mode(false);
        println!("Templates Handlebars chargés depuis : {}", templates_dir);
        Ok(Self { config, plugins, hbs: Arc::new(hbs) })
    }

    pub async fn handle(&self, mut req: Request<AppState>) -> tide::Result {
        let method = req.method().to_string().to_uppercase();
        let path   = req.url().path().to_string();

        let (action, url_params) = match self.resolve_action(&method, &path) {
            Some(r) => r,
            None    => return self.render_error(404,
                &format!("Route introuvable : {} {}", method, path)),
        };

        // ── Collecte des paramètres URL / query string ────────────────────────
        let mut params: HashMap<String, String> = url_params;
        for (k, v) in req.url().query_pairs() {
            params.insert(k.to_string(), v.to_string());
        }

        // ── Lecture du Content-Type et du body ────────────────────────────────
        let content_type = req
            .content_type()
            .map(|m| m.to_string())
            .unwrap_or_default();

        // Pour multipart : on lit le body brut (bytes)
        // Pour JSON/form classique : on parse les paramètres
        let body_bytes: Vec<u8> = if content_type.contains("multipart/form-data") {
            // Body brut pour le plugin_upload
            req.body_bytes().await.unwrap_or_default()
        } else if matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
            if content_type.contains("application/json") {
                let raw = req.body_bytes().await.unwrap_or_default();
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&raw) {
                    if let Some(obj) = val.as_object() {
                        for (k, v) in obj {
                            let s = match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            params.insert(k.clone(), s);
                        }
                    }
                }
                Vec::new()
            } else {
                // application/x-www-form-urlencoded
                let body_str = req.body_string().await.unwrap_or_default();
                for pair in body_str.split('&') {
                    let mut parts = pair.splitn(2, '=');
                    if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                        params.insert(urlencoding_decode(k), urlencoding_decode(v));
                    }
                }
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // ── Résolution de UPLOAD_DIR depuis .env ──────────────────────────────
        let upload_dir = action.upload_dir
            .clone()
            .unwrap_or_else(|| std::env::var("UPLOAD_DIR")
                .unwrap_or_else(|_| "./uploads".to_string()));

        // ── Construction du ActionContext ─────────────────────────────────────
        let ctx = ActionContext {
            sql:          action.sql.clone().unwrap_or_default(),
            collection:   action.collection.clone().unwrap_or_default(),
            filter:       action.filter.clone().unwrap_or_else(|| "{}".to_string()),
            operation:    action.operation.clone().unwrap_or_else(|| "find".to_string()),
            upload_dir,
            allowed_mime: action.allowed_mime.clone()
                .unwrap_or_else(|| "image/jpeg,image/png,application/pdf".to_string()),
            max_size_mb:  action.max_size_mb.clone().unwrap_or_else(|| "10".to_string()),
            params,
            view:         action.view.clone().unwrap_or_default(),
            return_type:  action.return_type.clone().unwrap_or_else(|| "json".to_string()),
            redirect_to:  action.redirect_to.clone(),
            body_bytes,
            content_type,
        };

        // ── Exécution du plugin ───────────────────────────────────────────────
        let data = if let Some(plugin_path) = &action.plugin {
            if plugin_path.is_empty() {
                serde_json::Value::Null
            } else {
                let plugin_name = self.plugin_name_from_path(plugin_path);
                match self.plugins.get(&plugin_name) {
                    Some(plugin) => {
                        let state = req.state().clone();
                        match plugin.execute(&ctx, &state) {
                            PluginResult::Data(v)  => v,
                            PluginResult::Error(e) => {
                                eprintln!("[dispatcher] Plugin error: {}", e);
                                return self.render_error(500, &e);
                            }
                        }
                    }
                    None => return self.render_error(500,
                        &format!("Plugin '{}' non chargé", plugin_name)),
                }
            }
        } else {
            serde_json::Value::Null
        };

        // ── Rendu selon return_type ───────────────────────────────────────────
        match ctx.return_type.as_str() {
            "json" => {
                let body = serde_json::to_string(&data)?;
                Ok(Response::builder(200)
                    .body(body)
                    .content_type("application/json")
                    .build())
            }
            "html" => {
                let view_name = ctx.view.trim_end_matches(".hbs");
                match self.hbs.render(view_name, &data) {
                    Ok(html) => Ok(Response::builder(200)
                        .body(html)
                        .content_type("text/html;charset=utf-8")
                        .build()),
                    Err(e) => self.render_error(500,
                        &format!("Erreur template '{}' : {}", view_name, e)),
                }
            }
            "redirect" => {
                let target = ctx.redirect_to.as_deref().unwrap_or("/");
                Ok(Response::builder(303)
                    .header("Location", target)
                    .build())
            }
            other => self.render_error(500,
                &format!("return_type inconnu : '{}'", other)),
        }
    }

    fn resolve_action(
        &self, method: &str, path: &str,
    ) -> Option<(ActionConfig, HashMap<String, String>)> {
        let key = format!("{}{}", method, path);
        if let Some(action) = self.config.get(&key) {
            return Some((action.clone(), HashMap::new()));
        }
        for (config_key, action) in &self.config {
            if !config_key.starts_with(method) { continue; }
            let config_path = &config_key[method.len()..];
            if let Some(params) = match_path_params(config_path, path) {
                return Some((action.clone(), params));
            }
        }
        None
    }

    fn plugin_name_from_path(&self, path: &str) -> String {
        let filename = path.split('/').last().unwrap_or(path);
        let no_ext   = filename.split('.').next().unwrap_or(filename);
        let no_lib   = no_ext.strip_prefix("lib").unwrap_or(no_ext);
        no_lib.strip_prefix("plugin_")
              .or_else(|| no_lib.strip_prefix("plugin-"))
              .unwrap_or(no_lib)
              .to_string()
    }

    fn render_error(&self, status: u16, msg: &str) -> tide::Result {
        Ok(Response::builder(status)
            .body(serde_json::json!({ "error": msg }).to_string())
            .content_type("application/json")
            .build())
    }
}

fn match_path_params(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let p_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let r_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if p_segs.len() != r_segs.len() { return None; }
    let mut params = HashMap::new();
    for (p, r) in p_segs.iter().zip(r_segs.iter()) {
        if let Some(name) = p.strip_prefix(':') {
            params.insert(name.to_string(), r.to_string());
        } else if p != r {
            return None;
        }
    }
    Some(params)
}

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
