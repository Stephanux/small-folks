# small-folks — Configuration-Driven Rust MVC Framework ([version française](https://github.com/Stephanux/small-folks/blob/main/README.md))

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="Rust Framework small-Folks logo" />
</p>

**Rust** web framework built on **Tide** with a dynamic plugin system (`cdylib`).
Routes, SQL/MongoDB queries, views and resources are entirely configured in `config_actions.json` **without recompiling**.

> **NB:** Small-Folks is developed for **teaching purposes** (IUT Networks & Telecommunications). It is not intended for production use.

---

## Table of contents

1. [Tech stack](#tech-stack)
2. [Workspace architecture](#workspace-architecture)
3. [HTTP request flow](#http-request-flow)
4. [The dispatcher in detail](#the-dispatcher-in-detail)
5. [config_actions.json — full reference](#config_actionsjson--full-reference)
6. [Config-driven forms](#config-driven-forms-fields_meta-and-fields)
7. [plugin-core — shared types (FFI)](#plugin-core--shared-types-ffi)
8. [Critical cdylib FFI rules (one runtime and pool per plugin)](#critical-cdylib-ffi-rules)
9. [Handlebars helpers](#handlebars-helpers-srchelpers_hbsrs)
10. [Security](#security-srcapp_securityrs)
11. [Authentication](#authentication-plugin_auth)
12. [File uploads](#file-uploads)
13. [Health check, MQTT, Chart.js, eBPF](#health-check-plugin_health)
14. [Environment variables](#environment-variables-env)
15. [Build, run and deploy (systemd)](#build-and-run)
16. [Load and concurrency tests](#load-and-concurrency-tests)
17. [Performance](#performance-release-localhost)

---

## Tech stack

| Component | Version | Role |
|---|---|---|
| Rust | 2021 edition | language |
| Tide | 0.16 | async HTTP server (async-std executor) |
| Tokio | 1 | multi-thread async runtime (one per plugin + one for the binary) |
| sqlx | 0.7 | MySQL access, prepared statements |
| mongodb | 3.5 | zero-copy MongoDB driver (RawDocumentBuf) |
| Handlebars | 6.4 | HTML templates with partials |
| libloading | 0.8 | dynamic loading of `.so` files |
| jsonwebtoken | 9 | HS256 JWT for authentication |
| multer | 3 | multipart/form-data parsing |
| uuid | 1 | unique identifier generation |
| sysinfo | 0.30 | RAM, disk, uptime metrics (plugin_health) |
| rumqttc | 0.24 | pure-Rust MQTT client (mqtt_worker) |
| aya | 0.12 | eBPF userspace framework (ebpf_worker) |
| libc | 0.2 | system calls (RLIMIT_MEMLOCK for eBPF) |
| chrono | 0.4 | dates/timestamps |

---

## Workspace architecture

### Modules overview

![Architecture and modules diagram of small-folks](https://mascaron.net/small-folks_sch%C3%A9ma_architecture_et_modules.png)

> The diagram labels are in French. Its "Frontière cdylib" box dates from before the runtime-per-plugin design; see [Critical cdylib FFI rules](#critical-cdylib-ffi-rules) for the current model.

### Directory tree

```
small-folks/
├── README.md                        ← French version
├── README_EN.md                     ← this file
├── config_actions.json              ← route directory
├── .env                             ← environment variables
├── .env.example                     ← environment variables template
├── Cargo.toml                       ← Rust workspace (edition 2024 for the binary)
├── dump/
│   └── R504TP_2026_04_23_dump       ← SQL dump of the demo database
├── src/
│   ├── main.rs                      ← startup, pools, plugin precaching
│   ├── dispatcher.rs                ← route resolution, rendering, auth
│   ├── app_security.rs              ← XSS, open redirect, header injection protection
│   ├── helpers_hbs.rs               ← custom Handlebars helpers
│   ├── mqtt_worker.rs               ← background MQTT client (sensor storage)
│   └── ebpf_worker.rs               ← userspace XDP firewall (optional, needs root/capabilities)
├── ebpf-firewall/                   ← kernel XDP program (separate crate, bpfel-unknown-none)
│   ├── Cargo.toml
│   ├── rust-toolchain.toml          ← nightly + rust-src (required for bpfel-unknown-none)
│   ├── .cargo/config.toml           ← bpfel-unknown-none target + build-std=core
│   └── src/main.rs                  ← XDP program: SYN counting + blacklist DROP
├── plugins/                         ← all plugins, edition 2021
│   ├── plugin-core/
│   │   └── src/
│   │       ├── lib.rs               ← traits, shared types (FFI) + named_to_positional
│   │       └── db.rs                ← Tokio runtime + MySQL pool OWNED BY EACH PLUGIN
│   ├── plugin_sql/src/lib.rs        ← generic SQL + resources + form
│   ├── plugin_mongo/src/lib.rs      ← MongoDB with a standalone MongoContext
│   ├── plugin_auth/src/lib.rs       ← login / logout / JWT / sessions
│   ├── plugin_upload/src/lib.rs     ← multipart upload → disk + MySQL
│   ├── plugin_sql_upload/src/lib.rs ← text form + file → SQL + disk
│   └── plugin_health/src/lib.rs     ← server metrics + DB ping
├── templates/
│   ├── generics/                    ← reusable templates
│   │   ├── tableGeneric.hbs         ← data table + optional link (row_link)
│   │   ├── formGeneric.hbs          ← generic form (inputs + selects + selected)
│   │   ├── listeGeneric.hbs         ← generic <select> list
│   │   ├── health_dashboard.hbs     ← server health dashboard
│   │   ├── login.hbs                ← login page
│   │   ├── upload_form.hbs          ← upload form
│   │   ├── upload_list.hbs          ← uploaded files list
│   │   ├── index.hbs                ← home page
│   │   ├── error.hbs                ← error page
│   │   └── success.hbs              ← success page
│   ├── partials/                    ← reusable fragments
│   │   ├── header.hbs
│   │   ├── nav.hbs
│   │   └── footer.hbs
│   └── specifics/                   ← project-specific templates
│       ├── chart_capteurs.hbs       ← Chart.js temperature + humidity curves
│       ├── form_user.hbs
│       └── form_countries.hbs
├── public/
│   ├── css/styles.css               ← global styles + forms + dashboard
│   └── images/                      ← favicon, logos
├── uploads/                         ← uploaded files (UUID.ext)
├── resources/                       ← framework logos and diagrams
└── sql/
    ├── create_countries.sql
    ├── create_uploads.sql
    ├── create_capteurs.sql          ← MQTT sensors table
    └── create_ebpf_blacklist.sql    ← eBPF/XDP blacklist table
```

---

## HTTP request flow

![Architecture diagram](https://mascaron.net/schema_architecture_small_folksv6.png)

```
HTTP client
  ↓ GET /regions  (session_id cookie present)
Tide — catch-all /*
  ↓
Dispatcher
  ├─ resolve_action("GET", "/regions") → config_actions.json
  ├─ extract URL params + query string + body
  ├─ inject session_id cookie → ctx.params
  ├─ auth check (if "auth": true in config)
  │     → session cache → OK or 401/redirect /login
  ↓
spawn_blocking → plugin_sql.execute(ctx, state)   ← synchronous (cdylib FFI constraint),
  │                                                  on a blocking-pool thread
  ├─ plugin_core::db::sql_ctx()       ← runtime + pool OWNED BY the plugin
  ├─ rt.block_on(main query ctx.sql)
  ├─ resource queries (ctx.sql_resources) if data_resources is defined
  │     → enrichment with { val, label, selected } per option
  └─ PluginResult::Data(json)
  ↓
Dispatcher: rendering according to return_type
  ├─ "html"     → inject row_link + form_action → hbs.render(view, data)
  ├─ "json"     → HTTP 200 application/json
  └─ "redirect" → HTTP 303 Location
  ↓
HTTP response
```

---

## The dispatcher in detail

### How a request is routed to a plugin

The `Dispatcher` (`src/dispatcher.rs`) owns three things: the action directory read from `config_actions.json`, the table of already-loaded plugins (`HashMap<String, Arc<dyn Plugin>>`) and the Handlebars engine. For each request:

1. **Action resolution**: the `"METHOD/path"` key (e.g. `GET/capteurs/chart/:sensor_id`) is looked up in `config_actions.json`. The URL `:param` segments, the query string and the form body are merged into `ctx.params`.
2. **Authentication**: if the action has `"auth": true`, the dispatcher checks the `session_id` cookie (see the Authentication section) before going further.
3. **Plugin selection**: if the `plugin` field is non-empty, the dispatcher extracts the plugin name from the `.so` path, then looks that name up in its plugin table. A missing plugin produces an HTTP 500 error ("Plugin not loaded").
4. **Execution**: the dispatcher calls `plugin.execute(&ctx, &state)`. The `ActionContext` carries everything the plugin needs (`sql`, `collection`, `params`...), so the plugin knows nothing about the route.
5. **Result handling**: depending on the returned `PluginResult` (`Data`, `Error`, `AuthSuccess`, `AuthError`, `AuthLogout`), the dispatcher renders a view, returns JSON, redirects (HTTP 303) or shows an error.

An action with neither a `plugin` nor a `sql` key is still possible: the dispatcher then builds a form itself from the `fields` key (see below).

> **Concurrency**: `Plugin::execute` is synchronous. The dispatcher runs it in `spawn_blocking` (tokio's blocking pool) so Tide's threads are never blocked. See [Threads and lifecycle](#threads-and-lifecycle).

### Data envelope before rendering

Before calling Handlebars, the dispatcher normalizes what the plugin returned:

| Data received from the plugin | Handling |
|---|---|
| SQL array | wrapped in `{ data, row_link, row_link_col }` |
| Custom object (health, auth) | passed through untouched |
| Object that already has a `data` key (enriched formGeneric) | passed through untouched |
| Action with `form_action` | `form_action`, `form_columns` and full-width fields injected into the envelope |

### Post-login redirect and the `next` parameter

After a successful login, the target is `LOGIN_REDIRECT` if `next` is empty, otherwise `sanitize_redirect(next)`. A value such as `//youtube.com`, `http://...`, `\evil` or `javascript:...` is rejected and replaced by `/` (see [Security](#security-srcapp_securityrs)).

---

## config_actions.json — full reference

### Available fields

| Field | Type | Default | Description |
|---|---|---|---|
| `plugin` | string | — | Path to the `.so` |
| `sql` | string | — | SQL query with `:param` (also used by plugin_auth) |
| `sql_upload` | string | — | INSERT query for the uploads table (plugin_sql_upload) |
| `collection` | string | — | MongoDB collection |
| `filter` | string | `{}` | BSON JSON filter with `:param` |
| `operation` | string | `find` | MongoDB or auth operation (`login`, `logout`, `me`, `status`, `dashboard`) |
| `form_action` | string | — | `action=""` URL of the HTML form |
| `form_columns` | number | `1` | Number of form columns: `1` or `2` |
| `form_fullwidth_fields` | array | `[]` | Fields spanning the full width in 2-column mode |
| `data_resources` | object | `{}` | `"column_name" → "resource_name"` for selects |
| `sql_resources` | object | `{}` | `"resource_name" → "SELECT ..."` |
| `row_link` | string | — | Base URL for the link on a `tableGeneric` column (e.g. `/view/animal`) |
| `row_link_col` | number | `1` | Index of the column carrying the link (default: column 1) |
| `fields_meta` | object | `{}` | Per-SQL-column display metadata: `label`, `type`, `fullwidth`, `readonly`, `options` |
| `fields` | array | `[]` | Form without plugin or SQL: **ordered** list of fields (`name` + same keys as `fields_meta` + `value`) |
| `upload_field` | string | — | Name of the file field in the form (plugin_sql_upload) |
| `allowed_mime` | string | `image/jpeg,image/png,application/pdf` | Allowed upload MIME types |
| `max_size_mb` | string | `10` | Max upload size in MB |
| `view` | string | — | Handlebars template name |
| `return_type` | string | `json` | `html`, `json` or `redirect` |
| `redirect_to` | string | `/` | Redirect URL |
| `auth` | bool | `false` | Requires a valid session |

### Template rule by use case

| Case | Template | JSON structure received |
|---|---|---|
| Simple list | `tableGeneric.hbs` | `{ data: [{...}], row_link: "", row_link_col: 1 }` |
| List with link | `tableGeneric.hbs` + `row_link` | `{ data: [{...}], row_link: "/view/x", row_link_col: 1 }` |
| 1-column form | `formGeneric.hbs` + `form_action` | `{ data: [{fields:[{key,value,fullwidth}]}], form_action, form_columns:1 }` |
| 2-column form | `formGeneric.hbs` + `form_columns:2` | same + fullwidth fields flagged `true` |
| Form with selects | same + `data_resources` | same + `resources: { field: [{val,label,selected}] }` |
| Form + file upload | specific template + `enctype="multipart/form-data"` | handled by `plugin_sql_upload` |

### JSON structure sent to tableGeneric.hbs

```json
{
  "data": [
    { "id": "1", "nom": "Lion", "espece": "Panthera leo" }
  ],
  "row_link":     "/view/animal",
  "row_link_col": 1
}
```

Without `row_link` in the config → `row_link: ""` → `{{#if row_link}}` is falsy → table without a link.

### JSON structure sent to formGeneric.hbs

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
      { "val": "DE", "label": "Germany", "selected": false },
      { "val": "FR", "label": "France",  "selected": true  }
    ]
  }
}
```

**`selected` is pre-computed on the Rust side** in `plugin_sql`.

- `insert_XXX` mode → empty values → `selected: false` everywhere → "-- choose --" is shown
- `update_XXX` mode → `selected: true` on the option matching the current value

The mode (`insert` / `update`) is detected by the JavaScript in `formGeneric.hbs` from the **path** of `form_action` (`new URL(form.action).pathname`), so it depends neither on the port nor on the domain name.

When the form contains a file field, the same script automatically sets `enctype="multipart/form-data"` on the `<form>` (otherwise `plugin_sql_upload` would receive `application/x-www-form-urlencoded` and answer *"Content-Type multipart/form-data sans boundary"*). A form without a file field stays in the default encoding, which `plugin_sql` can read.

---

## Config-driven forms: `fields_meta` and `fields`

Two optional keys describe how the `formGeneric.hbs` fields are displayed, without touching the Rust code or the template.

### Field keys

| Key | Default | Description |
|---|---|---|
| `label` | column name | Text displayed in front of the field |
| `type` | `text` | `text`, `number`, `date`, `email`, `password`, `textarea`, `file` |
| `fullwidth` | `false` | Full width in 2-column mode |
| `readonly` | `false` | Read-only (a select is rendered `disabled`: its value is not submitted) |
| `options` | — | Static select: `[{ "val": "csv", "label": "CSV" }, ...]` |
| `value` | `""` | Initial value (`fields` key only) |

### Variant 1 — `fields_meta`: SQL provides the values

SQL provides the columns and values, `fields_meta` only describes the display. Columns absent from `fields_meta` keep their name as label and the `text` type. Works for creation (empty values) as well as for editing.

```json
"GET/view/ia_dataset/:id": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT name, version, format, nb_examples FROM ia_datasets WHERE id = :id",
    "view": "generics/formGeneric.hbs",
    "form_action": "/update_ia_dataset",
    "form_columns": 2,
    "return_type": "html",
    "fields_meta": {
        "name":        { "label": "Dataset name" },
        "format":      { "label": "Format", "options": [
                          { "val": "csv", "label": "CSV" }, { "val": "jsonl", "label": "JSONL" } ] },
        "nb_examples": { "label": "Number of examples", "type": "number" }
    }
}
```

If a field is already defined by `data_resources`, the SQL select takes priority over `options`.

### Variant 2 — `fields`: form without plugin or database

Without a `plugin` or `sql` key, the dispatcher builds the form itself. `fields` is an array, so display order is guaranteed. `form_action` is mandatory.

```json
"GET/view/ia_datasets/add": {
    "view": "generics/formGeneric.hbs",
    "return_type": "html",
    "form_action": "/insert_ia_dataset",
    "form_columns": 2,
    "fields": [
        { "name": "name",        "label": "Dataset name" },
        { "name": "format",      "label": "Format", "options": [
                                   { "val": "csv", "label": "CSV" }, { "val": "jsonl", "label": "JSONL" } ] },
        { "name": "nb_examples", "label": "Number of examples", "type": "number" },
        { "name": "description", "label": "Description", "type": "textarea", "fullwidth": true }
    ]
}
```

The matching `POST` route remains a regular `plugin_sql` action:

```json
"POST/insert_ia_dataset": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "INSERT INTO ia_datasets (name, format, nb_examples, description) VALUES (:name, :format, NULLIF(:nb_examples,''), :description)",
    "return_type": "redirect",
    "redirect_to": "/view/ia_datasets"
}
```

### Things to watch

- In `insert_XXX` mode, the `formGeneric.hbs` script clears text fields: a default `value` is erased there.
- A `file` field requires `enctype="multipart/form-data"` (added automatically by the script) and a `plugin_sql_upload` route.
- Serde silently ignores unknown keys: a typo (`field` instead of `fields`) produces no error.
- Any change to `dispatcher.rs` requires `cargo build --release --all` and a restart; `config_actions.json` and `.hbs` files only need a restart.
- To keep the SQL column order in `fields_meta`, enable `serde_json = { version = "1", features = ["preserve_order"] }`.

---

## plugin-core — shared types (FFI)

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

// FFI trait — execute() MUST be synchronous (async_trait is forbidden)
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult;
}

// Function shared by all plugins — avoids duplication
pub fn named_to_positional(
    sql:    &str,
    params: &HashMap<String, String>,
) -> (String, Vec<String>)
```

### `plugin_core::db` — runtime and MySQL pool owned by each plugin

```rust
pub struct SqlContext { pub rt: Runtime, pub pool: MySqlPool }

// One context per plugin (each .so has its own copy of the static)
pub fn sql_ctx() -> Result<&'static SqlContext, String>
```

- **Lazy** initialization (`OnceLock`) on first call, from `DATABASE_URL`.
- Multi-thread Tokio runtime with **2 workers** (`plugin-sql-rt`).
- sqlx pool limited by `PLUGIN_DB_MAX_CONNECTIONS` (default **5**), `acquire_timeout` of 5 s, `SET NAMES utf8mb4` on each connection.
- Dependency in `plugin-core/Cargo.toml`: `tokio = { version = "1", features = ["rt", "rt-multi-thread", "time", "net"] }`, and `pub mod db;` in `plugin-core/src/lib.rs`.

---

## Critical cdylib FFI rules

### Rust edition
```
Main Cargo.toml (small-folks binary) → edition = "2024"
Plugin Cargo.toml files (cdylib)     → edition = "2021"
```
In edition 2024, `#[no_mangle]` becomes `#[unsafe(no_mangle)]`. Plugins stay on 2021.

### Golden rule: every plugin owns its runtime and its pool

Each `.so` embeds **its own static copy** of `tokio` and `parking_lot`. If a plugin uses the host's `Handle` or `MySqlPool` (`state.handle`, `state.pool`), the plugin's tokio code manipulates structures (locks, wake-up queues) created by **another copy** of tokio. Under load, wake-ups are lost and the server freezes: timeouts, `Bad Gateway` behind nginx, all Tide threads stuck in `block_on`.

Rule: **a plugin never touches the host's runtime or pool.** It creates its own, inside its own `.so`.

```rust
// ❌ DO NOT DO THIS ANYMORE (plugin tied to the host's runtime and pool)
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
| `plugin_sql`, `plugin_auth`, `plugin_upload`, `plugin_sql_upload` | `plugin_core::db` (2 workers) | `MySqlPool` from `plugin_core::db` |
| `plugin_health` | `plugin_core::db` (MySQL) + `HEALTH_RT` (MongoDB) | `plugin_core::db` pool + Mongo client |
| `plugin_mongo` | `MONGO_RT` | MongoDB client created **inside** `MONGO_RT` |

Consequences:
- `state` is no longer used by the SQL plugins (the parameter becomes `_state`), except `plugin_health` which reads `state.sessions`.
- No more `block_in_place`: `execute()` is called from a blocking-pool thread, not from a Tokio worker.
- Total MySQL connections = (`PLUGIN_DB_MAX_CONNECTIONS` × number of SQL plugins) + the host pool. With 5 SQL plugins at 5 connections each, plan for at least ~30 on the MariaDB side (`max_connections`).

### plugin_mongo — standalone MongoContext (CRITICAL)
The MongoDB client MUST be created inside `MONGO_RT`, not in `main.rs`. Otherwise its heartbeat is starved by the main runtime → 1–8 s latency. Same principle for `HealthContext` / `HEALTH_RT`.

### Threads and lifecycle

![Diagram of threads and their lifecycle](https://mascaron.net/schema_threads_small_folks.svg)

The process contains four families of threads:

| Family | Threads | Role | Lifetime |
|---|---|---|---|
| **Tide (async-std)** | one per core (`ASYNC_GLOBAL_EXECUTOR_THREADS` to change) | accept the connection, resolve the route, check auth, read the body, render the response | created on first `spawn`, until shutdown |
| **Host tokio blocking pool** | on demand (512 max by default) | run `plugin.execute()`, the **synchronous** code of the `.so` | created with the load, destroyed after ~10 s idle |
| **Each plugin's runtime** | 2 workers per runtime (`plugin_core::db`, `MONGO_RT`, `HEALTH_RT`) | async I/O to MySQL / MongoDB | created on the plugin's first call (`OnceLock`), until shutdown |
| **Background workers** | tasks of the host's tokio runtime | `mqtt_worker`, `ebpf_worker` (5 s loop) | started at launch if enabled |

Path of a request: Tide receives and prepares the request → the dispatcher hands `plugin.execute()` to the blocking pool with `spawn_blocking(...).await` → that blocking thread waits, through `rt.block_on`, on the plugin's own runtime which queries the database → the result flows back, the Tide thread resumes the request, renders the view and responds.

#### Why `spawn_blocking` in the dispatcher

`Plugin::execute` is synchronous (FFI constraint). Called directly from Tide, it would block one of the async-std executor threads for the whole request: on a small VPS (2 cores), two slow requests would be enough to freeze the server. The dispatcher therefore runs the plugin in tokio's blocking pool:

```rust
let plugin  = Arc::clone(plugin);          // plugins: HashMap<String, Arc<dyn Plugin>>
let state   = req.state().clone();
let handle  = state.handle.clone();        // Handle of the host's tokio runtime
let ctx_job = ctx.clone();
let result = match handle
    .spawn_blocking(move || plugin.execute(&ctx_job, &state))
    .await
{
    Ok(r)  => r,
    Err(e) => { /* the plugin panicked */ return self.render_error(500, "Internal plugin error"); }
};
```

Consequences:
- Tide threads are only busy during rendering: `ASYNC_GLOBAL_EXECUTOR_THREADS` is no longer needed.
- Concurrency is limited by the plugins' connection pools (`PLUGIN_DB_MAX_CONNECTIONS`, 5 by default), no longer by the number of cores.
- A panic inside a plugin produces a `500` response instead of losing a server thread.
- `ctx.clone()` also copies `body_bytes`: for a 10 MB upload, a 10 MB copy per request.
- There is no hot reload of the `.so` files: recompiling a plugin requires restarting the server (replacing one while the server runs can crash it).

### named_to_positional — shared function in plugin-core
```rust
// In every plugin that runs SQL:
let (sql_prepared, values) = plugin_core::named_to_positional(&ctx.sql, &ctx.params);
```
Do not duplicate it — `regex = "1"` in `plugin-core/Cargo.toml` is enough.

### SQL rules
- All columns must be `CHAR`/`VARCHAR` or cast: `CAST(id AS CHAR)`, `CAST(COUNT(*) AS CHAR)`
- Named `:param` parameters → automatically converted to positional `?`
- `plugin_auth` uses `ctx.sql` — the query is defined in `config_actions.json`

### Where are the SQL queries defined?

SQL queries are not hard-coded in the Rust code. They are externalized in
`config_actions.json`: each action (route) has a `"sql"` field containing the
query, with named `:param` parameters.

The dispatcher reads this configuration, then the plugin (`plugin_sql`, `plugin_auth`)
runs the query supplied in its context (`ctx.sql`). Named parameters are converted
to positional `?` by `plugin_core::named_to_positional`.

Changing an SQL query therefore requires no plugin recompilation: a simple server
restart is enough. The files in the `sql/` folder are only used to create the tables
(`create_*.sql`), not to define the actions' queries.

Example:

```json
"GET/capteurs/chart/:sensor_id": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT ... FROM capteurs WHERE sensor_id = :sensor_id",
    "view": "specifics/chart_capteurs.hbs"
}
```

### Handlebars Rust templates
- `{{this}}` and not `{{.}}`
- `{{#each data}}{{#if @first}}{{#each this}}<th>{{@key}}</th>{{/each}}{{/if}}{{/each}}` for headers
- `{{#each data.0.fields}}` + `{{key}}`, `{{value}}`, `{{label}}`, `{{#if fullwidth}}` for `formGeneric`
- `{{#each (lookup @root.resources key)}}` + `{{val}}`, `{{label}}`, `{{#if selected}}` for selects
- `../../../@key` is forbidden — Handlebars Rust does not support deep upward lookups
- `enctype="multipart/form-data"` is mandatory on upload forms (mind the closing quote!)

---

## Handlebars helpers (src/helpers_hbs.rs)

Centralized in `src/helpers_hbs.rs`, registered via `crate::helpers_hbs::register_all(&mut hbs)`.

### `compare` helper — conditional block

```handlebars
{{#compare val "actif"}}true{{/compare}}
{{#compare val "actif"}}true{{else}}false{{/compare}}
{{#compare role "admin"  operator="=="}}...{{/compare}}
{{#compare nb   "10"     operator=">"}}...{{/compare}}
```

Operators: `==` `===` `!=` `!==` `>` `>=` `<` `<=`

On the Rust side, this helper is a `CompareHelper` struct implementing `HelperDef`, with the mandatory `'reg: 'rc` lifetimes. A plain closure does not compile for this case.

### `json` helper — raw JSON injection into `<script>`

```handlebars
{{!-- Triple braces = no HTML escaping → valid JSON for JS --}}
<script>
  const data = {{{json data}}};
  const labels = data.map(r => r.timestamp);
</script>
```

### Native Handlebars Rust helpers (do not re-register)

`eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `and`, `or`, `not` — available as subexpressions:
```handlebars
{{#if (eq status "ok")}}...{{/if}}
{{#if (gt memory.usage_percent 85)}}...{{/if}}
```

---

## Security (src/app_security.rs)

| Function | Protects against | Example |
|---|---|---|
| `sanitize_redirect(url)` | Open Redirect | `http://evil.com`, `//evil.com`, `/\evil.com` → `/` |
| `sanitize_header(val)` | Header Splitting | `val\r\nX-Evil:` → `valX-Evil:` |
| `sanitize_log(msg)` | Log Injection | `msg\n[FAKE]` → `msg [FAKE]` |
| `sanitize_html(input)` | XSS outside templates | `<script>` → `&lt;script&gt;` |

### Open Redirect (`next` parameter)

`/login?next=//youtube.com` was accepted by an earlier version (a URL starting with `//` is interpreted by the browser as an absolute URL with the same scheme). `sanitize_redirect`:

1. strips control characters (`\t`, `\r`, `\n`... which browsers ignore inside a URL);
2. requires a **relative** URL starting with `/` and not `//`;
3. rejects backslashes (`/\evil.com` is normalized to `//evil.com` by browsers), `:` (`javascript:`, `data:` schemes) and `<script`;
4. returns `/` in every other case and logs the attempt.

```rust
pub fn sanitize_redirect(url: &str) -> String {
    let cleaned: String = url.chars().filter(|c| !c.is_control()).collect();
    let url = cleaned.trim();
    if url.is_empty() { return "/".to_string(); }
    let lower = url.to_lowercase();
    let is_safe = url.starts_with('/') && !url.starts_with("//") && !url.contains('\\')
        && !lower.contains(':') && !lower.contains("javascript") && !lower.contains("<script");
    if is_safe { url.to_string() } else {
        eprintln!("[security] Open Redirect blocked: '{}'", sanitize_log(url));
        "/".to_string()
    }
}
```

### Hardening ideas (outside the current teaching scope)

- Password hashing (argon2/bcrypt) instead of direct SQL comparison.
- `Secure`, `HttpOnly` and `SameSite=Lax` attributes on **all** cookies (`jwt_token` currently has neither `Secure` nor `HttpOnly`).
- Login attempt limiting (rate limiting), on top of the XDP firewall.
- Security headers (`Content-Security-Policy`, `X-Content-Type-Options`, `X-Frame-Options`).
- Request body size limit and masking of SQL error messages on the client side.
- Startup warning for write routes (`POST`) without `"auth": true`.

Unit tests: `cargo test -p small-folks`

---

## Authentication (plugin_auth)

### How it works

Authentication relies on three `plugin_auth` operations, chosen by the action's `operation` field: `login`, `logout` and `me`.

1. **Login**: the plugin runs the action's `ctx.sql` query (see below) with the `login` and password received (form fields `login` and `mdp`). If a row is found, it generates an HS256-signed **JWT** (key `JWT_SECRET`) and a session identifier. It returns `AuthSuccess`, and the dispatcher sets two cookies: `session_id` (`HttpOnly`, so unreachable from JavaScript) and `jwt_token`. If no row is found, `AuthError` redirects to `/login?error=1`.
2. **Sessions**: they are kept in server memory (`Arc<Mutex<HashMap>>`) and expire after `SESSION_TTL_SECONDS`. A server restart therefore wipes them all.
3. **Access control**: for any action with `"auth": true`, the dispatcher reads the `session_id` cookie (injected into `ctx.params`) and checks that it matches a non-expired session. Otherwise it denies access or redirects to `/login`.
4. **Logout**: `AuthLogout` deletes the session and redirects to `redirect_to`.

The JWT contains the user's id and name, their function, and the issue and expiry dates (`iat`, `exp`), taken from the SQL aliases below.

With Tide, several cookies in one response are set with `insert_header` for the first one, then `append_header` for the following ones. Calling `header()` twice would overwrite the first cookie.

### SQL alias convention

| Recommended alias | Fallbacks | Role |
|---|---|---|
| `id` | `id_users`, `id_utilisateur` | Primary key |
| `name` | — | Last name |
| `first_name` | `firstName`, `prenom` | First name |
| `login` | — | Identifier |
| `function` | `role` | Function/role |
| `office` | `department` | Office (`''` if absent) |

### config_actions.json examples

```json
"POST/login": {
    "plugin":    "./target/release/libplugin_auth.so",
    "operation": "login",
    "sql":       "SELECT id_users AS id, name, firstName AS first_name, login, function, office FROM users WHERE login = :login AND mdp = :mdp LIMIT 1",
    "return_type": "redirect",
    "redirect_to": "/index"
}
```

> **⚠️ bcrypt passwords**: direct SQL comparison does not work with `password_hash()`.

### Testing login with curl

```bash
# 1. Log in and store the cookies in a file (-c)
read -s -p "Password: " PW; echo
curl -i -c /tmp/cj -d "login=steph" --data-urlencode "mdp=$PW" http://localhost:8080/login
#   → 303 + Location: /index + Set-Cookie: session_id=... and jwt_token=...
#   → 303 + Location: /login?error=1 if the credentials are wrong

# 2. Reuse the session (-b) on a protected route
curl -s -o /dev/null -w "%{http_code} %{redirect_url}\n" -b /tmp/cj http://localhost:8080/users
#   → 200 if the session is valid, 303 to /login otherwise

# 3. Clean up
rm /tmp/cj
```

A cookie flagged `Secure` is only sent back by curl over HTTPS. The field names (`login`, `mdp`) are those of `templates/generics/login.hbs`. Sessions live in server memory: after a restart you must log in again.

---

## UTF-8 encoding

Two points ensure accents and special characters travel correctly through the framework:

- **MySQL connection**: each sqlx pool (host and plugins) runs `SET NAMES utf8mb4 COLLATE utf8mb4_unicode_ci` on every new connection (`after_connect`).
- **Form data**: `urlencoding_decode` accumulates decoded bytes in a `Vec<u8>`, then converts them in one go with `String::from_utf8_lossy`. Converting byte by byte (`byte as char`) would corrupt multi-byte characters (é, à, €...).

---

## File uploads

Both upload plugins use `plugin_core::db` (runtime and pool owned by the plugin), like `plugin_sql`.

### plugin_upload — standalone upload

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

### plugin_sql_upload — upload + business SQL

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

Optional photo UPDATE tip:
```sql
UPDATE users SET name=:name, image=COALESCE(NULLIF(:image,''), image) WHERE id=:id
```

The `uploads/` folder must exist at startup, since it is served by `serve_dir`. As Git does not track empty folders, add an `uploads/.gitkeep` file.

Behind nginx, `client_max_body_size` (1 MB by default) must be at least equal to `max_size_mb`, otherwise the response is `413`.

If a form posts to a `plugin_sql_upload` route without `enctype="multipart/form-data"`, the plugin answers *"Content-Type multipart/form-data sans boundary"*: the browser sent `application/x-www-form-urlencoded`, which has no boundary.

### Testing an upload with curl

```bash
# The field name (file) must match the route; the file must exist on the machine running curl
for i in $(seq 1 20); do
  curl -s -o /dev/null --max-time 10 -b /tmp/cj \
    -w "%{http_code} %{redirect_url}\n" \
    -F "file=@/tmp/t.png;type=image/png" http://localhost:8080/upload &
done | sort | uniq -c; wait
#   → 20 × "303 .../uploads" expected. 000 = local file not found or server stuck.
```

---

## Health check (plugin_health)

```
GET /health              → raw JSON
GET /health/dashboard    → HTML dashboard (auth: true recommended)
```

`plugin_health` combines two standalone contexts: the **MySQL** ping goes through `plugin_core::db` (runtime and pool owned by the plugin) and the **MongoDB** ping through `HealthContext` / `HEALTH_RT`. Neither uses the host's runtime or pool. It also reads `state.sessions` for the session statistics.

---

## MQTT client (mqtt_worker)

Background Tokio task started automatically if `MQTT_BROKER_URL` is defined in `.env`.

### Supported message formats

- `sensors/temperature` → float payload: `"25.3"`
- `sensors/humidity` → float payload: `"60.5"`
- `sensors/+/data` → JSON payload: `{"sensor_id":"DHT22-001","temperature":25.3,"humidity":60.5}`

`temperature`/`humidity` topics are buffered by `sensor_id`. The INSERT happens once both values are available.

### MySQL sensors table

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

### Suggested routes

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

## Chart.js charts (chart_capteurs.hbs)

The `specifics/chart_capteurs.hbs` template displays sensor readings as **temperature** and **humidity** curves (Chart.js), with sensor and period selectors and min / average / max statistics.

Two routes feed it, using the same template:

- `GET/capteurs/chart`: readings from all sensors (see "Suggested routes" above);
- `GET/capteurs/chart/:sensor_id`: readings from a single sensor, sorted by ascending date.

```json
"GET/capteurs/chart/:sensor_id": {
    "plugin":      "./target/release/libplugin_sql.so",
    "sql":         "SELECT CAST(timestamp AS CHAR) AS timestamp, CAST(temperature AS CHAR) AS temperature, CAST(humidity AS CHAR) AS humidity, sensor_id FROM capteurs WHERE sensor_id = :sensor_id ORDER BY timestamp ASC",
    "view":        "specifics/chart_capteurs.hbs",
    "return_type": "html",
    "auth":        true
}
```

SQL data is passed to the page's JavaScript with the `json` helper in **triple braces**: `const data = {{{json data}}};`. With double braces, Handlebars would escape the quotes and the JSON would become invalid.

---

## eBPF/XDP firewall (ebpf_worker)

Filters network packets **inside the Linux kernel** before the TCP/IP stack — protection against SYN flood attacks.

### Architecture

```
Incoming network packet (NIC)
  ↓
[xdp_firewall] ← runs in the kernel (bpfel-unknown-none)
  ├─ IP in BLACKLIST?          → XDP_DROP (~100ns)
  ├─ TCP SYN packet?           → increment CONN_COUNT[src_ip]
  │   └─ count > rate_limit?   → BLACKLIST[src_ip] = now → XDP_DROP
  └─ Otherwise                 → XDP_PASS
         ↕ BPF Maps (shared memory kernel ↔ userspace)
[ebpf_worker] ← runs in userspace (Tokio, every 5s)
  ├─ Reads STATS  → logs (every EBPF_STATS_LOG_SECS seconds)
  ├─ Syncs BLACKLIST → MySQL (ebpf_blacklist)
  └─ Auto-unblocks expired IPs
```

### BPF Maps

| Map | Type | Role |
|---|---|---|
| `CONN_COUNT` | `HashMap<u32, u32>` | SYN counter per IP |
| `CONN_FIRST` | `HashMap<u32, u64>` | First-SYN timestamp per IP |
| `BLACKLIST` | `HashMap<u32, u64>` | Blocked IPs + timestamp |
| `CONFIG` | `Array<u64>` | `[rate_limit, window_ns]` |
| `STATS` | `Array<u64>` | `[packets, drops, syn]` |

In the kernel program, an `Array` map offers neither `set()` nor `insert()`. A value is written by fetching a pointer with `get_ptr_mut()`, then assigning `*ptr = value`.

### Building the kernel program

```bash
# Prerequisites
rustup component add rust-src
cargo binstall bpf-linker        # (cargo install bpf-linker requires llvm-config: prefer cargo-binstall)

# Build the kernel program (from ebpf-firewall/)
cd ebpf-firewall
cargo build --release
# → target/bpfel-unknown-none/release/ebpf-firewall
```

The `ebpf-firewall/` folder is **excluded from the main workspace** because it uses the `bpfel-unknown-none` target. It has its own `[workspace]` in its `Cargo.toml` and a `rust-toolchain.toml` forcing nightly.

### Enabling in small-folks

```toml
# Cargo.toml — uncomment
aya = { version = "0.12", features = ["async_tokio"] }
```

```rust
// src/main.rs — uncomment
mod ebpf_worker;
// + the if EBPF_ENABLED block
```

```bash
# .env
EBPF_ENABLED=true
EBPF_INTERFACE=eth0
EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
EBPF_RATE_LIMIT=100
EBPF_WINDOW_SECS=60
EBPF_AUTO_UNBLOCK_SECS=300
EBPF_STATS_LOG_SECS=60     # 0 = never print the periodic statistics

# Run with the required privileges
sudo ./target/release/small-folks
```

**Required privileges**: loading an XDP program and creating BPF maps.
- Kernel **≥ 5.8**: `CAP_BPF` + `CAP_NET_ADMIN` (+ `CAP_PERFMON`) are enough.
- Kernel **< 5.8** (e.g. Ubuntu 20.04, kernel 5.4): `CAP_BPF` does not exist, you need `CAP_SYS_ADMIN` + `CAP_NET_ADMIN`, or root.
- Under systemd, the `failed to create map BLACKLIST with code -1` error comes from a too-low `RLIMIT_MEMLOCK`: add `LimitMEMLOCK=infinity` to the unit.

### MySQL ebpf_blacklist table

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

### Suggested routes

```json
"GET/security/blacklist": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT ip_address, CAST(blocked_at AS CHAR) AS blocked_at, reason, CAST(unblock_at AS CHAR) AS unblock_at FROM ebpf_blacklist WHERE unblocked_at IS NULL ORDER BY blocked_at DESC",
    "view": "generics/tableGeneric.hbs",
    "return_type": "html",
    "auth": true
}
```

### Observed performance

```
~19,000 packets dropped/second in the kernel
99.6% of packets blocked during a SYN flood
Tide / MySQL: no impact during the attack
```

---

## Environment variables (.env)

```bash
HOST=0.0.0.0                 # listen address: 0.0.0.0 = all interfaces, 127.0.0.1 = local only
PORT=8080
CONFIG_ACTIONS=./config_actions.json
TEMPLATES_DIR=./templates
DATABASE_URL=mysql://user:pass@localhost:3306/mydb
MONGODB_URI=mongodb://localhost:27017
MONGODB_DB=mydb
MONGODB_USER=admin
MONGODB_PASS=mypass
MONGODB_AUTH_DB=admin
UPLOAD_DIR=./uploads
JWT_SECRET=secret-string-32-chars-minimum
SESSION_TTL_SECONDS=3600
LOGIN_REDIRECT=/index

# Pools and threads
PLUGIN_DB_MAX_CONNECTIONS=5          # MySQL connections PER SQL plugin (default: 5)
# ASYNC_GLOBAL_EXECUTOR_THREADS=4    # optional: Tide executor threads (default: number of cores).
#                                    # Of little use since spawn_blocking in the dispatcher.

# MQTT (optional)
# MQTT_BROKER_URL=localhost
# MQTT_BROKER_PORT=1883
# MQTT_CLIENT_ID=small-folks-mqtt
# MQTT_TOPICS=sensors/#
# MQTT_QOS=1

# eBPF XDP firewall (optional — root or capabilities, see the eBPF section)
# EBPF_ENABLED=true
# EBPF_INTERFACE=eth0
# EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
# EBPF_RATE_LIMIT=100
# EBPF_WINDOW_SECS=60
# EBPF_AUTO_UNBLOCK_SECS=300
# EBPF_STATS_LOG_SECS=60
```

The server is reachable from outside if `HOST=0.0.0.0` (or the public IP) and the port is open in the firewall. Behind nginx, prefer `HOST=127.0.0.1`.

---

## Build and run

### Prerequisites (Ubuntu / Debian)

```bash
sudo apt install build-essential pkg-config libssl-dev    # openssl-sys
# eBPF (optional):
rustup toolchain install nightly && rustup component add rust-src --toolchain nightly
cargo install cargo-binstall && cargo binstall bpf-linker
```

### Build and run

```bash
# Build all plugins + the binary
cargo build --release --all

# (Optional) Build the eBPF kernel program
cd ebpf-firewall && cargo build --release && cd ..

# Run (without eBPF)
./target/release/small-folks

# Run (with eBPF)
sudo ./target/release/small-folks
```

- The `.so` paths in `config_actions.json` must point to `./target/release/`. Mixing a release binary with debug `.so` files causes a coredump.
- **Stop the server before recompiling**: replacing a `.so` that a running server has loaded can crash it.
- Rust code changes (`src/`, `plugins/`) → recompilation required. Changes to `config_actions.json` or a `.hbs` → a simple restart.

### systemd service

`/etc/systemd/system/small-folks.service`:

```ini
[Unit]
Description=small-folks (Rust / Tide framework)
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
# Without root, kernel >= 5.8:
# AmbientCapabilities=CAP_BPF CAP_NET_ADMIN CAP_PERFMON
# Without root, kernel 5.4:
# AmbientCapabilities=CAP_SYS_ADMIN CAP_NET_ADMIN

[Install]
WantedBy=multi-user.target
```

`User=root` is needed if eBPF is enabled and no capability is granted. `LimitMEMLOCK=infinity` is essential for eBPF maps.

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now small-folks
journalctl -u small-folks -f
```

### Behind nginx

```nginx
location / {
    proxy_pass         http://127.0.0.1:8080;
    proxy_set_header   Host $host;
    proxy_set_header   X-Real-IP $remote_addr;
    proxy_set_header   X-Forwarded-Proto $scheme;
    client_max_body_size 10m;      # ≥ max_size_mb of the uploads
}
```

---

## Load and concurrency tests

Procedure used to validate the "one runtime and one pool per plugin" model.

```bash
# 20 simultaneous requests on an SQL route
for i in $(seq 1 20); do
  curl -s -o /dev/null --max-time 10 -w "%{http_code} %{time_total}s\n" http://localhost:8080/users &
done; wait

# 200 requests, counting HTTP codes
for i in $(seq 1 200); do curl -s -o /dev/null --max-time 10 -w "%{http_code}\n" http://localhost:8080/users & done | sort | uniq -c; wait

# Mixed routes (SQL, login, health)
for i in $(seq 1 200); do
  curl -s -o /dev/null --max-time 10 -w "%{http_code}\n" http://localhost:8080/users &
  curl -s -o /dev/null --max-time 10 -w "%{http_code}\n" -d "login=x" -d "mdp=y" http://localhost:8080/login &
done | sort | uniq -c; wait
```

| Scenario | Before the fix | After the fix |
|---|---|---|
| 20 × `/users` in parallel (2-core VPS) | 2 responses out of 20, the rest timed out | 20 × `200` |
| 200 × `/users`, `/login`, `/health` | freeze / `Bad Gateway` | 100% success |
| 600 mixed requests | — | 600 / 600 |
| 20 × `/users` via nginx + HTTPS | — | 20 × `200`, 0.29–0.45 s |
| 20 × `POST /upload` with a session | — | 20 × `303` to `/uploads` |

**Reading the codes**: `000` = no HTTP response (server down, request stuck, or local file not found with `-F file=@...`); `303` to `/login` = no session; `413` = nginx `client_max_body_size` too low; `502/504` = server stuck behind nginx.

**Diagnosing a freeze**: while the requests are pending, capture the stacks of all threads.

```bash
sudo gdb -p $(pgrep -f target/release/small-folks | head -1) -batch \
  -ex "set pagination off" -ex "thread apply all bt" > /tmp/gdb_trace.txt 2>&1
```

`async-std` threads all stuck in `block_on` / `parking_lot` are the sign of a plugin sharing the host's runtime or pool.

---

## Performance (release, localhost)

| Component | Operation | Latency / Throughput |
|---|---|---|
| plugin_sql | SELECT 243 rows | 1-5ms |
| plugin_mongo | find 243 documents | 3-5ms |
| plugin_auth | full login | ~1ms |
| plugin_upload | upload 1 file | < 10ms |
| plugin_sql_upload | upload + SQL INSERT | < 15ms |
| plugin_health | all metrics + pings | < 10ms |
| mqtt_worker | sensor INSERT | < 5ms |
| ebpf_worker (XDP) | DROP blacklisted packet | ~100ns |
| ebpf_worker (XDP) | Throughput observed in tests | ~19,000 drops/s |

---

## System routes

```
GET /health            → {"status":"ok"} JSON
GET /health/dashboard  → HTML dashboard
GET /images/*          → public/images/
GET /css/*             → public/css/
GET /uploads/*         → uploads/
```

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v2_50pc.png" alt="Rust Framework small-Folks logo" />
</p>
