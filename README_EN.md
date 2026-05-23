# small-folks — Configuration-Driven Rust MVC Framework

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v1.png" alt="small-Folks Rust Framework Logo" />
</p>

A **Rust** web framework built on **Tide** with a dynamic plugin system (`cdylib`).
Routes, SQL/MongoDB queries, views and resources are entirely configured in `config_actions.json` **without recompilation**.
Note: Small-Folks is designed for educational purposes and is not intended for production use.

---

## Tech Stack

| Component | Version | Role |
|---|---|---|
| Rust | 2021 edition | language |
| Tide | 0.16 | async HTTP server |
| Tokio | 1 | async multi-thread runtime |
| sqlx | 0.7 | MySQL access, prepared statements |
| mongodb | 3.5 | MongoDB zero-copy driver (RawDocumentBuf) |
| Handlebars | 6.4 | HTML templates with partials |
| libloading | 0.8 | dynamic `.so` loading |
| jsonwebtoken | 9 | JWT HS256 authentication |
| multer | 3 | multipart/form-data parsing |
| uuid | 1 | unique identifier generation |
| sysinfo | 0.30 | RAM, disk, uptime metrics (plugin_health) |
| rumqttc | 0.24 | pure Rust MQTT client (mqtt_worker) |
| aya | 0.12 | eBPF userspace framework (ebpf_worker) |
| libc | 0.2 | system calls (RLIMIT_MEMLOCK for eBPF) |
| chrono | 0.4 | date/timestamp management |

---

## Workspace Architecture

```
small-folks/
├── README.md                        ← French documentation
├── README_EN.md                     ← this file
├── config_actions.json              ← route registry
├── .env                             ← environment variables
├── .env.example                     ← environment variables template
├── Cargo.toml                       ← Rust workspace (edition 2024 for binary)
├── dump/
│   └── R504TP_2026_04_23_dump       ← demo database SQL dump
├── src/
│   ├── main.rs                      ← startup, pools, plugin pre-cache
│   ├── dispatcher.rs                ← route resolution, rendering, auth
│   ├── app_security.rs              ← XSS, open redirect, header injection protection
│   ├── helpers_hbs.rs               ← custom Handlebars helpers
│   ├── mqtt_worker.rs               ← background MQTT client (sensor storage)
│   └── ebpf_worker.rs               ← XDP firewall userspace (optional, requires sudo)
├── ebpf-firewall/                   ← XDP kernel program (separate crate, bpfel-unknown-none)
│   ├── Cargo.toml
│   ├── rust-toolchain.toml          ← nightly + rust-src (required for bpfel-unknown-none)
│   ├── .cargo/config.toml           ← bpfel-unknown-none target + build-std=core
│   └── src/main.rs                  ← XDP program: SYN counting + blacklist DROP
├── plugins/                         ← all plugins in edition 2021
│   ├── plugin-core/src/lib.rs       ← shared traits, types (FFI) + named_to_positional
│   ├── plugin_sql/src/lib.rs        ← generic SQL + resources + forms
│   ├── plugin_mongo/src/lib.rs      ← MongoDB with autonomous MongoContext
│   ├── plugin_auth/src/lib.rs       ← login / logout / JWT / sessions
│   ├── plugin_upload/src/lib.rs     ← multipart upload → disk + MySQL
│   ├── plugin_sql_upload/src/lib.rs ← text fields + file → SQL + disk
│   └── plugin_health/src/lib.rs     ← server metrics + DB ping
├── templates/
│   ├── generics/                    ← reusable templates
│   │   ├── tableGeneric.hbs         ← generic data table + optional link (row_link)
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
│       └── form_countries.hbs
├── public/
│   ├── css/styles.css               ← global styles + forms + dashboard
│   └── images/                      ← favicon, logos
├── uploads/                         ← uploaded files (UUID.ext)
├── resources/                       ← logos and framework diagrams
└── sql/
    ├── create_countries.sql
    ├── create_uploads.sql
    ├── create_capteurs.sql          ← MQTT sensor table
    └── create_ebpf_blacklist.sql    ← eBPF/XDP blacklist table
```

---

## HTTP Request Flow

