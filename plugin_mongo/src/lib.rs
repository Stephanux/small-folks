use bson::{doc, oid::ObjectId, Document};
use mongodb::Collection;
use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub struct PluginMongo;

impl Plugin for PluginMongo {
    fn name(&self) -> &'static str {
        "mongo"
    }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        // Vérifie que le client MongoDB est disponible
        let mongo_client = match &state.mongo {
            Some(c) => c.clone(),
            None => {
                return PluginResult::Error(
                    "MongoDB non configuré (MONGODB_URI absent du .env)".to_string(),
                )
            }
        };

        // Résolution de la base de données et de la collection
        // Format attendu dans config_actions.json : "mabase.macollection"
        // ou juste "macollection" (utilise la DB par défaut MONGODB_DB)
        let (db_name, coll_name) = parse_collection(&ctx.collection);
        let db   = mongo_client.database(&db_name);
        let coll: Collection<Document> = db.collection(&coll_name);

        // Substitution des paramètres nommés dans le filtre JSON
        // ex: {"region": ":name"} + params{name:"Europe"} → {"region": "Europe"}
        let filter_str = substitute_params(&ctx.filter, &ctx.params);

        tokio::task::block_in_place(|| {
            state.handle.block_on(async move {
                match ctx.operation.as_str() {
                    "find"       => op_find(coll, &filter_str).await,
                    "find_one"   => op_find_one(coll, &filter_str).await,
                    "insert_one" => op_insert_one(coll, &ctx.params).await,
                    "update_one" => op_update_one(coll, &filter_str, &ctx.params).await,
                    "delete_one" => op_delete_one(coll, &filter_str).await,
                    other => PluginResult::Error(
                        format!("Opération MongoDB inconnue : '{}'", other)
                    ),
                }
            })
        })
    }
}

// ── Opérations MongoDB ────────────────────────────────────────────────────────

/// find : retourne tous les documents correspondant au filtre (tableau JSON)
async fn op_find(coll: Collection<Document>, filter_str: &str) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f)  => f,
        Err(e) => return PluginResult::Error(e),
    };
    println!("collection :  {:?}", coll.name());
    match coll.find(filter).await {
        Ok(cursor) => {
            use mongodb::error::Result as MResult;
            use futures_util::stream::TryStreamExt;
            let docs: MResult<Vec<Document>> = cursor.try_collect().await;
            match docs {
                Ok(list) => PluginResult::Data(Value::Array(
                    list.into_iter().map(doc_to_json).collect(),
                )),
                Err(e) => PluginResult::Error(e.to_string()),
            }
        }
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

/// find_one : retourne un seul document (ou null)
async fn op_find_one(coll: Collection<Document>, filter_str: &str) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f)  => f,
        Err(e) => return PluginResult::Error(e),
    };

    match coll.find_one(filter).await {
        Ok(Some(doc)) => PluginResult::Data(doc_to_json(doc)),
        Ok(None)      => PluginResult::Data(Value::Null),
        Err(e)        => PluginResult::Error(e.to_string()),
    }
}

