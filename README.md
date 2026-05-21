# small-folks — Framework MVC Rust piloté par configuration

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="Logo Rust Framework small-Folks" />
</p>

Framework web **Rust** basé sur **Tide** avec un système de plugins dynamiques (`cdylib`).
Routes, requêtes SQL/MongoDB, vues et ressources sont entièrement configurées dans `config_actions.json` **sans recompilation**.
NB : Small-Folks est développé pour un usage pédagogique, il n'est pas utilisable en production.

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
| rumqttc | 0.24 | client MQTT pur Rust (mqtt_worker) |
| aya | 0.12 | framework eBPF userspace (ebpf_worker) |
| libc | 0.2 | appels système (RLIMIT_MEMLOCK pour eBPF) |
| chrono | 0.4 | gestion des dates/timestamps |

---

## Architecture du workspace

```
small-folks/
├── README.md                        ← ce fichier
├── config_actions.json              ← annuaire des routes
├── .env                             ← variables d'environnement
├── .env.example                     ← template des variables d'environnement
├── Cargo.toml                       ← workspace Rust (edition 2024 pour le binaire)
├── dump/
│   └── R504TP_2026_04_23_dump       ← dump SQL de la base de démonstration
├── src/
│   ├── main.rs                      ← démarrage, pools, précache plugins
│   ├── dispatcher.rs                ← résolution routes, rendu, auth
│   ├── app_security.rs              ← protections XSS, open redirect, header injection
│   ├── helpers_hbs.rs               ← helpers Handlebars personnalisés
│   ├── mqtt_worker.rs               ← client MQTT de fond (stockage capteurs)
│   └── ebpf_worker.rs               ← firewall XDP userspace (optionnel, nécessite sudo)
├── ebpf-firewall/                   ← programme XDP kernel (crate séparée, bpfel-unknown-none)
│   ├── Cargo.toml
│   ├── rust-toolchain.toml          ← nightly + rust-src (requis pour bpfel-unknown-none)
│   ├── .cargo/config.toml           ← cible bpfel-unknown-none + build-std=core
│   └── src/main.rs                  ← programme XDP : comptage SYN + DROP blacklist
├── plugins/                         ← tous les plugins en edition 2021
│   ├── plugin-core/src/lib.rs       ← traits, types partagés (FFI) + named_to_positional
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
│       ├── chart_capteurs.hbs       ← courbes Chart.js température + humidité
│       └── form_countries.hbs
├── public/
│   ├── css/styles.css               ← styles globaux + formulaires + dashboard
│   └── images/                      ← favicon, logos
├── uploads/                         ← fichiers uploadés (UUID.ext)
├── resources/                       ← logos et schémas du framework
└── sql/
    ├── create_countries.sql
    ├── create_uploads.sql
    ├── create_capteurs.sql          ← table capteurs MQTT
    └── create_ebpf_blacklist.sql    ← table blacklist eBPF/XDP
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
| `sql_upload` | string | — | Requête INSERT pour la table uploads (plugin_sql_upload) |
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

```json
{
  "data": [
    { "id": "1", "nom": "Lion", "espece": "Panthera leo" }
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
      { "val": "FR", "label": "France",    "selected": true  }
    ]
  }
}
```

**`selected` est pré-calculé côté Rust** dans `plugin_sql`.

- Mode `insert_XXX` → valeurs vides → `selected: false` partout → "-- choisir --" affiché
- Mode `update_XXX` → `selected: true` sur l'option correspondant à la valeur courante

---

## plugin-core — types partagés (FFI)

```rust
pub struct ActionContext {
    pub sql:                   String,
    pub sql_upload:            String,        // INSERT uploads (plugin_sql_upload)
    pub collection:            String,
    pub filter:                String,
    pub operation:             String,
    pub upload_dir:            String,
    pub upload_field:          String,
    pub allowed_mime:          String,
    pub max_size_mb:           String,
    pub form_action:           Option<String>,
    pub form_columns:          u8,
    pub form_fullwidth_fields: Vec<String>,
    pub row_link:              Option<String>,
    pub row_link_col:          u8,
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

// Fonction partagée entre tous les plugins — évite la duplication
pub fn named_to_positional(
    sql:    &str,
    params: &HashMap<String, String>,
) -> (String, Vec<String>)
```

---

## Helpers Handlebars (src/helpers_hbs.rs)

Centralisés dans `src/helpers_hbs.rs`, enregistrés via `crate::helpers_hbs::register_all(&mut hbs)`.

### Helper `compare` — bloc conditionnel

```handlebars
{{#compare val "actif"}}vrai{{/compare}}
{{#compare val "actif"}}vrai{{else}}faux{{/compare}}
{{#compare role "admin"  operator="=="}}...{{/compare}}
{{#compare nb   "10"     operator=">"}}...{{/compare}}
```

Opérateurs : `==` `===` `!=` `!==` `>` `>=` `<` `<=`

### Helper `json` — injection JSON brut dans `<script>`

```handlebars
{{!-- Triple accolades = pas d'échappement HTML → JSON valide pour JS --}}
<script>
  const data = {{{json data}}};
  const labels = data.map(r => r.timestamp);
</script>
```

### Helpers natifs Handlebars Rust (ne pas réenregistrer)

`eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `and`, `or`, `not` — disponibles en subexpression :
```handlebars
{{#if (eq status "ok")}}...{{/if}}
{{#if (gt memory.usage_percent 85)}}...{{/if}}
```

---

## Sécurité (src/app_security.rs)

| Fonction | Protège contre | Exemple |
|---|---|---|
| `sanitize_redirect(url)` | Open Redirect | `http://evil.com` → `/` |
| `sanitize_header(val)` | Header Splitting | `val\r\nX-Evil:` → `valX-Evil:` |
| `sanitize_log(msg)` | Log Injection | `msg\n[FAKE]` → `msg [FAKE]` |
| `sanitize_html(input)` | XSS hors-template | `<script>` → `&lt;script&gt;` |

Tests unitaires : `cargo test -p small-folks`

---

## Règles critiques FFI cdylib

### Edition Rust
```
Cargo.toml principal (binaire small-folks) → edition = "2024"
Cargo.toml des plugins (cdylib)            → edition = "2021"
```
En edition 2024, `#[no_mangle]` devient `#[unsafe(no_mangle)]`. Les plugins restent en 2021.

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

**La même règle s'applique à `plugin_health`** (`HealthContext` + `HEALTH_RT`) et à **`plugin_sql_upload`** (`OnceLock<Runtime>` dédié).

### named_to_positional — fonction partagée dans plugin-core
```rust
// Dans tous les plugins qui exécutent du SQL :
let (sql_prepared, values) = plugin_core::named_to_positional(&ctx.sql, &ctx.params);
```
Ne pas dupliquer — `regex = "1"` dans `plugin-core/Cargo.toml` suffit.

### Règles SQL
- Toutes les colonnes doivent être `CHAR`/`VARCHAR` ou castées : `CAST(id AS CHAR)`, `CAST(COUNT(*) AS CHAR)`
- Paramètres nommés `:param` → convertis automatiquement en `?` positionnels
- `plugin_auth` utilise `ctx.sql` — requête définie dans `config_actions.json`

### Templates Handlebars Rust
- `{{this}}` et non `{{.}}`
- `{{#each data}}{{#if @first}}{{#each this}}<th>{{@key}}</th>{{/each}}{{/if}}{{/each}}` pour les headers
- `{{#each data.0.fields}}` + `{{key}}`, `{{value}}`, `{{#if fullwidth}}` pour `formGeneric`
- `{{#each (lookup @root.resources key)}}` + `{{val}}`, `{{label}}`, `{{#if selected}}` pour les selects
- `../../../@key` interdit — Handlebars Rust ne supporte pas la remontée profonde
- `enctype="multipart/form-data"` obligatoire sur les formulaires upload (guillemet fermant !)

---

## Authentification (plugin_auth)

### Convention d'alias SQL

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

> **⚠️ Mots de passe bcrypt** : la comparaison SQL directe ne fonctionne pas avec `password_hash()`.

---

## Upload de fichiers

### plugin_upload — upload autonome

```json
"POST/upload": {
    "plugin":       "./target/release/libplugin_upload.so",
    "sql":          "INSERT INTO uploads (uuid, filename, stored_as, mime_type, size_bytes, upload_dir) VALUES (:uuid, :filename, :stored_as, :mime_type, :size_bytes, :upload_dir)",
    "allowed_mime": "image/jpeg,image/png,application/pdf",
    "max_size_mb":  "10",
    "return_type":  "redirect",
    "redirect_to":  "/uploads"
}
```

### plugin_sql_upload — upload + SQL métier

```json
"POST/insert_user": {
    "plugin":       "./target/release/libplugin_sql_upload.so",
    "sql":          "INSERT INTO users (name, image) VALUES (:name, :image)",
    "sql_upload":   "INSERT INTO uploads (uuid, filename, stored_as, mime_type, size_bytes, upload_dir) VALUES (:uuid, :filename, :stored_as, :mime_type, :size_bytes, :upload_dir)",
    "upload_field": "image",
    "allowed_mime": "image/jpeg,image/png,image/webp",
    "max_size_mb":  "5",
    "return_type":  "redirect",
    "redirect_to":  "/users",
    "auth":         true
}
```

Astuce UPDATE photo optionnelle :
```sql
UPDATE users SET name=:name, image=COALESCE(NULLIF(:image,''), image) WHERE id=:id
```

---

## Health check (plugin_health)

```
GET /health              → JSON brut
GET /health/dashboard    → HTML dashboard (auth: true conseillé)
```

`plugin_health` utilise `HealthContext` avec `HEALTH_RT` autonome — même pattern que `plugin_mongo`.

---

## Client MQTT (mqtt_worker)

Tâche Tokio de fond démarrée automatiquement si `MQTT_BROKER_URL` est défini dans `.env`.

### Formats de messages supportés

- `sensors/temperature` → payload float : `"25.3"`
- `sensors/humidity` → payload float : `"60.5"`
- `sensors/+/data` → payload JSON : `{"sensor_id":"DHT22-001","temperature":25.3,"humidity":60.5}`

Les topics `temperature`/`humidity` sont mis en buffer par `sensor_id`. L'INSERT a lieu quand les deux valeurs sont disponibles.

### Table MySQL capteurs

```sql
CREATE TABLE IF NOT EXISTS capteurs (
    id          INT AUTO_INCREMENT PRIMARY KEY,
    sensor_id   VARCHAR(50) NOT NULL,
    temperature FLOAT       NOT NULL,
    humidity    FLOAT       NOT NULL,
    timestamp   DATETIME    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_sensor_id (sensor_id),
    INDEX idx_timestamp (timestamp)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
```

### Routes suggérées

```json
"GET/capteurs": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT CAST(id AS CHAR) AS id, sensor_id, CAST(temperature AS CHAR) AS temperature, CAST(humidity AS CHAR) AS humidity, CAST(timestamp AS CHAR) AS timestamp FROM capteurs ORDER BY timestamp DESC LIMIT 100",
    "view": "generics/tableGeneric.hbs",
    "return_type": "html",
    "auth": true
},
"GET/capteurs/chart": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT CAST(timestamp AS CHAR) AS timestamp, CAST(temperature AS CHAR) AS temperature, CAST(humidity AS CHAR) AS humidity, sensor_id FROM capteurs ORDER BY timestamp DESC LIMIT 200",
    "view": "specifics/chart_capteurs.hbs",
    "return_type": "html",
    "auth": true
}
```

---

## Firewall eBPF/XDP (ebpf_worker)

Filtre les paquets réseau **dans le kernel Linux** avant la stack TCP/IP — protection contre les attaques SYN flood.

### Architecture

```
Paquet réseau entrant (NIC)
  ↓
[xdp_firewall] ← tourne dans le kernel (bpfel-unknown-none)
  ├─ IP dans BLACKLIST ?       → XDP_DROP (~100ns)
  ├─ Paquet TCP SYN ?          → incrémenter CONN_COUNT[src_ip]
  │   └─ count > rate_limit ?  → BLACKLIST[src_ip] = now → XDP_DROP
  └─ Sinon                     → XDP_PASS
         ↕ BPF Maps (mémoire partagée kernel ↔ userspace)
[ebpf_worker] ← tourne en userspace (Tokio, toutes les 5s)
  ├─ Lit STATS  → logs
  ├─ Sync BLACKLIST → MySQL (ebpf_blacklist)
  └─ Auto-débloque les IPs expirées
```

### BPF Maps

| Map | Type | Rôle |
|---|---|---|
| `CONN_COUNT` | `HashMap<u32, u32>` | Compteur SYN par IP |
| `CONN_FIRST` | `HashMap<u32, u64>` | Timestamp premier SYN par IP |
| `BLACKLIST` | `HashMap<u32, u64>` | IPs bloquées + timestamp |
| `CONFIG` | `Array<u64>` | `[rate_limit, window_ns]` |
| `STATS` | `Array<u64>` | `[paquets, drops, syn]` |

### Compilation du programme kernel

```bash
# Prérequis
rustup component add rust-src
cargo install bpf-linker

# Compiler le programme kernel (depuis ebpf-firewall/)
cd ebpf-firewall
cargo build --release
# → target/bpfel-unknown-none/release/ebpf-firewall
```

Le dossier `ebpf-firewall/` est **exclu du workspace principal** car il utilise la cible `bpfel-unknown-none`. Il contient son propre `[workspace]` dans son `Cargo.toml` et un `rust-toolchain.toml` qui force nightly.

### Activation dans small-folks

```toml
# Cargo.toml — décommenter
aya = { version = "0.12", features = ["async_tokio"] }
```

```rust
// src/main.rs — décommenter
mod ebpf_worker;
// + le bloc if EBPF_ENABLED
```

```bash
# .env
EBPF_ENABLED=true
EBPF_INTERFACE=eth0
EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
EBPF_RATE_LIMIT=100
EBPF_WINDOW_SECS=60
EBPF_AUTO_UNBLOCK_SECS=300

# Lancer avec les droits nécessaires
sudo ./target/release/small-folks
# ou
sudo setcap cap_bpf,cap_net_admin+eip ./target/release/small-folks
./target/release/small-folks
```

### Table MySQL ebpf_blacklist

```sql
CREATE TABLE IF NOT EXISTS ebpf_blacklist (
    id           INT AUTO_INCREMENT PRIMARY KEY,
    ip_address   VARCHAR(15)  NOT NULL,
    blocked_at   DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    unblock_at   DATETIME     DEFAULT NULL,
    unblocked_at DATETIME     DEFAULT NULL,
    reason       VARCHAR(100) NOT NULL DEFAULT 'rate_limit_exceeded',
    UNIQUE KEY uq_ip_active (ip_address, unblocked_at),
    INDEX idx_ip (ip_address)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
```

### Routes suggérées

```json
"GET/security/blacklist": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT ip_address, CAST(blocked_at AS CHAR) AS blocked_at, reason, CAST(unblock_at AS CHAR) AS unblock_at FROM ebpf_blacklist WHERE unblocked_at IS NULL ORDER BY blocked_at DESC",
    "view": "generics/tableGeneric.hbs",
    "return_type": "html",
    "auth": true
}
```

### Performances observées

```
~19 000 paquets droppés/seconde dans le kernel
99.6% des paquets bloqués pendant un flood SYN
Tide / MySQL : aucun impact pendant l'attaque
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

# MQTT (optionnel)
# MQTT_BROKER_URL=localhost
# MQTT_BROKER_PORT=1883
# MQTT_CLIENT_ID=small-folks-mqtt
# MQTT_TOPICS=sensors/#
# MQTT_QOS=1

# eBPF Firewall XDP (optionnel — nécessite sudo ou CAP_BPF + kernel ≥ 5.8)
# EBPF_ENABLED=true
# EBPF_INTERFACE=eth0
# EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
# EBPF_RATE_LIMIT=100
# EBPF_WINDOW_SECS=60
# EBPF_AUTO_UNBLOCK_SECS=300
```

---

## Compilation et lancement

```bash
# Compiler tous les plugins + le binaire
cargo build --release --all

# (Optionnel) Compiler le programme eBPF kernel
cd ebpf-firewall && cargo build --release && cd ..

# Lancer (sans eBPF)
./target/release/small-folks

# Lancer (avec eBPF)
sudo ./target/release/small-folks
```

Les `.so` dans `config_actions.json` doivent pointer vers `./target/release/`.
Mélanger binaire release et `.so` debug provoque un coredump.

---

## Performances (release, localhost)

| Composant | Opération | Latence / Débit |
|---|---|---|
| plugin_sql | SELECT 243 lignes | 1-5ms |
| plugin_mongo | find 243 documents | 3-5ms |
| plugin_auth | login complet | ~1ms |
| plugin_upload | upload 1 fichier | < 10ms |
| plugin_sql_upload | upload + INSERT SQL | < 15ms |
| plugin_health | toutes métriques + pings | < 10ms |
| mqtt_worker | INSERT capteur | < 5ms |
| ebpf_worker (XDP) | DROP paquet blacklisté | ~100ns |
| ebpf_worker (XDP) | Débit observé en test | ~19 000 drops/s |

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