![Architecture diagram](https://mascaron.net/schema_architecture_small_folksv6.png)

```
HTTP Client
  ↓ GET /regions  (session_id cookie present)
Tide — catch-all /*
  ↓
Dispatcher
  ├─ resolve_action("GET", "/regions") → config_actions.json
  ├─ URL params + query string + body extraction
  ├─ session_id cookie injection → ctx.params
  ├─ auth check (if "auth": true in config)
  │     → session cache → OK or 401/redirect /login
  ↓
plugin_sql.execute(ctx, state)        ← synchronous (FFI cdylib constraint)
  ├─ block_in_place + handle.block_on
  ├─ main query (ctx.sql)
  ├─ resource queries (ctx.sql_resources) if data_resources defined
  │     → enriched with { val, label, selected } per option
  └─ PluginResult::Data(json)
  ↓
Dispatcher: render according to return_type
  ├─ "html"     → row_link + form_action injection → hbs.render(view, data)
  ├─ "json"     → HTTP 200 application/json
  └─ "redirect" → HTTP 303 Location
  ↓
HTTP response
```

---

## config_actions.json — Complete Reference

### Available Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `plugin` | string | — | Path to the `.so` file |
| `sql` | string | — | SQL query with `:param` (also used by plugin_auth) |
| `sql_upload` | string | — | INSERT query for uploads table (plugin_sql_upload) |
| `collection` | string | — | MongoDB collection |
| `filter` | string | `{}` | JSON BSON filter with `:param` |
| `operation` | string | `find` | MongoDB or auth operation (`login`, `logout`, `me`, `status`, `dashboard`) |
| `form_action` | string | — | HTML form `action=""` URL |
| `form_columns` | number | `1` | Form column count: `1` or `2` |
| `form_fullwidth_fields` | array | `[]` | Full-width fields in 2-column mode |
| `data_resources` | object | `{}` | `"column_name" → "resource_name"` for selects |
| `sql_resources` | object | `{}` | `"resource_name" → "SELECT ..."` |
| `row_link` | string | — | Base URL for link on a `tableGeneric` column (e.g. `/view/animal`) |
| `row_link_col` | number | `1` | Column index carrying the link (default: column 1) |
| `upload_field` | string | — | File field name in the form (plugin_sql_upload) |
| `allowed_mime` | string | `image/jpeg,image/png,application/pdf` | Allowed MIME types for upload |
| `max_size_mb` | string | `10` | Maximum upload size in MB |
| `view` | string | — | Handlebars template name |
| `return_type` | string | `json` | `html`, `json` or `redirect` |
| `redirect_to` | string | `/` | Redirect URL |
| `auth` | bool | `false` | Requires a valid session |

### Template Rules by Use Case

| Case | Template | JSON structure received |
|---|---|---|
| Simple list | `tableGeneric.hbs` | `{ data: [{...}], row_link: "", row_link_col: 1 }` |
| List with link | `tableGeneric.hbs` + `row_link` | `{ data: [{...}], row_link: "/view/x", row_link_col: 1 }` |
| 1-column form | `formGeneric.hbs` + `form_action` | `{ data: [{fields:[{key,value,fullwidth}]}], form_action, form_columns:1 }` |
| 2-column form | `formGeneric.hbs` + `form_columns:2` | same + fullwidth fields marked `true` |
| Form with selects | same + `data_resources` | same + `resources: { field: [{val,label,selected}] }` |
| Form + file upload | specific template + `enctype="multipart/form-data"` | handled by `plugin_sql_upload` |

### JSON sent to tableGeneric.hbs

```json
{
  "data": [
    { "id": "1", "name": "Lion", "species": "Panthera leo" }
  ],
  "row_link":     "/view/animal",
  "row_link_col": 1
}
```

Without `row_link` in config → `row_link: ""` → `{{#if row_link}}` is falsy → table without link.

### JSON sent to formGeneric.hbs

```json
{
  "form_action":  "/update_user",
  "form_columns": 2,
  "data": [
    {
      "fields": [
        { "key": "name",           "value": "Smith",  "fullwidth": false },
        { "key": "code_countries", "value": "GB",     "fullwidth": false },
        { "key": "address1",       "value": "Street", "fullwidth": true  }
      ]
    }
  ],
  "resources": {
    "code_countries": [
      { "val": "DE", "label": "Germany", "selected": false },
      { "val": "GB", "label": "UK",      "selected": true  },
      { "val": "US", "label": "USA",     "selected": false }
    ]
  }
}
```

**`selected` is pre-computed on the Rust side** in `plugin_sql`.

- `insert_XXX` mode → empty values → `selected: false` everywhere → "-- choose --" displayed
- `update_XXX` mode → `selected: true` on the option matching the current value

---

## plugin-core — Shared FFI Types

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

// FFI Trait — execute() MUST be synchronous (async_trait forbidden)
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult;
}

