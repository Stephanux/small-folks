use multer::bytes;
use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use serde_json::json;
use std::path::Path;
use std::sync::OnceLock;
use tokio::runtime::Runtime;
use uuid::Uuid;

// ── Runtime Tokio partagé (même pattern que plugin_mongo) ─────────────────────
static UPLOAD_RT: OnceLock<Runtime> = OnceLock::new();

fn get_runtime() -> &'static Runtime {
    UPLOAD_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("plugin-upload")
            .build()
            .expect("Impossible de créer le runtime Tokio du plugin upload")
    })
}

pub struct PluginUpload;

impl Plugin for PluginUpload {
    fn name(&self) -> &'static str { "upload" }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        // ── 1. Validation de la configuration ─────────────────────────────────
        if ctx.upload_dir.is_empty() {
            return PluginResult::Error(
                "UPLOAD_DIR non configuré dans .env".into()
            );
        }

        // Types MIME autorisés (liste depuis config_actions.json)
        let allowed: Vec<String> = ctx.allowed_mime
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        // Taille max en octets (défaut : 10 Mo)
        let max_bytes: u64 = ctx.max_size_mb
            .parse::<u64>()
            .unwrap_or(10)
            * 1024 * 1024;

        // ── 2. Extraction du boundary multipart depuis le Content-Type ─────────
        // Content-Type: multipart/form-data; boundary=----WebKitFormBoundary...
        let boundary = match extract_boundary(&ctx.content_type) {
            Some(b) => b,
            None    => return PluginResult::Error(
                "Content-Type multipart/form-data sans boundary".into()
            ),
        };

        // ── 3. Parse multipart + écriture sur disque + insertion MySQL ─────────
        let upload_dir  = ctx.upload_dir.clone();
        let body_bytes  = ctx.body_bytes.clone();
        let pool        = state.pool.clone();

        get_runtime().block_on(async move {
            process_upload(
                body_bytes,
                boundary,
                upload_dir,
                allowed,
                max_bytes,
                pool,
            ).await
        })
    }
}

// ── Traitement principal ──────────────────────────────────────────────────────

async fn process_upload(
    body_bytes: Vec<u8>,
    boundary:   String,
    upload_dir: String,
    allowed:    Vec<String>,
    max_bytes:  u64,
    pool:       sqlx::MySqlPool,
) -> PluginResult {

    // Crée le dossier de destination s'il n'existe pas
    if let Err(e) = tokio::fs::create_dir_all(&upload_dir).await {
        return PluginResult::Error(
            format!("Impossible de créer le dossier '{}' : {}", upload_dir, e)
        );
    }

    // Configure le parser multer avec la taille max
    let constraints = multer::Constraints::new()
        .allowed_fields(vec!["file"])
        .size_limit(multer::SizeLimit::new().whole_stream(max_bytes));

    let mut multipart = multer::Multipart::with_constraints(
        futures_util::stream::once(async move {
            Ok::<_, std::io::Error>(bytes::Bytes::from(body_bytes))
        }),
        boundary,
        constraints,
    );

    let mut results: Vec<serde_json::Value> = Vec::new();

    // Parcourt tous les champs du formulaire multipart
    loop {
        match multipart.next_field().await {
            Err(e)       => return PluginResult::Error(format!("Erreur multipart : {}", e)),
            Ok(None)     => break, // plus de champs
            Ok(Some(field)) => {
                // On ne traite que les champs qui ont un nom de fichier
                let filename_orig = match field.file_name() {
                    Some(f) if !f.is_empty() => f.to_string(),
                    _                        => continue, // champ texte, pas un fichier
                };

                // Détection du type MIME
                let mime_type = field
                    .content_type()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| {
                        // Fallback : deviner depuis l'extension
                        let ext = Path::new(&filename_orig)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("");
                        mime_from_ext(ext)
                    });

                // Validation du type MIME
                if !allowed.is_empty() && !allowed.contains(&mime_type.to_lowercase()) {
                    return PluginResult::Error(format!(
                        "Type MIME '{}' non autorisé. Autorisés : {}",
                        mime_type,
                        allowed.join(", ")
                    ));
                }

                // Lecture des bytes du fichier
                let data = match field.bytes().await {
                    Ok(b)  => b,
                    Err(e) => return PluginResult::Error(
                        format!("Erreur lecture fichier '{}' : {}", filename_orig, e)
                    ),
                };

                let size_bytes = data.len() as i64;

                // Génération du nom UUID + extension originale
                let ext = Path::new(&filename_orig)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                let uuid      = Uuid::new_v4().to_string();
                let stored_as = if ext.is_empty() {
                    uuid.clone()
                } else {
                    format!("{}.{}", uuid, ext)
                };

                // Écriture sur disque
                let dest_path = format!("{}/{}", upload_dir, stored_as);
                if let Err(e) = tokio::fs::write(&dest_path, &data).await {
                    return PluginResult::Error(
                        format!("Erreur écriture '{}' : {}", dest_path, e)
                    );
                }

                // Insertion des métadonnées en MySQL
                let insert_result = sqlx::query(
                    "INSERT INTO uploads
                        (uuid, filename, stored_as, mime_type, size_bytes, upload_dir)
                     VALUES (?, ?, ?, ?, ?, ?)"
                )
                .bind(&uuid)
                .bind(&filename_orig)
                .bind(&stored_as)
                .bind(&mime_type)
                .bind(size_bytes)
                .bind(&upload_dir)
                .execute(&pool)
                .await;

                match insert_result {
                    Ok(r) => {
                        results.push(json!({
                            "id":         r.last_insert_id(),
                            "uuid":       uuid,
                            "filename":   filename_orig,
                            "stored_as":  stored_as,
                            "mime_type":  mime_type,
                            "size_bytes": size_bytes,
                            "upload_dir": upload_dir,
                        }));
                    }
                    Err(e) => {
                        // Fichier écrit mais MySQL a échoué → on supprime le fichier
                        let _ = tokio::fs::remove_file(&dest_path).await;
                        return PluginResult::Error(
                            format!("Erreur MySQL lors de l'insertion : {}", e)
                        );
                    }
                }
            }
        }
    }

    if results.is_empty() {
        return PluginResult::Error(
            "Aucun fichier trouvé dans la requête multipart".into()
        );
    }

    // Retourne le tableau des fichiers uploadés
    // Le dispatcher gèrera le redirect ou le rendu JSON selon return_type
    PluginResult::Data(serde_json::Value::Array(results))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extrait le boundary depuis le Content-Type.
/// "multipart/form-data; boundary=----WebKitFormBoundary7MA4YWxkTrZu0gW"
/// →  "----WebKitFormBoundary7MA4YWxkTrZu0gW"
fn extract_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .map(|s| s.trim())
        .find(|s| s.to_lowercase().starts_with("boundary="))
        .map(|s| s["boundary=".len()..].trim_matches('"').to_string())
}

/// Deviner le type MIME depuis l'extension quand le navigateur ne l'envoie pas.
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

#[no_mangle]
pub fn plugin_entry(registrar: &mut dyn PluginRegistrar) {
    registrar.register_plugin(Box::new(PluginUpload));
}
