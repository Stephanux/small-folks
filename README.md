# small-folks — Framework MVC Rust piloté par configuration

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="Logo Rust Framework small-Folks" />
</p>

Framework web **Rust** basé sur **Tide** avec un système de plugins dynamiques (`cdylib`).
Routes, requêtes SQL/MongoDB, vues et ressources sont entièrement configurées dans `config_actions.json` **sans recompilation**.

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
| sysinfo | 0.30 | métriques RAM, disque, uptime (plugin_health) |

---

## Architecture du workspace

```
small-folks/
├── README.md                        ← ce fichier
├── config_actions.json              ← annuaire des routes
├── .env                             ← variables d'environnement
├── Cargo.toml                       ← workspace Rust (edition 2024 pour le binaire)
├── dump/
    └── R504TP_2026_04_23_dump       ← Fichier SQL pour générer la base de données de démonstration   
├── src/
│   ├── main.rs                      ← démarrage, pools, précache plugins
│   └── dispatcher.rs                ← résolution routes, rendu, auth
├── plugins/                         ← tous les plugins en edition 2021
│   ├── plugin-core/src/lib.rs       ← traits et types partagés (FFI)
│   ├── plugin_sql/src/lib.rs        ← SQL générique + ressources + form
│   ├── plugin_mongo/src/lib.rs      ← MongoDB avec MongoContext autonome
│   ├── plugin_auth/src/lib.rs       ← login / logout / JWT / sessions
│   ├── plugin_upload/src/lib.rs     ← upload multipart → disque + MySQL
│   ├── plugin_sql_upload/src/lib.rs ← formulaire texte + fichier → SQL + disque
│   └── plugin_health/src/lib.rs     ← métriques serveur + ping BDD
├── templates/
│   ├── generics/                    ← templates réutilisables
│   │   ├── tableGeneric.hbs         ← tableau de données générique
│   │   ├── formGeneric.hbs          ← formulaire générique (inputs + selects)
│   │   ├── listeGeneric.hbs         ← liste <select> générique
│   │   ├── health_dashboard.hbs     ← dashboard santé serveur
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
│   ├── css/styles.css               ← styles globaux + formulaires + dashboard
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
| `operation` | string | `find` | Opération MongoDB ou auth (`login`, `logout`, `me`, `status`, `dashboard`) |
| `form_action` | string | — | URL `action=""` du formulaire HTML |
| `form_columns` | number | `1` | Nombre de colonnes du formulaire : `1` ou `2` |
| `form_fullwidth_fields` | array | `[]` | Champs sur toute la largeur en mode 2 colonnes |
| `data_resources` | objet | `{}` | `"nom_colonne" → "nom_ressource"` pour les selects |
| `sql_resources` | objet | `{}` | `"nom_ressource" → "SELECT ..."` |
| `upload_field` | string | — | Nom du champ fichier dans le formulaire (plugin_sql_upload) |
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
| Formulaire 1 colonne | `formGeneric.hbs` + `form_action` | `{ data: [{fields:[{key,value,fullwidth}]}], form_action, form_columns:1 }` |
| Formulaire 2 colonnes | `formGeneric.hbs` + `form_columns:2` | idem + champs fullwidth marqués `true` |
| Formulaire avec selects | idem + `data_resources` | idem + `resources: { field: [[val,lbl]] }` |
| Formulaire + upload fichier | template spécifique + `enctype="multipart/form-data"` | traité par `plugin_sql_upload` |

### Structure JSON envoyée à formGeneric.hbs

```json
{
  "form_action":  "/users",
  "form_columns": 2,
  "data": [
    {
      "fields": [
        { "key": "name",      "value": "Mascaron", "fullwidth": false },
        { "key": "firstName", "value": "Stéphane", "fullwidth": false },
        { "key": "addresse1", "value": "Rue ...",  "fullwidth": true  },
        { "key": "city",      "value": "Mont...",  "fullwidth": true  }
      ]
    }
  ],
  "resources": {
    "code_countries": [["FR", "France"], ["DE", "Allemagne"]]
  }
}
```

### Exemples de routes

```json
{
    "GET/countries": {
        "plugin": "./target/release/libplugin_sql.so",
        "sql": "SELECT code, name_us, name_fr FROM countries ORDER BY name_us",
        "view": "generics/tableGeneric.hbs",
        "return_type": "html"
    },
    "GET/view/user/:id": {
        "plugin":      "./target/release/libplugin_sql.so",
        "sql":         "SELECT CAST(id_users AS CHAR) AS id, name, firstName, code_countries FROM users WHERE id_users = :id",
        "view":        "generics/formGeneric.hbs",
        "form_action": "/users",
        "form_columns": 2,
        "form_fullwidth_fields": ["addresse1", "addresse2", "city"],
        "return_type": "html",
        "auth": true,
        "data_resources": { "code_countries": "countries" },
        "sql_resources":  { "countries": "SELECT code, name_fr FROM countries ORDER BY name_fr" }
    },
    "POST/insert_user": {
        "plugin":       "./target/release/libplugin_sql_upload.so",
        "sql":          "INSERT INTO users (name, firstName, login, image) VALUES (:name, :firstName, :login, :image)",
        "upload_field": "image",
        "allowed_mime": "image/jpeg,image/png,image/webp",
        "max_size_mb":  "5",
        "return_type":  "redirect",
        "redirect_to":  "/users",
        "auth":         true
    },
    "POST/login": {
        "plugin": "./target/release/libplugin_auth.so",
        "operation": "login",
        "return_type": "redirect",
        "redirect_to": "/index"
    },
    "GET/health": {
        "plugin": "./target/release/libplugin_health.so",
        "operation": "status",
        "return_type": "json"
    },
    "GET/health/dashboard": {
        "plugin": "./target/release/libplugin_health.so",
        "operation": "dashboard",
        "view": "generics/health_dashboard.hbs",
        "return_type": "html",
        "auth": true
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
    pub sql:                   String,
    pub collection:            String,
    pub filter:                String,
    pub operation:             String,
    pub upload_dir:            String,
    pub upload_field:          String,        // champ fichier (plugin_sql_upload)
    pub allowed_mime:          String,
    pub max_size_mb:           String,
    pub form_action:           Option<String>,
    pub form_columns:          u8,            // 1 (défaut) ou 2
    pub form_fullwidth_fields: Vec<String>,   // champs pleine largeur en mode 2 col
    pub data_resources:        HashMap<String, String>,
    pub sql_resources:         HashMap<String, String>,
    pub params:                HashMap<String, String>,
    pub view:                  String,
    pub return_type:           String,
    pub redirect_to:           Option<String>,
    pub body_bytes:            Vec<u8>,
    pub content_type:          String,
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

### Edition Rust
```
Cargo.toml principal (binaire small-folks) → edition = "2024"
Cargo.toml des plugins (cdylib)            → edition = "2021"
```
En edition 2024, `#[no_mangle]` devient `#[unsafe(no_mangle)]`. Les plugins restent en 2021 pour éviter cette contrainte.

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

tokio::task::block_in_place(|| {
    get_mongo_ctx().rt.block_on(async move { ... })
})
```

**La même règle s'applique à `plugin_health`** (`HealthContext` + `HEALTH_RT`) et à **`plugin_sql_upload`** (`OnceLock<Runtime>` dédié).

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
- `{{#each this}}` + `{{@key}}` pour `tableGeneric.hbs`
- `{{#each data.0.fields}}` + `{{key}}`, `{{value}}`, `{{#if fullwidth}}` pour `formGeneric.hbs`
- `{{#if (lookup @root.resources key)}}` pour afficher un `<select>` conditionnel
- `../../../@key` est interdit — Handlebars Rust ne supporte pas la remontée profonde
- Partials : `{{> partials/header page_title="Titre"}}`
- **Formulaire avec upload** : `enctype="multipart/form-data"` obligatoire sur la balise `<form>` (guillemet fermant !)

---

## Formulaire générique — mode 2 colonnes

Le formulaire générique supporte 1 ou 2 colonnes via CSS Grid, configurable par route.

`plugin_sql` enrichit chaque champ avec un flag `fullwidth: bool` calculé depuis `form_fullwidth_fields` :

```
config_actions.json          plugin_sql                   formGeneric.hbs
─────────────────────        ──────────────               ───────────────────────
form_columns: 2          →   data[0].fields = [       →   {{#each data.0.fields}}
form_fullwidth_fields:         {key:"name",    fw:false}     {{#if fullwidth}} ← class
  ["addresse1","city"]         {key:"addresse1",fw:true }    {{/if}}
                               {key:"city",    fw:true }   {{/each}}
                             ]
```

```css
.form-2col { grid-template-columns: 1fr 1fr; }
.form-2col .form-group-full { grid-column: 1 / -1; }
@media (max-width: 640px) { .form-2col { grid-template-columns: 1fr; } }
```

---

## Upload de fichiers (plugin_upload)

Upload autonome sans SQL métier — stocke le fichier + métadonnées dans la table `uploads`.

```
POST /upload (multipart/form-data)
  → validation MIME + taille
  → UUID.ext sur disque
  → INSERT INTO uploads
  → redirect
```

---

## Upload + SQL métier (plugin_sql_upload)

Fusion de l'upload et d'une requête SQL en une seule action — idéal pour les formulaires mixtes (champs texte + fichier image).

### Flux

```
POST /insert_user (multipart/form-data)
  ├─ 1. Parse multipart → champs texte dans params
  ├─ 2. Validation MIME + taille du fichier
  ├─ 3. Renommage UUID.ext → écriture sur disque
  ├─ 4. INSERT INTO uploads (métadonnées)
  ├─ 5. params[upload_field] = "uuid.ext"
  ├─ 6. Exécution du SQL métier avec tous les params
  └─ redirect ou json
```

Si aucun fichier n'est fourni → `params[upload_field] = ""` → SQL reçoit `NULL` (colonne doit accepter NULL).

En cas d'erreur SQL → le fichier uploadé est supprimé du disque (rollback partiel).

### Exemple config_actions.json

```json
"POST/insert_user": {
    "plugin":       "./target/release/libplugin_sql_upload.so",
    "sql":          "INSERT INTO users (name, firstName, login, image) VALUES (:name, :firstName, :login, :image)",
    "upload_field": "image",
    "allowed_mime": "image/jpeg,image/png,image/webp",
    "max_size_mb":  "5",
    "return_type":  "redirect",
    "redirect_to":  "/users",
    "auth":         true
}
```

### Astuce UPDATE avec photo optionnelle

Si l'utilisateur ne change pas la photo lors d'une édition, utiliser `COALESCE` pour conserver l'ancienne valeur :

```json
"sql": "UPDATE users SET name=:name, image=COALESCE(NULLIF(:image,''), image) WHERE id=:id"
```

`NULLIF(:image,'')` convertit la chaîne vide en `NULL`, `COALESCE` conserve alors l'ancienne valeur de la colonne.

### Template HTML requis

Le formulaire doit obligatoirement déclarer `enctype="multipart/form-data"` (attention au guillemet fermant) :

```html
<form method="POST" action="{{form_action}}" enctype="multipart/form-data">
  <!-- champs texte normaux -->
  <input type="text" name="name">
  <!-- champ fichier -->
  <input type="file" name="image" accept=".jpg,.jpeg,.png,.webp">
  <button type="submit">Enregistrer</button>
</form>
```

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

## Health check (plugin_health)

### Routes
```
GET /health              → JSON brut (monitoring)
GET /health/dashboard    → HTML dashboard (auth: true conseillé)
```

### JSON retourné
```json
{
  "status": "ok",
  "sessions":  { "active": 2, "expired": 0, "total": 2 },
  "databases": {
    "mysql":   { "status": "ok", "latency_ms": 1 },
    "mongodb": { "status": "ok", "latency_ms": 3 }
  },
  "memory":  { "total_mb": 16000, "used_mb": 8000, "free_mb": 8000, "usage_percent": 50 },
  "disk":    { "mount": "/", "total_gb": 500, "used_gb": 120, "free_gb": 380, "usage_percent": 24 },
  "uptime":  { "seconds": 3600, "formatted": "1h 0min" }
}
```

`status` passe en `"warning"` si MySQL ou MongoDB KO, ou si RAM/disque > 90%.
`plugin_health` utilise `HealthContext` avec son propre `HEALTH_RT` — même pattern que `plugin_mongo`.

---

## Table MySQL uploads

Utilisée par `plugin_upload` et `plugin_sql_upload`.

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

dump de la base de données de démonstration dans le dossier ./dump

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
./target/release/small-folks
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
| plugin_sql_upload | upload + INSERT SQL | < 15ms |
| plugin_health | toutes métriques + pings | < 10ms |

---

## Routes système

```
GET /health            → {"status":"ok"} JSON
GET /health/dashboard  → dashboard HTML
GET /images/*          → public/images/
GET /css/*             → public/css/
GET /uploads/*         → uploads/
```

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v2_50pc.png" alt="Logo Rust Framework small-Folks" />
</p>