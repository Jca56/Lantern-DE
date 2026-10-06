//! Markdown read and written.

use lntrn_core::encoding::base64_encode;
use lntrn_image::{Image, encode_png};

use super::*;
use crate::doc::{OBJECT, Pos};

/// Each paragraph's text, style, kind of item and level.
fn shape(doc: &Doc) -> Vec<(&str, Style, List, u8)> {
    doc.paras.iter().map(|p| (p.text.as_str(), p.attrs.style, p.attrs.list, p.attrs.level)).collect()
}

/// What a run has, a letter each: bold, italic, struck, underlined, code.
fn letters(a: &TextAttrs) -> String {
    [(a.bold, 'b'), (a.italic, 'i'), (a.strike, 's'), (a.underline, 'u'), (a.font == Some(Font::named(MONO)), 'c')].iter().filter(|(on, _)| *on).map(|(_, c)| *c).collect()
}

/// A paragraph's dressed runs: their text and letters.
fn runs(p: &Para) -> Vec<(&str, String)> {
    p.spans.list().iter().map(|s| (&p.text[s.start..s.end], letters(&s.attrs))).collect()
}

/// Bytes `from..to` of a paragraph, and the letters of what they have.
type Dress<'a> = &'a [(usize, usize, &'a str)];

/// One paragraph, dressed so.
fn dressed(text: &str, runs: Dress) -> Doc {
    let mut doc = Doc::from_text(text);
    for &(from, to, letters) in runs {
        let has = |c: char| letters.contains(c);
        doc.format(Pos::new(0, from), Pos::new(0, to), |a| (a.bold, a.italic, a.strike, a.underline, a.font) = (a.bold || has('b'), a.italic || has('i'), a.strike || has('s'), a.underline || has('u'), a.font.or(has('c').then(|| Font::named(MONO)))));
    }
    doc
}

fn back(doc: &Doc) -> Doc {
    read(&write(doc), None)
}

const NOTES: &str = "\u{FEFF}# Field Notes\r\n\r\nSoft-wrapped prose that goes on\nover two lines, with **bold**, *italic*, ***both***,\n__bold__ and _italic_ again, ~~struck~~, <u>under</u> and `code`.\nA hard break ends here  \nand here\\\nthen the last line.\n\n## Chapter\n### Section ###\n#### Sub\n###### Deeper\n\n> a quote\n> over two lines\nand a lazy third\n\n- one\n  - two\n      - three\n  - two again\n- back\n* star\n+ plus\n\n1. first\n2. second\n   1) child\n\t- tabbed\n\n- [ ] todo\n- [x] done\n- [X] DONE\n\n```rust\nfn main() {\n\n    let x = \"**1**\";\n}\n```\nafter\n";

#[test]
fn a_markdown_file_is_read_into_its_paragraphs_lists_and_runs() {
    let doc = read(NOTES, None);
    let (b, n, t) = (List::Bullet, List::Number, List::None);
    assert_eq!(shape(&doc)[..9], [("Field Notes", Style::Title, t, 0), ("Soft-wrapped prose that goes on over two lines, with bold, italic, both, bold and italic again, struck, under and code. A hard break ends here", Style::Body, t, 0), ("and here", Style::Body, t, 0), ("then the last line.", Style::Body, t, 0), ("Chapter", Style::Heading1, t, 0), ("Section", Style::Heading2, t, 0), ("Sub", Style::Heading3, t, 0), ("Deeper", Style::Heading3, t, 0), ("a quote over two lines and a lazy third", Style::Quote, t, 0)]);
    assert_eq!(runs(doc.para(1)), [("bold", "b".to_owned()), ("italic", "i".to_owned()), ("both", "bi".to_owned()), ("bold", "b".to_owned()), ("italic", "i".to_owned()), ("struck", "s".to_owned()), ("under", "u".to_owned()), ("code", "c".to_owned())]);
    // An item further in than the one over it is a level down, by two
    // spaces, four or a tab; one back out is level with the item it
    // lines up with.
    let items: Vec<(&str, List, u8)> = shape(&doc)[9..23].iter().map(|(text, _, list, level)| (*text, *list, *level)).collect();
    assert_eq!(items, [("one", b, 0), ("two", b, 1), ("three", b, 2), ("two again", b, 1), ("back", b, 0), ("star", b, 0), ("plus", b, 0), ("first", n, 0), ("second", n, 0), ("child", n, 1), ("tabbed", b, 1), ("todo", List::Check(false), 0), ("done", List::Check(true), 0), ("DONE", List::Check(true), 0)]);
    // Code is a paragraph a line, as it was written, and wholly mono.
    assert_eq!(shape(&doc)[23..], [("fn main() {", Style::Body, t, 0), ("", Style::Body, t, 0), ("    let x = \"**1**\";", Style::Body, t, 0), ("}", Style::Body, t, 0), ("after", Style::Body, t, 0)]);
    assert_eq!((runs(doc.para(25)), doc.para(24).spans.list().len()), (vec![("    let x = \"**1**\";", "c".to_owned())], 0));
    // Written, it is Markdown again: the same paragraphs, one a line.
    let written = write(&doc);
    assert!(written.starts_with("# Field Notes\n\nSoft-wrapped prose that goes on over two lines, with **bold**, *italic*, ***both***, **bold** and *italic* again, ~~struck~~, <u>under</u> and `code`. A hard break ends here\n\nand here\n\nthen the last line.\n\n## Chapter\n\n### Section\n\n#### Sub\n\n#### Deeper\n\n> a quote over two lines and a lazy third\n\n- one\n    - two\n        - three\n    - two again\n- back\n"), "{written}");
    assert!(written.ends_with("- plus\n1. first\n2. second\n    1. child\n    - tabbed\n- [ ] todo\n- [x] done\n- [x] DONE\n\n```\nfn main() {\n\n    let x = \"**1**\";\n}\n```\n\nafter\n"), "{written}");
    assert_eq!(read(&written, None).paras, doc.paras);
    // Nothing at all is one empty paragraph, and writes as nothing.
    for empty in ["", "\n", "\n\n   \n\t\n"] {
        assert_eq!((read(empty, None).paras, write(&read(empty, None))), (Doc::default().paras, String::new()));
    }
}

#[test]
fn what_is_written_reads_back_as_the_document_it_was() {
    let png = encode_png(&Image::solid(2, 2, [1, 2, 3, 255]));
    let file = format!("# Title\n\n## One\n\n### Two\n\n#### Three\n\nPlain with **bold**, *italic*, ***both***, ~~struck~~, <u>under</u> and `code`.\n\n> A quote, *one* paragraph.\n\n- bullet\n    - nested\n        1. numbered\n        2. numbered again\n    - [x] ticked\n- [ ] to tick\n1. first\n2. second\n> - a quoted item\n- ## a heading in a list\n\n```\nlet x = `tick`;\n\n    indented\n```\n\n![](data:image/png;base64,{})\n\nlast\n", base64_encode(&png));
    let doc = read(&file, None);
    assert_eq!(write(&doc), file);
    // The same document, made by hand.
    let mut made = Doc::from_text("Title\nOne\nTwo\nThree\nPlain with bold, italic, both, struck, under and code.\nA quote, one paragraph.\nbullet\nnested\nnumbered\nnumbered again\nticked\nto tick\nfirst\nsecond\na quoted item\na heading in a list\nlet x = `tick`;\n\n    indented\n\u{FFFC}\nlast");
    for (i, style) in [(0, Style::Title), (1, Style::Heading1), (2, Style::Heading2), (3, Style::Heading3), (5, Style::Quote), (14, Style::Quote), (15, Style::Heading1)] {
        made.set_paras(i, i, |a| a.style = style);
    }
    for (i, list, level) in [(6, List::Bullet, 0), (7, List::Bullet, 1), (8, List::Number, 2), (9, List::Number, 2), (10, List::Check(true), 1), (11, List::Check(false), 0), (12, List::Number, 0), (13, List::Number, 0), (14, List::Bullet, 0), (15, List::Bullet, 0)] {
        made.set_paras(i, i, |a| (a.list, a.level) = (list, level));
    }
    for (para, from, to, letters) in [(4, 11, 15, "b"), (4, 17, 23, "i"), (4, 25, 29, "bi"), (4, 31, 37, "s"), (4, 39, 44, "u"), (4, 49, 53, "c"), (5, 9, 12, "i"), (16, 0, 15, "c"), (18, 0, 12, "c")] {
        let has = |c: char| letters.contains(c);
        made.format(Pos::new(para, from), Pos::new(para, to), |a| *a = TextAttrs { bold: has('b'), italic: has('i'), strike: has('s'), underline: has('u'), font: has('c').then(|| Font::named(MONO)), ..TextAttrs::default() });
    }
    let id = made.pictures.add(png).unwrap();
    made.para_mut(19).picture = Some(Placed { id, width: 0.0 });
    assert_eq!((made.para(19).text.as_str(), made.number(9), made.number(13)), (OBJECT, 2, 2));
    assert_eq!(doc.paras, made.paras);
    assert_eq!((write(&made), back(&made).paras, back(&made).pictures), (file, made.paras.clone(), made.pictures.clone()));
    assert!(holds(&made));
}

#[test]
fn runs_that_nest_and_overlap_are_closed_and_opened_again() {
    // (the text, its runs, the Markdown for it)
    let exact: [(&str, Dress, &str); 12] = [
        ("one two three", &[(0, 7, "i"), (4, 7, "b")], "*one **two*** three\n"),
        ("one two three", &[(0, 13, "b"), (4, 7, "i")], "**one *two* three**\n"),
        ("one two three", &[(4, 7, "bi")], "one ***two*** three\n"),
        ("one two three", &[(0, 13, "u"), (4, 7, "b"), (4, 13, "s")], "<u>one ~~**two** three~~</u>\n"),
        ("call it done", &[(0, 12, "b"), (5, 7, "c")], "**call `it` done**\n"),
        ("x", &[(0, 1, "bisu")], "<u>~~***x***~~</u>\n"),
        // Inside a word a star still reads; an underscore would not.
        ("unbelievable", &[(2, 8, "i")], "un*believ*able\n"),
        // A bold run that starts in the middle of an italic one: italic
        // closes around bold's end and bold opens again after it.
        ("abcdefghi", &[(0, 6, "i"), (3, 9, "b")], "_abc**def**_**ghi**\n"),
        ("one-two-three", &[(0, 7, "i"), (4, 13, "b")], "_one-**two**_**-three**\n"),
        ("xab", &[(0, 2, "b"), (1, 3, "i")], "**x*a***_b_\n"),
        // An underline is a tag, and is kept on space too.
        ("a  b", &[(0, 4, "b"), (1, 2, "u")], "**a<u> </u> b**\n"),
        (" a ", &[(0, 3, "u")], "<u> a </u>\n"),
    ];
    for (text, dress, markdown) in exact {
        let doc = dressed(text, dress);
        assert_eq!((write(&doc).as_str(), back(&doc).paras), (markdown, doc.paras.clone()), "{text} {dress:?}");
    }
    // Stars cannot open before a space or close after one, so what is
    // bold, italic or struck is not kept on the space at a stretch's end.
    let word = dressed("hello world", &[(0, 6, "b")]);
    assert_eq!((write(&word).as_str(), runs(back(&word).para(0))), ("**hello** world\n", vec![("hello", "b".to_owned())]));
    let across = dressed("one two three", &[(0, 7, "i"), (4, 13, "b")]);
    assert_eq!((write(&across).as_str(), runs(back(&across).para(0))), ("*one **two*** **three**\n", vec![("one ", "i".to_owned()), ("two", "bi".to_owned()), ("three", "b".to_owned())]));
    let apart = dressed("a b", &[(0, 1, "s"), (2, 3, "s")]);
    assert_eq!((write(&apart).as_str(), back(&apart).paras), ("~~a~~ ~~b~~\n", apart.paras.clone()), "space between two struck words is not struck for it");
    // Nor do they open at punctuation in the middle of a word, or close
    // after it there: the stretch gives up that end, and if nothing of it
    // is left, is written plain. The text is whole either way.
    let odd = dressed("foo(bar) baz", &[(3, 8, "b"), (9, 12, "i")]);
    assert_eq!((write(&odd).as_str(), runs(back(&odd).para(0))), ("foo(**bar)** *baz*\n", vec![("bar)", "b".to_owned()), ("baz", "i".to_owned())]));
    let quoted = dressed("これは「強調」です", &[(9, 21, "b")]);
    assert_eq!((write(&quoted).as_str(), runs(back(&quoted).para(0))), ("これは「**強調**」です\n", vec![("強調", "b".to_owned())]));
    let none = dressed("a.b", &[(1, 2, "s")]);
    assert_eq!((write(&none).as_str(), back(&none).para(0).spans.list().len()), ("a.b\n", 0));
    // Markdown from elsewhere: both spellings, nested, and run together.
    for (markdown, text, dress) in [("__a__ _b_ **_c_** _**d**_ ***e***", "a b c d e", vec![("a", "b"), ("b", "i"), ("c", "bi"), ("d", "bi"), ("e", "bi")]), ("**a*b*c** *d**e**f*", "abc def", vec![("a", "b"), ("b", "bi"), ("c", "b"), ("d", "i"), ("e", "bi"), ("f", "i")]), ("~~a **b** <U>c</U>~~", "a b c", vec![("a ", "s"), ("b", "bs"), (" ", "s"), ("c", "su")])] {
        let doc = read(markdown, None);
        assert_eq!((doc.para(0).text.as_str(), runs(doc.para(0))), (text, dress.into_iter().map(|(text, letters)| (text, letters.to_owned())).collect()), "{markdown}");
    }
}

/// Text that looks like Markdown, a line each.
const TRICKY: &str = "# not a heading\n#hashtag\n## two\n#\n###\n> not a quote\n- not a bullet\n+ plus\n* star\n1. not a list\n12) nor this\n1986. A year\n-\n*\n_\n**\n- [ ] not a box\n[ ] nor this\n[x] nor this\na * b * c\n*stars* and **more** and ***most***\n_under_ and __dunder__ and snake_case_name\n~~tilde~~ and ~ and ~~~\n`ticks` and ``double`` and ```\nback\\slash \\* and C:\\Users\\me\\\n\\\n<u>tag</u> and </U> and <\n[link](http://x.y/_a_*b*) and ]( and ]\n![img](http://x.y/z.png)\n---\n- - -\n***\n___\n* * *\n| a | *b* |\n[a]: http://x.y\na  b\tc\nends with a hash #\nends with two ##\na_ and _a and *a and a*\né*ü*ö_ñ_ and 日本*語* and \u{a0}\n!@$%^&()=+{};:'\",.<>/?";

#[test]
fn text_full_of_markdown_survives_as_text() {
    let plain = Doc::from_text(TRICKY);
    let lines = plain.paras.len();
    assert_eq!(read(&write(&plain), None).paras, plain.paras);
    // And as a title, a heading in a list, a quote, a quote in a list, a
    // bullet, a numbered item and a ticked box, where the same text is
    // behind another mark.
    for dress in [|a: &mut ParaAttrs| a.style = Style::Title, |a: &mut ParaAttrs| (a.style, a.list) = (Style::Heading2, List::Number), |a: &mut ParaAttrs| a.style = Style::Quote, |a: &mut ParaAttrs| (a.style, a.list) = (Style::Quote, List::Bullet), |a: &mut ParaAttrs| a.list = List::Bullet, |a: &mut ParaAttrs| a.list = List::Number, |a: &mut ParaAttrs| a.list = List::Check(true)] {
        let mut doc = plain.clone();
        doc.set_paras(1, lines - 1, dress);
        assert_eq!(back(&doc).paras, doc.paras, "{}", write(&doc));
    }
    // Underlined, each line is in a tag and still the text it was.
    let mut under = plain.clone();
    for i in 0..lines {
        under.format(Pos::new(i, 0), Pos::new(i, under.para(i).text.len()), |a| a.underline = true);
    }
    assert_eq!(back(&under).paras, under.paras);
    // Only what would be read as markup gets a backslash.
    for (text, markdown) in [("2 * 3 * 4, snake_case, C:\\Users\\me, a ~ b, #1, 1.5, [x](y_z_), a - b", "2 * 3 * 4, snake_case, C:\\Users\\me, a ~ b, #1, 1.5, [x](y_z_), a - b"), ("*a* _b_ `c` ~~d~~ <u>e</u> \\* f\\", "\\*a\\* \\_b\\_ \\`c\\` \\~\\~d\\~\\~ \\<u>e\\</u> \\\\\\* f\\\\"), ("# h", "\\# h"), ("> q", "\\> q"), ("- b", "\\- b"), ("+ b", "\\+ b"), ("* b", "\\* b"), ("12. n", "12\\. n"), ("3) n", "3\\) n"), ("---", "---")] {
        assert_eq!(write(&Doc::from_text(text)), format!("{markdown}\n"));
    }
    let mut title = Doc::from_text("C# and F# #");
    title.set_paras(0, 0, |a| a.style = Style::Title);
    assert_eq!(write(&title), "# C# and F# \\#\n");
    // Escapes written by hand are read, and a backslash before anything
    // else is a backslash.
    assert_eq!(read("\\# \\*a\\* \\_ \\\\ \\a 1\\. \\[\\]", None).para(0).text, "# *a* _ \\ \\a 1. []");
}

#[test]
fn code_is_kept_as_it_is_written() {
    // Fences of either kind, further in, longer than what they hold, and
    // never closed.
    let doc = read("  ~~~ sh\n  ls *.md\n   # deeper\nout\n  ~~~~\ntext\n````\n```\n`a`\n````\n```\nto the end\n- not a list", None);
    assert_eq!(doc.paras.iter().map(|p| (p.text.as_str(), letters(&p.spans.at(0)))).collect::<Vec<_>>(), [("ls *.md", "c".to_owned()), (" # deeper", "c".to_owned()), ("out", "c".to_owned()), ("text", String::new()), ("```", "c".to_owned()), ("`a`", "c".to_owned()), ("to the end", "c".to_owned()), ("- not a list", "c".to_owned())]);
    // Two blocks with nothing between them are one run of code.
    assert_eq!(write(&doc), "```\nls *.md\n # deeper\nout\n```\n\ntext\n\n````\n```\n`a`\nto the end\n- not a list\n````\n");
    assert_eq!(back(&doc).paras, doc.paras);
    // Lines of code with empty paragraphs between them are one block;
    // the empty ones around it are not written, like any other.
    let mut block = Doc::from_text("\nfirst\n\n\nlast\n\nafter");
    for i in [1, 4] {
        block.format(Pos::new(i, 0), Pos::new(i, block.para(i).text.len()), |a| a.font = Some(Font::named(MONO)));
    }
    assert_eq!(write(&block), "```\nfirst\n\n\nlast\n```\n\nafter\n");
    assert_eq!(back(&block).paras, [&block.paras[1..5], &block.paras[6..]].concat());
    // Code in a line: as many backticks as it takes, and a space where
    // one would be taken for padding.
    let line = "Use `x`, `` `tick` ``, ``a`b``, ` `, `  two  `, **`bold`** and <u>`under` it</u>.\n";
    let doc = read(line, None);
    assert_eq!(runs(doc.para(0)), [("x", "c".to_owned()), ("`tick`", "c".to_owned()), ("a`b", "c".to_owned()), (" ", "c".to_owned()), (" two ", "c".to_owned()), ("bold", "bc".to_owned()), ("under", "uc".to_owned()), (" it", "u".to_owned())]);
    assert_eq!(write(&doc), line);
    // An item or a heading that is all code is code in a line; a plain
    // paragraph that is all code is a block.
    let item = read("- `ls -la`\n\n# `main`\n\n`all of it`\n", None);
    assert_eq!((write(&item).as_str(), back(&item).paras), ("- `ls -la`\n\n# `main`\n\n```\nall of it\n```\n", item.paras.clone()));
    // Code keeps the space at its ends, in a line too.
    let spaced = dressed("  x  and y", &[(0, 5, "c")]);
    assert_eq!((write(&spaced).as_str(), back(&spaced).paras), ("`   x   `and y\n", spaced.paras.clone()));
}

#[test]
fn pictures_come_from_the_file_itself_or_from_beside_it() {
    let png = encode_png(&Image::solid(2, 2, [1, 2, 3, 255]));
    let data = format!("data:image/png;base64,{}", base64_encode(&png));
    // A picture written into the file.
    let doc = read(&format!("before\n![a dot]({data} \"A dot\")\nafter\n\n- ![]({data})\n"), None);
    assert_eq!(shape(&doc), [("before", Style::Body, List::None, 0), (OBJECT, Style::Body, List::None, 0), ("after", Style::Body, List::None, 0), (OBJECT, Style::Body, List::Bullet, 0)]);
    let placed = doc.para(1).picture.expect("a picture");
    assert_eq!((doc.pictures.get(placed.id).map(|p| (p.image.width, p.image.height, p.extension())), placed.width, doc.para(3).picture), (Some((2, 2, "png")), 0.0, Some(placed)));
    assert_eq!(write(&doc), format!("before\n\n![]({data})\n\nafter\n\n- ![]({data})\n"));
    assert_eq!((back(&doc).paras, back(&doc).pictures), (doc.paras.clone(), doc.pictures.clone()));
    // Pictures in files beside the document, however they are named.
    let dir = std::env::temp_dir().join(format!("lntrn-notepad-markdown-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("art")).unwrap();
    for name in ["dot.png", "art/my dot.png"] {
        std::fs::write(dir.join(name), &png).unwrap();
    }
    std::fs::write(dir.join("notes.txt"), "not a picture").unwrap();
    let here = dir.display();
    let found = format!("![](dot.png)\n![named](<art/my dot.png> 'A title')\n![](art/my%20dot.png)\n![]({here}/dot.png)\n![](file://{here}/dot.png)\n");
    let doc = read(&found, Some(&dir));
    assert_eq!((doc.paras.len(), doc.paras.iter().all(|p| p.picture == Some(placed)), doc.pictures.iter().count()), (5, true, 1));
    // Whatever is not a picture that is here stays the text it was:
    // nothing is fetched, a file that is no picture is not one, a device
    // is not read, and an image among words is words.
    let other = "![](missing.png)\n![](notes.txt)\n![](art)\n![](/dev/zero)\n![](https://example.com/dot.png)\n![](data:image/png;base64,AAAA)\n![](data:text/plain,hello)\nsee ![](dot.png)\n![](dot.png) and ![](dot.png)";
    let doc = read(other, Some(&dir));
    assert_eq!((doc.paras.len(), doc.para(0).text.as_str(), doc.pictures.is_empty()), (1, other.replace('\n', " ").as_str(), true));
    // With no folder to look in, only a whole path is a file.
    let doc = read(&found, None);
    assert_eq!(doc.paras.iter().map(|p| p.is_picture()).collect::<Vec<_>>(), [false, true, true], "the first three are one paragraph of text");
    std::fs::remove_dir_all(&dir).unwrap();
    // A picture the document does not have is not written, and runs
    // that are not on its characters' edges do not stop the text.
    let mut broken = Doc::from_paras(vec![Para::of_picture(Placed { id: 7, width: 0.0 }), Para::plain("héllo")]);
    broken.format(Pos::new(1, 2), Pos::new(1, 4), |a| a.bold = true);
    assert_eq!(write(&broken), "héllo\n");
}

#[test]
fn what_markdown_has_that_a_document_has_not_stays_text() {
    // Links, a table, HTML, a rule, a footnote and definitions come back
    // as they were written, line for line.
    let file = "See [the *docs*](https://example.com/a_b/_c_?q=*x*&r=`y`) and ![far](https://example.com/cat.png).\n\n| Name | Qty |\n|:-----|----:|\n| `nut` | **2** |\n\n<div align=\"center\">a &amp; b <br> <b>c</b></div>\n\n---\n\nA footnote[^1] and a [reference][ref].\n\n[^1]: The note.\n\n[ref]: https://example.com/x_y_ \"Title\"\n";
    let doc = read(file, None);
    assert_eq!(write(&doc), file);
    assert_eq!(doc.paras.iter().map(|p| p.text.as_str()).collect::<Vec<_>>()[..5], ["See [the docs](https://example.com/a_b/_c_?q=*x*&r=`y`) and ![far](https://example.com/cat.png).", "| Name | Qty |", "|:-----|----:|", "| nut | 2 |", "<div align=\"center\">a &amp; b <br> <b>c</b></div>"]);
    assert_eq!((runs(doc.para(0)), runs(doc.para(3))), (vec![("docs", "i".to_owned())], vec![("nut", "c".to_owned()), ("2", "b".to_owned())]));
    // Rows, rules and definitions are lines of their own even with no
    // blank line around them; anything else is folded into its paragraph.
    let tight = read("text\n| a |\n| b |\nmore\n***\n[a]: x\n[b]: y\nlast\n= = =", None);
    assert_eq!(tight.paras.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["text", "| a |", "| b |", "more", "***", "[a]: x", "[b]: y", "last = = ="]);
    assert_eq!(write(&tight), "text\n\n| a |\n| b |\n\nmore\n\n***\n\n[a]: x\n\n[b]: y\n\nlast = = =\n");
    // A paragraph underlined is a heading, written back the newer way.
    // Under a list item, a quote or nothing, the same line is no heading;
    // nor is anything in the front matter a file may start with.
    let under = read("Big\nnews\n===\nSmaller\n---\n\n---\n- item\n---\n> said\n===\n", None);
    assert_eq!(under.paras.iter().map(|p| (p.text.as_str(), p.attrs.style)).collect::<Vec<_>>(), [("Big news", Style::Title), ("Smaller", Style::Heading1), ("---", Style::Body), ("item", Style::Body), ("---", Style::Body), ("said ===", Style::Quote)]);
    assert!(write(&under).starts_with("# Big news\n\n## Smaller\n\n---\n"));
    let fronted = read("---\ntitle: A note\ntags: x\n---\nwords\n---\n", None);
    assert_eq!(fronted.paras.iter().map(|p| (p.text.as_str(), p.attrs.style)).collect::<Vec<_>>(), [("---", Style::Body), ("title: A note tags: x", Style::Body), ("---", Style::Body), ("words", Style::Heading1)]);
    // A marker that closes nothing is text, and is written so that it
    // stays text.
    for (open, written) in [("**bold and *italic", "\\*\\*bold and \\*italic"), ("a ** b __ c ~~ d", "a ** b __ c \\~\\~ d"), ("`code and ``more", "\\`code and \\`\\`more"), ("<u>under and </i>", "\\<u>under and </i>"), ("</u> before <u>", "\\</u> before \\<u>"), ("~struck~ and ~~~not~~~", "~struck~ and \\~\\~\\~not\\~\\~\\~"), ("[link](to nowhere", "[link\\](to nowhere"), ("*a ~~b* c~~", "*a \\~\\~b* c\\~\\~"), ("5 * 4 * 3 and snake_case_", "5 * 4 * 3 and snake_case\\_")] {
        let doc = read(open, None);
        assert_eq!((doc.para(0).text.as_str(), write(&doc)), (open.replace("*a ~~b* c~~", "a ~~b c~~").as_str(), format!("{written}\n")), "{open}");
        assert_eq!(back(&doc).paras, doc.paras);
    }
    // A list in a quote nests as any other, and a line under an item is
    // more of the item.
    let quoted = read("> - a\n>   - b\n>     c\n> - d\n", None);
    assert_eq!((shape(&quoted), write(&quoted).as_str()), (vec![("a", Style::Quote, List::Bullet, 0), ("b c", Style::Quote, List::Bullet, 1), ("d", Style::Quote, List::Bullet, 0)], "> - a\n>     - b c\n> - d\n"));
    assert_eq!(back(&quoted).paras, quoted.paras);
    // Where a list or a quote ends, and what is no list at all.
    let doc = read("1. one\n2. two\nstill two\n\n   and its second paragraph\n3. three\n\nThe war ended in\n1945. A new line\n- but a bullet breaks in\n> and a quote\n# and a heading\n>\n> > deeper\n- > quoted item\n-\n", None);
    assert_eq!(shape(&doc), [("one", Style::Body, List::Number, 0), ("two still two", Style::Body, List::Number, 0), ("and its second paragraph", Style::Body, List::None, 0), ("three", Style::Body, List::Number, 0), ("The war ended in 1945. A new line", Style::Body, List::None, 0), ("but a bullet breaks in", Style::Body, List::Bullet, 0), ("and a quote", Style::Quote, List::None, 0), ("and a heading", Style::Title, List::None, 0), ("deeper", Style::Quote, List::None, 0), ("quoted item", Style::Quote, List::Bullet, 0), ("", Style::Body, List::Bullet, 0)]);
}

#[test]
fn markdown_holds_only_what_it_can_say() {
    let mut doc = read(NOTES, None);
    let id = doc.pictures.add(encode_png(&Image::solid(1, 1, [0, 0, 0, 255]))).unwrap();
    doc.paras.push(std::sync::Arc::new(Para::of_picture(Placed { id, width: 0.0 })));
    assert!(holds(&doc) && holds(&Doc::default()));
    let last = doc.paras.len() - 1;
    let changes: [fn(&mut Doc, usize); 11] = [
        |d, _| d.format(Pos::new(1, 0), Pos::new(1, 4), |a| a.size = Some(30.0)),
        |d, _| d.format(Pos::new(1, 0), Pos::new(1, 4), |a| a.font = Some(Font::named("Lora"))),
        |d, _| d.format(Pos::new(1, 0), Pos::new(1, 4), |a| a.color = Some(0xcc3a34)),
        |d, _| d.format(Pos::new(1, 0), Pos::new(1, 4), |a| a.highlight = Some(0xfff0c0)),
        |d, _| d.set_paras(0, 0, |a| a.align = Align::Center),
        |d, _| d.set_paras(1, 1, |a| a.align = Align::Justify),
        |d, _| d.set_paras(1, 1, |a| a.space_before = 4.0),
        |d, _| d.set_paras(1, 1, |a| a.space_after = 4.0),
        |d, _| d.set_paras(1, 1, |a| a.first_indent = 24.0),
        |d, last| d.para_mut(last).picture = d.para(last).picture.map(|placed| Placed { width: 320.0, ..placed }),
        |d, _| _ = d.insert_text(Pos::new(1, 0), "\t", TextAttrs::default()),
    ];
    for change in changes {
        let mut dressed = doc.clone();
        change(&mut dressed, last);
        assert!(!holds(&dressed) && dressed.paras != doc.paras);
        // It is written all the same, as what Markdown can say of it.
        assert_eq!(back(&dressed).paras, doc.paras);
    }
    // What Markdown says in its own way and no other. A list starts at
    // the edge and nests a level at a time.
    let mut list = Doc::from_text("a\nb\nc\nd\ne");
    for (i, level) in [2, 5, 3, 0, 2].into_iter().enumerate() {
        list.set_paras(i, i, |a| (a.list, a.level) = (List::Bullet, level));
    }
    assert_eq!((write(&list).as_str(), back(&list).paras.iter().map(|p| p.attrs.level).collect::<Vec<_>>()), ("- a\n    - b\n    - c\n- d\n    - e\n", vec![0, 1, 1, 0, 1]));
    // Space at a paragraph's ends is not kept, and a paragraph of
    // nothing else is an empty one.
    let spaced = Doc::from_text("  two in, two out  \n \t \n\tafter a tab");
    assert_eq!((write(&spaced).as_str(), back(&spaced).text()), ("two in, two out\n\nafter a tab\n", "two in, two out\nafter a tab".to_owned()));
    // Empty items leave their list in one piece, numbered as it was; an
    // empty paragraph parts two lists.
    let mut items = Doc::from_text("one\n\nthree\n\nfive");
    items.set_paras(0, 2, |a| a.list = List::Number);
    items.set_paras(4, 4, |a| a.list = List::Number);
    assert_eq!(write(&items), "1. one\n3. three\n\n1. five\n");
}

/// The next number under `n` of a row that is the same every time.
fn random(seed: &mut u64, n: usize) -> usize {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed % n as u64) as usize
}

/// Some text, of `most` pieces at the most, thick with what Markdown
/// reads as marks; with `lines`, with what makes lines and blocks of it
/// too.
fn jumble(seed: &mut u64, most: usize, lines: bool) -> String {
    const MARKS: [&str; 20] = ["<u>", "</u>", "**", "~~", "](", "- ", "1. ", "# ", "> ", "```", "[ ] ", "---", "\\", "![](x)", "\n", "\n\n", "\n    ", "\n|", "  \n", "~~~"];
    let letters: Vec<char> = "abc def ghi  *_~`\\<>u/[]()#-+.!|1é語\t".chars().collect();
    (0..=random(seed, most)).map(|_| if random(seed, 7) == 0 { MARKS[random(seed, if lines { 20 } else { 14 })].to_owned() } else { letters[random(seed, letters.len())].to_string() }).collect()
}

/// What each character of a paragraph has. Space says nothing of bold,
/// italic or struck.
fn each(p: &Para) -> Vec<String> {
    p.text.char_indices().map(|(i, c)| letters(&p.spans.at(i)).chars().filter(|l| !c.is_whitespace() || matches!(l, 'u' | 'c')).collect()).collect()
}

/// Whether a paragraph is what another was, as far as Markdown goes: the
/// same text and kind, underlined and code where that was, and nothing
/// bold, italic or struck that was not.
fn kept_as(now: &Para, was: &Para) -> bool {
    let fixed = |s: &str| s.chars().filter(|l| matches!(l, 'u' | 'c')).collect::<String>();
    (&now.text, now.attrs.style, now.attrs.list) == (&was.text, was.attrs.style, was.attrs.list) && each(now).iter().zip(each(was)).all(|(now, was)| fixed(now) == fixed(&was) && now.chars().all(|l| was.contains(l)))
}

#[test]
fn anything_at_all_is_read_and_written_without_harm() {
    let seed = &mut 0x2545_F491_4F6C_DD1Du64;
    for round in 0..2000 {
        // Any text, dressed anyhow, as any kind of paragraph, comes back,
        // and what it comes back as is what Markdown has of it from then
        // on.
        let text = jumble(seed, 30, false);
        let text = text.trim_matches([' ', '\t']);
        let mut doc = Doc::from_text(if text.is_empty() { "x" } else { text });
        let edges: Vec<usize> = doc.para(0).text.char_indices().map(|(i, _)| i).chain([doc.para(0).text.len()]).collect();
        for _ in 0..random(seed, 6) {
            let (a, b, dress) = (edges[random(seed, edges.len())], edges[random(seed, edges.len())], random(seed, 5));
            doc.format(Pos::new(0, a.min(b)), Pos::new(0, a.max(b)), |x| match dress {
                0 => x.bold = true,
                1 => x.italic = true,
                2 => x.strike = true,
                3 => x.underline = true,
                _ => x.font = Some(Font::named(MONO)),
            });
        }
        let kind = random(seed, 16);
        doc.set_paras(0, 0, |a| (a.style, a.list) = ([Style::Body, Style::Title, Style::Quote, Style::Heading2][kind % 4], [List::None, List::Bullet, List::Number, List::Check(true)][kind / 4]));
        let once = back(&doc);
        assert!(once.paras.len() == 1 && kept_as(once.para(0), doc.para(0)), "round {round}: {:?} as {}", doc.para(0), write(&doc));
        assert_eq!(back(&once).paras, once.paras, "round {round}: {} then {}", write(&doc), write(&once));
        // Any file at all is read, and written, and read as it was.
        let file = jumble(seed, 60, true);
        let first = read(&file, None);
        let (first, second): (Vec<_>, Vec<_>) = (first.paras.iter().filter(|p| !p.text.is_empty()).cloned().collect(), back(&first).paras.iter().filter(|p| !p.text.is_empty()).cloned().collect());
        assert!(first.len() == second.len() && second.iter().zip(&first).all(|(now, was)| kept_as(now, was)), "round {round}: {file:?}");
    }
}
