//! A small XML reader: a document as a tree of elements, each with its
//! attributes and what is inside it, and text made safe to write into
//! one. The parts of a Word document are written in it.
//!
//! - Names are kept as written, prefix and all (`w:p`): namespaces are
//!   not worked out.
//! - Text is kept exactly, blanks and all, with entities turned into
//!   their characters and line ends into `\n`, as XML says. A CDATA
//!   section is text like any other.
//! - The declaration, comments, processing instructions and a DOCTYPE
//!   are passed over. An entity a DOCTYPE defines is not known: only the
//!   five XML has, and numbered characters, are.
//! - A document that is not well formed is an error saying what is
//!   wrong, as is one nested deeper than [`MAX_DEPTH`]. Reading keeps
//!   its own list of what is open, so no document overflows the stack,
//!   and none makes whoever walks the tree afterwards do it either.

use std::borrow::Cow;

/// The deepest elements may nest.
pub const MAX_DEPTH: usize = 256;

/// The blanks XML knows.
const BLANKS: [char; 4] = [' ', '\t', '\n', '\r'];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    /// As written, with its prefix: `w:p`.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// What the attribute called `name` says.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    /// The elements right inside this one.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The first element right inside this one called `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    /// Every element right inside this one called `name`.
    pub fn named(&self, name: &str) -> impl Iterator<Item = &Element> {
        self.elements().filter(move |e| e.name == name)
    }

    /// The first element called `name` anywhere inside this one, each
    /// element looked into before the one after it.
    pub fn find(&self, name: &str) -> Option<&Element> {
        self.elements().find_map(|e| if e.name == name { Some(e) } else { e.find(name) })
    }

    /// Go through this element and all inside it, each before what it
    /// holds. `visit` says whether to go into the one it was shown.
    pub fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a Element) -> bool) {
        if visit(self) {
            for e in self.elements() {
                e.walk(visit);
            }
        }
    }

    /// All the text inside it, at any depth, in order.
    pub fn text(&self) -> String {
        fn gather(e: &Element, out: &mut String) {
            for node in &e.children {
                match node {
                    Node::Text(text) => out.push_str(text),
                    Node::Element(inner) => gather(inner, out),
                }
            }
        }
        let mut out = String::new();
        gather(self, &mut out);
        out
    }

    /// More text at its end, joined to the text already there.
    fn push_text(&mut self, text: &str) {
        match self.children.last_mut() {
            Some(Node::Text(last)) => last.push_str(text),
            _ if text.is_empty() => {}
            _ => self.children.push(Node::Text(text.to_owned())),
        }
    }
}

/// The start of a name, for a message: a broken document's may be as
/// long as the document.
fn brief(name: &str) -> &str {
    name.char_indices().nth(40).map_or(name, |(end, _)| &name[..end])
}

/// Whether XML lets a character be in a document at all.
fn legal(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | ' '..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..=char::MAX)
}

/// Text made safe to put between tags or in an attribute. What XML has
/// no way to hold (most control characters) is left out; a tab and a
/// line end are written by number, so an attribute keeps them too.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push_str(&format!("&#{};", u32::from(c))),
            c if legal(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Line ends as XML reads them: `\r\n` and a lone `\r` are one `\n`.
fn newlines(text: &str) -> Cow<'_, str> {
    if text.contains('\r') { Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n")) } else { Cow::Borrowed(text) }
}

/// The character an entity's name stands for: one of the five XML has,
/// or a numbered one that a document may hold.
fn entity(name: &str) -> Result<char, String> {
    let numbered = |n: &str| match n.strip_prefix('x') {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => n.parse().ok(),
    };
    let c = match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => name.strip_prefix('#').and_then(numbered).and_then(char::from_u32).filter(|c| legal(*c)),
    };
    c.ok_or_else(|| format!("&{}; is not a character", brief(name)))
}

/// Text as it was meant: line ends made one, and `&name;` and `&#N;` the
/// characters they stand for. In an attribute every blank is a space.
fn unescape(raw: &str, attr: bool) -> Result<String, String> {
    let raw = newlines(raw);
    let mut out = String::with_capacity(raw.len());
    let mut rest: &str = &raw;
    while let Some(at) = rest.find(|c: char| c == '&' || (attr && matches!(c, '\t' | '\n'))) {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        rest = if rest.as_bytes()[at] == b'&' {
            let (name, tail) = tail.split_once(';').ok_or("an & begins nothing")?;
            out.push(entity(name)?);
            tail
        } else {
            out.push(' ');
            tail
        };
    }
    out.push_str(rest);
    Ok(out)
}

