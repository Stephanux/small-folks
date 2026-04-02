use bson::{doc, oid::ObjectId, raw::RawDocumentBuf, Document};
use mongodb::Collection;
use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use serde_json::{json, Map, Value};
use tokio::runtime::Runtime;
use std::collections::HashMap;
use std::sync::{OnceLock};

// ── Runtime Tokio partagé — créé une seule fois au chargement du plugin ──────
// OnceLock garantit l'initialisation thread-safe sans mutex à chaque appel.
// Le runtime persiste pour toute la durée de vie du plugin (durée du processus).
// Les connexions MongoDB du pool sont ainsi réutilisées entre les requêtes.
static MONGO_RT: OnceLock<Runtime> = OnceLock::new();

fn get_runtime() -> &'static Runtime {
    MONGO_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)          // 2 threads suffisent pour les I/O MongoDB
            .enable_all()
            .thread_name("plugin-mongo")
            .build()
            .expect("Impossible de créer le runtime Tokio du plugin MongoDB")
    })
}


pub struct PluginMongo;

impl Plugin for PluginMongo {
    fn name(&self) -> &'static str { "mongo" }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        let mongo_client = match &state.mongo {
            Some(c) => c.clone(),
            None    => return PluginResult::Error(
                "MongoDB non configuré (MONGODB_URI absent du .env)".into()
            ),
        };

        let (db_name, coll_name) = parse_collection(&ctx.collection);
        let db       = mongo_client.database(&db_name);
        let coll_raw = db.collection::<RawDocumentBuf>(&coll_name);
        let coll_doc = db.collection::<Document>(&coll_name);

        let filter_str = substitute_params(&ctx.filter, &ctx.params);
        let params     = ctx.params.clone();
        let operation  = ctx.operation.clone();

        // Récupère le runtime partagé (déjà initialisé, zéro overhead)
        let rt = get_runtime();

        // block_on sur le runtime PARTAGÉ :
        // - Le pool MongoDB reste chaud entre les requêtes
        // - Pas de création/destruction de runtime à chaque appel
        // - Les connexions sont réutilisées → latence réduite
        rt.block_on(async move {
            match operation.as_str() {
                "find"       => op_find(coll_raw, &filter_str).await,
                "find_one"   => op_find_one(coll_raw, &filter_str).await,
                "insert_one" => op_insert_one(coll_doc, &params).await,
                "update_one" => op_update_one(coll_doc, &filter_str, &params).await,
                "delete_one" => op_delete_one(coll_doc, &filter_str).await,
                other        => PluginResult::Error(
                    format!("Opération MongoDB inconnue : '{}'", other)
                ),
            }
        })
    }
}

// ── Opérations LECTURE — Collection<RawDocumentBuf> ──────────────────────────

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
                            .unwrap_or_else(|| "unknown".into()),
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
        if k != "_id" { set_doc.insert(k.clone(), v.clone()); }
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

fn raw_bson_ref_to_json(val: bson::raw::RawBsonRef<'_>) -> Value {
    use bson::raw::RawBsonRef;
    match val {
        RawBsonRef::ObjectId(oid)    => Value::String(oid.to_hex()),
        RawBsonRef::String(s)        => Value::String(s.to_string()),
        RawBsonRef::Int32(i)         => json!(i),
        RawBsonRef::Int64(i)         => json!(i),
        RawBsonRef::Double(f)        => json!(f),
        RawBsonRef::Boolean(b)       => Value::Bool(b),
        RawBsonRef::Null             => Value::Null,
        RawBsonRef::Undefined        => Value::Null,
        RawBsonRef::DateTime(dt)     => Value::String(dt.to_string()),
        RawBsonRef::Timestamp(ts)    => json!({ "t": ts.time, "i": ts.increment }),
        RawBsonRef::Decimal128(d)    => Value::String(d.to_string()),
        RawBsonRef::Document(subdoc) => {
            let mut map = Map::new();
            for item in subdoc.iter() {
                if let Ok((k, v)) = item {
                    map.insert(k.to_string(), raw_bson_ref_to_json(v));
                }
            }
            Value::Object(map)
        }
        RawBsonRef::Array(arr) => Value::Array(
            arr.into_iter().filter_map(|r| r.ok()).map(raw_bson_ref_to_json).collect()
        ),
        other => Value::String(format!("{:?}", other)),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

fn substitute_params(filter: &str, params: &HashMap<String, String>) -> String {
    let re = regex::Regex::new(r#"":([a-zA-Z_][a-zA-Z0-9_]*)""#).unwrap();
    re.replace_all(filter, |caps: &regex::Captures| {
        let name = &caps[1];
        match params.get(name) {
            Some(v) => format!("\"{}\"", v.replace('"', "\\\"")),
            None    => "null".to_string(),
        }
    }).to_string()
}

fn parse_collection(s: &str) -> (String, String) {
    match s.split_once('.') {
        Some((db, coll)) => (db.to_string(), coll.to_string()),
        None => (
            std::env::var("MONGODB_DB").unwrap_or_else(|_| "test".to_string()),
            s.to_string()
        ),
    }
}

#[no_mangle]
pub fn plugin_entry(registrar: &mut dyn PluginRegistrar) {
    registrar.register_plugin(Box::new(PluginMongo));
}