use plugin_core::{ActionContext, AppState, Plugin, PluginRegistrar, PluginResult};
use sqlx::Column;

pub struct PluginRegions;

impl Plugin for PluginRegions {
    fn name(&self) -> &'static str {
        "regions"
    }

    fn execute(&self, ctx: &ActionContext, state: &AppState) -> PluginResult {
        let (sql_prepared, param_values) = named_to_positional(&ctx.sql, &ctx.params);

        tokio::task::block_in_place(|| {
            state.handle.block_on(async {
                let mut query = sqlx::query(&sql_prepared);
                for val in &param_values {
                    query = query.bind(val);
                }

                let sql_upper = ctx.sql.trim().to_uppercase();

                if sql_upper.starts_with("SELECT") {
                    match query.fetch_all(&state.pool).await {
                        Ok(rows) => {
                            let data: Vec<serde_json::Value> = rows.iter().map(|row| {
                                use sqlx::Row;
                                let mut obj = serde_json::Map::new();
                                for (i, col) in row.columns().iter().enumerate() {
                                    let val: Option<String> = row.try_get(i).ok();
                                    obj.insert(
                                        col.name().to_string(),
                                        val.map(serde_json::Value::String) // NB: Les données en sortie de SQL doivent être des "CHAR"
                                            .unwrap_or(serde_json::Value::Null),  // sinon on affecte Null à la valeur en sortie vers Handlebars
                                    );
                                }
                                serde_json::Value::Object(obj)
                            }).collect();
                            PluginResult::Data(serde_json::Value::Array(data))
                        }
                        Err(e) => PluginResult::Error(e.to_string()),
                    }
                } else {
                    match query.execute(&state.pool).await {
                        Ok(result) => {
                            PluginResult::Data(serde_json::json!({
                                "rows_affected": result.rows_affected(),
                                "last_insert_id": result.last_insert_id(),
                            }))
                        }
                        Err(e) => PluginResult::Error(e.to_string()),
                    }
                }
            })
        })
    }
}

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
    registrar.register_plugin(Box::new(PluginRegions));
}
