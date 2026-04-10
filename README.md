# small-folks — Framework MVC Rust piloté par configuration

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="Logo Rust Framework small-Folks" />
</p>

Framework web **Rust** basé sur **Tide** avec un système de plugins dynamiques (`cdylib`).
Routes, requêtes SQL/MongoDB, vues et ressources entièrement configurées dans `config_actions.json` **sans recompilation**.

---

## Stack technique

| Composant | Version | Rôle |
|---|---|---|
| Rust | 2021 edition | langage |
| Tide | 0.16 | serveur HTTP async |
| Tokio | 1 | runtime async multi-thread |
| sqlx | 0.7 | accès MySQL, requêtes préparées |
| mongodb | 3.5 | driver MongoDB zero-copy (RawDocumentBuf) |
| Handlebars | 6.4 | templates HTML avec partials |
| libloading | 0.8 | chargement dynamique des `.so` |
| jsonwebtoken | 9 | JWT HS256 pour l'authentification |
| multer | 3 | parsing multipart/form-data |
| uuid | 1 | génération d'identifiants uniques |

---

## Architecture du workspace

```
small-folks/
├── README.md                        ← ce fichier
├── config_actions.json              ← annuaire des routes
├── .env                             ← variables d'environnement
├── Cargo.toml                       ← workspace Rust
├── src/
│   ├── main.rs                      ← démarrage, pools, précache plugins
│   └── dispatcher.rs                ← résolution routes, rendu, auth
├── plugins/
│   ├── plugin-core/src/lib.rs       ← traits et types partagés (FFI)
│   ├── plugin_sql/src/lib.rs        ← SQL générique + ressources
│   ├── plugin_mongo/src/lib.rs      ← MongoDB avec MongoContext autonome
│   ├── plugin_auth/src/lib.rs       ← login / logout / JWT / sessions
│   └── plugin_upload/src/lib.rs     ← upload multipart → disque + MySQL
├── templates/
│   ├── generics/                    ← templates réutilisables
│   │   ├── tableGeneric.hbs         ← tableau de données générique
│   │   ├── formGeneric.hbs          ← formulaire générique (inputs + selects)
│   │   ├── listeGeneric.hbs         ← liste <select> générique
│   │   ├── login.hbs                ← page de connexion
│   │   ├── upload_form.hbs          ← formulaire d'upload
│   │   ├── upload_list.hbs          ← liste des fichiers uploadés
│   │   ├── index.hbs                ← page d'accueil
│   │   ├── error.hbs                ← page d'erreur
│   │   └── success.hbs              ← page de succès
│   ├── partials/                    ← fragments réutilisables
│   │   ├── header.hbs
│   │   ├── nav.hbs
│   │   └── footer.hbs
│   └── specifics/                   ← templates spécifiques au projet
│       └── form_countries.hbs
├── public/
│   ├── css/styles.css               ← styles globaux + formulaires
│   └── images/                      ← favicon, logos
├── uploads/                         ← fichiers uploadés (UUID.ext)
├── resources/                       ← logos et schémas du framework
└── sql/
    ├── create_countries.sql
    └── create_uploads.sql
```

---

## Flux d'une requête HTTP

