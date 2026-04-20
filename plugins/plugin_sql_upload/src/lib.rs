// plugin_sql_upload — Fusion plugin_sql + plugin_upload
//
// Flux :
//   1. Parse multipart/form-data → champs texte dans params + fichier sur disque
//   2. Valide MIME et taille du fichier
//   3. Renomme en UUID.ext, écrit sur disque
//   4. INSERT dans la table `uploads` (métadonnées)
//   5. Injecte le stored_as (uuid.ext) dans params[upload_field]
//   6. Exécute le SQL métier avec tous les params (texte + nom fichier)
//
// Si aucun fichier n'est fourni : params[upload_field] = NULL → SQL optionnel

use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use tokio::runtime::Runtime;
use uuid::Uuid;

// ── Runtime Tokio partagé (même pattern que plugin_upload) ───────────────────
static UPLOAD_RT: OnceLock<Runtime> = OnceLock::new();

fn get_runtime() -> &'static Runtime {
    UPLOAD_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("plugin-sql-upload")
            .build()
            .expect("Impossible de créer le runtime du plugin_sql_upload")
    })
}

pub struct PluginSqlUpload;

impl Plugin for PluginSqlUpload {
    fn name(&self) -> &'static str { "sql_upload" }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        // ── Validation configuration ──────────────────────────────────────────
        if ctx.upload_dir.is_empty() {
            return PluginResult::Error(
                "UPLOAD_DIR non configuré dans .env".into()
            );
        }
        if ctx.sql.trim().is_empty() {
            return PluginResult::Error(
                "Champ 'sql' manquant dans config_actions.json".into()
            );
        }

        let allowed: Vec<String> = ctx.allowed_mime
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        let max_bytes: u64 = ctx.max_size_mb
            .parse::<u64>()
            .unwrap_or(10) * 1024 * 1024;

        // ── Extraction boundary multipart ─────────────────────────────────────
        let boundary = match extract_boundary(&ctx.content_type) {
            Some(b) => b,
            None    => return PluginResult::Error(
                "Content-Type multipart/form-data sans boundary".into()
            ),
        };

        let upload_dir   = ctx.upload_dir.clone();
        let upload_field = ctx.upload_field.clone();
        let body_bytes   = ctx.body_bytes.clone();
        let sql          = ctx.sql.clone();
        let mut params   = ctx.params.clone();
        let pool         = state.pool.clone();

