# Small-Folks — Framework MVC Rust piloté par configuration

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="Logo Rust Framework small-Folks" />
</p>

Framework web Rust basé sur **Tide** avec un système de plugins dynamiques (`cdylib`),
des routes et requêtes SQL/MongoDB entièrement configurées dans `config_actions.json`
sans recompilation.

## Stack technique

- **Rust** (rustc 1.94.0 [4a4ef493e 2026-03-02])
- **Tide 0.16** — serveur web async
- **Tokio 1** — runtime async (multi-thread)
- **sqlx 0.7** — accès MySQL avec requêtes préparées
- **mongodb 3.5** — driver MongoDB avec RawDocumentBuf (zero-copy)
- **Handlebars 6.4** — templates HTML avec partials
- **libloading 0.8** — chargement dynamique des plugins `.so`

## Architecture du workspace

```
small-folks/
├── README.md                      ← ce fichier
├── config_actions.json            ← annuaire des routes (SQL + MongoDB + upload)
├── .env                           ← DATABASE_URL, MONGODB_URI, UPLOAD_DIR, ...
├── templates/                     ← vues Handlebars
│   ├── partials/                  ← header.hbs, nav.hbs, footer.hbs
│   ├── tableGeneric.hbs           ← tableau générique avec DataTables.js
│   ├── listeGeneric.hbs           ← liste générique <select>
│   ├── upload_form.hbs            ← formulaire upload
│   └── upload_list.hbs            ← liste des fichiers uploadés
├── plugins/                       ← plugins du Framework
│   ├── plugin-core/src/lib.rs         ← traits et types partagés entre tous les plugins
│   ├── plugin_sql/src/lib.rs          ← plugin MySQL générique (SELECT/INSERT/UPDATE/DELETE)
│   ├── plugin_mongo/src/lib.rs        ← plugin MongoDB générique (find/insert/update/delete)
│   ├── plugin_upload/src/lib.rs       ← plugin upload multipart → disque + MySQL
├── src/main.rs                    ← démarrage serveur, pool MySQL, client MongoDB, précache plugins
├── src/dispatcher.rs              ← résolution routes, extraction params, rendu html/json/redirect
└── sql/
    ├── create_countries.sql
    └── create_uploads.sql
```

## Flux d'une requête HTTP

