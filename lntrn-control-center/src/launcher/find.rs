//! Search: the apps a query finds, best first. A name that starts with it
//! beats one with a word that does, which beats one that has it anywhere;
//! then what kind of app it is, its keywords and its id. Ties keep the
//! apps' own order, by name.

use lntrn_apps::App;

/// The indices into `apps` that `query` finds, best first.
pub fn matches(apps: &[App], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..apps.len()).collect();
    }
    let mut ranked: Vec<(u8, usize)> = apps.iter().enumerate().filter_map(|(i, app)| rank(app, &q).map(|r| (r, i))).collect();
    ranked.sort();
    ranked.into_iter().map(|(_, i)| i).collect()
}

/// How well `app` matches `q` (lower is better), or `None`.
fn rank(app: &App, q: &str) -> Option<u8> {
    let name = app.name.to_lowercase();
    if name.starts_with(q) {
        return Some(0);
    }
    if name.split(|c: char| c.is_whitespace() || matches!(c, '-' | '.' | ':' | '_')).any(|w| w.starts_with(q)) {
        return Some(1);
    }
    if name.contains(q) {
        return Some(2);
    }
    let has = |s: &str| s.to_lowercase().contains(q);
    (has(&app.generic) || app.keywords.iter().any(|k| has(k)) || has(&app.id)).then_some(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, generic: &str, keywords: &[&str], id: &str) -> App {
        App { id: id.into(), name: name.into(), generic: generic.into(), keywords: keywords.iter().map(|k| (*k).to_owned()).collect(), ..App::default() }
    }

    #[test]
    fn the_best_match_comes_first() {
        // Already by name, as installed() gives them.
        let apps = [app("Code", "Text Editor", &[], "lntrn-code"), app("Google Chrome", "Web Browser", &["internet"], "google-chrome"), app("Mozilla Firefox", "Web Browser", &["www"], "firefox-bin"), app("Visual Code", "", &[], "vscode")];
        assert_eq!(matches(&apps, "code"), vec![0, 3], "the name's start before a later word");
        assert_eq!(matches(&apps, "fire"), vec![2]);
        assert_eq!(matches(&apps, "ozil"), vec![2], "anywhere in the name");
        assert_eq!(matches(&apps, "browser"), vec![1, 2], "what kind of app it is");
        assert_eq!(matches(&apps, " WWW "), vec![2], "a keyword, whatever its case and spaces");
        assert_eq!(matches(&apps, "lntrn"), vec![0], "the id");
        assert!(matches(&apps, "zzz").is_empty());
        assert_eq!(matches(&apps, "").len(), 4);
    }
}
