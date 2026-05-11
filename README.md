# small-folks — Framework MVC Rust piloté par configuration

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="Logo Rust Framework small-Folks" />
</p>

Framework web **Rust** basé sur **Tide** avec un système de plugins dynamiques (`cdylib`).
Routes, requêtes SQL/MongoDB, vues et ressources sont entièrement configurées dans `config_actions.json` **sans recompilation**.
NB : Small-Folks est développé pour un usage pédagogique, il n'est pas utilsable en production.

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
│   └── R504TP_2026_04_23_dump       ← dump SQL de la base de démonstration
├── src/
│   ├── main.rs                      ← démarrage, pools, précache plugins
│   ├── dispatcher.rs                ← résolution routes, rendu, auth
│   └── helpers_hbs.rs               ← helpers Handlebars personnalisés
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
│   │   ├── tableGeneric.hbs         ← tableau de données + lien optionnel (row_link)
│   │   ├── formGeneric.hbs          ← formulaire générique (inputs + selects + selected)
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
  │     → enrichissement avec { val, label, selected } par option
  └─ PluginResult::Data(json)
  ↓
Dispatcher : rendu selon return_type
  ├─ "html"     → injection row_link + form_action → hbs.render(view, data)
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
| `sql` | string | — | Requête SQL avec `:param` (utilisé aussi par plugin_auth) |
| `collection` | string | — | Collection MongoDB |
| `filter` | string | `{}` | Filtre BSON JSON avec `:param` |
| `operation` | string | `find` | Opération MongoDB ou auth (`login`, `logout`, `me`, `status`, `dashboard`) |
| `form_action` | string | — | URL `action=""` du formulaire HTML |
| `form_columns` | number | `1` | Nombre de colonnes du formulaire : `1` ou `2` |
| `form_fullwidth_fields` | array | `[]` | Champs sur toute la largeur en mode 2 colonnes |
| `data_resources` | objet | `{}` | `"nom_colonne" → "nom_ressource"` pour les selects |
| `sql_resources` | objet | `{}` | `"nom_ressource" → "SELECT ..."` |
| `row_link` | string | — | URL de base pour le lien sur une colonne de `tableGeneric` (ex: `/view/animal`) |
| `row_link_col` | number | `1` | Index de la colonne qui porte le lien (défaut : colonne 1) |
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
| Liste simple | `tableGeneric.hbs` | `{ data: [{...}], row_link: "", row_link_col: 1 }` |
| Liste avec lien | `tableGeneric.hbs` + `row_link` | `{ data: [{...}], row_link: "/view/x", row_link_col: 1 }` |
| Formulaire 1 colonne | `formGeneric.hbs` + `form_action` | `{ data: [{fields:[{key,value,fullwidth}]}], form_action, form_columns:1 }` |
| Formulaire 2 colonnes | `formGeneric.hbs` + `form_columns:2` | idem + champs fullwidth marqués `true` |
| Formulaire avec selects | idem + `data_resources` | idem + `resources: { field: [{val,label,selected}] }` |
| Formulaire + upload fichier | template spécifique + `enctype="multipart/form-data"` | traité par `plugin_sql_upload` |

### Structure JSON envoyée à tableGeneric.hbs

Les données sont **toujours** wrappées dans un objet par le dispatcher — `row_link` vide = falsy en Handlebars :

```json
{
  "data": [
    { "id": "1", "nom": "Lion", "espece": "Panthera leo" },
    { "id": "2", "nom": "Tigre", "espece": "Panthera tigris" }
  ],
  "row_link":     "/view/animal",
  "row_link_col": 1
}
```

Sans `row_link` dans la config → `row_link: ""` → `{{#if row_link}}` est falsy → tableau sans lien.

### Structure JSON envoyée à formGeneric.hbs