/// insert_one : insère les params comme nouveau document
async fn op_insert_one(
    coll: Collection<Document>,
    params: &HashMap<String, String>,
) -> PluginResult {
    // Construit le document depuis les paramètres (exclut les champs vides)
    let mut doc = Document::new();
    for (k, v) in params {
        if k == "_id" { continue; } // _id auto-généré par MongoDB
        doc.insert(k.clone(), v.clone());
    }

    match coll.insert_one(doc).await {
        Ok(result) => {
            // Convertit l'ObjectId inséré en String
            let inserted_id = result
                .inserted_id
                .as_object_id()
                .map(|oid| oid.to_hex())
                .unwrap_or_else(|| "unknown".to_string());
            PluginResult::Data(json!({
                "inserted_id": inserted_id,
                "success": true
            }))
        }
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

/// update_one : met à jour les champs envoyés dans params (opérateur $set)
async fn op_update_one(
    coll: Collection<Document>,
    filter_str: &str,
    params: &HashMap<String, String>,
) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f)  => f,
        Err(e) => return PluginResult::Error(e),
    };

    // Construit le $set avec tous les params sauf _id
    let mut set_doc = Document::new();
    for (k, v) in params {
        if k == "_id" { continue; }
        set_doc.insert(k.clone(), v.clone());
    }
    let update = doc! { "$set": set_doc };

    match coll.update_one(filter, update).await {
        Ok(result) => PluginResult::Data(json!({
            "matched_count":  result.matched_count,
            "modified_count": result.modified_count,
            "success": result.modified_count > 0
        })),
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

/// delete_one : supprime le premier document correspondant au filtre
async fn op_delete_one(coll: Collection<Document>, filter_str: &str) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f)  => f,
        Err(e) => return PluginResult::Error(e),
    };

    match coll.delete_one(filter).await {
        Ok(result) => PluginResult::Data(json!({
            "deleted_count": result.deleted_count,
            "success": result.deleted_count > 0
        })),
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convertit un Document BSON en serde_json::Value.
/// L'ObjectId est converti en String hexadécimale (plus simple côté client).
fn doc_to_json(doc: Document) -> Value {
    let mut map = Map::new();
    for (k, v) in doc {
        let json_val = bson_to_json(v);
        map.insert(k, json_val);
    }
    Value::Object(map)
}

/// Conversion récursive BSON → JSON.
/// ObjectId → String hex, DateTime → String ISO, les autres types → natifs.
fn bson_to_json(val: bson::Bson) -> Value {
    match val {
        bson::Bson::ObjectId(oid)     => Value::String(oid.to_hex()),
        bson::Bson::String(s)         => Value::String(s),
        bson::Bson::Int32(i)          => json!(i),
        bson::Bson::Int64(i)          => json!(i),
        bson::Bson::Double(f)         => json!(f),
        bson::Bson::Boolean(b)        => Value::Bool(b),
        bson::Bson::Null              => Value::Null,
        bson::Bson::DateTime(dt)      => Value::String(dt.to_string()),
        bson::Bson::Document(subdoc)  => doc_to_json(subdoc),
        bson::Bson::Array(arr)        => {
            Value::Array(arr.into_iter().map(bson_to_json).collect())
        }
        other => Value::String(other.to_string()),
    }
}

/// Parse une chaîne JSON en Document BSON.
/// Gère la conversion spéciale {"_id": "<hex>"} → {"_id": ObjectId("<hex>")}
fn parse_filter(filter_str: &str) -> Result<Document, String> {
    if filter_str.trim().is_empty() || filter_str.trim() == "{}" {
        return Ok(Document::new());
    }

    // Parse d'abord en serde_json::Value pour manipuler facilement
    let mut val: Value = serde_json::from_str(filter_str)
        .map_err(|e| format!("Filtre JSON invalide '{}' : {}", filter_str, e))?;

    // Conversion spéciale : si _id est une String hex valide → ObjectId
    if let Value::Object(ref mut map) = val {
        if let Some(Value::String(id_str)) = map.get("_id") {
            if let Ok(oid) = ObjectId::parse_str(id_str) {
                map.insert("_id".to_string(), Value::String(oid.to_hex()));
                // On reconstruit via bson directement
                let bson_doc = doc! { "_id": oid };
                // Merge les autres champs
                let other_fields: Document = map
                    .iter()
                    .filter(|(k, _)| k.as_str() != "_id")
                    .fold(Document::new(), |mut d, (k, v)| {
                        if let Ok(bv) = bson::to_bson(v) {
                            d.insert(k.clone(), bv);
                        }
                        d
                    });
                let mut result = bson_doc;
                result.extend(other_fields);
                return Ok(result);
            }
        }
    }

    // Conversion générique JSON → BSON
    bson::to_document(&val)
        .map_err(|e| format!("Conversion BSON échouée : {}", e))
}

/// Substitue les paramètres nommés dans le filtre.
/// ex: {"region": ":name"} + {name: "Europe"} → {"region": "Europe"}
fn substitute_params(filter: &str, params: &HashMap<String, String>) -> String {
    let re = regex::Regex::new(r#"":([a-zA-Z_][a-zA-Z0-9_]*)""#).unwrap();
    re.replace_all(filter, |caps: &regex::Captures| {
        let name = &caps[1];
        match params.get(name) {
            Some(val) => format!("\"{}\"", val.replace('"', "\\\"")),
            None      => "null".to_string(),
        }
    }).to_string()
}

/// Parse "db.collection" ou "collection" (utilise MONGODB_DB par défaut)
fn parse_collection(s: &str) -> (String, String) {
    match s.split_once('.') {
        Some((db, coll)) => (db.to_string(), coll.to_string()),
        None => {
            let db = std::env::var("MONGODB_DB")
                .unwrap_or_else(|_| "test".to_string());
            (db, s.to_string())
        }
    }
}

#[no_mangle]
pub fn plugin_entry(registrar: &mut dyn PluginRegistrar) {
    registrar.register_plugin(Box::new(PluginMongo));
}
