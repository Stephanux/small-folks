# small-folks — Framework MVC Rust piloté par configuration ([english version](https://github.com/Stephanux/small-folks/blob/main/README_EN.md))

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="Logo Rust Framework small-Folks" />
</p>

Framework web **Rust** basé sur **Tide** avec un système de plugins dynamiques (`cdylib`).
Routes, requêtes SQL/MongoDB, vues et ressources sont entièrement configurées dans `config_actions.json` **sans recompilation**.

> **NB :** Small-Folks est développé pour un usage **pédagogique** (IUT Réseaux & Télécoms), il n'est pas destiné à la production.

---

## Sommaire

1. [Stack technique](#stack-technique)
2. [Architecture du workspace](#architecture-du-workspace)
3. [Flux d'une requête HTTP](#flux-dune-requête-http)
4. [Le dispatcher en détail](#le-dispatcher-en-détail)
5. [config_actions.json — référence complète](#config_actionsjson--référence-complète)
6. [Formulaires pilotés par la config](#formulaires-pilotés-par-la-config--fields_meta-et-fields)
7. [plugin-core — types partagés (FFI)](#plugin-core--types-partagés-ffi)
8. [Règles critiques FFI cdylib (runtime et pool par plugin)](#règles-critiques-ffi-cdylib)
9. [Helpers Handlebars](#helpers-handlebars-srchelpers_hbsrs)
10. [Sécurité](#sécurité-srcapp_securityrs)
11. [Authentification](#authentification-plugin_auth)
12. [Upload de fichiers](#upload-de-fichiers)
13. [Health check, MQTT, Chart.js, eBPF](#health-check-plugin_health)
14. [Variables d'environnement](#variables-denvironnement-env)
15. [Compilation, lancement et déploiement (systemd)](#compilation-et-lancement)
16. [Tests de charge et de concurrence](#tests-de-charge-et-de-concurrence)
17. [Performances](#performances-release-localhost)

---

## Stack technique

| Composant | Version | Rôle |
|---|---|---|
| Rust | 2021 edition | langage |
| Tide | 0.16 | serveur HTTP async (exécuteur async-std) |
| Tokio | 1 | runtime async multi-thread (un par plugin + un pour le binaire) |
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
│   └── ebpf_worker.rs               ← firewall XDP userspace (optionnel, nécessite root/capabilities)
├── ebpf-firewall/                   ← programme XDP kernel (crate séparée, bpfel-unknown-none)
│   ├── Cargo.toml
│   ├── rust-toolchain.toml          ← nightly + rust-src (requis pour bpfel-unknown-none)
│   ├── .cargo/config.toml           ← cible bpfel-unknown-none + build-std=core
│   └── src/main.rs                  ← programme XDP : comptage SYN + DROP blacklist
├── plugins/                         ← tous les plugins en edition 2021
│   ├── plugin-core/
│   │   └── src/
│   │       ├── lib.rs               ← traits, types partagés (FFI) + named_to_positional
│   │       └── db.rs                ← runtime Tokio + pool MySQL PROPRES À CHAQUE PLUGIN
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
│       ├── form_user.hbs
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
spawn_blocking → plugin_sql.execute(ctx, state)   ← synchrone (contrainte FFI cdylib),
  │                                                  sur un thread du pool bloquant
  ├─ plugin_core::db::sql_ctx()       ← runtime + pool PROPRES au plugin
  ├─ rt.block_on(requête principale ctx.sql)
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

## Le dispatcher en détail

### Comment une requête est routée vers un plugin

Le `Dispatcher` (`src/dispatcher.rs`) possède trois éléments : l'annuaire des actions lu dans `config_actions.json`, la table des plugins déjà chargés (`HashMap<String, Box<dyn Plugin>>`) et le moteur Handlebars. Pour chaque requête :

1. **Résolution de l'action** : la clé `"MÉTHODE/chemin"` (ex. `GET/capteurs/chart/:sensor_id`) est recherchée dans `config_actions.json`. Les segments `:param` de l'URL, la query string et le corps du formulaire sont fusionnés dans `ctx.params`.
2. **Authentification** : si l'action porte `"auth": true`, le dispatcher vérifie le cookie `session_id` (voir la section Authentification) avant d'aller plus loin.
3. **Choix du plugin** : si le champ `plugin` est non vide, le dispatcher en extrait le nom à partir du chemin du `.so`, puis cherche ce nom dans sa table de plugins. Un plugin absent produit une erreur HTTP 500 (« Plugin non chargé »).
4. **Exécution** : le dispatcher appelle `plugin.execute(&ctx, &state)`. L'`ActionContext` transporte tout ce dont le plugin a besoin (`sql`, `collection`, `params`...), si bien que le plugin ignore tout de la route.
5. **Traitement du résultat** : selon le `PluginResult` renvoyé (`Data`, `Error`, `AuthSuccess`, `AuthError`, `AuthLogout`), le dispatcher rend une vue, renvoie du JSON, redirige (HTTP 303) ou affiche une erreur.

Une action sans clé `plugin` ni `sql` reste possible : le dispatcher construit alors lui-même un formulaire à partir de la clé `fields` (voir plus bas).

> **Concurrence** : `Plugin::execute` est synchrone. Le dispatcher l'exécute dans `spawn_blocking` (pool bloquant de tokio) pour ne pas bloquer les threads de Tide. Voir [Threads et cycle de vie](#threads-et-cycle-de-vie).

### Enveloppe des données avant le rendu

Avant d'appeler Handlebars, le dispatcher normalise ce que le plugin a renvoyé :

| Donnée reçue du plugin | Traitement |
|---|---|
| Tableau SQL | enveloppé dans `{ data, row_link, row_link_col }` |
| Objet personnalisé (health, auth) | passe intact |
| Objet qui contient déjà une clé `data` (formGeneric enrichi) | passe intact |
| Action avec `form_action` | injection de `form_action`, `form_columns` et des champs pleine largeur dans l'enveloppe |

### Redirection après login et paramètre `next`

Après un login réussi, la cible est `LOGIN_REDIRECT` si `next` est vide, sinon `sanitize_redirect(next)`. Une valeur comme `//youtube.com`, `http://...`, `\evil` ou `javascript:...` est refusée et remplacée par `/` (voir [Sécurité](#sécurité-srcapp_securityrs)).

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
| `fields_meta` | objet | `{}` | Métadonnées d'affichage par colonne SQL : `label`, `type`, `fullwidth`, `readonly`, `options` |
| `fields` | array | `[]` | Formulaire sans plugin ni SQL : liste **ordonnée** de champs (`name` + mêmes clés que `fields_meta` + `value`) |
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

Le mode (`insert` / `update`) est détecté par le JavaScript de `formGeneric.hbs` à partir du **chemin** de `form_action` (`new URL(form.action).pathname`), il ne dépend donc ni du port ni du nom de domaine.

Quand le formulaire contient un champ fichier, le même script ajoute automatiquement `enctype="multipart/form-data"` à la balise `<form>` (sinon `plugin_sql_upload` reçoit du `application/x-www-form-urlencoded` et répond *« Content-Type multipart/form-data sans boundary »*). Un formulaire sans champ fichier garde l'encodage par défaut, que `plugin_sql` sait lire.

---

## Formulaires pilotés par la config : `fields_meta` et `fields`

Deux clés optionnelles décrivent l'affichage des champs de `formGeneric.hbs` sans toucher au Rust ni au template.

### Clés d'un champ

| Clé | Défaut | Description |
|---|---|---|
| `label` | nom de la colonne | Texte affiché devant le champ |
| `type` | `text` | `text`, `number`, `date`, `email`, `password`, `textarea`, `file` |
| `fullwidth` | `false` | Pleine largeur en mode 2 colonnes |
| `readonly` | `false` | Lecture seule (un select est rendu `disabled` : sa valeur n'est pas envoyée) |
| `options` | — | Select statique : `[{ "val": "csv", "label": "CSV" }, ...]` |
| `value` | `""` | Valeur initiale (clé `fields` uniquement) |

### Variante 1 — `fields_meta` : le SQL fournit les valeurs

Le SQL donne les colonnes et les valeurs, `fields_meta` ne décrit que l'affichage. Les colonnes absentes de `fields_meta` gardent leur nom comme label et le type `text`. Fonctionne en création (valeurs vides) comme en modification.

```json
"GET/view/ia_dataset/:id": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT name, version, format, nb_examples FROM ia_datasets WHERE id = :id",
    "view": "generics/formGeneric.hbs",
    "form_action": "/update_ia_dataset",
    "form_columns": 2,
    "return_type": "html",
    "fields_meta": {
        "name":        { "label": "Nom du dataset" },
        "format":      { "label": "Format", "options": [
                          { "val": "csv", "label": "CSV" }, { "val": "jsonl", "label": "JSONL" } ] },
        "nb_examples": { "label": "Nombre d'exemples", "type": "number" }
    }
}
```

Si un champ est déjà défini par `data_resources`, le select SQL est prioritaire sur `options`.

### Variante 2 — `fields` : formulaire sans plugin ni base

Sans clé `plugin` ni `sql`, le dispatcher construit le formulaire lui-même. `fields` est un tableau, donc l'ordre d'affichage est garanti. `form_action` est obligatoire.

```json
"GET/view/ia_datasets/add": {
    "view": "generics/formGeneric.hbs",
    "return_type": "html",
    "form_action": "/insert_ia_dataset",
    "form_columns": 2,
    "fields": [
        { "name": "name",        "label": "Nom du dataset" },
        { "name": "format",      "label": "Format", "options": [
                                   { "val": "csv", "label": "CSV" }, { "val": "jsonl", "label": "JSONL" } ] },
        { "name": "nb_examples", "label": "Nombre d'exemples", "type": "number" },
        { "name": "description", "label": "Description", "type": "textarea", "fullwidth": true }
    ]
}
```

La route `POST` associée reste une action `plugin_sql` classique :

```json
"POST/insert_ia_dataset": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "INSERT INTO ia_datasets (name, format, nb_examples, description) VALUES (:name, :format, NULLIF(:nb_examples,''), :description)",
    "return_type": "redirect",
    "redirect_to": "/view/ia_datasets"
}
```

### Points d'attention

- En mode `insert_XXX`, le script de `formGeneric.hbs` vide les champs texte : une `value` par défaut y est effacée.
- Un champ `file` demande `enctype="multipart/form-data"` (ajouté automatiquement par le script) et une route `plugin_sql_upload`.
- Serde ignore silencieusement les clés inconnues : une faute (`field` au lieu de `fields`) ne produit aucune erreur.
- Toute modification de `dispatcher.rs` exige `cargo build --release --all` puis un redémarrage ; `config_actions.json` et les `.hbs` ne demandent qu'un redémarrage.
- Pour conserver l'ordre des colonnes SQL dans `fields_meta`, activer `serde_json = { version = "1", features = ["preserve_order"] }`.

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

### `plugin_core::db` — runtime et pool MySQL propres à chaque plugin

```rust
pub struct SqlContext { pub rt: Runtime, pub pool: MySqlPool }

// Un contexte par plugin (chaque .so a sa propre copie de la static)
pub fn sql_ctx() -> Result<&'static SqlContext, String>
```

- Initialisation **paresseuse** (`OnceLock`) au premier appel, à partir de `DATABASE_URL`.
- Runtime Tokio multi-thread à **2 workers** (`plugin-sql-rt`).
- Pool sqlx limité par `PLUGIN_DB_MAX_CONNECTIONS` (défaut **5**), `acquire_timeout` de 5 s, `SET NAMES utf8mb4` à chaque connexion.
- Dépendance dans `plugin-core/Cargo.toml` : `tokio = { version = "1", features = ["rt", "rt-multi-thread", "time", "net"] }` et `pub mod db;` dans `plugin-core/src/lib.rs`.

---

## Règles critiques FFI cdylib

### Edition Rust
```
Cargo.toml principal (binaire small-folks) → edition = "2024"
Cargo.toml des plugins (cdylib)            → edition = "2021"
```
En edition 2024, `#[no_mangle]` devient `#[unsafe(no_mangle)]`. Les plugins restent en 2021.

### Règle d'or : chaque plugin possède son runtime et son pool

Chaque `.so` embarque **sa propre copie statique** de `tokio` et de `parking_lot`. Si un plugin utilise le `Handle` ou le `MySqlPool` de l'hôte (`state.handle`, `state.pool`), le code de tokio du plugin manipule des structures (verrous, files de réveil) créées par une **autre copie** de tokio. Sous charge, des réveils sont perdus et le serveur se fige : timeouts, `Bad Gateway` derrière nginx, threads Tide tous bloqués dans `block_on`.

Règle : **un plugin ne touche jamais au runtime ni au pool de l'hôte.** Il crée les siens, dans son propre `.so`.

```rust
// ❌ À NE PLUS FAIRE (plugin lié au runtime et au pool de l'hôte)
tokio::task::block_in_place(|| {
    state.handle.block_on(async {
        sqlx::query(...).fetch_all(&state.pool).await
    })
})

// ✅ plugin_sql, plugin_auth, plugin_upload, plugin_sql_upload, plugin_health
let c = match plugin_core::db::sql_ctx() {
    Ok(c)  => c,
    Err(e) => return PluginResult::Error(e),
};
let pool = c.pool.clone();
c.rt.block_on(async move {
    sqlx::query(...).fetch_all(&pool).await
})
```

| Plugin | Runtime | Client / pool |
|---|---|---|
| `plugin_sql`, `plugin_auth`, `plugin_upload`, `plugin_sql_upload` | `plugin_core::db` (2 workers) | `MySqlPool` de `plugin_core::db` |
| `plugin_health` | `plugin_core::db` (MySQL) + `HEALTH_RT` (MongoDB) | pool `plugin_core::db` + client Mongo |
| `plugin_mongo` | `MONGO_RT` | client MongoDB créé **dans** `MONGO_RT` |

Conséquences :
- `state` n'est plus utilisé par les plugins SQL (le paramètre devient `_state`), sauf `plugin_health` qui lit `state.sessions`.
- Plus de `block_in_place` : `execute()` est appelé depuis un thread Tide, pas depuis un worker Tokio.
- Le nombre total de connexions MySQL = (`PLUGIN_DB_MAX_CONNECTIONS` × nombre de plugins SQL) + pool de l'hôte. Avec 5 plugins SQL et 5 connexions chacun, prévoir au moins ~30 connexions côté MariaDB (`max_connections`).

### plugin_mongo — MongoContext autonome (CRITIQUE)
Le client MongoDB DOIT être créé dans `MONGO_RT`, pas dans `main.rs`. Sinon son heartbeat est affamé par le runtime principal → latence 1-8 s. Même principe pour `HealthContext` / `HEALTH_RT`.

### Threads et cycle de vie

![Schéma des threads et de leur cycle de vie](https://mascaron.net/schema_threads_small_folks.svg)

Le processus contient quatre familles de threads :

| Famille | Threads | Rôle | Durée de vie |
|---|---|---|---|
| **Tide (async-std)** | un par cœur (`ASYNC_GLOBAL_EXECUTOR_THREADS` pour changer) | accepter la connexion, résoudre la route, vérifier l'auth, lire le body, rendre la réponse | créés au premier `spawn`, jusqu'à l'arrêt |
| **Pool bloquant de tokio (hôte)** | à la demande (512 max par défaut) | exécuter `plugin.execute()`, code **synchrone** du `.so` | créés selon la charge, détruits après ~10 s d'inactivité |
| **Runtime de chaque plugin** | 2 workers par runtime (`plugin_core::db`, `MONGO_RT`, `HEALTH_RT`) | E/S asynchrones vers MySQL / MongoDB | créés au premier appel du plugin (`OnceLock`), jusqu'à l'arrêt |
| **Workers de fond** | tâches du runtime tokio de l'hôte | `mqtt_worker`, `ebpf_worker` (boucle de 5 s) | démarrés au lancement si activés |

Trajet d'une requête : Tide reçoit et prépare la requête → le dispatcher confie `plugin.execute()` au pool bloquant avec `spawn_blocking(...).await` → ce thread bloquant attend, via `rt.block_on`, le runtime propre au plugin qui interroge la base → le résultat remonte, le thread Tide reprend la requête, rend la vue et répond.

#### Pourquoi `spawn_blocking` dans le dispatcher

`Plugin::execute` est synchrone (contrainte FFI). Appelé directement depuis Tide, il bloquerait un des threads de l'exécuteur async-std pendant toute la requête : sur un petit VPS (2 cœurs), deux requêtes lentes suffiraient à figer le serveur. Le dispatcher exécute donc le plugin dans le pool bloquant de tokio :

```rust
let plugin  = Arc::clone(plugin);          // plugins: HashMap<String, Arc<dyn Plugin>>
let state   = req.state().clone();
let handle  = state.handle.clone();        // Handle du runtime tokio de l'hôte
let ctx_job = ctx.clone();
let result = match handle
    .spawn_blocking(move || plugin.execute(&ctx_job, &state))
    .await
{
    Ok(r)  => r,
    Err(e) => { /* le plugin a paniqué */ return self.render_error(500, "Erreur interne du plugin"); }
};
```

Conséquences :
- Les threads Tide ne restent occupés que pour le rendu : `ASYNC_GLOBAL_EXECUTOR_THREADS` n'est plus nécessaire.
- La concurrence est limitée par les pools de connexions des plugins (`PLUGIN_DB_MAX_CONNECTIONS`, 5 par défaut), plus par le nombre de cœurs.
- Une panique dans un plugin produit une réponse `500` au lieu de perdre un thread du serveur.
- `ctx.clone()` copie aussi `body_bytes` : pour un upload de 10 Mo, une copie de 10 Mo par requête.
- Il n'y a pas de rechargement à chaud des `.so` : recompiler un plugin impose de redémarrer le serveur (le remplacer pendant qu'il tourne peut le faire planter).

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

### Où sont définies les requêtes SQL ?

Les requêtes SQL ne sont pas écrites en dur dans le code Rust. Elles sont
externalisées dans `config_actions.json` : chaque action (route) possède un
champ `"sql"` contenant la requête, avec des paramètres nommés `:param`.

Le dispatcher lit cette configuration, puis le plugin (`plugin_sql`, `plugin_auth`)
exécute la requête fournie dans son contexte (`ctx.sql`). Les paramètres nommés
sont convertis en `?` positionnels par `plugin_core::named_to_positional`.

Modifier une requête SQL ne demande donc aucune recompilation du plugin : un simple
redémarrage du serveur suffit. Les fichiers du dossier `sql/` servent uniquement à
créer les tables (`create_*.sql`), pas à définir les requêtes des actions.

Exemple :

```json
"GET/capteurs/chart/:sensor_id": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT ... FROM capteurs WHERE sensor_id = :sensor_id",
    "view": "specifics/chart_capteurs.hbs"
}
```

### Templates Handlebars Rust
- `{{this}}` et non `{{.}}`
- `{{#each data}}{{#if @first}}{{#each this}}<th>{{@key}}</th>{{/each}}{{/if}}{{/each}}` pour les headers
- `{{#each data.0.fields}}` + `{{key}}`, `{{value}}`, `{{label}}`, `{{#if fullwidth}}` pour `formGeneric`
- `{{#each (lookup @root.resources key)}}` + `{{val}}`, `{{label}}`, `{{#if selected}}` pour les selects
- `../../../@key` interdit — Handlebars Rust ne supporte pas la remontée profonde
- `enctype="multipart/form-data"` obligatoire sur les formulaires upload (guillemet fermant !)

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

Côté Rust, ce helper est une structure `CompareHelper` qui implémente `HelperDef`, avec les lifetimes `'reg: 'rc` obligatoires. Une simple closure ne compile pas pour ce cas.

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
| `sanitize_redirect(url)` | Open Redirect | `http://evil.com`, `//evil.com`, `/\evil.com` → `/` |
| `sanitize_header(val)` | Header Splitting | `val\r\nX-Evil:` → `valX-Evil:` |
| `sanitize_log(msg)` | Log Injection | `msg\n[FAKE]` → `msg [FAKE]` |
| `sanitize_html(input)` | XSS hors-template | `<script>` → `&lt;script&gt;` |

### Open Redirect (paramètre `next`)

`/login?next=//youtube.com` était accepté par une version antérieure (une URL qui commence par `//` est interprétée par le navigateur comme une URL absolue de même schéma). `sanitize_redirect` :

1. supprime les caractères de contrôle (`\t`, `\r`, `\n`... que les navigateurs ignorent dans une URL) ;
2. exige une URL **relative** qui commence par `/` et pas par `//` ;
3. refuse le backslash (`/\evil.com` est normalisé en `//evil.com` par les navigateurs), les `:` (schémas `javascript:`, `data:`), `<script` ;
4. renvoie `/` dans tous les autres cas et journalise la tentative.

```rust
pub fn sanitize_redirect(url: &str) -> String {
    let cleaned: String = url.chars().filter(|c| !c.is_control()).collect();
    let url = cleaned.trim();
    if url.is_empty() { return "/".to_string(); }
    let lower = url.to_lowercase();
    let is_safe = url.starts_with('/') && !url.starts_with("//") && !url.contains('\\')
        && !lower.contains(':') && !lower.contains("javascript") && !lower.contains("<script");
    if is_safe { url.to_string() } else {
        eprintln!("[security] Open Redirect bloqué : '{}'", sanitize_log(url));
        "/".to_string()
    }
}
```

### Pistes de durcissement (hors périmètre pédagogique actuel)

- Hachage des mots de passe (argon2/bcrypt) au lieu de la comparaison SQL directe.
- Attributs `Secure`, `HttpOnly` et `SameSite=Lax` sur **tous** les cookies (le `jwt_token` n'a aujourd'hui ni `Secure` ni `HttpOnly`).
- Limitation du nombre de tentatives de login (rate limiting), en complément du firewall XDP.
- En-têtes de sécurité (`Content-Security-Policy`, `X-Content-Type-Options`, `X-Frame-Options`).
- Limite de taille du corps des requêtes et masquage des messages d'erreur SQL côté client.
- Alerte au démarrage pour les routes d'écriture (`POST`) sans `"auth": true`.

Tests unitaires : `cargo test -p small-folks`

---

## Authentification (plugin_auth)

### Fonctionnement

L'authentification repose sur trois opérations de `plugin_auth`, choisies par le champ `operation` de l'action : `login`, `logout` et `me`.

1. **Login** : le plugin exécute la requête `ctx.sql` de l'action (voir plus bas) avec le `login` et le mot de passe reçus (champs de formulaire `login` et `mdp`). Si une ligne est trouvée, il génère un **JWT** signé en HS256 (clé `JWT_SECRET`) et un identifiant de session. Il renvoie `AuthSuccess`, et le dispatcher pose deux cookies : `session_id` (en `HttpOnly`, donc inaccessible à JavaScript) et `jwt_token`. Si aucune ligne n'est trouvée, `AuthError` redirige vers `/login?error=1`.
2. **Sessions** : elles sont conservées en mémoire du serveur (`Arc<Mutex<HashMap>>`) et expirent après `SESSION_TTL_SECONDS`. Un redémarrage du serveur les efface donc toutes.
3. **Contrôle d'accès** : pour toute action portant `"auth": true`, le dispatcher lit le cookie `session_id` (injecté dans `ctx.params`) et vérifie qu'il correspond à une session non expirée. Sinon, il refuse l'accès ou redirige vers `/login`.
4. **Logout** : `AuthLogout` supprime la session et redirige vers `redirect_to`.

Le JWT contient l'identifiant et le nom de l'utilisateur, sa fonction et les dates d'émission et d'expiration (`iat`, `exp`), repris des alias SQL ci-dessous.

Avec Tide, plusieurs cookies dans une même réponse se posent avec `insert_header` pour le premier, puis `append_header` pour les suivants. Appeler `header()` deux fois écraserait le premier cookie.

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

### Tester le login avec curl

```bash
# 1. Se connecter et stocker les cookies dans un fichier (-c)
read -s -p "Mot de passe : " PW; echo
curl -i -c /tmp/cj -d "login=steph" --data-urlencode "mdp=$PW" http://localhost:8080/login
#   → 303 + Location: /index + Set-Cookie: session_id=... et jwt_token=...
#   → 303 + Location: /login?error=1 si les identifiants sont faux

# 2. Réutiliser la session (-b) sur une route protégée
curl -s -o /dev/null -w "%{http_code} %{redirect_url}\n" -b /tmp/cj http://localhost:8080/users
#   → 200 si la session est valide, 303 vers /login sinon

# 3. Nettoyer
rm /tmp/cj
```

Un cookie marqué `Secure` n'est renvoyé par curl qu'en HTTPS. Les noms des champs (`login`, `mdp`) sont ceux de `templates/generics/login.hbs`. Les sessions sont en mémoire du serveur : après un redémarrage, il faut se reconnecter.

---

## Encodage UTF-8

Deux points garantissent que les accents et caractères spéciaux traversent correctement le framework :

- **Connexion MySQL** : chaque pool sqlx (hôte et plugins) exécute `SET NAMES utf8mb4 COLLATE utf8mb4_unicode_ci` à chaque nouvelle connexion (`after_connect`).
- **Données de formulaire** : `urlencoding_decode` accumule les octets décodés dans un `Vec<u8>`, puis les convertit en une seule fois avec `String::from_utf8_lossy`. Convertir octet par octet (`byte as char`) corromprait les caractères sur plusieurs octets (é, à, €...).

---

## Upload de fichiers

Les deux plugins d'upload utilisent `plugin_core::db` (runtime et pool propres au plugin), comme `plugin_sql`.

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

Le dossier `uploads/` doit exister au démarrage, car il est servi par `serve_dir`. Comme Git ne versionne pas les dossiers vides, ajoutez un fichier `uploads/.gitkeep`.

Derrière nginx, `client_max_body_size` (1 Mo par défaut) doit être au moins égal à `max_size_mb`, sinon la réponse est `413`.

Si un formulaire poste vers une route `plugin_sql_upload` sans `enctype="multipart/form-data"`, le plugin répond *« Content-Type multipart/form-data sans boundary »* : le navigateur a envoyé du `application/x-www-form-urlencoded`, qui n'a pas de boundary.

### Tester un upload avec curl

```bash
# Le nom du champ (file) doit être celui de la route ; le fichier doit exister sur la machine qui lance curl
for i in $(seq 1 20); do
  curl -s -o /dev/null --max-time 10 -b /tmp/cj \
    -w "%{http_code} %{redirect_url}\n" \
    -F "file=@/tmp/t.png;type=image/png" http://localhost:8080/upload &
done | sort | uniq -c; wait
#   → 20 × "303 .../uploads" attendu. 000 = fichier local introuvable ou serveur bloqué.
```

---

## Health check (plugin_health)

```
GET /health              → JSON brut
GET /health/dashboard    → HTML dashboard (auth: true conseillé)
```

`plugin_health` combine deux contextes autonomes : le ping **MySQL** passe par `plugin_core::db` (runtime et pool propres au plugin) et le ping **MongoDB** par `HealthContext` / `HEALTH_RT`. Aucun des deux n'utilise le runtime ou le pool de l'hôte. Il lit aussi `state.sessions` pour les statistiques de sessions.

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

## Graphiques Chart.js (chart_capteurs.hbs)

Le template `specifics/chart_capteurs.hbs` affiche les mesures des capteurs sous forme de courbes **température** et **humidité** (Chart.js), avec des sélecteurs de capteur et de période et des statistiques min / moyenne / max.

Deux routes l'alimentent, qui utilisent le même template :

- `GET/capteurs/chart` : les mesures de tous les capteurs (voir « Routes suggérées » ci-dessus) ;
- `GET/capteurs/chart/:sensor_id` : les mesures d'un seul capteur, triées par date croissante.

```json
"GET/capteurs/chart/:sensor_id": {
    "plugin":      "./target/release/libplugin_sql.so",
    "sql":         "SELECT CAST(timestamp AS CHAR) AS timestamp, CAST(temperature AS CHAR) AS temperature, CAST(humidity AS CHAR) AS humidity, sensor_id FROM capteurs WHERE sensor_id = :sensor_id ORDER BY timestamp ASC",
    "view":        "specifics/chart_capteurs.hbs",
    "return_type": "html",
    "auth":        true
}
```

Les données SQL sont transmises au JavaScript de la page avec le helper `json` en **triple accolades** : `const data = {{{json data}}};`. Avec deux accolades, Handlebars échapperait les guillemets et le JSON deviendrait invalide.

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
  ├─ Lit STATS  → logs (toutes les EBPF_STATS_LOG_SECS secondes)
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

Dans le programme kernel, une map de type `Array` n'offre ni `set()` ni `insert()`. On écrit une valeur en récupérant un pointeur avec `get_ptr_mut()`, puis en affectant `*ptr = valeur`.

### Compilation du programme kernel

```bash
# Prérequis
rustup component add rust-src
cargo binstall bpf-linker        # (cargo install bpf-linker exige llvm-config : préférer cargo-binstall)

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
EBPF_STATS_LOG_SECS=60     # 0 = ne jamais afficher les statistiques périodiques

# Lancer avec les droits nécessaires
sudo ./target/release/small-folks
```

**Droits nécessaires** : chargement d'un programme XDP et création des maps BPF.
- Noyau **≥ 5.8** : `CAP_BPF` + `CAP_NET_ADMIN` (+ `CAP_PERFMON`) suffisent.
- Noyau **< 5.8** (ex. Ubuntu 20.04, noyau 5.4) : `CAP_BPF` n'existe pas, il faut `CAP_SYS_ADMIN` + `CAP_NET_ADMIN`, ou root.
- Sous systemd, l'erreur `failed to create map BLACKLIST with code -1` vient d'une limite `RLIMIT_MEMLOCK` trop basse : ajouter `LimitMEMLOCK=infinity` à l'unité.

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
HOST=0.0.0.0                 # adresse d'écoute : 0.0.0.0 = toutes les interfaces, 127.0.0.1 = local seul
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

# Pools et threads
PLUGIN_DB_MAX_CONNECTIONS=5          # connexions MySQL PAR plugin SQL (défaut : 5)
# ASYNC_GLOBAL_EXECUTOR_THREADS=4    # optionnel : threads de l'exécuteur Tide (défaut : nb de cœurs).
#                                    # Peu utile depuis spawn_blocking dans le dispatcher.

# MQTT (optionnel)
# MQTT_BROKER_URL=localhost
# MQTT_BROKER_PORT=1883
# MQTT_CLIENT_ID=small-folks-mqtt
# MQTT_TOPICS=sensors/#
# MQTT_QOS=1

# eBPF Firewall XDP (optionnel — root ou capabilities, voir la section eBPF)
# EBPF_ENABLED=true
# EBPF_INTERFACE=eth0
# EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
# EBPF_RATE_LIMIT=100
# EBPF_WINDOW_SECS=60
# EBPF_AUTO_UNBLOCK_SECS=300
# EBPF_STATS_LOG_SECS=60
```

Le serveur est joignable depuis l'extérieur si `HOST=0.0.0.0` (ou l'IP publique) et si le port est ouvert dans le pare-feu. Derrière nginx, préférer `HOST=127.0.0.1`.

---

## Compilation et lancement

### Prérequis (Ubuntu / Debian)

```bash
sudo apt install build-essential pkg-config libssl-dev    # openssl-sys
# eBPF (optionnel) :
rustup toolchain install nightly && rustup component add rust-src --toolchain nightly
cargo install cargo-binstall && cargo binstall bpf-linker
```

### Compiler et lancer

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

- Les `.so` dans `config_actions.json` doivent pointer vers `./target/release/`. Mélanger binaire release et `.so` debug provoque un coredump.
- **Arrêter le serveur avant de recompiler** : remplacer un `.so` chargé par un serveur en marche peut le faire planter.
- Modification de code Rust (`src/`, `plugins/`) → recompilation obligatoire. Modification de `config_actions.json` ou d'un `.hbs` → simple redémarrage.

### Service systemd

`/etc/systemd/system/small-folks.service` :

```ini
[Unit]
Description=small-folks (framework Rust / Tide)
After=network-online.target mariadb.service
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/small-folks
ExecStart=/opt/small-folks/target/release/small-folks
Restart=on-failure
RestartSec=3
User=root
LimitNOFILE=65535
LimitMEMLOCK=infinity
# Sans root, noyau >= 5.8 :
# AmbientCapabilities=CAP_BPF CAP_NET_ADMIN CAP_PERFMON
# Sans root, noyau 5.4 :
# AmbientCapabilities=CAP_SYS_ADMIN CAP_NET_ADMIN

[Install]
WantedBy=multi-user.target
```

`User=root` est nécessaire si l'eBPF est activé et qu'aucune capability n'est accordée. `LimitMEMLOCK=infinity` est indispensable pour les maps eBPF.

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now small-folks
journalctl -u small-folks -f
```

### Derrière nginx

```nginx
location / {
    proxy_pass         http://127.0.0.1:8080;
    proxy_set_header   Host $host;
    proxy_set_header   X-Real-IP $remote_addr;
    proxy_set_header   X-Forwarded-Proto $scheme;
    client_max_body_size 10m;      # ≥ max_size_mb des uploads
}
```

---

## Tests de charge et de concurrence

Procédure utilisée pour valider le modèle « un runtime et un pool par plugin ».

```bash
# 20 requêtes simultanées sur une route SQL
for i in $(seq 1 20); do
  curl -s -o /dev/null --max-time 10 -w "%{http_code} %{time_total}s\n" http://localhost:8080/users &
done; wait

# 200 requêtes, comptage des codes HTTP
for i in $(seq 1 200); do curl -s -o /dev/null --max-time 10 -w "%{http_code}\n" http://localhost:8080/users & done | sort | uniq -c; wait

# Mélange de routes (SQL, login, health)
for i in $(seq 1 200); do
  curl -s -o /dev/null --max-time 10 -w "%{http_code}\n" http://localhost:8080/users &
  curl -s -o /dev/null --max-time 10 -w "%{http_code}\n" -d "login=x" -d "mdp=y" http://localhost:8080/login &
done | sort | uniq -c; wait
```

| Scénario | Avant correctif | Après correctif |
|---|---|---|
| 20 × `/users` en parallèle (VPS 2 cœurs) | 2 réponses sur 20, le reste en timeout | 20 × `200` |
| 200 × `/users`, `/login`, `/health` | blocage / `Bad Gateway` | 100 % de succès |
| 600 requêtes mélangées | — | 600 / 600 |
| 20 × `/users` via nginx + HTTPS | — | 20 × `200`, 0,29–0,45 s |
| 20 × `POST /upload` avec session | — | 20 × `303` vers `/uploads` |

**Lecture des codes** : `000` = pas de réponse HTTP (serveur arrêté, requête bloquée, ou fichier local introuvable avec `-F file=@...`) ; `303` vers `/login` = session absente ; `413` = `client_max_body_size` nginx trop bas ; `502/504` = serveur bloqué derrière nginx.

**Diagnostiquer un blocage** : pendant que les requêtes sont en attente, capturer les piles de tous les threads.

```bash
sudo gdb -p $(pgrep -f target/release/small-folks | head -1) -batch \
  -ex "set pagination off" -ex "thread apply all bt" > /tmp/gdb_trace.txt 2>&1
```

Des threads `async-std` tous bloqués dans `block_on` / `parking_lot` sont le signe d'un plugin qui partage le runtime ou le pool de l'hôte.

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