```json
{
  "form_action":  "/update_user",
  "form_columns": 2,
  "data": [
    {
      "fields": [
        { "key": "name",           "value": "Mascaron", "fullwidth": false },
        { "key": "code_countries", "value": "FR",       "fullwidth": false },
        { "key": "addresse1",      "value": "Rue ...",  "fullwidth": true  }
      ]
    }
  ],
  "resources": {
    "code_countries": [
      { "val": "DE", "label": "Allemagne", "selected": false },
      { "val": "FR", "label": "France",    "selected": true  },
      { "val": "US", "label": "États-Unis","selected": false }
    ]
  }
}
```

**`selected` est pré-calculé côté Rust** dans `plugin_sql` en comparant `val` avec la valeur courante du champ. Cela évite toute remontée de contexte (`../value`) impossible en Handlebars Rust.

- Mode `insert_XXX` → valeurs vides → `selected: false` partout → "-- choisir --" affiché
- Mode `update_XXX` → `selected: true` sur l'option correspondant à la valeur courante

### Exemples de routes

```json
{
    "GET/animaux": {
        "plugin":       "./target/release/libplugin_sql.so",
        "sql":          "SELECT CAST(id AS CHAR) AS id, nom, espece FROM animaux ORDER BY nom",
        "view":         "generics/tableGeneric.hbs",
        "return_type":  "html",
        "row_link":     "/view/animal",
        "row_link_col": 1,
        "auth":         true
    },
    "GET/view/user/:id": {
        "plugin":      "./target/release/libplugin_sql.so",
        "sql":         "SELECT CAST(id_users AS CHAR) AS id, name, firstName, code_countries FROM users WHERE id_users = :id",
        "view":        "generics/formGeneric.hbs",
        "form_action": "/update_user/:id",
        "form_columns": 2,
        "form_fullwidth_fields": ["addresse1", "addresse2", "city"],
        "return_type": "html",
        "auth":        true,
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
        "plugin":      "./target/release/libplugin_auth.so",
        "operation":   "login",
        "sql":         "SELECT id_users AS id, name, firstName AS first_name, login, function, office FROM users WHERE login = :login AND mdp = :mdp LIMIT 1",
        "return_type": "redirect",
        "redirect_to": "/index"
    },
    "GET/health": {
        "plugin":      "./target/release/libplugin_health.so",
        "operation":   "status",
        "return_type": "json"
    },
    "GET/health/dashboard": {
        "plugin":      "./target/release/libplugin_health.so",
        "operation":   "dashboard",
        "view":        "generics/health_dashboard.hbs",
        "return_type": "html",
        "auth":        true
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
    pub row_link:              Option<String>,// URL de base pour lien colonne tableGeneric
    pub row_link_col:          u8,            // index colonne du lien (défaut 1)
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

## Helpers Handlebars (src/helpers_hbs.rs)

Les helpers Handlebars personnalisés sont regroupés dans `src/helpers_hbs.rs`, séparé de `dispatcher.rs` pour faciliter l'ajout de nouveaux helpers sans toucher au dispatcher.

### Enregistrement

```rust
// Dans dispatcher.rs — Dispatcher::new()
crate::helpers_hbs::register_all(&mut hbs);

