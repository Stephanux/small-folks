use handlebars::{DirectorySourceOptionsBuilder, Handlebars};
use plugin_core::{ActionContext, AppState, Plugin, PluginResult};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tide::{Request, Response};

/// Structure d'une action dans config_actions.json
fn default_form_columns() -> u8 { 1 }

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
    /// Nom du champ fichier dans le formulaire multipart (plugin_sql_upload)
    #[serde(default)]
    pub upload_field: String,
     // Ressources pour listes déroulantes
    // data_resources : { "nom_colonne": "nom_ressource" }
    #[serde(default)]
    pub data_resources: std::collections::HashMap<String, String>,
    // sql_resources : { "nom_ressource": "SELECT ..." }
    #[serde(default)]
    pub sql_resources:  std::collections::HashMap<String, String>,

    // Rendu
    pub view:         Option<String>,
    pub return_type:  Option<String>,
    pub redirect_to:  Option<String>,
    /// Si true : exige un cookie session_id valide avant d'exécuter le plugin
    #[serde(default)]
    pub auth:         bool,
    #[serde(default)]
    pub form_action: Option<String>,   // ← nouveau action a placer dans l'attribut du même nom du formulaire.
    #[serde(default = "default_form_columns")]
    pub form_columns: u8,
    // Champs qui occupent toute la largeur même en mode 2 colonnes
    #[serde(default)]
    pub form_fullwidth_fields: Vec<String>,
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

        // ── Collecte des paramètres ───────────────────────────────────────────
        let mut params: HashMap<String, String> = url_params;
        for (k, v) in req.url().query_pairs() {
            params.insert(k.to_string(), v.to_string());
        }

        // ── Cookie session_id → injecté dans params pour plugin_auth ─────────
        if let Some(cookie) = req.cookie("session_id") {
            params.insert("session_id".to_string(), cookie.value().to_string());
        }

        // ── Lecture Content-Type et body ──────────────────────────────────────
        let content_type = req
            .content_type()
            .map(|m| m.to_string())
            .unwrap_or_default();

        let body_bytes: Vec<u8> = if content_type.contains("multipart/form-data") {
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

        let upload_dir = action.upload_dir.clone()
            .unwrap_or_else(|| std::env::var("UPLOAD_DIR")
                .unwrap_or_else(|_| "./uploads".to_string()));

        let ctx = ActionContext {
            sql:          action.sql.clone().unwrap_or_default(),
            collection:   action.collection.clone().unwrap_or_default(),
            filter:       action.filter.clone().unwrap_or_else(|| "{}".to_string()),
            operation:    action.operation.clone().unwrap_or_else(|| "find".to_string()),
            upload_dir,
            upload_field: action.upload_field.clone(),
            allowed_mime: action.allowed_mime.clone()
                .unwrap_or_else(|| "image/jpeg,image/png,application/pdf".to_string()),
            max_size_mb:  action.max_size_mb.clone().unwrap_or_else(|| "10".to_string()),
            data_resources: action.data_resources.clone(),
            sql_resources:  action.sql_resources.clone(),
            params,
            view:         action.view.clone().unwrap_or_default(),
            return_type:  action.return_type.clone().unwrap_or_else(|| "json".to_string()),
            redirect_to:  action.redirect_to.clone(),
            body_bytes,
            content_type,
            form_action: action.form_action.clone(),   // ← nouveau
             form_columns:            action.form_columns,
            form_fullwidth_fields:   action.form_fullwidth_fields.clone(),
        };

        // ── Vérification authentification ────────────────────────────────────
        if action.auth {
            let session_id = ctx.params.get("session_id").cloned().unwrap_or_default();
            let authenticated = if session_id.is_empty() {
                false
            } else {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let sess = req.state().sessions.lock().unwrap();
                match sess.get(&session_id) {
                    Some(user) if user.expires_at > now => true,
                    _ => false,
                }
            };

            if !authenticated {
                // API → 401 JSON
                // Page HTML → redirect vers /login
                if ctx.return_type == "json" {
                    return Ok(Response::builder(401)
                        .body(serde_json::json!({"error": "Non authentifié"}).to_string())
                        .content_type("application/json")
                        .build());
                } else {
                    let login_url = format!("/login?next={}", urlencoding_encode(&path));
                    return Ok(Response::builder(303)
                        .header("Location", login_url.as_str())
                        .build());
                }
            }
        }

        // ── Exécution du plugin ───────────────────────────────────────────────
        if let Some(plugin_path) = &action.plugin {
            if !plugin_path.is_empty() {
                let plugin_name = self.plugin_name_from_path(plugin_path);
                match self.plugins.get(&plugin_name) {
                    None => return self.render_error(500,
                        &format!("Plugin '{}' non chargé", plugin_name)),
                    Some(plugin) => {
                        let state = req.state().clone();
                        match plugin.execute(&ctx, &state) {

                            // ── Cas standard ─────────────────────────────────
                            PluginResult::Data(data) => {
                                return self.render_data(data, &ctx);
                            }
                            PluginResult::Error(e) => {
                                eprintln!("[dispatcher] Plugin error: {}", e);
                                return self.render_error(500, &e);
                            }

                            // ── Login réussi ──────────────────────────────────
                            // IMPORTANT : Tide écrase les headers en double.
                            // On doit utiliser insert_cookie() pour poser
                            // plusieurs cookies sur la même réponse.
                            PluginResult::AuthSuccess { session_id, jwt, redirect_to, user: _ } => {
                                let session_ttl = std::env::var("SESSION_TTL_SECONDS")
                                    .unwrap_or_else(|_| "3600".to_string())
                                    .parse::<i64>()
                                    .unwrap_or(3600);

                                let mut res = tide::Response::new(303);
                                res.insert_header("Location", redirect_to.as_str());

                                // Cookie session_id — HttpOnly (non accessible JS)
                                res.insert_header(
                                    "Set-Cookie",
                                    format!(
                                        "session_id={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
                                        session_id, session_ttl
                                    ).as_str(),
                                );
                                // Cookie jwt — accessible JS pour les appels API
                                // On utilise append_header pour ne pas écraser session_id
                                res.append_header(
                                    "Set-Cookie",
                                    format!(
                                        "jwt_token={}; Path=/; Max-Age={}; SameSite=Lax",
                                        jwt, session_ttl
                                    ).as_str(),
                                );
                                return Ok(res);
                            }

                            // ── Login échoué ──────────────────────────────────
                            PluginResult::AuthError(msg) => {
                                eprintln!("[auth] Échec login : {}", msg);
                                return Ok(Response::builder(303)
                                    .header("Location", "/login?error=1")
                                    .build());
                            }

                            // ── Logout ────────────────────────────────────────
                            PluginResult::AuthLogout { redirect_to } => {
                                return Ok(Response::builder(303)
                                    .header("Location",   &redirect_to)
                                    .header("Set-Cookie", "session_id=; Path=/; Max-Age=0; HttpOnly")
                                    .header("Set-Cookie", "jwt_token=; Path=/; Max-Age=0")
                                    .build());
                            }
                        }
                    }
                }
            }
        }

        // Route sans plugin
        self.render_data(serde_json::Value::Null, &ctx)
    }
        // ── Rendu selon return_type ───────────────────────────────────────────
        fn render_data(&self, data: serde_json::Value, ctx: &ActionContext) -> tide::Result {
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
                    // DEBUG temporaire — à supprimer après diagnostic
                   /* println!("[dispatcher] data envoyée au template '{}' :\n{}", 
                        view_name, 
                        serde_json::to_string_pretty(&data).unwrap_or_default()
                    );*/
                    // Injecter form_action + form_columns + form_fullwidth_fields
                    // si défini et que les données ne sont pas déjà enrichies par plugin_sql
                    let data = if let Some(fa) = &ctx.form_action {
                        match &data {
                            serde_json::Value::Object(map) if map.contains_key("data") => data,
                            serde_json::Value::Array(arr) => {
                                // Enrichir chaque record avec le flag fullwidth par champ
                                let fullwidth_set: std::collections::HashSet<&String> =
                                    ctx.form_fullwidth_fields.iter().collect();
                                let data_with_meta: Vec<serde_json::Value> = arr.iter()
                                    .map(|row| {
                                        if let serde_json::Value::Object(obj) = row {
                                            let fields: Vec<serde_json::Value> = obj.iter()
                                                .map(|(k, v)| serde_json::json!({
                                                    "key":       k,
                                                    "value":     v,
                                                    "fullwidth": fullwidth_set.contains(k),
                                                }))
                                                .collect();
                                            serde_json::json!({ "fields": fields })
                                        } else { row.clone() }
                                    }).collect();
                                let mut w = serde_json::Map::new();
                                w.insert("data".into(), serde_json::Value::Array(data_with_meta));
                                w.insert("form_action".into(),
                                    serde_json::Value::String(fa.clone()));
                                w.insert("form_columns".into(),
                                    serde_json::Value::Number(ctx.form_columns.into()));
                                serde_json::Value::Object(w)
                            }
                            _ => data,
                        }
                    } else {
                        data
                    };

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

fn urlencoding_encode(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9'
            | '-' | '_' | '.' | '~' | '/' => result.push(c),
            c => {
                let bytes = c.to_string();
                for b in bytes.as_bytes() {
                    result.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    result
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