![alt Schéma architecture](https://mascaron.net/schema_architecture_Rust_Small-FolksV4.png)

```
Client HTTP
  ↓ GET /mongo/countries
Tide (main.rs) — catch-all /*
  ↓
dispatcher.rs
  ├─ resolve_action("GET", "/mongo/countries") → config_actions.json
  ├─ extrait params URL / query string / body
  ├─ construit ActionContext { sql, collection, filter, operation, params, ... }
  ↓
plugin_mongo.execute(ctx, state)      ← synchrone (contrainte FFI)
  ├─ get_mongo_ctx() → MongoContext { rt: MONGO_RT, client }
  ├─ block_in_place(|| MONGO_RT.block_on(op_find(...)))
  └─ PluginResult::Data(json)
  ↓
dispatcher : return_type = "html" → hbs.render("tableGeneric", data)
  ↓
HTTP 200 text/html
```

## config_actions.json — structure d'une action

```json
"GET/countries/:code": {
    "plugin":      "./target/release/libplugin_sql.so",
    "sql":         "SELECT code, name_us FROM countries WHERE code = :code",
    "view":        "tableGeneric.hbs",
    "return_type": "html"
}
```

| Champ          | Valeurs                                  | Description                          |
|----------------|------------------------------------------|--------------------------------------|
| `plugin`       | chemin vers `.so`                        | Plugin à utiliser                    |
| `sql`          | requête SQL avec `:param`                | Paramètres nommés pour plugin_sql    |
| `collection`   | `"madb.macollection"` ou `"collection"`  | Collection MongoDB                   |
| `filter`       | `"{\"region\": \":name\"}"`              | Filtre BSON avec `:param`            |
| `operation`    | `find` `find_one` `insert_one` ...       | Opération MongoDB                    |
| `allowed_mime` | `"image/jpeg,image/png,application/pdf"` | Types MIME autorisés (upload)        |
| `max_size_mb`  | `"10"`                                   | Taille max en Mo (upload)            |
| `view`         | nom du template `.hbs`                   | Ignoré si return_type ≠ html         |
| `return_type`  | `html` `json` `redirect`                 | Mode de réponse                      |
| `redirect_to`  | URL                                      | Destination si redirect              |

## plugin-core — types partagés

```rust
pub struct AppState {
    pub pool:   MySqlPool,      // pool MySQL partagé
    pub handle: Handle,         // handle runtime Tokio principal
    pub mongo:  Option<Client>, // client MongoDB (sert de flag "MongoDB activé")
}

pub struct ActionContext {
    pub sql:          String,              // requête SQL avec :param
    pub collection:   String,              // collection MongoDB
    pub filter:       String,              // filtre BSON JSON avec :param
    pub operation:    String,              // find | find_one | insert_one | ...
    pub upload_dir:   String,              // dossier destination upload
    pub allowed_mime: String,              // types MIME autorisés
    pub max_size_mb:  String,              // taille max en Mo
    pub params:       HashMap<String, String>, // params URL + query + body
    pub view:         String,              // nom template Handlebars
    pub return_type:  String,              // html | json | redirect
    pub redirect_to:  Option<String>,      // URL de redirect
    pub body_bytes:   Vec<u8>,             // body brut (multipart)
    pub content_type: String,              // Content-Type complet de la requête
}

// Trait FFI — SYNCHRONE obligatoire (async_trait interdit sur cdylib)
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult;
}
```

## Règles critiques — frontière FFI cdylib

### execute() doit être SYNCHRONE
`async fn` dans un trait FFI (`cdylib`) provoque un coredump en release.
`async_trait` génère `Pin<Box<dyn Future>>` qui traverse la frontière FFI → vtables instables.

### plugin_sql — pattern block_in_place + handle.block_on
```rust
tokio::task::block_in_place(|| {
    state.handle.block_on(async {
        sqlx::query(...).fetch_all(&state.pool).await
    })
})
```
sqlx a besoin du runtime principal (handle) car il utilise fetch_all() qui ne fait qu'un seul aller-retour réseau → compatible avec block_on.

### plugin_mongo — MongoContext autonome (CRITIQUE)
```rust
struct MongoContext { rt: Runtime, client: mongodb::Client }
static MONGO_CTX: OnceLock<MongoContext> = OnceLock::new();
```
**Le client MongoDB DOIT être créé dans MONGO_RT, pas dans main.rs.**

Raison : le driver MongoDB lance des tâches de fond (heartbeat, surveillance pool).
Si le client est créé dans le runtime principal, ces tâches tournent sur ses threads.
Quand plugin_sql appelle block_in_place, il affame ces tâches → heartbeat manqué
→ pool dégradé → reconnexion → latence de 1-8s sur la requête suivante.

En créant le client dans MONGO_RT, block_in_place du runtime principal n'a
aucun impact sur MongoDB. Les deux runtimes sont totalement isolés.

```rust
// Utilisation dans execute()
tokio::task::block_in_place(|| {
    get_mongo_ctx().rt.block_on(async move {
        // opérations MongoDB — s'exécutent dans MONGO_RT
    })
})
```

### Règles SQL pour le plugin_sql générique
- Toutes les colonnes de SELECT doivent être de type `CHAR`/`VARCHAR` ou castées
- `CAST(COUNT(*) AS CHAR)` pour les agrégats
- `CAST(created_at AS CHAR)` pour les dates
- Sinon `try_get(i)` retourne `None` → `Null` dans le JSON

### Templates Handlebars Rust
- `{{this}}` et non `{{.}}` (alias JS non supporté)
- `{{#each this.0}}` / `{{@key}}` pour les en-têtes de colonnes génériques
- Champs optionnels : `{{#if (lookup @root "nav_extra")}}` (évite l'erreur "Cannot access array with string index")
- Partials : `{{> partials/header page_title="Titre"}}`

## Variables .env

```bash
DATABASE_URL=mysql://user:pass@localhost:3306/mabase
HOST=127.0.0.1
PORT=8080
CONFIG_ACTIONS=./config_actions.json
TEMPLATES_DIR=./templates

# MongoDB
MONGODB_URI=mongodb://localhost:27017
MONGODB_DB=mabase
MONGODB_USER=admin
MONGODB_PASS=monpass
MONGODB_AUTH_DB=admin

# Upload
UPLOAD_DIR=./uploads
```

## Compilation et lancement

```bash
# Compilation release (obligatoire pour les performances)
cargo build --release --all

# Lancement — vérifier que config_actions.json pointe vers target/release/
./target/release/rust-plugin-tide
```

**Important** : `config_actions.json` le plugin doit pointer vers `./target/release/libplugin_xxx.so`.
Mélanger binaire release et `.so` debug (ou inversement) provoque un coredump.

## Performance observée (release, localhost)

| Plugin      | Opération           | Latence  |
|-------------|---------------------|----------|
| plugin_sql  | SELECT 243 lignes   | 1-5ms    |
| plugin_mongo| find 243 documents  | 3-5ms    |
| plugin_upload| upload 1 fichier   | < 10ms   |

## Table MySQL uploads

```sql
CREATE TABLE IF NOT EXISTS uploads (
    id          INT AUTO_INCREMENT PRIMARY KEY,
    uuid        CHAR(36)     NOT NULL UNIQUE,
    filename    VARCHAR(255) NOT NULL,
    stored_as   VARCHAR(255) NOT NULL,
    mime_type   VARCHAR(100) NOT NULL,
    size_bytes  BIGINT       NOT NULL,
    upload_dir  VARCHAR(255) NOT NULL,
    created_at  DATETIME     DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
```
<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v2_50pc.png" alt="Logo Rust Framework small-Folks" />
</p>
