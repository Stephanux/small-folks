use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use sqlx::{Column, Row};
use serde_json::{json, Map, Value};

pub struct PluginCountries;

impl Plugin for PluginCountries {
    fn name(&self) -> &'static str {
        "sql"
    }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        // Convertit les paramètres nommés ":param" en "?" pour sqlx
        // et collecte les valeurs dans l'ordre d'apparition
        let (sql_prepared, param_values) = named_to_positional(&ctx.sql, &ctx.params);
        let has_resources = !ctx.data_resources.is_empty();

        tokio::task::block_in_place(|| {
            state.handle.block_on(async {
                // ── Requête principale ────────────────────────────────────────
                let mut query = sqlx::query(&sql_prepared);
                for val in &param_values {
                    query = query.bind(val);
                }

                let sql_upper = ctx.sql.trim().to_uppercase();

                if sql_upper.starts_with("SELECT") {
                    let rows = match query.fetch_all(&state.pool).await {
                        Ok(r)  => r,
                        Err(e) => return PluginResult::Error(e.to_string()),
                    };

                    let data: Vec<Value> = rows.iter().map(|row| {
                        let mut obj = Map::new();
                        for (i, col) in row.columns().iter().enumerate() {
                            let val: Option<String> = row.try_get(i).ok();
                            obj.insert(
                                col.name().to_string(),
                                val.map(Value::String).unwrap_or(Value::Null),
                            );
                        }
                        Value::Object(obj)
                    }).collect();

                    // ── Sans ressources → retour simple (tableGeneric, etc.) ──
                    if !has_resources {
                        return PluginResult::Data(Value::Array(data));
                    }

                    // ── Avec ressources → on exécute les sql_resources ────────
                    // resources : { "code_countries": [["FR","France"], ...] }
                    let mut resources: Map<String, Value> = Map::new();

                    for (field_name, resource_name) in &ctx.data_resources {
                        // Trouver le SQL associé à ce nom de ressource
                        let sql_res = match ctx.sql_resources.get(resource_name) {
                            Some(s) => s.clone(),
                            None    => {
                                eprintln!("[plugin_sql] sql_resources manquant pour '{}'", resource_name);
                                continue;
                            }
                        };

                        let res_rows = match sqlx::query(&sql_res)
                            .fetch_all(&state.pool)
                            .await
                        {
                            Ok(r)  => r,
                            Err(e) => {
                                eprintln!("[plugin_sql] Erreur ressource '{}' : {}", resource_name, e);
                                continue;
                            }
                        };

                        // Prend les deux premières colonnes : [valeur, label]
                        // Si une seule colonne : valeur = label
                        let pairs: Vec<Value> = res_rows.iter().map(|row| {
                            let col0: Option<String> = row.try_get(0).ok();
                            let col1: Option<String> = row.try_get(1).ok();
                            let val   = col0.clone().unwrap_or_default();
                            let label = col1.unwrap_or_else(|| col0.unwrap_or_default());
                            json!([val, label])
                        }).collect();

                        resources.insert(field_name.clone(), Value::Array(pairs));
                    }

                    // Structure finale : { data: [...], resources: { field: [[val,lbl],...] } }
                    PluginResult::Data(json!({
                        "data":      data,
                        "resources": resources,
                        "form_action": ctx.form_action.clone().unwrap_or_default(),
                    }))

                } else {
                    // ── Écriture (INSERT / UPDATE / DELETE) ──────────────────
                    match query.execute(&state.pool).await {
                        Ok(result) => PluginResult::Data(json!({
                            "rows_affected":  result.rows_affected(),
                            "last_insert_id": result.last_insert_id(),
                        })),
                        Err(e) => PluginResult::Error(e.to_string()),
                    }
                }
            })
        })
    }
}

/// Transforme "SELECT * FROM t WHERE code = :code AND name = :name"
/// en ("SELECT * FROM t WHERE code = ? AND name = ?", ["FR", "France"])
/// en respectant l'ordre d'apparition des paramètres dans la requête.
fn named_to_positional(
    sql: &str,
    params: &std::collections::HashMap<String, String>,
) -> (String, Vec<String>) {
    let re = regex::Regex::new(r":([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
    let mut values = Vec::new();
    let sql_out = re.replace_all(sql, |caps: &regex::Captures| {
        let name = &caps[1];
        values.push(params.get(name).cloned().unwrap_or_default());
        "?"
    });
    (sql_out.to_string(), values)
}

#[no_mangle]
pub fn plugin_entry(registrar: &mut dyn PluginRegistrar) {
    registrar.register_plugin(Box::new(PluginCountries));
}
