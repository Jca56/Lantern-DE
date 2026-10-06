//! Tests of Word documents written and read: one of ours with everything
//! in it there and back, and one put together by hand the way Word and
//! others write theirs.

use std::collections::HashMap;

use lntrn_image::{Image, encode_jpeg, encode_png, encode_qoi};

use super::*;
use crate::doc::{Font, Pos};
use crate::io::xml::Element;

/// The XML part of a package called `name`, read.
fn part(parts: &[(String, Vec<u8>)], name: &str) -> Element {
    xml::parse(std::str::from_utf8(&parts.iter().find(|(part, _)| part == name).unwrap_or_else(|| panic!("no {name}")).1).unwrap()).unwrap()
}

/// What `python3 -c script args` printed, or `None` on a machine without
/// python3.
fn python(script: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("python3").arg("-c").arg(script).args(args).output().ok()?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The names of the elements right inside the fullest `name` there is.
fn fullest<'a>(root: &'a Element, name: &str) -> Vec<&'a str> {
    let mut most = Vec::new();
    root.walk(&mut |e| {
        if e.name == name && e.elements().count() > most.len() {
            most = e.elements().map(|inner| inner.name.as_str()).collect();
        }
        true
    });
    most
}

/// Each paragraph's text and what it is, without its runs and room.
fn outline(doc: &Doc) -> Vec<(&str, Style, Align, List, u8)> {
    doc.paras.iter().map(|p| (p.text.as_str(), p.attrs.style, p.attrs.align, p.attrs.list, p.attrs.level)).collect()
}