        get_runtime().block_on(async move {
            // ── Étape 1 : créer le dossier de destination ─────────────────────
            if let Err(e) = tokio::fs::create_dir_all(&upload_dir).await {
                return PluginResult::Error(
                    format!("Impossible de créer '{}' : {}", upload_dir, e)
                );
            }

            // ── Étape 2 : parser le multipart ────────────────────────────────
            let constraints = multer::Constraints::new()
                .size_limit(multer::SizeLimit::new().whole_stream(max_bytes));

            let mut multipart = multer::Multipart::with_constraints(
                futures_util::stream::once(async move {
                    Ok::<_, std::io::Error>(bytes::Bytes::from(body_bytes))
                }),
                boundary,
                constraints,
            );

            let mut file_stored_as: Option<String> = None;
            let mut file_uuid:      Option<String> = None;
            let mut file_name_orig: Option<String> = None;
            let mut file_mime:      Option<String> = None;
            let mut file_size:      Option<i64>    = None;

            // Parcourir tous les champs du formulaire
            loop {
                match multipart.next_field().await {
                    Err(e)       => return PluginResult::Error(
                        format!("Erreur multipart : {}", e)
                    ),
                    Ok(None)     => break,
                    Ok(Some(field)) => {
                        let field_name = field.name()
                            .unwrap_or("")
                            .to_string();

                        if field.file_name().is_some()
                            && !field.file_name().unwrap_or("").is_empty()
                        {
                            // ── Champ fichier ─────────────────────────────────
                            // On ne traite que le champ désigné par upload_field
                            if field_name != upload_field {
                                continue;
                            }

                            let filename_orig = field.file_name()
                                .unwrap_or("fichier")
                                .to_string();

                            let detected_mime = field
                                .content_type()
                                .map(|m| m.to_string())
                                .unwrap_or_else(|| {
                                    let ext = Path::new(&filename_orig)
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("");
                                    mime_from_ext(ext)
                                });

                            // Validation MIME
                            if !allowed.is_empty()
                                && !allowed.contains(&detected_mime.to_lowercase())
                            {
                                return PluginResult::Error(format!(
                                    "Type MIME '{}' non autorisé. Autorisés : {}",
                                    detected_mime,
                                    allowed.join(", ")
                                ));
                            }

                            // Lecture des bytes
                            let data = match field.bytes().await {
                                Ok(b)  => b,
                                Err(e) => return PluginResult::Error(
                                    format!("Erreur lecture fichier : {}", e)
                                ),
                            };

                            let size = data.len() as i64;

                            // Génération UUID + extension
                            let ext = Path::new(&filename_orig)
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("");
                            let uuid_str  = Uuid::new_v4().to_string();
                            let stored_as = if ext.is_empty() {
                                uuid_str.clone()
                            } else {
                                format!("{}.{}", uuid_str, ext)
                            };

                            // Écriture sur disque
                            let dest = format!("{}/{}", upload_dir, stored_as);
                            if let Err(e) = tokio::fs::write(&dest, &data).await {
                                return PluginResult::Error(
                                    format!("Erreur écriture '{}' : {}", dest, e)
                                );
                            }

                            file_stored_as = Some(stored_as);
                            file_uuid      = Some(uuid_str);
                            file_name_orig = Some(filename_orig);
                            file_mime      = Some(detected_mime);
                            file_size      = Some(size);

                        } else {
                            // ── Champ texte ───────────────────────────────────
                            let value = field.text().await.unwrap_or_default();
                            params.insert(field_name, value);
                        }
                    }
                }
            }

            // ── Étape 3 : INSERT dans uploads (si fichier présent) ────────────
            if let (Some(stored_as), Some(uuid_s), Some(fname), Some(mime), Some(size))
                = (&file_stored_as, &file_uuid, &file_name_orig, &file_mime, &file_size)
            {
                let insert_upload = sqlx::query(
                    "INSERT INTO uploads
                        (uuid, filename, stored_as, mime_type, size_bytes, upload_dir)
                     VALUES (?, ?, ?, ?, ?, ?)"
                )
                .bind(uuid_s)
                .bind(fname)
                .bind(stored_as)
                .bind(mime)
                .bind(size)
                .bind(&upload_dir)
                .execute(&pool)
                .await;

                if let Err(e) = insert_upload {
                    // Fichier écrit mais MySQL KO → on supprime le fichier orphelin
                    let _ = tokio::fs::remove_file(
                        format!("{}/{}", upload_dir, stored_as)
                    ).await;
                    return PluginResult::Error(
                        format!("Erreur INSERT uploads : {}", e)
                    );
                }

                // ── Étape 4 : injecter stored_as dans params ──────────────────
                // Le SQL métier peut utiliser :image, :photo, :avatar, etc.
                params.insert(upload_field.clone(), stored_as.clone());

            } else {
                // Pas de fichier → le champ upload_field reste absent de params
                // → named_to_positional le remplacera par "" (chaîne vide)
                // Pour un NULL SQL, il faut que la colonne accepte NULL
                // et qu'on n'injecte pas de valeur — on insère explicitement None
                params.insert(upload_field.clone(), String::new());
            }

            // ── Étape 5 : exécuter le SQL métier ─────────────────────────────
            let (sql_prepared, param_values) = named_to_positional(&sql, &params);

            let mut query = sqlx::query(&sql_prepared);
            for val in &param_values {
                if val.is_empty() {
                    // Valeur vide → bind NULL pour les colonnes nullable
                    query = query.bind(None::<String>);
                } else {
                    query = query.bind(val);
                }
            }

            match query.execute(&pool).await {
                Ok(result) => PluginResult::Data(json!({
                    "rows_affected":  result.rows_affected(),
                    "last_insert_id": result.last_insert_id(),
                    "file_stored_as": file_stored_as,
                })),
                Err(e) => {
                    // SQL métier KO → supprimer le fichier uploadé si présent
                    if let Some(ref stored) = file_stored_as {
                        let _ = tokio::fs::remove_file(
                            format!("{}/{}", upload_dir, stored)
                        ).await;
                    }
                    PluginResult::Error(format!("Erreur SQL métier : {}", e))
                }
            }
        })
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extrait le boundary du Content-Type multipart/form-data
fn extract_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .map(|s| s.trim())
        .find(|s| s.to_lowercase().starts_with("boundary="))
        .map(|s| s["boundary=".len()..].trim_matches('"').to_string())
}

/// Deviner le type MIME depuis l'extension
fn mime_from_ext(ext: &str) -> String {
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png"          => "image/png",
        "gif"          => "image/gif",
        "webp"         => "image/webp",
        "pdf"          => "application/pdf",
        "txt"          => "text/plain",
        "csv"          => "text/csv",
        "json"         => "application/json",
        "zip"          => "application/zip",
        _              => "application/octet-stream",
    }.to_string()
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
    registrar.register_plugin(Box::new(PluginSqlUpload));
}