/// The name `text` starts with, and what is after it.
fn name_at(text: &str) -> Option<(&str, &str)> {
    let end = text.find(|c: char| BLANKS.contains(&c) || matches!(c, '/' | '>' | '=' | '<' | '"' | '\'' | '&')).unwrap_or(text.len());
    (end > 0).then(|| text.split_at(end))
}

/// A tag from after its `<`: the element it starts, whether it is whole
/// in itself (`<a/>`), and what comes after it.
fn start_tag(text: &str) -> Result<(Element, bool, &str), String> {
    let (name, mut rest) = name_at(text).ok_or("a tag has no name")?;
    let mut element = Element { name: name.to_owned(), ..Element::default() };
    let unended = || format!("<{}> is never ended", brief(name));
    loop {
        rest = rest.trim_start_matches(BLANKS);
        if let Some(tail) = rest.strip_prefix("/>") {
            return Ok((element, true, tail));
        }
        if let Some(tail) = rest.strip_prefix('>') {
            return Ok((element, false, tail));
        }
        if rest.is_empty() {
            return Err(unended());
        }
        let (key, after) = name_at(rest).ok_or_else(|| format!("<{}> has something in it that is no attribute", brief(name)))?;
        let value = after.trim_start_matches(BLANKS).strip_prefix('=').ok_or_else(|| format!("the attribute {} of <{}> has no value", brief(key), brief(name)))?.trim_start_matches(BLANKS);
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\'')).ok_or_else(|| format!("the attribute {} of <{}> is not in quotes", brief(key), brief(name)))?;
        let (value, tail) = value[1..].split_once(quote).ok_or_else(unended)?;
        element.attrs.push((key.to_owned(), unescape(value, true)?));
        rest = tail;
    }
}

/// What comes after a `<!DOCTYPE ...>`, from after its `<!`. It may hold
/// quoted words and a bracketed list of declarations with `>` in them.
fn after_doctype(text: &str) -> Result<&str, String> {
    let (mut depth, mut quote) = (0usize, None);
    for (at, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth = depth.saturating_sub(1),
            (None, '>') if depth == 0 => return Ok(&text[at + 1..]),
            (None, _) => {}
        }
    }
    Err("a declaration is never closed".to_owned())
}

/// An element that is finished goes into the one it is in, or is the
/// document's outermost.
fn close(done: Element, open: &mut [Element], root: &mut Option<Element>) {
    match open.last_mut() {
        Some(parent) => parent.children.push(Node::Element(done)),
        None => *root = Some(done),
    }
}

/// Read a document: its outermost element, with everything inside it.
pub fn parse(text: &str) -> Result<Element, String> {
    const OUTSIDE: &str = "there is text outside the outermost element";
    let mut rest = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    // The elements not yet closed, outermost first, and the outermost of
    // all once it is.
    let mut open: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    while !rest.is_empty() {
        let Some(tag) = rest.strip_prefix('<') else {
            let (text, tail) = rest.split_at(rest.find('<').unwrap_or(rest.len()));
            match open.last_mut() {
                Some(e) => e.push_text(&unescape(text, false)?),
                None if text.trim_matches(BLANKS).is_empty() => {}
                None => return Err(OUTSIDE.to_owned()),
            }
            rest = tail;
            continue;
        };
        if let Some(after) = tag.strip_prefix("!--") {
            rest = after.split_once("-->").ok_or("a comment is never closed")?.1;
        } else if let Some(after) = tag.strip_prefix("![CDATA[") {
            let (data, tail) = after.split_once("]]>").ok_or("a CDATA section is never closed")?;
            open.last_mut().ok_or(OUTSIDE)?.push_text(&newlines(data));
            rest = tail;
        } else if let Some(after) = tag.strip_prefix('?') {
            rest = after.split_once("?>").ok_or("a processing instruction is never closed")?.1;
        } else if let Some(after) = tag.strip_prefix('!') {
            rest = after_doctype(after)?;
        } else if let Some(after) = tag.strip_prefix('/') {
            let (name, tail) = after.split_once('>').ok_or("a closing tag is never ended")?;
            let name = name.trim_end_matches(BLANKS);
            let done = open.pop().ok_or_else(|| format!("</{}> closes nothing", brief(name)))?;
            if done.name != name {
                return Err(format!("<{}> is closed by </{}>", brief(&done.name), brief(name)));
            }
            close(done, &mut open, &mut root);
            rest = tail;
        } else {
            let (element, whole, tail) = start_tag(tag)?;
            if open.is_empty() && root.is_some() {
                return Err(format!("<{}> is a second outermost element", brief(&element.name)));
            }
            if open.len() >= MAX_DEPTH {
                return Err(format!("elements are nested more than {MAX_DEPTH} deep"));
            }
            if whole {
                close(element, &mut open, &mut root);
            } else {
                open.push(element);
            }
            rest = tail;
        }
    }
    match open.last() {
        Some(e) => Err(format!("<{}> is never closed", brief(&e.name))),
        None => root.ok_or_else(|| "there is no element in it".to_owned()),
    }
}

