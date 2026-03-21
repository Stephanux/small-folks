# rust-plugin-tide v3 — Framework MVC piloté par configuration

Serveur web Tide modulaire en Rust, où **routes, requêtes SQL et vues sont
entièrement configurées dans `config_actions.json`** sans recompilation.

## Architecture

```
rust-plugin-tide/
├── config_actions.json   ← annuaire des routes (SQL + plugin + vue + return_type)
├── templates/            ← vues Handlebars (.hbs)
│   ├── tableau.hbs
│   ├── error.hbs
│   └── success.hbs
├── plugin-core/          ← traits Plugin, ActionContext, PluginResult
├── plugin_countries/     ← plugin générique CRUD countries (cdylib)
├── plugin_regions/       ← plugin générique regions (cdylib)
└── src/
    ├── main.rs           ← démarrage, précache plugins, serveur Tide
    └── dispatcher.rs     ← résolution routes, extraction params, rendu
```

## Flux d'une requête

```
HTTP GET /countries/FR
  ↓ dispatcher.rs
  ↓ lookup "GET/countries/:code" dans config_actions.json
  ↓ extrait params : { code: "FR" }
  ↓ charge plugin "countries" (précaché)
  ↓ plugin.execute(ctx, state) → SQL "SELECT ... WHERE code = ?" avec bind("FR")
  ↓ return_type = "html" → render("tableau.hbs", data)
  ↓ HTTP 200 text/html
```
![alt Schéma architecture]'https://mascaron.net/architecture_rust_plugin_tide.png)


## config_actions.json — Structure d'une action

```json
"GET/countries/:code": {
    "plugin":      "./target/debug/libplugin_countries.so",
    "sql":         "SELECT code, name_us, name_fr FROM countries WHERE code = :code",
    "view":        "tableau.hbs",
    "return_type": "html"
}
```

| Champ         | Valeurs possibles              | Description                              |
|---------------|-------------------------------|------------------------------------------|
| `plugin`      | chemin vers .so / .dll        | Plugin à utiliser                        |
| `sql`         | toute requête SQL              | Paramètres nommés `:param`               |
| `view`        | nom du template .hbs           | Ignoré si return_type = json/redirect    |
| `return_type` | `html` / `json` / `redirect`  | Mode de réponse                          |
| `redirect_to` | URL                            | Destination si return_type = redirect    |

## Compilation

```bash
cargo build --all
```

## Lancement

```bash
# Copier et adapter la config
cp .env.example .env

# Lancer le serveur
cargo run
```

## Routes disponibles (exemple)

| Méthode | URL                  | return_type | Description                    |
|---------|----------------------|-------------|-------------------------------|
| GET     | /countries           | html        | Tableau HTML de tous les pays |
| GET     | /api/countries       | json        | JSON de tous les pays         |
| GET     | /countries/FR        | html        | Détail pays FR (HTML)         |
| GET     | /api/countries/FR    | json        | Détail pays FR (JSON)         |
| POST    | /countries           | redirect    | Création d'un pays            |
| PUT     | /countries/FR        | redirect    | Mise à jour pays FR           |
| DELETE  | /api/countries/FR    | json        | Suppression pays FR           |
| GET     | /regions             | html        | Tableau des régions           |
| GET     | /api/regions         | json        | JSON des régions              |
| GET     | /regions/Europe      | html        | Pays de la région Europe      |
| GET     | /health              | json        | Santé du serveur              |
