use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult, SessionUser};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub struct PluginAuth;

// ── Structure JWT claims ──────────────────────────────────────────────────────
#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    sub:       String,   // login de l'utilisateur
    id_users:  i64,
    name:      String,
    function:  String,
    iat:       u64,      // issued at (epoch)
    exp:       u64,      // expiration (epoch)
}

// ── Résultat de la requête SQL users ─────────────────────────────────────────
#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id_users:   i64,
    name:       Option<String>,
    #[sqlx(rename = "firstName")]
    first_name: Option<String>,
    login:      Option<String>,
    function:   Option<String>,
    office:     Option<String>,
}

impl Plugin for PluginAuth {
    fn name(&self) -> &'static str { "auth" }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        let operation = ctx.operation.as_str();

        match operation {
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
    let next   = ctx.params.get("next").cloned().unwrap_or_default();
    println!("===> next value into plugin_auth : {}", next);

    if login.is_empty() || mdp.is_empty() {
        return PluginResult::AuthError("Login et mot de passe requis".into());
    }

    // Durée de session depuis .env (défaut 3600s = 1h)
    let session_ttl: u64 = std::env::var("SESSION_TTL_SECONDS")
        .unwrap_or_else(|_| "3600".to_string())
        .parse()
        .unwrap_or(3600);

    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "small-folks-secret-change-me".to_string());

    let pool    = state.pool.clone();
    let sessions = state.sessions.clone();

    tokio::task::block_in_place(|| {
        state.handle.block_on(async move {
            println!("en entrée de la fonction do_login");
            // Vérification login/mdp en base (mot de passe en clair)
            let row = sqlx::query_as::<_, UserRow>(
                "SELECT id_users, name, firstName, login, function, office
                 FROM users
                 WHERE login = ? AND mdp = ?
                 LIMIT 1"
            )
            .bind(&login)
            .bind(&mdp)
            .fetch_optional(&pool)
            .await;
            println!("Row : {:?}", row);
            match row {
                Err(e) => PluginResult::AuthError(
                    format!("Erreur base de données : {}", e)
                ),
                Ok(None) => PluginResult::AuthError(
                    "Identifiants incorrects".into()
                ),
                Ok(Some(user)) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    let expires_at = now + session_ttl;
                    let session_id = Uuid::new_v4().to_string();

                    let name       = user.name.clone().unwrap_or_default();
                    let first_name = user.first_name.clone().unwrap_or_default();
                    let function   = user.function.clone().unwrap_or_default();
                    let office     = user.office.clone().unwrap_or_default();
                    let login_str  = user.login.clone().unwrap_or_default();

                    // Stockage dans le cache de sessions
                    {
                        let mut sess = sessions.lock().unwrap();
                        // Nettoyer les sessions expirées au passage
                        sess.retain(|_, v| v.expires_at > now);
                        sess.insert(session_id.clone(), SessionUser {
                            id_users: user.id_users,
                            login:    login_str.clone(),
                            name:     name.clone(),
                            first_name: first_name.clone(),
                            function:   function.clone(),
                            office:     office.clone(),
                            expires_at,
                        });
                    }

                    // Génération du JWT
                    let claims = JwtClaims {
                        sub:      login_str.clone(),
                        id_users: user.id_users,
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
                        "id_users":   user.id_users,
                        "login":      login_str,
                        "name":       name,
                        "first_name": first_name,
                        "function":   function,
                        "office":     office,
                    });
                    println!("==> retour authentif : {:?}", user_json);
                    let redirect = std::env::var("LOGIN_REDIRECT")
                        .unwrap_or_else(|_| "/".to_string());

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
    // Récupère le session_id depuis les params (passé par le dispatcher via cookie)
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

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

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

#[no_mangle]
pub fn plugin_entry(registrar: &mut dyn PluginRegistrar) {
    registrar.register_plugin(Box::new(PluginAuth));
}
