use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult, SessionUser};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Column, Row};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub struct PluginAuth;

// ── Structure JWT claims ──────────────────────────────────────────────────────
#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    sub:      String,   // login de l'utilisateur
    id:       i64,
    name:     String,
    function: String,
    iat:      u64,
    exp:      u64,
}

impl Plugin for PluginAuth {
    fn name(&self) -> &'static str { "auth" }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        match ctx.operation.as_str() {
            "login"  => do_login(ctx, state),
            "logout" => do_logout(ctx, state),
            "me"     => do_me(ctx, state),
            other    => PluginResult::Error(
                format!("Opération auth inconnue : '{}'", other)
            ),
        }
    }
}

// ── LOGIN ─────────────────────────────────────────────────────────────────────

fn do_login(ctx: &ActionContext, state: &AppState) -> PluginResult {
    let login = ctx.params.get("login").cloned().unwrap_or_default();
    let mdp   = ctx.params.get("mdp").cloned().unwrap_or_default();
    let next  = ctx.params.get("next").cloned().unwrap_or_default();

    if login.is_empty() || mdp.is_empty() {
        return PluginResult::AuthError("Login et mot de passe requis".into());
    }

    // La requête SQL vient de config_actions.json
    // Si absente, on retourne une erreur claire
    if ctx.sql.trim().is_empty() {
        return PluginResult::AuthError(
            "Champ 'sql' manquant dans config_actions.json pour l'action login".into()
        );
    }

    // Remplacer :login et :mdp par ? pour sqlx
    let (sql_prepared, param_values) = named_to_positional(&ctx.sql, &ctx.params);

    let session_ttl: u64 = std::env::var("SESSION_TTL_SECONDS")
        .unwrap_or_else(|_| "3600".to_string())
        .parse()
        .unwrap_or(3600);

    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "small-folks-secret-change-me".to_string());

    let pool     = state.pool.clone();
    let sessions = state.sessions.clone();

    tokio::task::block_in_place(|| {
        state.handle.block_on(async move {
            // Exécution de la requête SQL depuis la config
            let mut query = sqlx::query(&sql_prepared);
            for val in &param_values {
                query = query.bind(val);
            }

            let row = query.fetch_optional(&pool).await;
            println!("données authentification : {:?}", row);
            match row {
                Err(e) => PluginResult::AuthError(
                    format!("Erreur base de données : {}", e)
                ),
                Ok(None) => PluginResult::AuthError(
                    "Identifiants incorrects".into()
                ),
                Ok(Some(row)) => {
                    // ── Lecture des colonnes par alias ────────────────────────
                    // Convention : les alias SQL définissent le mapping
                    // Les colonnes sont lues dynamiquement par leur nom
                    let cols: HashMap<String, String> = row.columns().iter()
                        .enumerate()
                        .map(|(i, col)| {
                            let val: Option<String> = row.try_get(i).ok();
                            (col.name().to_string(), val.unwrap_or_default())
                        })
                        .collect();

                    // Colonnes attendues (avec fallbacks sur noms courants)
                    // "id"    → alias recommandé pour la clé primaire
                    // "name"  → alias pour le nom
                    // etc.
                    let id_str = cols.get("id")
                        .or_else(|| cols.get("id_users"))
                        .or_else(|| cols.get("id_user"))
                        .cloned()
                        .unwrap_or_default();

                    let id: i64 = id_str.parse().unwrap_or(0);

                    let name = cols.get("name")
                        .cloned()
                        .unwrap_or_default();

                    let first_name = cols.get("first_name")
                        .or_else(|| cols.get("firstName"))
                        .or_else(|| cols.get("firstname"))
                        .cloned()
                        .unwrap_or_default();

                    let login_str = cols.get("login")
                        .cloned()
                        .unwrap_or(login.clone());

                    let function = cols.get("function")
                        .or_else(|| cols.get("role"))
                        .cloned()
                        .unwrap_or_default();

                    let office = cols.get("office")
                        .or_else(|| cols.get("department"))
                        .cloned()
                        .unwrap_or_default();

                    let now        = now_secs();
                    let expires_at = now + session_ttl;
                    let session_id = Uuid::new_v4().to_string();

                    // Stockage dans le cache de sessions
                    {
                        let mut sess = sessions.lock().unwrap();
                        sess.retain(|_, v| v.expires_at > now);
                        sess.insert(session_id.clone(), SessionUser {
                            id_users:   id,
                            login:      login_str.clone(),
                            name:       name.clone(),
                            first_name: first_name.clone(),
                            function:   function.clone(),
                            office:     office.clone(),
                            expires_at,
                        });
                    }

                    // Génération du JWT
                    let claims = JwtClaims {
                        sub:      login_str.clone(),
                        id,
                        name:     format!("{} {}", first_name, name),
                        function: function.clone(),
                        iat:      now,
                        exp:      expires_at,
                    };

                    let jwt = match jsonwebtoken::encode(
                        &jsonwebtoken::Header::default(),
                        &claims,
                        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.as_bytes()),
                    ) {
                        Ok(t)  => t,
                        Err(e) => return PluginResult::AuthError(
                            format!("Erreur génération JWT : {}", e)
                        ),
                    };

                    let user_json = json!({
                        "id":         id,
                        "login":      login_str,
                        "name":       name,
                        "first_name": first_name,
                        "function":   function,
                        "office":     office,
                    });

                    PluginResult::AuthSuccess {
                        session_id,
                        jwt,
                        redirect_to: next,
                        user: user_json,
                    }
                }
            }
        })
    })
}

// ── LOGOUT ────────────────────────────────────────────────────────────────────

fn do_logout(ctx: &ActionContext, state: &AppState) -> PluginResult {
    if let Some(session_id) = ctx.params.get("session_id") {
        let mut sess = state.sessions.lock().unwrap();
        sess.remove(session_id);
    }
    let redirect = ctx.redirect_to.clone()
        .unwrap_or_else(|| "/login".to_string());
    PluginResult::AuthLogout { redirect_to: redirect }
}

// ── ME (infos utilisateur courant) ───────────────────────────────────────────

fn do_me(ctx: &ActionContext, state: &AppState) -> PluginResult {
    let session_id = match ctx.params.get("session_id") {
        Some(s) => s.clone(),
        None    => return PluginResult::Error("Non authentifié".into()),
    };

    let now  = now_secs();
    let sess = state.sessions.lock().unwrap();

    match sess.get(&session_id) {
        None => PluginResult::Error("Session invalide ou expirée".into()),
        Some(user) if user.expires_at <= now => {
            PluginResult::Error("Session expirée".into())
        }
        Some(user) => PluginResult::Data(json!({
            "id_users":   user.id_users,
            "login":      user.login,
            "name":       user.name,
            "first_name": user.first_name,
            "function":   user.function,
            "office":     user.office,
            "expires_at": user.expires_at,
        })),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Convertit ":param" → "?" et collecte les valeurs dans l'ordre d'apparition
fn named_to_positional(
    sql: &str,
    params: &HashMap<String, String>,
) -> (String, Vec<String>) {
    let re = regex::Regex::new(r":([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
    let mut values = Vec::new();
    let sql_out = re.replace_all(sql, |caps: &regex::Captures| {
        let name = &caps[1];
        values.push(params.get(name).cloned().unwrap_or_default());
        "?"
    });
    (sql_out.to_string(), values)
}

#[no_mangle]
pub fn plugin_entry(registrar: &mut dyn PluginRegistrar) {
    registrar.register_plugin(Box::new(PluginAuth));
}