![alt Schéma architecture](https://mascaron.net/schema_architecture_small_folksv6.png)

```
Client HTTP
  ↓ GET /regions  (cookie session_id présent)
Tide — catch-all /*
  ↓
Dispatcher
  ├─ resolve_action("GET", "/regions") → config_actions.json
  ├─ extraction params URL + query string + body
  ├─ injection cookie session_id → ctx.params
  ├─ vérification auth (si "auth": true dans config)
  │     → session cache → OK ou 401/redirect /login
  ↓
plugin_sql.execute(ctx, state)        ← synchrone (contrainte FFI cdylib)
  ├─ block_in_place + handle.block_on
  ├─ requête principale (ctx.sql)
  ├─ requêtes ressources (ctx.sql_resources) si data_resources défini
  └─ PluginResult::Data(json)
  ↓
Dispatcher : rendu selon return_type
  ├─ "html"     → hbs.render(view, data)
  ├─ "json"     → HTTP 200 application/json
  └─ "redirect" → HTTP 303 Location
  ↓
HTTP response
```

---

## config_actions.json — référence complète

### Champs disponibles

| Champ | Type | Défaut | Description |
|---|---|---|---|
| `plugin` | string | — | Chemin vers le `.so` |
| `sql` | string | — | Requête SQL avec `:param` |
| `collection` | string | — | Collection MongoDB |
| `filter` | string | `{}` | Filtre BSON JSON avec `:param` |
| `operation` | string | `find` | Opération MongoDB |
| `form_action` | string | — | URL action du formulaire HTML |
| `data_resources` | objet | `{}` | `"nom_colonne" → "nom_ressource"` |
| `sql_resources` | objet | `{}` | `"nom_ressource" → "SELECT ..."` |
| `allowed_mime` | string | `image/jpeg,image/png,application/pdf` | Types MIME upload |
| `max_size_mb` | string | `10` | Taille max upload en Mo |
| `view` | string | — | Nom du template Handlebars |
| `return_type` | string | `json` | `html`, `json` ou `redirect` |
| `redirect_to` | string | `/` | URL de redirection |
| `auth` | bool | `false` | Exige une session valide |

### Règle template selon le cas d'usage

| Cas | Template | Structure JSON reçue |
|---|---|---|
| Liste de données | `tableGeneric.hbs` | `[{...}]` → `{{#each this}}` |
| Formulaire simple | `formGeneric.hbs` + `form_action` | `{ data: [{...}], form_action: "..." }` |
| Formulaire avec selects | `formGeneric.hbs` + `form_action` + `data_resources` | `{ data: [{...}], resources: { field: [[val,lbl]] }, form_action: "..." }` |

### Exemples de routes

```json
{
    "GET/countries": {
        "plugin": "./target/release/libplugin_sql.so",
        "sql": "SELECT code, name_us, name_fr FROM countries ORDER BY name_us",
        "view": "generics/tableGeneric.hbs",
        "return_type": "html"
    },
    "GET/regions": {
        "plugin": "./target/release/libplugin_sql.so",
        "sql": "SELECT COALESCE(region,'Unknown') AS region, CAST(COUNT(*) AS CHAR) AS total FROM countries GROUP BY region",
        "view": "generics/tableGeneric.hbs",
        "return_type": "html",
        "auth": true
    },
    "GET/view/user/:id": {
        "plugin": "./target/release/libplugin_sql.so",
        "sql": "SELECT CAST(id_users AS CHAR) AS id_users, name, firstName, code_countries FROM users WHERE id_users = :id",
        "view": "generics/formGeneric.hbs",
        "form_action": "/users",
        "return_type": "html",
        "auth": true,
        "data_resources": { "code_countries": "countries" },
        "sql_resources":  { "countries": "SELECT code, name_fr FROM countries ORDER BY name_fr" }
    },
    "POST/login": {
        "plugin": "./target/release/libplugin_auth.so",
        "operation": "login",
        "return_type": "redirect",
        "redirect_to": "/index"
    },
    "GET/logout": {
        "plugin": "./target/release/libplugin_auth.so",
        "operation": "logout",
        "return_type": "redirect",
        "redirect_to": "/login"
    },
    "POST/upload": {
        "plugin": "./target/release/libplugin_upload.so",
        "allowed_mime": "image/jpeg,image/png,application/pdf",
        "max_size_mb": "10",
        "return_type": "redirect",
        "redirect_to": "/uploads"
    },
    "GET/mongo/countries": {
        "plugin": "./target/release/libplugin_mongo.so",
        "collection": "countries",
        "filter": "{}",
        "operation": "find",
        "view": "generics/tableGeneric.hbs",
        "return_type": "html"
    }
}
```

---

## plugin-core — types partagés (FFI)

```rust
pub struct AppState {
    pub pool:     MySqlPool,
    pub handle:   Handle,
    pub mongo:    Option<mongodb::Client>,
    pub sessions: Arc<Mutex<HashMap<String, SessionUser>>>,
}

pub struct ActionContext {
    pub sql:            String,
    pub collection:     String,
    pub filter:         String,
    pub operation:      String,
    pub upload_dir:     String,
    pub allowed_mime:   String,
    pub max_size_mb:    String,
    pub form_action:    Option<String>,
    pub data_resources: HashMap<String, String>,  // "code_countries" → "countries"
    pub sql_resources:  HashMap<String, String>,  // "countries" → "SELECT ..."
    pub params:         HashMap<String, String>,
    pub view:           String,
    pub return_type:    String,
    pub redirect_to:    Option<String>,
    pub body_bytes:     Vec<u8>,
    pub content_type:   String,
}

pub enum PluginResult {
    Data(serde_json::Value),
    Error(String),
    AuthSuccess { session_id: String, jwt: String, redirect_to: String, user: Value },
    AuthError(String),
    AuthLogout { redirect_to: String },
}

// Trait FFI — execute() DOIT être synchrone (async_trait interdit)
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult;
}
```

---

## Règles critiques FFI cdylib

### plugin_sql — block_in_place + handle.block_on
```rust
tokio::task::block_in_place(|| {
    state.handle.block_on(async {
        sqlx::query(...).fetch_all(&state.pool).await
    })
})
```

### plugin_mongo — MongoContext autonome (CRITIQUE)
Le client MongoDB DOIT être créé dans `MONGO_RT`, pas dans `main.rs`.
Sinon `block_in_place` de `plugin_sql` affame le heartbeat MongoDB → latence 1-8s.

```rust
struct MongoContext { rt: Runtime, client: mongodb::Client }
static MONGO_CTX: OnceLock<MongoContext> = OnceLock::new();

// Dans execute() :
tokio::task::block_in_place(|| {
    get_mongo_ctx().rt.block_on(async move { ... })
})
```

### Règles SQL
- Toutes les colonnes doivent être `CHAR`/`VARCHAR` ou castées : `CAST(COUNT(*) AS CHAR)`, `CAST(id AS CHAR)`, `CAST(created_at AS CHAR)`
- Paramètres nommés `:param` convertis automatiquement en `?` positionnels

### Cookies multiples avec Tide
```rust
// ❌ .header() deux fois → le second écrase le premier
// ✅ insert_header() puis append_header()
res.insert_header("Set-Cookie", "session_id=...; HttpOnly");
res.append_header("Set-Cookie", "jwt_token=...");
```

### Templates Handlebars Rust
- `{{this}}` et non `{{.}}`
- `{{#each this.0}}` + `{{@key}}` pour les en-têtes génériques
- `{{#if (lookup @root.resources @key)}}` pour afficher un `<select>` conditionnel
- Partials : `{{> partials/header page_title="Titre"}}`

---

## Authentification

### Flux login
```
POST /login (login=x&mdp=y)
  → plugin_auth.do_login()
  → SELECT users WHERE login=? AND mdp=?
  → OK : UUID session → cache mémoire + JWT HS256
  → dispatcher : cookie session_id (HttpOnly) + jwt_token
  → redirect vers next= ou LOGIN_REDIRECT
  → KO : redirect /login?error=1
```

### Opérations plugin_auth

| Opération | Route conseillée | Description |
|---|---|---|
| `login` | `POST /login` | Authentifie, crée session + JWT |
| `logout` | `GET /logout` | Supprime la session du cache |
| `me` | `GET /api/me` | Retourne les infos de l'utilisateur courant (JSON) |

### Protection d'une route
```json
"GET/ma-route": { "auth": true, ... }
```
- Non authentifié + `return_type: json` → HTTP 401
- Non authentifié + `return_type: html` → redirect `/login?next=/ma-route`

---

## Upload de fichiers

### Flux
```
POST /upload (multipart/form-data)
  → plugin_upload.execute()
  → validation MIME (allowed_mime)
  → renommage UUID.ext
  → écriture sur disque (UPLOAD_DIR)
  → INSERT INTO uploads (...)
  → redirect /uploads
```

### Table MySQL uploads
```sql
CREATE TABLE IF NOT EXISTS uploads (
    id          INT AUTO_INCREMENT PRIMARY KEY,
    uuid        CHAR(36)     NOT NULL UNIQUE,
    filename    VARCHAR(255) NOT NULL,
    stored_as   VARCHAR(255) NOT NULL,
    mime_type   VARCHAR(100) NOT NULL,
    size_bytes  BIGINT       NOT NULL,
    upload_dir  VARCHAR(255) NOT NULL,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
```

---

## Variables d'environnement (.env)

```bash
HOST=0.0.0.0
PORT=8080
CONFIG_ACTIONS=./config_actions.json
TEMPLATES_DIR=./templates
DATABASE_URL=mysql://user:pass@localhost:3306/mabase
MONGODB_URI=mongodb://localhost:27017
MONGODB_DB=mabase
MONGODB_USER=admin
MONGODB_PASS=monpass
MONGODB_AUTH_DB=admin
UPLOAD_DIR=./uploads
JWT_SECRET=chaine-secrete-32-chars-minimum
SESSION_TTL_SECONDS=3600
LOGIN_REDIRECT=/index
```

---

## Compilation et lancement

```bash
cargo build --release --all
./target/release/rust-plugin-tide
```

Les `.so` dans `config_actions.json` doivent pointer vers `./target/release/`.
Mélanger binaire release et `.so` debug provoque un coredump.

---

## Performances (release, localhost)

| Plugin | Opération | Latence |
|---|---|---|
| plugin_sql | SELECT 243 lignes | 1-5ms |
| plugin_mongo | find 243 documents | 3-5ms |
| plugin_auth | login complet | ~1ms |
| plugin_upload | upload 1 fichier | < 10ms |

---

## Routes système

```
GET /health   → {"status":"ok"}
GET /images/* → public/images/
GET /css/*    → public/css/
GET /uploads/* → uploads/
```

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v2_50pc.png" alt="Logo Rust Framework small-Folks" />
</p>
