use bson::{doc, oid::ObjectId, raw::RawDocumentBuf, Document};
use mongodb::{options::{AuthMechanism, ClientOptions, Credential}, Collection};
use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

// ── Contexte MongoDB autonome — créé une seule fois au premier appel ──────────
//
// ARCHITECTURE CORRECTE :
//   Le client MongoDB est créé DANS le MONGO_RT, pas dans le runtime principal.
//   Toutes les tâches de fond du driver (heartbeat, pool de connexions,
//   surveillance de topologie) s'exécutent sur les threads de MONGO_RT.
//
// POURQUOI c'est critique pour les performances :
//   Si le client est créé dans main.rs (runtime principal), ses tâches de fond
//   tournent sur les threads principaux. Quand plugin_sql appelle block_in_place,
//   il bloque temporairement un thread principal → les tâches MongoDB sont
//   affamées → le heartbeat manque un battement → le pool se dégrade →
//   reconnexion forcée → latence de 1-8s sur la requête suivante.
//
//   En créant le client DANS MONGO_RT, block_in_place sur le runtime principal
//   n'a AUCUN impact sur les tâches de fond MongoDB. Les deux runtimes sont
//   complètement isolés.
struct MongoContext {
    rt:     Runtime,
    client: mongodb::Client,
}

static MONGO_CTX: OnceLock<MongoContext> = OnceLock::new();

fn get_mongo_ctx() -> &'static MongoContext {
    MONGO_CTX.get_or_init(|| {
        // 1. Créer le runtime dédié MongoDB
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("plugin-mongo")
            .build()
            .expect("Impossible de créer le runtime Tokio du plugin MongoDB");

        // 2. Créer le client DANS ce runtime
        //    → les tâches de fond du driver s'ancrent sur MONGO_RT
        let client = rt.block_on(async {
            let uri = std::env::var("MONGODB_URI")
                .expect("[plugin_mongo] MONGODB_URI absent du .env");

            let mut opts = ClientOptions::parse(&uri).await
                .expect("[plugin_mongo] URI MongoDB invalide");

            // Authentification SCRAM-SHA-256 si les credentials sont définis
            if let Ok(user) = std::env::var("MONGODB_USER") {
                let pass    = std::env::var("MONGODB_PASS").unwrap_or_default();
                let auth_db = std::env::var("MONGODB_AUTH_DB")
                    .unwrap_or_else(|_| "admin".to_string());
                opts.credential = Some(
                    Credential::builder()
                        .username(user)
                        .password(pass)
                        .source(auth_db)
                        .mechanism(AuthMechanism::ScramSha256)
                        .build()
                );
            }

            mongodb::Client::with_options(opts)
                .expect("[plugin_mongo] Impossible de créer le client MongoDB")
        });

        eprintln!("[plugin_mongo] Client MongoDB initialisé dans MONGO_RT");
        MongoContext { rt, client }
    })
}

pub struct PluginMongo;

impl Plugin for PluginMongo {
    fn name(&self) -> &'static str { "mongo" }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        // Vérifie que MongoDB est activé (MONGODB_URI présent dans .env)
        if state.mongo.is_none() {
            return PluginResult::Error(
                "MongoDB non configuré (MONGODB_URI absent du .env)".into()
            );
        }

        // Utilise le contexte autonome (client créé dans MONGO_RT)
        let mongo_ctx  = get_mongo_ctx();

        let (db_name, coll_name) = parse_collection(&ctx.collection);
        let db       = mongo_ctx.client.database(&db_name);
        let coll_raw = db.collection::<RawDocumentBuf>(&coll_name);
        let coll_doc = db.collection::<Document>(&coll_name);

        let filter_str = substitute_params(&ctx.filter, &ctx.params);
        let params     = ctx.params.clone();
        let operation  = ctx.operation.clone();

        // block_in_place : notifie le runtime principal que ce thread va bloquer
        // MONGO_RT.block_on : s'exécute dans un contexte totalement indépendant
        // → aucune interférence possible avec plugin_sql ou d'autres plugins
        tokio::task::block_in_place(|| {
            mongo_ctx.rt.block_on(async move {
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
    let mut val: serde_json::Value = serde_json::from_str(filter_str)
        .map_err(|e| format!("Filtre JSON invalide '{}' : {}", filter_str, e))?;

    if let serde_json::Value::Object(ref mut map) = val {
        if let Some(serde_json::Value::String(id_str)) = map.get("_id") {
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