#[test]
fn everything_a_document_has_survives_being_a_word_document() {
    let lines = ["Big Title", "Chapter One", "A Section", "A smaller one", "Said by someone", "body with styled words and\ta tab", "", "first bullet", "deeper bullet", "one", "two", "inside one", "inside two", "three", "inside again", "not an item", "starts again", "to do", "done", "goes on", "a bullet between", "and again", "\u{FFFC}", "\u{FFFC}", "  spaces kept  ", "\u{FFFC}", "\u{FFFC}", "the end <&> \"quoted\" 'single' \u{e9} \u{1F3EE}"];
    let mut doc = Doc::from_text(&lines.join("\n"));
    // Every style and alignment, with room and an indent of its own.
    doc.set_paras(0, 0, |a| (a.style, a.align, a.space_after) = (Style::Title, Align::Center, 8.0));
    doc.format(Pos::new(0, 0), Pos::new(0, 9), |a| (a.bold, a.size) = (true, Some(48.0)));
    doc.set_paras(1, 1, |a| (a.style, a.space_before) = (Style::Heading1, 12.0));
    doc.set_paras(2, 2, |a| (a.style, a.align) = (Style::Heading2, Align::Right));
    doc.set_paras(3, 3, |a| a.style = Style::Heading3);
    doc.set_paras(4, 4, |a| *a = ParaAttrs { style: Style::Quote, align: Align::Justify, list: List::Bullet, level: 1, space_before: 2.0, space_after: 4.0, first_indent: 24.0 });
    // Every look a run has, one of them across a tab.
    doc.format(Pos::new(5, 0), Pos::new(5, 4), |a| a.font = Some(Font::named("Comic Neue 2")));
    doc.format(Pos::new(5, 5), Pos::new(5, 9), |a| (a.bold, a.size) = (true, Some(32.0)));
    doc.format(Pos::new(5, 10), Pos::new(5, 16), |a| *a = TextAttrs { bold: true, italic: true, underline: true, strike: true, size: Some(20.0), font: Some(Font::named("Lora")), color: Some(0xcc3a34), highlight: Some(0xfff0c0) });
    doc.format(Pos::new(5, 17), Pos::new(5, 22), |a| (a.color, a.highlight) = (Some(0x000001), Some(0x00ff00)));
    doc.format(Pos::new(5, 23), Pos::new(5, 29), |a| a.underline = true);
    // Every kind of list, nested, and numbers that start again: after
    // an item less deep, after what is no item, after a bullet.
    doc.set_paras(7, 7, |a| a.list = List::Bullet);
    doc.set_paras(8, 8, |a| (a.list, a.level) = (List::Bullet, 2));
    for (i, level) in [(9, 0), (10, 0), (11, 1), (12, 1), (13, 0), (14, 1), (16, 0), (19, 0), (21, 0)] {
        doc.set_paras(i, i, |a| (a.list, a.level) = (List::Number, level));
    }
    doc.set_paras(17, 17, |a| (a.list, a.level) = (List::Check(false), 1));
    doc.set_paras(18, 18, |a| (a.list, a.level) = (List::Check(true), MAX_LEVEL));
    doc.set_paras(20, 20, |a| a.list = List::Bullet);
    let numbers: Vec<Option<u32>> = (0..doc.paras.len()).map(|i| (doc.para(i).attrs.list == List::Number).then(|| doc.number(i))).collect();
    assert_eq!(numbers.iter().flatten().copied().collect::<Vec<_>>(), [1, 2, 1, 2, 3, 1, 1, 2, 1]);
    // Pictures: one at a width, one as it is, one shown twice, and one
    // of another kind of file. One nothing shows stays behind.
    let wide = doc.pictures.add(encode_png(&Image::solid(3, 2, [1, 2, 3, 255]))).unwrap();
    let small = doc.pictures.add(encode_png(&Image::solid(4, 4, [9, 9, 9, 255]))).unwrap();
    let photo = doc.pictures.add(encode_jpeg(&Image::solid(16, 8, [200, 120, 40, 255]), 90)).unwrap();
    doc.pictures.add(encode_png(&Image::solid(1, 1, [7, 7, 7, 255]))).unwrap();
    for (i, id, width) in [(22, wide, 320.0), (23, small, 0.0), (25, photo, 0.0), (26, wide, 0.0)] {
        doc.para_mut(i).picture = Some(Placed { id, width });
    }
    doc.set_paras(22, 22, |a| a.align = Align::Center);

    let file = write(&doc, 24.0, Paper::LETTER);
    let back = read(&file).expect("a Word document");
    for (i, (got, put)) in back.paras.iter().zip(&doc.paras).enumerate() {
        assert_eq!(got, put, "paragraph {i}");
    }
    assert_eq!(back.paras.len(), doc.paras.len());
    assert_eq!(back.pictures.iter().count(), 3);
    assert_eq!([wide, small, photo].map(|id| back.pictures.get(id).map(|p| (p.image.width, p.image.height, p.extension()))), [Some((3, 2, "png")), Some((4, 4, "png")), Some((16, 8, "jpg"))]);
    assert_eq!(write(&back, 24.0, Paper::LETTER), file, "the same document makes the same file");

    // The parts Word looks for, each of them XML that reads.
    let parts = zip::read(&file).unwrap();
    assert_eq!(parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["[Content_Types].xml", "_rels/.rels", "word/document.xml", "word/_rels/document.xml.rels", "word/styles.xml", "word/numbering.xml", "word/settings.xml", "word/media/image1.png", "word/media/image2.png", "word/media/image3.jpg"]);
    let (document, styles, numbering, types, uses) = (part(&parts, "word/document.xml"), part(&parts, "word/styles.xml"), part(&parts, "word/numbering.xml"), part(&parts, "[Content_Types].xml"), part(&parts, "word/_rels/document.xml.rels"));
    assert_eq!((part(&parts, "_rels/.rels").find("Relationship").and_then(|r| r.attr("Target")), part(&parts, "word/settings.xml").find("w:compatSetting").and_then(|c| c.attr("w:val"))), (Some("word/document.xml"), Some("15")));
    // Word wants what a paragraph and a run have in its schema's order.
    assert_eq!(fullest(&document, "w:pPr"), ["w:pStyle", "w:numPr", "w:spacing", "w:ind", "w:jc"]);
    assert_eq!(fullest(&document, "w:rPr"), ["w:rFonts", "w:b", "w:bCs", "w:i", "w:iCs", "w:strike", "w:color", "w:sz", "w:szCs", "w:u", "w:shd"]);
    assert_eq!(fullest(&document, "wp:inline"), ["wp:extent", "wp:effectExtent", "wp:docPr", "wp:cNvGraphicFramePr", "a:graphic"]);
    assert_eq!(fullest(&styles, "w:style"), ["w:name", "w:basedOn", "w:next", "w:uiPriority", "w:qFormat", "w:pPr", "w:rPr"]);
    assert_eq!(fullest(&numbering, "w:lvl"), ["w:start", "w:numFmt", "w:lvlText", "w:lvlJc", "w:pPr", "w:rPr"]);
    // A pixel is half a point: body text of 24 is 12 points, which are
    // 24 half points, and each style is its multiple of that. A style's
    // room is its own, and a paragraph's is written on top of it.
    let size = |id: &str| styles.named("w:style").find(|s| s.attr("w:styleId") == Some(id)).and_then(|s| s.find("w:sz")).and_then(|s| s.attr("w:val")).map(str::to_owned);
    assert_eq!((styles.find("w:rPrDefault").and_then(|d| d.find("w:sz")).and_then(|s| s.attr("w:val")), size("Title"), size("Heading1"), size("Heading2"), size("Heading3"), size("Quote")), (Some("24"), Some("48".to_owned()), Some("38".to_owned()), Some("31".to_owned()), Some("27".to_owned()), None));
    let rooms: Vec<(Option<&str>, Option<&str>)> = document.child("w:body").unwrap().named("w:p").filter_map(|p| p.find("w:spacing")).map(|s| (s.attr("w:before"), s.attr("w:after"))).collect();
    assert_eq!(rooms, [(None, Some("248")), (Some("331"), None), (Some("80"), Some("100"))]);
    // A picture keeps its shape at the width it is given, each file is
    // packed once, and every one a drawing means is there.
    let extents: Vec<(Option<&str>, Option<&str>)> = document.child("w:body").unwrap().named("w:p").filter_map(|p| p.find("wp:extent")).map(|e| (e.attr("cx"), e.attr("cy"))).collect();
    assert_eq!(extents, [(Some("2032000"), Some("1354667")), (Some("25400"), Some("25400")), (Some("101600"), Some("50800")), (Some("19050"), Some("12700"))]);
    let targets: HashMap<&str, &str> = uses.named("Relationship").filter_map(|r| Some((r.attr("Id")?, r.attr("Target")?))).collect();
    let mut embedded = Vec::new();
    document.walk(&mut |e| {
        embedded.extend(e.attr("r:embed").map(|id| targets[id]));
        true
    });
    assert_eq!(embedded, ["media/image1.png", "media/image2.png", "media/image3.jpg", "media/image1.png"]);
    assert_eq!(types.named("Default").filter_map(|d| d.attr("Extension")).collect::<Vec<_>>(), ["rels", "xml", "png", "jpg"]);
    assert_eq!((types.named("Override").count(), uses.named("Relationship").count()), (4, 6));
    // Word counts each list through the whole document, a level at a
    // time: counted that way, the numbers are the ones shown here, and
    // every numbered list is told to start at 1.
    let mut counts: HashMap<(&str, &str), u32> = HashMap::new();
    let mut word_numbers = Vec::new();
    for p in document.child("w:body").unwrap().named("w:p") {
        let list = p.find("w:numId").and_then(|n| n.attr("w:val")).zip(p.find("w:ilvl").and_then(|l| l.attr("w:val")));
        let n = list.map(|list| {
            let n = counts.entry(list).or_default();
            *n += 1;
            *n
        });
        word_numbers.push(n.filter(|_| list.is_some_and(|(id, _)| id.parse::<usize>().is_ok_and(|id| id > TICKED))));
    }
    assert_eq!(word_numbers, numbers);
    let starts: Vec<(Option<&str>, Option<&str>)> = numbering.named("w:num").skip(TICKED).map(|n| (n.child("w:lvlOverride").and_then(|o| o.attr("w:ilvl")), n.find("w:startOverride").and_then(|s| s.attr("w:val")))).collect();
    assert_eq!(starts, ["0", "1", "1", "0", "0"].map(|level| (Some(level), Some("1"))));
    assert_eq!(numbering.named("w:abstractNum").map(|a| a.named("w:lvl").count()).collect::<Vec<_>>(), [LEVELS; 4]);

    // Python finds the zip whole and every part of it XML.
    let path = std::env::temp_dir().join(format!("lntrn-notepad-docx-{}.docx", std::process::id()));
    std::fs::write(&path, &file).unwrap();
    let script = "import sys, zipfile, xml.dom.minidom\nz = zipfile.ZipFile(sys.argv[1])\nparts = [n for n in z.namelist() if n.endswith(('.xml', '.rels'))]\nfor n in parts:\n    xml.dom.minidom.parseString(z.read(n))\nprint(z.testzip(), len(parts))";
    let seen = python(script, &[path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
    assert!(seen.is_none_or(|seen| seen == "None 7\n"));
}

#[test]
fn a_word_document_from_elsewhere_is_read_for_what_it_has() {
    let spaces = format!("xmlns:w=\"{MAIN}\" xmlns:r=\"{DOCUMENT}\" xmlns:wp=\"{DRAWING}/wordprocessingDrawing\" xmlns:a=\"{DRAWING}/main\" xmlns:pic=\"{DRAWING}/picture\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\"");
    let item = |style: &str, list: u8, level: u8, text: &str| format!("<w:p><w:pPr>{style}<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{list}\"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>\n");
    let styled = |style: &str, text: &str| format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>\n");
    let picture = |cx: u32, rel: &str| format!("<w:r><w:drawing><wp:inline distT=\"0\"><wp:extent cx=\"{cx}\" cy=\"12700\"/><wp:docPr id=\"1\" name=\"p\"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed=\"{rel}\"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>");
    let boxed = "<w:txbxContent><w:p><w:r><w:t>in the box</w:t></w:r></w:p></w:txbxContent>";
    let document = [
        format!("{HEAD}<w:document {spaces}>\r\n<w:body>\r\n"),
        // A style known by Word's name for it under another id, looks
        // that are turned off or are only black on white spelled out,
        // and a field with its code.
        "<w:p><w:pPr><w:pStyle w:val=\"berschrift2\"/></w:pPr><w:r><w:rPr><w:color w:val=\"000000\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"FFFFFF\"/></w:rPr><w:t xml:space=\"preserve\">Zweite </w:t></w:r><w:r><w:rPr><w:b w:val=\"0\"/><w:i w:val=\"false\"/><w:u w:val=\"none\"/><w:color w:val=\"auto\"/></w:rPr><w:t>Ebene</w:t></w:r></w:p>\n".to_owned(),
        "<w:p><w:pPr><w:pStyle w:val=\"Fancy\"/><w:spacing w:before=\"320\" w:after=\"180\"/></w:pPr><w:r><w:t>Fancy title</w:t></w:r></w:p>\n".to_owned(),
        "<w:p><w:pPr><w:pStyle w:val=\"Loop\"/></w:pPr><w:r><w:t xml:space=\"preserve\">page </w:t></w:r><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:fldSimple w:instr=\"PAGE\"><w:r><w:t>7</w:t></w:r></w:fldSimple><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>\n".to_owned(),
        // Runs of every sort: dressed, in a link, taken out and put in by
        // tracked changes, in a smart tag, and broken across a line.
        "<w:p>\n  <w:pPr><w:jc w:val=\"both\"/><w:ind w:firstLine=\"240\"/><w:rPr><w:b/></w:rPr></w:pPr>\n  <w:r><w:rPr><w:rFonts w:ascii=\"Lora\" w:hAnsi=\"Lora\"/><w:b/><w:i w:val=\"1\"/><w:sz w:val=\"32\"/></w:rPr><w:t xml:space=\"preserve\">bold </w:t></w:r>\n".to_owned(),
        "  <w:r><w:rPr><w:b w:val=\"0\"/><w:highlight w:val=\"yellow\"/><w:u w:val=\"double\"/></w:rPr><w:t>plain</w:t><w:tab/><w:t>tabbed</w:t></w:r>\n  <w:hyperlink r:id=\"rId9\"><w:r><w:rPr><w:rStyle w:val=\"Hyperlink\"/><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr><w:t>a link</w:t></w:r></w:hyperlink>\n".to_owned(),
        "  <w:del w:id=\"1\"><w:r><w:delText>gone</w:delText></w:r></w:del><w:ins w:id=\"2\"><w:r><w:t xml:space=\"preserve\"> added</w:t></w:r></w:ins>\n  <w:smartTag><w:r><w:rPr><w:strike/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"FFF0C0\"/></w:rPr><w:t xml:space=\"preserve\"> tagged</w:t></w:r></w:smartTag>\n  <w:r><w:br/><w:t>next line</w:t></w:r>\n</w:p>\n".to_owned(),
        // Lists: through numbering, through a style, a box, one with no
        // mark, none at all, and one that has a level its own way.
        item("<w:pStyle w:val=\"Listenabsatz\"/>", 1, 0, "bullet") + &item("", 1, 1, "deeper") + &item("", 2, 0, "first") + &item("", 2, 1, "lettered") + &item("", 2, 8, "deepest"),
        styled("Aufzhlung", "by its style") + &item("", 3, 0, "a box") + &item("", 4, 0, "unmarked") + &item("<w:pStyle w:val=\"Aufzhlung\"/>", 0, 0, "no list") + &item("", 5, 0, "its own way") + &item("", 77, 0, "of no list there is"),
        // A table with a table in it, and a content control.
        "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"4000\"/></w:tblGrid><w:tr><w:trPr/><w:tc><w:tcPr><w:tcW w:w=\"4000\" w:type=\"dxa\"/></w:tcPr><w:p><w:r><w:t>r1c1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>r1c2</w:t></w:r></w:p><w:p><w:r><w:t>r1c2 more</w:t></w:r></w:p></w:tc></w:tr>\n".to_owned(),
        "<w:tr><w:tc><w:tbl><w:tr><w:tc><w:p><w:r><w:t>nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/></w:tc><w:tc><w:p><w:r><w:t>r2c2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\n<w:sdt><w:sdtPr><w:alias w:val=\"x\"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>in a control</w:t></w:r></w:p></w:sdtContent></w:sdt>\n".to_owned(),
        // Pictures: in a line of text, as wide as it is, and one that is
        // no picture this reads. A text box, written twice as Word does.
        format!("<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:t>before</w:t></w:r>{}<w:r><w:t>after</w:t></w:r></w:p>\n<w:p>{}</w:p>\n<w:p>{}<w:r><w:t>beside what is no picture</w:t></w:r></w:p>\n", picture(1270000, "rId5"), picture(25400, "rId5"), picture(25400, "rId6")),
        format!("<w:p><w:r><w:t>has a box</w:t></w:r><w:r><mc:AlternateContent><mc:Choice Requires=\"wps\"><w:drawing><wp:anchor><wp:extent cx=\"100\" cy=\"100\"/><a:graphic><a:graphicData><wps:wsp><wps:txbx>{boxed}</wps:txbx></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape><v:textbox>{boxed}</v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>\n"),
        "<w:p><w:r><w:pict><v:shape style=\"width:30pt\"><v:imagedata r:id=\"rId5\"/></v:shape></w:pict></w:r></w:p>\n".to_owned(),
        // Breaks, and styles known only by their ids or by another name.
        "<w:p><w:r><w:br w:type=\"page\"/></w:r><w:r><w:t>new page</w:t></w:r><w:r><w:cr/><w:t>&amp; so on</w:t><w:noBreakHyphen/><w:t>on</w:t></w:r></w:p>\n".to_owned(),
        styled("Heading7", "deep heading") + &styled("Zitat", "quoted") + "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr>\n</w:body>\n</w:document>\n",
    ]
    .concat();
    let style = |id: &str, name: &str, based: &str, more: &str| format!("<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"{based}\"/>{more}</w:style>\n");
    let styles = [
        format!("{HEAD}<w:styles xmlns:w=\"{MAIN}\">\n<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"259\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\n"),
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Standard\"><w:name w:val=\"Normal\"/></w:style>\n<w:style w:type=\"character\" w:styleId=\"Titel\"><w:name w:val=\"Quote\"/></w:style>\n".to_owned(),
        style("berschrift2", "heading 2", "Standard", "<w:pPr><w:numPr><w:ilvl w:val=\"1\"/><w:numId w:val=\"2\"/></w:numPr></w:pPr><w:rPr><w:b/></w:rPr>") + &style("Titel", "Title", "Standard", "<w:pPr><w:jc w:val=\"center\"/></w:pPr>") + &style("Fancy", "Fancy", "Titel", "<w:pPr><w:spacing w:before=\"240\"/></w:pPr>"),
        style("Zitat", "Intense Quote", "Standard", "") + &style("Listenabsatz", "List Paragraph", "Standard", "") + &style("Aufzhlung", "List Bullet", "Standard", "<w:pPr><w:numPr><w:numId w:val=\"1\"/></w:numPr></w:pPr>") + &style("Loop", "Loop", "Loop", "") + "</w:styles>",
    ]
    .concat();
    let level = |level: u8, format: &str, mark: &str| format!("<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\"{mark}\"/></w:lvl>");
    let numbering = [
        format!("{HEAD}<w:numbering xmlns:w=\"{MAIN}\">\n<w:abstractNum w:abstractNumId=\"0\">{}{}</w:abstractNum>\n<w:abstractNum w:abstractNumId=\"1\">{}{}{}</w:abstractNum>\n", level(0, "bullet", "&#xF0B7;"), level(1, "bullet", "o"), level(0, "decimal", "%1."), level(1, "lowerLetter", "%2)"), level(8, "lowerRoman", "%9.")),
        format!("<w:abstractNum w:abstractNumId=\"2\">{}</w:abstractNum>\n<w:abstractNum w:abstractNumId=\"3\">{}</w:abstractNum>\n", level(0, "bullet", "\u{2610}"), level(0, "none", "")),
        "<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num><w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num><w:num w:numId=\"3\"><w:abstractNumId w:val=\"2\"/></w:num><w:num w:numId=\"4\"><w:abstractNumId w:val=\"3\"/></w:num>\n".to_owned(),
        format!("<w:num w:numId=\"5\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/>{}</w:lvlOverride></w:num>\n</w:numbering>", level(0, "bullet", "-")),
    ]
    .concat();
    let rel = |id: &str, kind: &str, target: &str| format!("<Relationship Id=\"{id}\" Type=\"{DOCUMENT}/{kind}\" Target=\"{target}\"/>");
    let uses = format!("{HEAD}<Relationships xmlns=\"{PACKAGE}/relationships\">{}{}{}{}<Relationship Id=\"rId9\" Type=\"{DOCUMENT}/hyperlink\" Target=\"https://example.com/\" TargetMode=\"External\"/></Relationships>", rel("rId1", "styles", "styles.xml"), rel("rId2", "numbering", "/word/lists/numbering.xml"), rel("rId5", "image", "media/image1.png"), rel("rId6", "image", "../word/media/chart.emf"));
    let top = format!("{HEAD}<Relationships xmlns=\"{PACKAGE}/relationships\">{}</Relationships>", rel("rId1", "officeDocument", "/word/document2.xml"));
    let png = encode_png(&Image::solid(4, 2, [1, 2, 3, 255]));
    // The document is the part the package says it is, whatever it is
    // called, and uses the parts it says it does, wherever they are.
    let file = zip::write(&[("_rels/.rels", top.as_bytes()), ("word/document2.xml", document.as_bytes()), ("word/_rels/document2.xml.rels", uses.as_bytes()), ("word/Styles.xml", styles.as_bytes()), ("word/lists/numbering.xml", numbering.as_bytes()), ("word/media/image1.png", &png), ("word/media/chart.emf", b"no picture at all")]);
    let doc = read(&file).expect("a Word document");
    let id = doc.pictures.iter().next().map(|(id, _)| id).expect("its one picture");
    let (b, l, c, j) = (Style::Body, Align::Left, Align::Center, Align::Justify);
    let expected = [
        ("Zweite Ebene", Style::Heading2, l, List::None, 0),
        ("Fancy title", Style::Title, c, List::None, 0),
        ("page 7", b, l, List::None, 0),
        ("bold plain\ttabbeda link added tagged", b, j, List::None, 0),
        ("next line", b, j, List::None, 0),
        ("bullet", b, l, List::Bullet, 0),
        ("deeper", b, l, List::Bullet, 1),
        ("first", b, l, List::Number, 0),
        ("lettered", b, l, List::Number, 1),
        ("deepest", b, l, List::Number, MAX_LEVEL),
        ("by its style", b, l, List::Bullet, 0),
        ("a box", b, l, List::Check(false), 0),
        ("unmarked", b, l, List::None, 0),
        ("no list", b, l, List::None, 0),
        ("its own way", b, l, List::Bullet, 0),
        ("of no list there is", b, l, List::None, 0),
        ("r1c1", b, l, List::None, 0),
        ("r1c2", b, l, List::None, 0),
        ("r1c2 more", b, l, List::None, 0),
        ("nested", b, l, List::None, 0),
        ("", b, l, List::None, 0),
        ("r2c2", b, l, List::None, 0),
        ("in a control", b, l, List::None, 0),
        ("before", b, c, List::None, 0),
        ("\u{FFFC}", b, c, List::None, 0),
        ("after", b, c, List::None, 0),
        ("\u{FFFC}", b, l, List::None, 0),
        ("beside what is no picture", b, l, List::None, 0),
        ("has a box", b, l, List::None, 0),
        ("in the box", b, l, List::None, 0),
        ("\u{FFFC}", b, l, List::None, 0),
        ("new page", b, l, List::None, 0),
        ("& so on\u{2011}on", b, l, List::None, 0),
        ("deep heading", Style::Heading3, l, List::None, 0),
        ("quoted", Style::Quote, l, List::None, 0),
    ];
    assert_eq!(outline(&doc), expected);
    // Room is what a paragraph has over its style's, and its style's is
    // found through the styles it is based on and the document's own.
    assert_eq!((doc.para(1).attrs.space_before, doc.para(1).attrs.space_after, doc.para(3).attrs.first_indent, doc.para(4).attrs.first_indent, doc.para(0).attrs.space_after), (8.0, 2.0, 24.0, 24.0, 0.0));
    // Only the looks a run has itself, the ones turned off left off,
    // and black on white no colours of its own.
    assert!(doc.para(0).spans.list().is_empty() && doc.para(2).spans.list().is_empty() && doc.para(4).spans.list().is_empty());
    let run = |start: usize, end: usize, attrs: TextAttrs| (start, end, attrs);
    let plain = TextAttrs::default();
    let runs = [
        run(0, 5, TextAttrs { bold: true, italic: true, size: Some(32.0), font: Some(Font::named("Lora")), ..plain }),
        run(5, 17, TextAttrs { underline: true, highlight: Some(0xFFFF00), ..plain }),
        run(17, 23, TextAttrs { underline: true, color: Some(0x0563C1), ..plain }),
        run(29, 36, TextAttrs { strike: true, highlight: Some(0xFFF0C0), ..plain }),
    ];
    assert_eq!(doc.para(3).spans.list().iter().map(|s| (s.start, s.end, s.attrs)).collect::<Vec<_>>(), runs);
    // A picture as wide as it is told, as wide as it is, and drawn the
    // old way; all of them the one file.
    assert_eq!([24, 26, 30].map(|i| doc.para(i).picture), [Some(Placed { id, width: 200.0 }), Some(Placed { id, width: 0.0 }), Some(Placed { id, width: 0.0 })]);
    assert_eq!((doc.pictures.iter().count(), doc.paras.iter().filter(|p| p.is_picture()).count()), (1, 3));
    // Written as ours and read again, it is the same document.
    assert_eq!(read(&write(&doc, 24.0, Paper::LETTER)).unwrap().paras, doc.paras);
}

#[test]
fn what_is_no_word_document_says_so_and_what_is_missing_is_done_without() {
    let body = |inside: &str| format!("{HEAD}<w:document xmlns:w=\"{MAIN}\"><w:body>{inside}</w:body></w:document>");
    // Not a zip, the kind of file Word wrote before it wrote zips, a
    // zip that is no document, a document cut short, and one that is
    // not XML: each says which, in words for a person.
    assert_eq!(read(b"just some words"), Err("This doesn't look like a Word document (not a zip archive).".to_owned()));
    assert_eq!(read(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 and so on"), Err("This is a Word document of the old kind (.doc) or one locked with a password, and neither can be read.".to_owned()));
    assert_eq!(read(&zip::write(&[("mimetype", b"application/epub+zip")])), Err("This doesn't look like a Word document (there is no word/document.xml in it).".to_owned()));
    let whole = zip::write(&[("word/document.xml", body("").as_bytes())]);
    assert_eq!(read(&whole[..whole.len() - 30]), Err("This Word document is damaged (the zip archive is cut short or damaged).".to_owned()));
    assert_eq!(read(&zip::write(&[("word/document.xml", b"<w:document><w:body><w:p></w:body></w:document>")])), Err("This Word document is damaged (<w:p> is closed by </w:body>).".to_owned()));
    // Nothing in it is one empty paragraph. Styles and lists that are
    // missing or broken are done without: a style is known by its id,
    // and an item of no list is no item.
    assert!(read(&zip::write(&[("word/document.xml", body("").as_bytes())])).unwrap().is_blank());
    let text = body("<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/></w:numPr><w:jc w:val=\"distribute\"/><w:spacing w:after=\"12pt\"/></w:pPr><w:r><w:rPr><w:sz w:val=\"12pt\"/></w:rPr><w:t>still read</w:t></w:r></w:p>");
    for side in [None, Some(b"<w:styles><w:style>".as_slice())] {
        let mut parts: Vec<(&str, &[u8])> = vec![("word/document.xml", text.as_bytes())];
        parts.extend(side.iter().flat_map(|bytes| [("word/styles.xml", *bytes), ("word/numbering.xml", *bytes), ("word/_rels/document.xml.rels", *bytes)]));
        let doc = read(&zip::write(&parts)).unwrap();
        assert_eq!(outline(&doc), [("still read", Style::Heading2, Align::Justify, List::None, 0)]);
        assert_eq!((doc.para(0).attrs.space_after, doc.para(0).spans.at(0).size), (24.0, Some(24.0)), "lengths with their units, as a strict document has them");
    }
    // The other encoding a part may be in.
    let wide: Vec<u8> = format!("\u{FEFF}{}", body("<w:p><w:r><w:t>caf\u{e9}</w:t></w:r></w:p>")).encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(read(&zip::write(&[("word/document.xml", &wide)])).unwrap().text(), "caf\u{e9}");
    // What a Word document cannot hold is left out of it, and it still
    // opens: characters XML has no place for, a picture the document
    // does not have, body text of no size.
    let mut doc = Doc::from_text("a\u{1}b\u{FFFC}c\n\u{FFFC}\nend");
    doc.para_mut(1).picture = Some(Placed { id: 42, width: 10.0 });
    for size in [0.0, -4.0, f32::NAN, f32::INFINITY] {
        let file = write(&doc, size, Paper::LETTER);
        assert_eq!(read(&file).unwrap().text(), "abc\n\nend");
        assert_eq!(part(&zip::read(&file).unwrap(), "word/styles.xml").find("w:sz").and_then(|s| s.attr("w:val")), Some("24"));
    }
    // A picture of a kind Word does not read is written as a PNG, and
    // one wider than the page's text is as wide as that.
    let mut doc = Doc::default();
    let id = doc.pictures.add(encode_qoi(&Image::solid(1000, 10, [5, 6, 7, 255]))).unwrap();
    *doc.para_mut(0) = Para::of_picture(Placed { id, width: 0.0 });
    let file = write(&doc, 24.0, Paper::LETTER);
    let parts = zip::read(&file).unwrap();
    assert!(parts.iter().any(|(name, bytes)| name == "word/media/image1.png" && bytes.starts_with(b"\x89PNG")));
    assert_eq!(part(&parts, "word/document.xml").find("wp:extent").map(|e| (e.attr("cx"), e.attr("cy"))), Some((Some("5943600"), Some("59436"))));
    let back = read(&file).unwrap();
    let shown = back.para(0).picture.expect("the picture");
    assert_eq!((shown.width, back.pictures.get(shown.id).map(|p| (p.image.width, p.image.height, p.image.pixel(0, 0), p.extension()))), (936.0, Some((1000, 10, [5, 6, 7, 255], "png"))));
}
