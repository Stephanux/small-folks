//! Helpers Handlebars personnalisés pour small-folks.
//!
//! Chaque helper est implémenté comme une struct qui implémente `HelperDef`.
//! Cette approche est nécessaire pour les helpers de blocs ({{#helper}}...{{/helper}})
//! car les lifetimes 'reg: 'rc ne peuvent pas être exprimées dans une closure.
//!
//! ## Ajouter un nouveau helper
//! 1. Créer une struct `MonHelper;`
//! 2. Implémenter `HelperDef` avec `fn call<'reg: 'rc, 'rc>(...)`
//! 3. L'enregistrer dans `register_all()` via `hbs.register_helper(...)`
//!
//! ## Utilisation dans dispatcher.rs
//! ```rust
//! use crate::helpers_hbs;
//! helpers_hbs::register_all(&mut hbs);
//! ```

use handlebars::{Context, Handlebars, Helper, HelperDef, HelperResult, Output, Renderable, RenderContext, ScopedJson};

/// Enregistre tous les helpers dans l'instance Handlebars.
/// À appeler une seule fois dans `Dispatcher::new()`.
pub fn register_all(hbs: &mut Handlebars) {
    hbs.register_helper("compare", Box::new(CompareHelper));
    hbs.register_helper("json",    Box::new(JsonHelper));
        // eq, gt, lt, gte, lte, ne, and, or, not sont des helpers NATIFS
    // de Handlebars Rust — pas besoin de les enregistrer manuellement
    // ← ajouter les futurs helpers ici
}

// ── Helper : compare ─────────────────────────────────────────────────────────
//
// Compare deux valeurs chaînes avec un opérateur configurable.
// Traduit depuis le helper JS hbs.registerHelper('compare', ...).
//
// Syntaxe :
//   {{#compare val "actif"}}vrai{{/compare}}
//   {{#compare val "actif"}}vrai{{else}}faux{{/compare}}
//   {{#compare role "admin"  operator="=="}}...{{/compare}}
//   {{#compare nb   "10"     operator=">"}}...{{/compare}}
//   {{#compare nb   "10"     operator="<="}}...{{/compare}}
//   {{#compare val  "x"      operator="!="}}...{{/compare}}
//
// Opérateurs supportés : == === != !== > >= < <=
// Pour > >= < <= : comparaison numérique si possible, sinon lexicographique.

struct CompareHelper;

impl HelperDef for CompareHelper {
    fn call<'reg: 'rc, 'rc>(
        &self,
        h:        &Helper<'rc>,
        hbs_inst: &'reg Handlebars<'reg>,
        ctx:      &'rc Context,
        rc:       &mut RenderContext<'reg, 'rc>,
        out:      &mut dyn Output,
    ) -> HelperResult {

        // ── Paramètres positionnels ───────────────────────────────────────────
        let lvalue = h.param(0)
            .and_then(|v| v.value().as_str())
            .unwrap_or("")
            .to_string();

        let rvalue = h.param(1)
            .and_then(|v| v.value().as_str())
            .unwrap_or("")
            .to_string();

        // ── Opérateur depuis le hash (défaut "==") ────────────────────────────
        let operator = h.hash_get("operator")
            .and_then(|v| v.value().as_str())
            .unwrap_or("==");

        // ── Évaluation ────────────────────────────────────────────────────────
        let result = match operator {
            "==" | "===" => lvalue == rvalue,
            "!=" | "!==" => lvalue != rvalue,
            ">"  => num_cmp(&lvalue, &rvalue, |l, r| l > r,  |l: &str, r: &str| l > r),
            ">=" => num_cmp(&lvalue, &rvalue, |l, r| l >= r, |l: &str, r: &str| l >= r),
            "<"  => num_cmp(&lvalue, &rvalue, |l, r| l < r,  |l: &str, r: &str| l < r),
            "<=" => num_cmp(&lvalue, &rvalue, |l, r| l <= r, |l: &str, r: &str| l <= r),
            op   => {
                eprintln!("[compare helper] Opérateur inconnu : '{}'", op);
                false
            }
        };

        // ── Rendu du bloc fn (vrai) ou inverse/else (faux) ───────────────────
        match if result { h.template() } else { h.inverse() } {
            Some(t) => t.render(hbs_inst, ctx, rc, out),
            None    => Ok(()),
        }
    }
}

/// Comparaison avec fallback numérique → lexicographique.
fn num_cmp<F, G>(l: &str, r: &str, num_op: F, str_op: G) -> bool
where
    F: Fn(f64, f64) -> bool,
    G: Fn(&str, &str) -> bool,
{
    if let (Ok(ln), Ok(rn)) = (l.parse::<f64>(), r.parse::<f64>()) {
        num_op(ln, rn)
    } else {
        str_op(l, r)
    }
}

// ── Helper : json ─────────────────────────────────────────────────────────────
//
// Sérialise une valeur Handlebars en JSON brut pour injection dans <script>.
// À utiliser avec TRIPLE accolades pour désactiver l'échappement HTML :
//
//   {{{json data}}}       ← tableau JSON complet
//   {{{json data.0}}}     ← premier élément seulement
//
// Sans les triples accolades, les " seraient transformés en &quot;
// et le JSON serait invalide en JavaScript.
//
// Exemple dans un template :
//   <script>
//     const rows = {{{json data}}};
//     const labels = rows.map(r => r.timestamp);
//     const temps  = rows.map(r => parseFloat(r.temperature));
//   </script>

struct JsonHelper;

impl HelperDef for JsonHelper {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h:  &Helper<'rc>,
        _:  &'reg Handlebars<'reg>,
        _:  &'rc Context,
        _:  &mut RenderContext<'reg, 'rc>,
    ) -> Result<ScopedJson<'rc>, handlebars::RenderError> {
        let value = h.param(0)
            .map(|v| v.value().clone())
            .unwrap_or(serde_json::Value::Null);

        let json_str = serde_json::to_string(&value)
            .unwrap_or_else(|_| "null".to_string());

        Ok(ScopedJson::Derived(serde_json::Value::String(json_str)))
    }
}