/// Read a document from a file's bytes: UTF-16 when they start as that
/// does, else UTF-8. Bytes that are not text in it become U+FFFD rather
/// than a refusal.
pub fn parse_bytes(bytes: &[u8]) -> Result<Element, String> {
    let utf16 = |big: bool| {
        let unit = |pair: &[u8]| if big { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) };
        String::from_utf16_lossy(&bytes.chunks_exact(2).map(unit).collect::<Vec<_>>())
    };
    match bytes {
        [0xFF, 0xFE, ..] | [b'<', 0, ..] => parse(&utf16(false)),
        [0xFE, 0xFF, ..] | [0, b'<', ..] => parse(&utf16(true)),
        _ => parse(&String::from_utf8_lossy(bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Node {
        Node::Text(s.to_owned())
    }

    fn element(name: &str, attrs: &[(&str, &str)], children: Vec<Node>) -> Node {
        Node::Element(Element { name: name.to_owned(), attrs: attrs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(), children })
    }

    #[test]
    fn a_document_with_everything_in_it_is_read_as_it_is_written() {
        let file = concat!(
            "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n",
            "<!DOCTYPE w:doc [ <!ENTITY who \"them > us\"> <!ELEMENT w:doc ANY> ]>\n",
            "<!-- a comment, with <tags> & such in it -->\n",
            "<w:doc xmlns:w=\"urn:w\" w:id = '7' title=\"a &lt;b&gt; &amp; &quot;c&quot; &apos;d&apos; &#233;&#x1F3EE;\" lines=\"one\r\ntwo\tthree&#10;four\">\n",
            "  <w:p>plain <w:b>bold</w:b> tail</w:p >\n",
            "  <empty/><spaced  a=\"1\"   b='say \"hi\"'/>\n",
            "  <![CDATA[<not a tag> & not &amp; an entity]]><?target some instruction?> x &lt; y &gt; z &#65;&#x42;\r\n",
            "  <w:t xml:space=\"preserve\">  kept  </w:t><!-- between --></w:doc>\n",
            "<!-- after --> <?and this?>\n"
        );
        let doc = parse(file).unwrap();
        assert_eq!((doc.name.as_str(), doc.attr("w:id"), doc.attr("title"), doc.attr("lines"), doc.attr("id")), ("w:doc", Some("7"), Some("a <b> & \"c\" 'd' \u{e9}\u{1F3EE}"), Some("one two three\nfour"), None));
        let inside = vec![
            text("\n  "),
            element("w:p", &[], vec![text("plain "), element("w:b", &[], vec![text("bold")]), text(" tail")]),
            text("\n  "),
            element("empty", &[], vec![]),
            element("spaced", &[("a", "1"), ("b", "say \"hi\"")], vec![]),
            text("\n  <not a tag> & not &amp; an entity x < y > z AB\n  "),
            element("w:t", &[("xml:space", "preserve")], vec![text("  kept  ")]),
        ];
        assert_eq!(doc.children, inside);
        // What is right inside it, what is anywhere inside it, and all
        // of it gone through in order, with one branch left out.
        assert_eq!(doc.elements().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["w:p", "empty", "spaced", "w:t"]);
        assert_eq!((doc.child("w:t").map(Element::text), doc.child("w:b"), doc.find("w:b").map(Element::text)), (Some("  kept  ".to_owned()), None, Some("bold".to_owned())));
        assert_eq!((doc.named("w:p").count(), doc.named("nothing").count(), doc.child("w:p").unwrap().text()), (1, 0, "plain bold tail".to_owned()));
        let mut seen = Vec::new();
        doc.walk(&mut |e| {
            seen.push(e.name.as_str());
            e.name != "w:p"
        });
        assert_eq!(seen, ["w:doc", "w:p", "empty", "spaced", "w:t"]);
        // Line ends are one newline, unless written by number.
        assert_eq!(parse("<a>one\r\ntwo\rthree&#13;</a>").unwrap().text(), "one\ntwo\nthree\r");
        // What is escaped comes back, in text and in an attribute; what
        // XML cannot hold is left out.
        let odd = "a<b>&\"c\" 'd'\t\n\r \u{e9}\u{1F3EE}]]>";
        assert_eq!(escape(&format!("{odd}\u{1}\u{FFFE}")), "a&lt;b&gt;&amp;&quot;c&quot; &apos;d&apos;&#9;&#10;&#13; \u{e9}\u{1F3EE}]]&gt;");
        let back = parse(&format!("<a b=\"{0}\">{0}</a>", escape(odd))).unwrap();
        assert_eq!((back.attr("b"), back.text().as_str()), (Some(odd), odd));
        // Bytes in either of the encodings a file may be in.
        let wide = |big: bool| "\u{FEFF}<a>h\u{e9}\u{1F3EE}</a>".encode_utf16().flat_map(|u| if big { u.to_be_bytes() } else { u.to_le_bytes() }).collect::<Vec<u8>>();
        for bytes in [wide(false), wide(true), wide(false)[2..].to_vec(), wide(true)[2..].to_vec(), "\u{FEFF}<a>h\u{e9}\u{1F3EE}</a>".as_bytes().to_vec()] {
            assert_eq!(parse_bytes(&bytes).unwrap().text(), "h\u{e9}\u{1F3EE}");
        }
        assert_eq!(parse_bytes(b"<a>caf\xE9</a>").unwrap().text(), "caf\u{FFFD}");
    }

    #[test]
    fn a_document_that_is_not_well_formed_is_an_error_and_never_a_panic() {
        let wrong = [
            ("", "there is no element in it"),
            ("  <!-- only this -->  ", "there is no element in it"),
            ("just text", "there is text outside the outermost element"),
            ("<a/> trailing", "there is text outside the outermost element"),
            ("<![CDATA[early]]><a/>", "there is text outside the outermost element"),
            ("<a><b></b>", "<a> is never closed"),
            ("<a></b>", "<a> is closed by </b>"),
            ("<a><b></a></b>", "<b> is closed by </a>"),
            ("</a>", "</a> closes nothing"),
            ("<a/><b/>", "<b> is a second outermost element"),
            ("<a>&nope;</a>", "&nope; is not a character"),
            ("<a>&#0;</a>", "&#0; is not a character"),
            ("<a b=\"&#xD800;\"/>", "&#xD800; is not a character"),
            ("<a>&#x110000;</a>", "&#x110000; is not a character"),
            ("<a>this & that</a>", "an & begins nothing"),
            ("<a b=c/>", "the attribute b of <a> is not in quotes"),
            ("<a b/>", "the attribute b of <a> has no value"),
            ("<a b=\"c/>", "<a> is never ended"),
            ("<a \"b\"/>", "<a> has something in it that is no attribute"),
            ("<a", "<a> is never ended"),
            ("<a></a", "a closing tag is never ended"),
            ("< a/>", "a tag has no name"),
            ("<a><!-- open</a>", "a comment is never closed"),
            ("<a><![CDATA[ open</a>", "a CDATA section is never closed"),
            ("<?xml open", "a processing instruction is never closed"),
            ("<!DOCTYPE a [ <!ENTITY b \"c\"> <a/>", "a declaration is never closed"),
        ];
        for (file, why) in wrong {
            assert_eq!(parse(file), Err(why.to_owned()), "{file}");
        }
        // As deep as is let, and one deeper; then far deeper than any
        // stack, closed or not.
        let nested = |depth: usize| format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
        let deepest = parse(&nested(MAX_DEPTH)).unwrap();
        let mut depth = 0;
        deepest.walk(&mut |_| {
            depth += 1;
            true
        });
        assert_eq!(depth, MAX_DEPTH);
        let too_deep = Err("elements are nested more than 256 deep".to_owned());
        assert_eq!((parse(&nested(MAX_DEPTH + 1)), parse(&nested(200_000)), parse(&"<a>".repeat(1_000_000))), (too_deep.clone(), too_deep.clone(), too_deep.clone()));
        assert_eq!(parse(&format!("{}<b/>{}", "<a>".repeat(MAX_DEPTH), "</a>".repeat(MAX_DEPTH))), too_deep);
        // A name out of a broken file is not quoted at its whole length.
        assert!(parse(&format!("<{}", "x".repeat(100_000))).unwrap_err().len() < 100);
        // Whatever is cut off, wherever, is an error or a document.
        let file = "<?xml version=\"1.0\"?><!DOCTYPE a [<!ENTITY b 'c'>]><a b=\"c &amp; d\" e='f'><!-- g --><h>i &#106; <![CDATA[k]]></h><l/>\u{e9}</a>";
        for cut in (0..file.len()).filter(|cut| file.is_char_boundary(*cut)) {
            assert!(parse(&file[..cut]).is_err(), "{}", &file[..cut]);
            let _ = parse(&file[cut..]);
        }
    }
}
