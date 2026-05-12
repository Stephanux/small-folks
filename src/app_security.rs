    //! Fonctions de sécurité pour small-folks.
//!
//! Ce module centralise les protections contre :
//! - Open Redirect        → sanitize_redirect()
//! - Header Injection     → sanitize_header()
//! - Log Injection        → sanitize_log()
//! - XSS dans les erreurs → sanitize_html()
//!
//! Note : la protection XSS principale est assurée par Handlebars
//! qui échappe automatiquement toutes les valeurs `{{value}}`.
//! Ces fonctions couvrent les cas hors-template.

/// Valide qu'une URL de redirection est relative (commence par /).
/// Protège contre les Open Redirect vers des sites externes.
///
/// # Exemples
/// ```
/// assert_eq!(sanitize_redirect("/users"),              "/users");
/// assert_eq!(sanitize_redirect("/login?next=/users"),  "/login?next=/users");
/// assert_eq!(sanitize_redirect("http://evil.com"),     "/");  // ← bloqué
/// assert_eq!(sanitize_redirect("//evil.com/path"),     "/");  // ← bloqué
/// assert_eq!(sanitize_redirect("javascript:alert(1)"), "/");  // ← bloqué
/// assert_eq!(sanitize_redirect(""),                    "/");
/// ```
pub fn sanitize_redirect(url: &str) -> String {
    let url = url.trim();

    // URL vide → racine
    if url.is_empty() {
        return "/".to_string();
    }

    // Doit commencer par / mais pas par // (protocol-relative URL)
    // et ne pas contenir de schéma (http:, https:, javascript:, etc.)
    let lower = url.to_lowercase();
    let is_safe = url.starts_with('/')
        && !url.starts_with("//")
        && !lower.contains(':')
        && !lower.contains("javascript")
        && !lower.contains("<script");

    if is_safe {
        // Supprimer les sauts de ligne qui pourraient causer du header splitting
        url.replace(['\r', '\n'], "")
    } else {
        eprintln!("[security] Open Redirect bloqué : '{}'", sanitize_log(url));
        "/".to_string()
    }
}

/// Nettoie une valeur destinée à être placée dans un header HTTP.
/// Protège contre le Header Splitting (injection de \r\n).
///
/// # Exemples
/// ```
/// assert_eq!(sanitize_header("abc123-xyz"),          "abc123-xyz");
/// assert_eq!(sanitize_header("val\r\nX-Evil: hack"), "valX-Evil: hack");
/// assert_eq!(sanitize_header("val\nSet-Cookie: x"),  "valSet-Cookie: x");
/// ```
pub fn sanitize_header(value: &str) -> String {
    value.replace(['\r', '\n'], "")
}

/// Nettoie un message destiné aux logs.
/// Protège contre le Log Injection (faux messages de log forgés via \n).
///
/// # Exemples
/// ```
/// assert_eq!(sanitize_log("erreur normale"),          "erreur normale");
/// assert_eq!(sanitize_log("msg\n[FAKE] Admin login"), "msg [FAKE] Admin login");
/// ```
pub fn sanitize_log(msg: &str) -> String {
    msg.replace('\n', " ").replace('\r', " ")
}

/// Échappe les caractères HTML spéciaux dans un message d'erreur
/// destiné à être inséré dans une réponse JSON ou HTML hors-template.
/// Protège contre le XSS dans les messages d'erreur renvoyés au client.
///
/// # Exemples
/// ```
/// assert_eq!(sanitize_html("<script>alert(1)</script>"),
///            "&lt;script&gt;alert(1)&lt;/script&gt;");
/// assert_eq!(sanitize_html("erreur \"normale\" & valide"),
///            "erreur &quot;normale&quot; &amp; valide");
/// ```
pub fn sanitize_html(input: &str) -> String {
    input
        .replace('&',  "&amp;")
        .replace('<',  "&lt;")
        .replace('>',  "&gt;")
        .replace('"',  "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_redirect_safe() {
        assert_eq!(sanitize_redirect("/users"),             "/users");
        assert_eq!(sanitize_redirect("/login?next=/users"), "/login?next=/users");
        assert_eq!(sanitize_redirect("/health/dashboard"),  "/health/dashboard");
    }

    #[test]
    fn test_sanitize_redirect_blocked() {
        assert_eq!(sanitize_redirect("http://evil.com"),      "/");
        assert_eq!(sanitize_redirect("https://evil.com"),     "/");
        assert_eq!(sanitize_redirect("//evil.com/path"),      "/");
        assert_eq!(sanitize_redirect("javascript:alert(1)"),  "/");
        assert_eq!(sanitize_redirect(""),                     "/");
        assert_eq!(sanitize_redirect("relative/no/slash"),    "/");
    }

    #[test]
    fn test_sanitize_header() {
        assert_eq!(sanitize_header("normal-value"),           "normal-value");
        assert_eq!(sanitize_header("val\r\nX-Evil: h"),       "valX-Evil: h");
        assert_eq!(sanitize_header("val\nSet-Cookie: x=1"),   "valSet-Cookie: x=1");
    }

    #[test]
    fn test_sanitize_log() {
        assert_eq!(sanitize_log("erreur normale"),            "erreur normale");
        assert_eq!(sanitize_log("msg\n[FAKE] Admin"),         "msg [FAKE] Admin");
        assert_eq!(sanitize_log("a\rb"),                      "a b");
    }

    #[test]
    fn test_sanitize_html() {
        assert_eq!(sanitize_html("<script>alert(1)</script>"),
                   "&lt;script&gt;alert(1)&lt;/script&gt;");
        assert_eq!(sanitize_html("test & \"value\""),
                   "test &amp; &quot;value&quot;");
        assert_eq!(sanitize_html("it's fine"),
                   "it&#x27;s fine");
    }
}