// Shared function across all plugins — avoids code duplication
pub fn named_to_positional(
    sql:    &str,
    params: &HashMap<String, String>,
) -> (String, Vec<String>)
```

---

## Handlebars Helpers (src/helpers_hbs.rs)

Centralized in `src/helpers_hbs.rs`, registered via `crate::helpers_hbs::register_all(&mut hbs)`.

### `compare` helper — conditional block

```handlebars
{{#compare val "active"}}true{{/compare}}
{{#compare val "active"}}true{{else}}false{{/compare}}
{{#compare role "admin"  operator="=="}}...{{/compare}}
{{#compare nb   "10"     operator=">"}}...{{/compare}}
```

Operators: `==` `===` `!=` `!==` `>` `>=` `<` `<=`

### `json` helper — raw JSON injection in `<script>`

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
| `sanitize_redirect(url)` | Open Redirect | `http://evil.com` → `/` |
| `sanitize_header(val)` | Header Splitting | `val\r\nX-Evil:` → `valX-Evil:` |
| `sanitize_log(msg)` | Log Injection | `msg\n[FAKE]` → `msg [FAKE]` |
| `sanitize_html(input)` | XSS outside templates | `<script>` → `&lt;script&gt;` |

Unit tests: `cargo test -p small-folks`

---

## Critical FFI cdylib Rules

### Rust Edition
```
Main Cargo.toml (small-folks binary) → edition = "2024"
Plugin Cargo.toml (cdylib)           → edition = "2021"
```
In edition 2024, `#[no_mangle]` becomes `#[unsafe(no_mangle)]`. Plugins stay on 2021.

### plugin_sql — block_in_place + handle.block_on
```rust
tokio::task::block_in_place(|| {
    state.handle.block_on(async {
        sqlx::query(...).fetch_all(&state.pool).await
    })
})
```

### plugin_mongo — Autonomous MongoContext (CRITICAL)
The MongoDB client MUST be created inside `MONGO_RT`, not in `main.rs`.
Otherwise `block_in_place` from `plugin_sql` starves the MongoDB heartbeat → 1-8s latency.

**The same rule applies to `plugin_health`** (`HealthContext` + `HEALTH_RT`) and **`plugin_sql_upload`** (dedicated `OnceLock<Runtime>`).

### named_to_positional — shared function in plugin-core
```rust
// In all plugins that execute SQL:
let (sql_prepared, values) = plugin_core::named_to_positional(&ctx.sql, &ctx.params);
```
Do not duplicate — `regex = "1"` in `plugin-core/Cargo.toml` is sufficient.

### SQL Rules
- All columns must be `CHAR`/`VARCHAR` or cast: `CAST(id AS CHAR)`, `CAST(COUNT(*) AS CHAR)`
- Named parameters `:param` → automatically converted to positional `?`
- `plugin_auth` uses `ctx.sql` — query defined in `config_actions.json`

### Handlebars Rust Templates
- `{{this}}` not `{{.}}`
- `{{#each data}}{{#if @first}}{{#each this}}<th>{{@key}}</th>{{/each}}{{/if}}{{/each}}` for headers
- `{{#each data.0.fields}}` + `{{key}}`, `{{value}}`, `{{#if fullwidth}}` for `formGeneric`
- `{{#each (lookup @root.resources key)}}` + `{{val}}`, `{{label}}`, `{{#if selected}}` for selects
- `../../../@key` is forbidden — Handlebars Rust does not support deep context climbing
- `enctype="multipart/form-data"` mandatory on upload forms (closing quote!)

---

## Authentication (plugin_auth)

### SQL Alias Convention

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
    "sql":       "SELECT id AS id, name, first_name, login, function, office FROM users WHERE login = :login AND password = :mdp LIMIT 1",
    "return_type": "redirect",
    "redirect_to": "/index"
}
```

> **⚠️ bcrypt passwords**: direct SQL comparison does not work with `password_hash()`. You need to modify `plugin_auth` to verify with the `bcrypt` crate on the Rust side.

### plugin_auth Operations

| Operation | Route | Description |
|---|---|---|
| `login` | `POST /login` | Executes `ctx.sql`, creates session + JWT |
| `logout` | `GET /logout` | Removes session from cache |
| `me` | `GET /api/me` | Returns current user info (JSON) |

### Route Protection
```json
"GET/my-route": { "auth": true, ... }
```
- Unauthenticated + `return_type: json` → HTTP 401
- Unauthenticated + `return_type: html` → redirect `/login?next=/my-route`

---

## File Upload

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

Optional photo UPDATE trick:
```sql
UPDATE users SET name=:name, image=COALESCE(NULLIF(:image,''), image) WHERE id=:id
```

---

## Health Check (plugin_health)

```
GET /health              → raw JSON
GET /health/dashboard    → HTML dashboard (auth: true recommended)
```

`plugin_health` uses `HealthContext` with autonomous `HEALTH_RT` — same pattern as `plugin_mongo`.

---

## MQTT Client (mqtt_worker)

Background Tokio task started automatically if `MQTT_BROKER_URL` is set in `.env`.

### Supported Message Formats

- `sensors/temperature` → float payload: `"25.3"`
- `sensors/humidity` → float payload: `"60.5"`
- `sensors/+/data` → JSON payload: `{"sensor_id":"DHT22-001","temperature":25.3,"humidity":60.5}`

`temperature`/`humidity` topics are buffered by `sensor_id`. INSERT occurs when both values are available.

### MySQL sensors Table

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

### Suggested Routes

```json
"GET/sensors": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT CAST(id AS CHAR) AS id, sensor_id, CAST(temperature AS CHAR) AS temperature, CAST(humidity AS CHAR) AS humidity, CAST(timestamp AS CHAR) AS timestamp FROM capteurs ORDER BY timestamp DESC LIMIT 100",
    "view": "generics/tableGeneric.hbs",
    "return_type": "html",
    "auth": true
},
"GET/sensors/chart": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT CAST(timestamp AS CHAR) AS timestamp, CAST(temperature AS CHAR) AS temperature, CAST(humidity AS CHAR) AS humidity, sensor_id FROM capteurs ORDER BY timestamp DESC LIMIT 200",
    "view": "specifics/chart_capteurs.hbs",
    "return_type": "html",
    "auth": true
}
```

---

## eBPF/XDP Firewall (ebpf_worker)

Filters network packets **inside the Linux kernel** before the TCP/IP stack — protection against SYN flood attacks.

### Architecture

```
Incoming network packet (NIC)
  ↓
[xdp_firewall] ← runs in the kernel (bpfel-unknown-none)
  ├─ IP in BLACKLIST ?        → XDP_DROP (~100ns)
  ├─ TCP SYN packet ?         → increment CONN_COUNT[src_ip]
  │   └─ count > rate_limit ? → BLACKLIST[src_ip] = now → XDP_DROP
  └─ Otherwise                → XDP_PASS
         ↕ BPF Maps (shared kernel ↔ userspace memory)
[ebpf_worker] ← runs in userspace (Tokio, every 5s)
  ├─ Reads STATS  → logs
  ├─ Syncs BLACKLIST → MySQL (ebpf_blacklist)
  └─ Auto-unblocks expired IPs
```

### BPF Maps

| Map | Type | Role |
|---|---|---|
| `CONN_COUNT` | `HashMap<u32, u32>` | SYN counter per IP |
| `CONN_FIRST` | `HashMap<u32, u64>` | First SYN timestamp per IP |
| `BLACKLIST` | `HashMap<u32, u64>` | Blocked IPs + timestamp |
| `CONFIG` | `Array<u64>` | `[rate_limit, window_ns]` |
| `STATS` | `Array<u64>` | `[packets, drops, syn]` |

### Compiling the Kernel Program

```bash
# Prerequisites
rustup component add rust-src
cargo install bpf-linker

# Compile the kernel program (from ebpf-firewall/)
cd ebpf-firewall
cargo build --release
# → target/bpfel-unknown-none/release/ebpf-firewall
```

The `ebpf-firewall/` directory is **excluded from the main workspace** as it uses the `bpfel-unknown-none` target. It has its own `[workspace]` in its `Cargo.toml` and a `rust-toolchain.toml` that forces nightly.

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

# Run with required privileges
sudo ./target/release/small-folks
# or grant capabilities permanently
sudo setcap cap_bpf,cap_net_admin+eip ./target/release/small-folks
./target/release/small-folks
```

### MySQL ebpf_blacklist Table

```sql
CREATE TABLE IF NOT EXISTS ebpf_blacklist (
    id           INT AUTO_INCREMENT PRIMARY KEY,
    ip_address   VARCHAR(15)  NOT NULL,
    blocked_at   DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    unblock_at   DATETIME     DEFAULT NULL     COMMENT 'NULL = permanent block',
    unblocked_at DATETIME     DEFAULT NULL     COMMENT 'NULL = still blocked',
    reason       VARCHAR(100) NOT NULL DEFAULT 'rate_limit_exceeded',
    UNIQUE KEY uq_ip_active (ip_address, unblocked_at),
    INDEX idx_ip      (ip_address),
    INDEX idx_blocked (blocked_at),
    INDEX idx_unblock (unblock_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
  COMMENT='History of IPs blocked by the eBPF/XDP firewall';
```

### Suggested Routes

```json
"GET/security/blacklist": {
    "plugin": "./target/release/libplugin_sql.so",
    "sql": "SELECT ip_address, CAST(blocked_at AS CHAR) AS blocked_at, reason, CAST(unblock_at AS CHAR) AS unblock_at FROM ebpf_blacklist WHERE unblocked_at IS NULL ORDER BY blocked_at DESC",
    "view": "generics/tableGeneric.hbs",
    "return_type": "html",
    "auth": true
}
```

### Observed Performance

```
~19,000 packets dropped/second in the kernel
99.6% of packets blocked during a SYN flood
Tide / MySQL: zero impact during the attack
```

---

## Environment Variables (.env)

```bash
HOST=0.0.0.0
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

# MQTT (optional)
# MQTT_BROKER_URL=localhost
# MQTT_BROKER_PORT=1883
# MQTT_CLIENT_ID=small-folks-mqtt
# MQTT_TOPICS=sensors/#
# MQTT_QOS=1

# eBPF XDP Firewall (optional — requires sudo or CAP_BPF + kernel ≥ 5.8)
# EBPF_ENABLED=true
# EBPF_INTERFACE=eth0
# EBPF_PROGRAM=./ebpf-firewall/target/bpfel-unknown-none/release/ebpf-firewall
# EBPF_RATE_LIMIT=100
# EBPF_WINDOW_SECS=60
# EBPF_AUTO_UNBLOCK_SECS=300
```

---

## Build & Run

```bash
# Build all plugins + binary
cargo build --release --all

# (Optional) Build the eBPF kernel program
cd ebpf-firewall && cargo build --release && cd ..

# Run (without eBPF)
./target/release/small-folks

# Run (with eBPF)
sudo ./target/release/small-folks
```

The `.so` files in `config_actions.json` must point to `./target/release/`.
Mixing a release binary with debug `.so` files causes a coredump.

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
| mqtt_worker | INSERT sensor | < 5ms |
| ebpf_worker (XDP) | DROP blacklisted packet | ~100ns |
| ebpf_worker (XDP) | Observed throughput in test | ~19,000 drops/s |

---

## System Routes

```
GET /health            → {"status":"ok"} JSON
GET /health/dashboard  → HTML dashboard
GET /images/*          → public/images/
GET /css/*             → public/css/
GET /uploads/*         → uploads/
```

<p align="center">
  <img src="https://mascaron.net/logo_small-folks-v2_50pc.png" alt="small-Folks Rust Framework Logo" />
</p>
