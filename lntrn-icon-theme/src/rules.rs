//! The icon theme's association tables: `<regex pattern=… icon=…
//! priority=…/>` elements read straight off the XML (no more XML than
//! that is needed), each with its pattern compiled.
//!
//! A table is asked for the first of its rules that takes a name. Nearly
//! every rule ends in `$` after a literal ending (`\.rs$`), so the rules
//! are also kept by the character a name has to end on: a name is tried
//! against the few that can take it, not all eighteen hundred.

use std::collections::HashMap;

use crate::pattern::Pattern;

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
    // One walk over the text, tag by tag. (Searching for each kind of
    // element from where the last one ended read the rest of the table
    // again for every element: a quarter of a second at every start.)
    while let Some(open) = rest.find('<') {
        let body = &rest[open..];
        if !(body.starts_with("<regex") || body.starts_with("<type ")) {
            rest = &body[1..];
            continue;
        }
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
        rest = &body[close + 1..];
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

/// One table's rules in the order they are tried.
#[derive(Default)]
pub struct Table {
    rules: Vec<Rule>,
    /// By the character a name ends on: the rules (their places in
    /// `rules`, in order) that take such a name and no other.
    by_last: HashMap<char, Vec<usize>>,
    /// The rules that may take a name ending on anything.
    any_last: Vec<usize>,
}

impl Table {
    pub fn new(rules: Vec<Rule>) -> Self {
        let mut by_last: HashMap<char, Vec<usize>> = HashMap::new();
        let mut any_last = Vec::new();
        for (at, rule) in rules.iter().enumerate() {
            match rule.pattern.last_chars() {
                Some(chars) => chars.into_iter().for_each(|c| by_last.entry(c).or_default().push(at)),
                None => any_last.push(at),
            }
        }
        Self { rules, by_last, any_last }
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// The first rule that takes `text`.
    pub fn first(&self, text: &str) -> Option<&Rule> {
        let chars: Vec<char> = text.chars().collect();
        let mut ending = chars.last().and_then(|c| self.by_last.get(c)).map_or(&[][..], Vec::as_slice).iter().peekable();
        let mut any = self.any_last.iter().peekable();
        // Both lists are in rule order: the earlier rule of the two is next.
        loop {
            let at = match (ending.peek(), any.peek()) {
                (Some(a), Some(b)) if a < b => ending.next(),
                (Some(_), None) => ending.next(),
                _ => any.next(),
            }?;
            let rule = &self.rules[*at];
            if rule.pattern.is_match_chars(text, &chars) {
                return Some(rule);
            }
        }
    }
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

    #[test]
    fn tags_are_found_wherever_they_are() {
        // Other tags, text and a comment between the elements; an
        // element spread over lines; one cut off at the end of the file.
        let xml = "<?xml version=\"1.0\"?><!-- a < b --><associations>\n  <other pattern=\"no\" icon=\"no.svg\"/>\n  <regex\n     pattern='^a$'\n     icon=\"/a.svg\"/> text <typed/>\n  <regex pattern=\"^b &amp; c$\" icon=\"/b.svg\"></regex><regex pattern=\"^d$\" icon=\"/d.svg\"";
        let r = parse(xml);
        assert_eq!(r.iter().map(|r| r.icon.as_str()).collect::<Vec<_>>(), ["a.svg", "b.svg"]);
        assert!(r[1].pattern.is_match("b & c"));
    }

    /// The first rule that takes `text`, every rule tried in order.
    fn plain_first<'a>(rules: &'a [Rule], text: &str) -> Option<&'a Rule> {
        rules.iter().find(|r| r.pattern.is_match_plain(text))
    }

    fn agree(table: &Table, names: &[String]) {
        for name in names {
            let (quick, plain) = (table.first(name), plain_first(&table.rules, name));
            assert_eq!(quick.map(|r| (r.icon.as_str(), r.priority)), plain.map(|r| (r.icon.as_str(), r.priority)), "{name:?}");
        }
    }

    #[test]
    fn the_first_rule_wins_whichever_list_it_is_kept_in() {
        let rule = |pattern: &str, icon: &str, priority| format!(r#"<regex pattern="{pattern}" icon="/{icon}.svg" priority="{priority}"/>"#);
        // Rules that name their ending and rules that do not, interleaved
        // by priority, several of them taking the same names.
        let xml = [
            rule(r"^readme(\.md)?$", "readme", 900),
            rule(r"^read.*", "read", 800),
            rule(r".*\.md$", "markdown", 700),
            rule(r".*\.(md|rs)$", "either", 600),
            rule(r"\.r", "dot-r", 500),
            rule(r".*\.rs$", "rust", 400),
            rule(r".*", "anything", 100),
        ]
        .concat();
        let table = Table::new(parse(&xml));
        assert_eq!(table.len(), 7);
        let icon = |name: &str| table.first(name).map(|r| r.icon.as_str());
        assert_eq!(icon("readme.md"), Some("readme.svg"));
        assert_eq!(icon("reading.md"), Some("read.svg"));
        assert_eq!(icon("notes.md"), Some("markdown.svg"));
        assert_eq!(icon("main.rs"), Some("either.svg"));
        assert_eq!(icon("a.rb"), Some("dot-r.svg"));
        assert_eq!(icon("plain"), Some("anything.svg"));
        assert_eq!(icon(""), Some("anything.svg"));
        assert_eq!(Table::default().first("main.rs").map(|r| r.icon.as_str()), None);
        let names: Vec<String> = ["readme.md", "readme", "reading.md", "notes.md", "main.rs", "a.rb", "plain", "", "x.md\n", "Ünï.md", ".rs", "md"].map(str::to_owned).into();
        agree(&table, &names);
    }

    /// The names a table gives as examples (`fileNames="a,b"`), which its
    /// rules were written for, and a few of no kind.
    fn examples(xml: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for attr in ["fileNames=\"", "folderNames=\""] {
            for piece in xml.split(attr).skip(1) {
                let list = piece.split('"').next().unwrap_or_default();
                names.extend(list.split(',').map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()));
            }
        }
        names.extend(["noidea.zzz", "holiday notes final (2).txt", "a", "some_really_long_file_name_with_many_parts.backup.tar.gz", "weird\nname.rs", "données.json", "."].map(str::to_owned));
        names.sort();
        names.dedup();
        names
    }

    fn installed(table: &str) -> Option<String> {
        let home = std::env::var_os("HOME")?;
        std::fs::read_to_string(std::path::Path::new(&home).join(".lantern/icons/atom-material").join(table)).ok()
    }

    /// The installed theme, when there is one: the quick way picks what
    /// the plain way picks, for a spread of the names the tables list.
    #[test]
    fn quick_and_plain_agree_on_the_installed_tables() {
        for file in ["icon_associations.xml", "folder_associations.xml"] {
            let Some(xml) = installed(file) else {
                eprintln!("no theme installed; skipped");
                return;
            };
            let table = Table::new(parse(&xml));
            let names: Vec<String> = examples(&xml).into_iter().step_by(40).collect();
            agree(&table, &names);
        }
    }

    /// Every example name of both tables, the plain way: minutes without
    /// `--release`. `cargo test -p lntrn-icon-theme --release -- --ignored`
    #[test]
    #[ignore]
    fn quick_and_plain_agree_on_every_example() {
        for file in ["icon_associations.xml", "folder_associations.xml"] {
            let xml = installed(file).expect("the theme is installed");
            let table = Table::new(parse(&xml));
            let names = examples(&xml);
            eprintln!("{file}: {} rules ({} tried for every name), {} names", table.len(), table.any_last.len(), names.len());
            agree(&table, &names);
        }
    }
}
