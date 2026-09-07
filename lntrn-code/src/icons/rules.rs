//! The icon theme's association tables: `<regex pattern=… icon=…
//! priority=…/>` elements read straight off the XML (no more XML than
//! that is needed), each with its pattern compiled.

use super::pattern::Pattern;

#[derive(Clone, Debug)]
pub struct Rule {
    pub pattern: Pattern,
    /// The icon's file name (`rust.svg`), wherever the table put it.
    pub icon: String,
    pub priority: i64,
}

/// Every `<regex …/>` (or `<type …/>`) element's attributes, in order.
fn elements(xml: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find("<regex").or_else(|| rest.find("<type ")).map(|k| if rest[k..].starts_with("<regex") { k } else { k }) {
        // Whichever comes first.
        let r = rest.find("<regex");
        let t = rest.find("<type ");
        let open = match (r, t) {
            (Some(a), Some(b)) => a.min(b),
            _ => open,
        };
        let body = &rest[open..];
        let Some(close) = body.find("/>").or_else(|| body.find('>')) else { break };
        let tag = &body[..close];
        let mut attrs = Vec::new();
        let mut s = tag;
        while let Some(eq) = s.find('=') {
            let key = s[..eq].trim().rsplit(|c: char| c.is_whitespace()).next().unwrap_or("").to_owned();
            let after = &s[eq + 1..];
            let Some(q) = after.chars().next() else { break };
            if q != '"' && q != '\'' {
                s = after;
                continue;
            }
            let Some(end) = after[1..].find(q) else { break };
            attrs.push((key, after[1..1 + end].replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"")));
            s = &after[1 + end + 1..];
        }
        out.push(attrs);
        rest = &rest[open + close + 1..];
    }
    out
}

/// The rules of one table, highest priority first (ties keep their order).
pub fn parse(xml: &str) -> Vec<Rule> {
    let mut rules: Vec<Rule> = elements(xml)
        .into_iter()
        .filter_map(|attrs| {
            let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
            let pattern = Pattern::new(get("pattern")?)?;
            let icon = get("icon")?.rsplit('/').next()?.to_owned();
            let priority = get("priority").and_then(|p| p.parse().ok()).unwrap_or(0);
            Some(Rule { pattern, icon, priority })
        })
        .collect();
    rules.sort_by_key(|r| std::cmp::Reverse(r.priority));
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rules_by_priority() {
        let xml = r#"<associations><regex fileNames="a" name="A" priority="10" iconType="FILE" pattern="\.a$" icon="/icons/files/a.svg"/>
        <type name="Db" type="Database Element" iconType="FILE" priority="10" icon="/icons/files/db.svg"/>
        <regex name="B" priority="1000" pattern="^b\.(md|txt)$" icon="/b.svg" defaultState="false"/></associations>"#;
        let r = parse(xml);
        assert_eq!(r.len(), 2, "the type element has no pattern");
        assert_eq!((r[0].icon.as_str(), r[0].priority), ("b.svg", 1000));
        assert!(r[0].pattern.is_match("b.md") && r[1].pattern.is_match("x.a"));
    }
}
