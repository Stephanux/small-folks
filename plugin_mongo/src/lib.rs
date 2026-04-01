use bson::{doc, oid::ObjectId, raw::RawDocumentBuf, Document};
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
        
        // ── Lecture : Collection<RawDocumentBuf> ──────────────────────────────
        // Le driver reçoit les bytes BSON du réseau et les stocke directement
        // dans un Vec<u8> SANS parser les paires clé/valeur.
        // Le parsing est LAZY : seuls les champs parcourus dans raw_doc_to_json
        // sont désérialisés. Gain ~30% sur des collections volumineuses.
        let coll_raw: Collection<RawDocumentBuf> = db.collection(&coll_name);

        // ── Écriture : Collection<Document> ──────────────────────────────────
        // Pour insert/update/delete on construit des filtres BSON typés.
        // RawDocumentBuf n'apporte rien ici (pas de curseur à lire).
        let coll: Collection<Document> = db.collection(&coll_name);

        // Substitution des paramètres nommés dans le filtre JSON
        // ex: {"region": ":name"} + params{name:"Europe"} → {"region": "Europe"}
        let filter_str = substitute_params(&ctx.filter, &ctx.params);

        tokio::task::block_in_place(|| {
            state.handle.block_on(async move {
                match ctx.operation.as_str() {
                    "find"       => op_find(coll_raw, &filter_str).await,
                    "find_one"   => op_find_one(coll_raw, &filter_str).await,
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

// ── Opérations LECTURE — Collection<RawDocumentBuf> ──────────────────────────

/// find : retourne un tableau JSON.
/// Chemin zero-copy : réseau → Vec<u8> → itération lazy → serde_json::Value
/// Aucune HashMap<String,Bson> intermédiaire contrairement à Document.
async fn op_find(
    coll: Collection<RawDocumentBuf>,
    filter_str: &str,
) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f) => f, Err(e) => return PluginResult::Error(e),
    };
    use futures_util::stream::TryStreamExt;
    match coll.find(filter).await {
        Ok(cursor) => match cursor.try_collect::<Vec<RawDocumentBuf>>().await {
            Ok(docs) => PluginResult::Data(Value::Array(
                docs.iter().map(raw_doc_to_json).collect()
            )),
            Err(e) => PluginResult::Error(e.to_string()),
        },
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

/// find_one : retourne un document unique (JSON) ou null.
async fn op_find_one(
    coll: Collection<RawDocumentBuf>,
    filter_str: &str,
) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f) => f, Err(e) => return PluginResult::Error(e),
    };
    match coll.find_one(filter).await {
        Ok(Some(raw)) => PluginResult::Data(raw_doc_to_json(&raw)),
        Ok(None)      => PluginResult::Data(Value::Null),
        Err(e)        => PluginResult::Error(e.to_string()),
    }
}

// ── Opérations ÉCRITURE — Collection<Document> ───────────────────────────────

async fn op_insert_one(
    coll: Collection<Document>,
    params: &HashMap<String, String>,
) -> PluginResult {
    let mut doc = Document::new();
    for (k, v) in params {
        if k == "_id" { continue; }
        doc.insert(k.clone(), v.clone());
    }
    match coll.insert_one(doc).await {
        Ok(r) => PluginResult::Data(json!({
            "inserted_id": r.inserted_id.as_object_id()
                            .map(|o| o.to_hex())
                            .unwrap_or_else(|| "unknown".to_string()),
            "success": true
        })),
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

async fn op_update_one(
    coll: Collection<Document>,
    filter_str: &str,
    params: &HashMap<String, String>,
) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f) => f, Err(e) => return PluginResult::Error(e),
    };
    let mut set_doc = Document::new();
    for (k, v) in params {
        if k == "_id" { continue; }
        set_doc.insert(k.clone(), v.clone());
    }
    match coll.update_one(filter, doc! { "$set": set_doc }).await {
        Ok(r) => PluginResult::Data(json!({
            "matched_count":  r.matched_count,
            "modified_count": r.modified_count,
            "success":        r.modified_count > 0
        })),
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

async fn op_delete_one(
    coll: Collection<Document>,
    filter_str: &str,
) -> PluginResult {
    let filter = match parse_filter(filter_str) {
        Ok(f) => f, Err(e) => return PluginResult::Error(e),
    };
    match coll.delete_one(filter).await {
        Ok(r) => PluginResult::Data(json!({
            "deleted_count": r.deleted_count,
            "success":       r.deleted_count > 0
        })),
        Err(e) => PluginResult::Error(e.to_string()),
    }
}

// ── Conversion zero-copy RawDocumentBuf → serde_json::Value ──────────────────

/// Convertit un RawDocumentBuf en JSON par itération lazy.
///
/// Principe zero-copy :
///   RawDocumentBuf = Vec<u8> de bytes BSON bruts (pas de HashMap).
///   raw.iter() produit des RawBsonRef<'_> : références directes sur ces bytes.
///   Les strings BSON sont empruntées (&str) et ne sont copiées qu'une fois
///   au moment de l'insertion dans la Map JSON.
///   Les scalaires (int, float, bool) sont lus depuis les bytes sans allocation.
fn raw_doc_to_json(raw: &RawDocumentBuf) -> Value {
    let mut map = Map::new();
    for item in raw.iter() {
        match item {
            Ok((key, val)) => { map.insert(key.to_string(), raw_bson_ref_to_json(val)); }
            Err(e)         => { eprintln!("[plugin_mongo] BSON parse error: {}", e); }
        }
    }
    Value::Object(map)
}

/// Convertit un RawBsonRef<'_> (vue sur les bytes) en serde_json::Value.
///
/// RawBsonRef est une référence empruntée sur le buffer RawDocumentBuf :
///   - String  → &str (zero-copy), converti en String seulement ici
///   - Int32/64, Double, Boolean → lecture directe depuis les bytes
///   - ObjectId → String hex (seule "vraie" allocation nécessaire pour JSON)
///   - Document/Array imbriqués → récursion avec vues empruntées
fn raw_bson_ref_to_json(val: bson::raw::RawBsonRef<'_>) -> Value {
    use bson::raw::RawBsonRef;
    match val {
        RawBsonRef::ObjectId(oid)     => Value::String(oid.to_hex()),
        RawBsonRef::String(s)         => Value::String(s.to_string()),
        RawBsonRef::Int32(i)          => json!(i),
        RawBsonRef::Int64(i)          => json!(i),
        RawBsonRef::Double(f)         => json!(f),
        RawBsonRef::Boolean(b)        => Value::Bool(b),
        RawBsonRef::Null              => Value::Null,
        RawBsonRef::Undefined         => Value::Null,
        RawBsonRef::DateTime(dt)      => Value::String(dt.to_string()),
        RawBsonRef::Timestamp(ts)     => json!({ "t": ts.time, "i": ts.increment }),
        RawBsonRef::Decimal128(d)     => Value::String(d.to_string()),
        RawBsonRef::Document(subdoc)  => {
            let mut map = Map::new();
            for item in subdoc.iter() {
                if let Ok((k, v)) = item {
                    map.insert(k.to_string(), raw_bson_ref_to_json(v));
                }
            }
            Value::Object(map)
        }
        RawBsonRef::Array(arr) => Value::Array(
            arr.into_iter()
               .filter_map(|r| r.ok())
               .map(raw_bson_ref_to_json)
               .collect()
        ),
        other => Value::String(format!("{:?}", other)),
    }
}

// ── Helpers communs ───────────────────────────────────────────────────────────

/// Parse une chaîne JSON en Document BSON pour les filtres.
/// Conversion spéciale : {"_id": "<hex>"} → {"_id": ObjectId("<hex>")}
fn parse_filter(filter_str: &str) -> Result<Document, String> {
    if filter_str.trim().is_empty() || filter_str.trim() == "{}" {
        return Ok(Document::new());
    }
    let mut val: Value = serde_json::from_str(filter_str)
        .map_err(|e| format!("Filtre JSON invalide '{}' : {}", filter_str, e))?;

    if let Value::Object(ref mut map) = val {
        if let Some(Value::String(id_str)) = map.get("_id") {
            if let Ok(oid) = ObjectId::parse_str(id_str) {
                let mut result = doc! { "_id": oid };
                for (k, v) in map.iter().filter(|(k, _)| k.as_str() != "_id") {
                    if let Ok(bv) = bson::to_bson(v) { result.insert(k.clone(), bv); }
                }
                return Ok(result);
            }
        }
    }
    bson::to_document(&val)
        .map_err(|e| format!("Conversion BSON échouée : {}", e))
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
/*fn parse_filter(filter_str: &str) -> Result<Document, String> {
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
}*/

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