// Dans main.rs
mod helpers_hbs;
```

### Ajouter un helper

```rust
// Dans helpers_hbs.rs
pub fn register_all(hbs: &mut Handlebars) {
    hbs.register_helper("compare", Box::new(CompareHelper));
    hbs.register_helper("mon_helper", Box::new(MonHelper)); // ← ajouter ici
}
```

### Helper `compare`

Compare deux valeurs chaînes avec un opérateur configurable. Les helpers de blocs doivent implémenter `HelperDef` (pas une closure) pour exprimer correctement les lifetimes `'reg: 'rc` requis par `Renderable::render`.

**Syntaxe :**
```handlebars
{{#compare val "actif"}}vrai{{/compare}}
{{#compare val "actif"}}vrai{{else}}faux{{/compare}}
{{#compare role "admin"  operator="=="}}...{{/compare}}
{{#compare nb   "10"     operator=">"}}...{{/compare}}
{{#compare nb   "10"     operator="<="}}...{{/compare}}
{{#compare val  "x"      operator="!="}}...{{/compare}}
```

**Opérateurs supportés :** `==` `===` `!=` `!==` `>` `>=` `<` `<=`

Pour les opérateurs ordinaux (`>` `>=` `<` `<=`) : comparaison numérique si les deux valeurs sont des nombres, lexicographique sinon.

**Pourquoi une struct et pas une closure :**
```rust
// ❌ Closure — impossible d'exprimer 'reg: 'rc
|h: &Helper, hbs: &Handlebars, ...| -> HelperResult { t.render(...) }

// ✅ Struct avec HelperDef — lifetimes explicites
impl HelperDef for CompareHelper {
    fn call<'reg: 'rc, 'rc>(&self, h: &Helper<'rc>,
        hbs: &'reg Handlebars<'reg>, ...) -> HelperResult {
        t.render(hbs, ctx, rc, out)  // OK
    }
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
- `plugin_auth` utilise aussi `ctx.sql` — la requête est définie dans `config_actions.json`

### Cookies multiples avec Tide
```rust
// ❌ .header() deux fois → le second écrase le premier
// ✅ insert_header() puis append_header()
res.insert_header("Set-Cookie", "session_id=...; HttpOnly");
res.append_header("Set-Cookie", "jwt_token=...");
```

### Templates Handlebars Rust
- `{{this}}` et non `{{.}}`
- `{{#each data}}{{#if @first}}{{#each this}}<th>{{@key}}</th>{{/each}}{{/if}}{{/each}}` pour les headers de `tableGeneric`
- `{{#each data.0.fields}}` + `{{key}}`, `{{value}}`, `{{#if fullwidth}}` pour `formGeneric`
- `{{#each (lookup @root.resources key)}}` + `{{val}}`, `{{label}}`, `{{#if selected}}` pour les options select
- `{{#if row_link}}` fonctionne car `row_link` vaut `""` (falsy) quand absent
- `../../../@key` est interdit — Handlebars Rust ne supporte pas la remontée profonde
- Partials : `{{> partials/header page_title="Titre"}}`
- **Formulaire avec upload** : `enctype="multipart/form-data"` obligatoire (guillemet fermant !)

---

## Formulaire générique — mode 2 colonnes + selects

### Colonnes CSS Grid

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

### Selects avec valeur pré-sélectionnée

`plugin_sql` pré-calcule `selected: bool` sur chaque option en comparant `val` avec la valeur courante du champ. Cela évite toute remontée de contexte impossible en Handlebars Rust :

```
plugin_sql                               formGeneric.hbs
──────────────────────────────           ────────────────────────────────────
current_value = data[0]["code_countries"]  {{#each (lookup @root.resources key)}}
= "FR"                                     <option value="{{val}}"
                                             {{#if selected}}selected{{/if}}>
resources["code_countries"] = [              {{label}}
  {val:"DE", label:"Allemagne", sel:false}  </option>
  {val:"FR", label:"France",    sel:true }  {{/each}}
  {val:"US", label:"États-Unis",sel:false}
]
```

| Mode form_action | Valeurs data | selected | Résultat |
|---|---|---|---|
| `insert_XXX` | `""` (vides) | `false` partout | "-- choisir --" affiché |
| `update_XXX` | `"FR"`, `"admin"`... | `true` sur la valeur courante | option pré-sélectionnée |

---

## Tableau générique avec lien (tableGeneric + row_link)

Quand `row_link` est défini dans `config_actions.json`, DataTables génère automatiquement un lien sur la colonne `row_link_col` :

```json
"GET/animaux": {
    "plugin":       "./target/release/libplugin_sql.so",
    "sql":          "SELECT CAST(id AS CHAR) AS id, nom, espece FROM animaux",
    "view":         "generics/tableGeneric.hbs",
    "return_type":  "html",
    "row_link":     "/view/animal",
    "row_link_col": 1
}
```

Le lien est construit comme `row_link + "/" + data` → `/view/animal/Lion`.

Sans `row_link` → `row_link: ""` dans le JSON → `{{#if row_link}}` est falsy → tableau standard sans lien. Les données sont **toujours** wrappées dans `{ data, row_link, row_link_col }` par le dispatcher.

---

## Authentification (plugin_auth)

### Principe — SQL dans la config

La requête de vérification est définie dans `config_actions.json`. Le plugin mappe les colonnes via leurs alias SQL.

### Convention d'alias

| Alias recommandé | Fallbacks | Rôle |
|---|---|---|
| `id` | `id_users`, `id_utilisateur` | Clé primaire |
| `name` | — | Nom |
| `first_name` | `firstName`, `prenom` | Prénom |
| `login` | — | Identifiant |
| `function` | `role` | Fonction/rôle |
| `office` | `department` | Bureau (`''` si absent) |

### Exemples config_actions.json

```json
"POST/login": {
    "plugin":    "./target/release/libplugin_auth.so",
    "operation": "login",
    "sql":       "SELECT id_users AS id, name, firstName AS first_name, login, function, office FROM users WHERE login = :login AND mdp = :mdp LIMIT 1",
    "return_type": "redirect",
    "redirect_to": "/index"
}
```

Table avec email comme identifiant et condition `actif` :
```json
"POST/login": {
    "plugin":    "./target/release/libplugin_auth.so",
    "operation": "login",
    "sql":       "SELECT id_utilisateur AS id, nom AS name, prenom AS first_name, email AS login, role AS function, '' AS office FROM utilisateur WHERE email = :login AND mot_de_passe = :mdp AND actif = 1 LIMIT 1",
    "return_type": "redirect",
    "redirect_to": "/index"
}
```

> **⚠️ Mots de passe bcrypt** : la comparaison SQL directe ne fonctionne pas avec `password_hash()`. Il faut modifier `plugin_auth` pour vérifier avec la crate `bcrypt` côté Rust.

### Opérations plugin_auth

| Opération | Route | Description |
|---|---|---|
| `login` | `POST /login` | Exécute `ctx.sql`, crée session + JWT |
| `logout` | `GET /logout` | Supprime la session du cache |
| `me` | `GET /api/me` | Retourne les infos de l'utilisateur courant (JSON) |

### Protection d'une route
```json
"GET/ma-route": { "auth": true, ... }
```
- Non authentifié + `return_type: json` → HTTP 401
- Non authentifié + `return_type: html` → redirect `/login?next=/ma-route`

---

## Upload de fichiers (plugin_upload)

Upload autonome sans SQL métier — stocke fichier + métadonnées dans `uploads`.

---

## Upload + SQL métier (plugin_sql_upload)

### Flux
```
POST /insert_user (multipart/form-data)
  ├─ 1. Parse multipart → champs texte dans params
  ├─ 2. Validation MIME + taille
  ├─ 3. UUID.ext → écriture disque
  ├─ 4. INSERT INTO uploads
  ├─ 5. params[upload_field] = "uuid.ext"
  ├─ 6. Exécution SQL métier
  └─ redirect ou json
```

Si aucun fichier → `params[upload_field] = ""` → SQL reçoit `NULL`.
Erreur SQL → fichier supprimé du disque (rollback partiel).

### Astuce UPDATE avec photo optionnelle
```sql
UPDATE users SET name=:name, image=COALESCE(NULLIF(:image,''), image) WHERE id=:id
```

---

## Health check (plugin_health)

```
GET /health              → JSON brut
GET /health/dashboard    → HTML dashboard (auth: true conseillé)
```

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

`plugin_health` utilise `HealthContext` avec `HEALTH_RT` autonome — même pattern que `plugin_mongo`.

---

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
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
```

Dump de la base de démonstration dans `./dump/`.

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
