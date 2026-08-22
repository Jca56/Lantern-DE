//! Top-level actions invoked by the menu, keyboard, and mouse handlers —
//! file dialogs, clipboard, and a few other glue helpers. Lives outside
//! `main.rs` purely to keep the file under the size limit.

use crate::editor::Editor;
use crate::TextHandler;

/// Export an editor's content to a `.docx` file. Lives here (not on
/// `Editor`) to keep the editor module focused on text editing.
pub fn export_docx(
    editor: &Editor,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::format::Alignment;
    use docx_rs::{
        AbstractNumbering, AlignmentType, Docx, IndentLevel, Level, LevelJc, LevelText,
        LineSpacing, LineSpacingType, NumberFormat, Numbering, NumberingId, Paragraph, Run,
        RunFonts, SpecialIndentType, Start,
    };
    use std::fs::File;

    let file = File::create(path)?;
    // Bullet-list numbering definition (id 1) for `para.bullet` paragraphs.
    let mut doc = Docx::new()
        .add_abstract_numbering(AbstractNumbering::new(1).add_level(Level::new(
            0,
            Start::new(1),
            NumberFormat::new("bullet"),
            LevelText::new("•"),
            LevelJc::new("left"),
        )))
        .add_numbering(Numbering::new(1, 1));
    for (i, line) in editor.lines.iter().enumerate() {
        let mut para = Paragraph::new();
        let pa = editor.formats.get(i).para;
        if pa.bullet {
            para = para.numbering(NumberingId::new(1), IndentLevel::new(0));
        }

        // Paragraph alignment
        para = para.align(match pa.alignment {
            Alignment::Left => AlignmentType::Left,
            Alignment::Center => AlignmentType::Center,
            Alignment::Right => AlignmentType::Right,
            Alignment::Justify => AlignmentType::Justified,
        });

        // Line spacing: DOCX uses 240 twips = single spacing (1.0×)
        let line_val = (pa.line_spacing * 240.0) as i32;
        let mut spacing = LineSpacing::new()
            .line(line_val)
            .line_rule(LineSpacingType::Auto);
        if pa.space_before > 0.0 {
            // Convert logical px to twips (~15 twips/px)
            spacing = spacing.before((pa.space_before * 15.0) as u32);
        }
        if pa.space_after > 0.0 {
            spacing = spacing.after((pa.space_after * 15.0) as u32);
        }
        para = para.line_spacing(spacing);

        // First-line indent (logical px to twips)
        if pa.first_indent > 0.0 {
            let twips = (pa.first_indent * 15.0) as i32;
            para = para.indent(None, Some(SpecialIndentType::FirstLine(twips)), None, None);
        }

        // Character-level runs
        let spans = editor.formats.get(i).iter_spans(line.len());
        if spans.is_empty() {
            para = para.add_run(Run::new().add_text(""));
        }
        for span in &spans {
            let text = &line[span.start..span.end];
            let mut run = Run::new().add_text(text);
            if span.attrs.bold {
                run = run.bold();
            }
            if span.attrs.italic {
                run = run.italic();
            }
            if span.attrs.underline {
                run = run.underline("single");
            }
            if span.attrs.strikethrough {
                run = run.strike();
            }
            if let Some(fs) = span.attrs.font_size {
                // docx uses half-points (24pt = size 48)
                run = run.size((fs * 2.0) as usize);
            }
            if let Some(family) = span
                .attrs
                .font
                .and_then(|f| crate::fonts::family_for_index_static(f as usize))
            {
                run = run.fonts(RunFonts::new().ascii(family));
            }
            if let Some(rgb) = span.attrs.color {
                run = run.color(format!("{rgb:06X}"));
            }
            para = para.add_run(run);
        }
        doc = doc.add_paragraph(para);
    }
    doc.build().pack(file)?;
    Ok(())
}

/// Open a file via the lntrn-file-manager picker. Loads it into a new tab.
pub fn open_file_dialog(handler: &mut TextHandler) {
    let output = std::process::Command::new("lntrn-file-manager")
        .args(["--pick", "--title", "Open File"])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                let mut e = Editor::new();
                let _ = e.load_file(std::path::PathBuf::from(path));
                handler.tabs.push(e);
                handler.active_tab = handler.tabs.len() - 1;
            }
        }
    }
}

/// Save the active editor. If no file path is set, fall through to Save As.
pub fn save_file_dialog(handler: &mut TextHandler) {
    if handler.editor_mut().file_path.is_some() {
        let _ = handler.editor_mut().save_file();
        return;
    }
    save_as_dialog(handler);
}

/// The active editor's filename without its extension ("Untitled" fallback).
fn filename_stem(handler: &TextHandler) -> String {
    std::path::Path::new(&handler.editor().filename)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string())
}

/// Prompt for a path and save there. Suggests the rich `.lnote` extension —
/// this is also how an existing plain-text file upgrades to full formatting.
/// Typing any other extension still saves plain text.
pub fn save_as_dialog(handler: &mut TextHandler) {
    let suggested = format!("{}.lnote", filename_stem(handler));
    let output = std::process::Command::new("lntrn-file-manager")
        .args([
            "--pick-save",
            "--title",
            "Save As",
            "--save-name",
            &suggested,
        ])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                handler.editor_mut().file_path = Some(std::path::PathBuf::from(path));
                let _ = handler.editor_mut().save_file();
            }
        }
    }
}

/// Export the active editor's content as a `.docx` file via the picker.
pub fn export_docx_dialog(handler: &mut TextHandler) {
    let default_name = format!("{}.docx", filename_stem(handler));
    let output = std::process::Command::new("lntrn-file-manager")
        .args([
            "--pick-save",
            "--title",
            "Export as .docx",
            "--save-name",
            &default_name,
        ])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                if let Err(e) = export_docx(handler.editor(), std::path::Path::new(&path)) {
                    eprintln!("[lntrn-notepad] docx export error: {e}");
                }
            }
        }
    }
}

pub fn do_copy(handler: &mut TextHandler) {
    if let Some(text) = handler.editor().selected_text() {
        if let Some(cb) = &handler.clipboard {
            cb.set_text(&text);
        }
    }
}

pub fn do_cut(handler: &mut TextHandler) {
    if let Some(text) = handler.editor().selected_text() {
        if let Some(cb) = &handler.clipboard {
            cb.set_text(&text);
        }
        handler.editor_mut().delete_selection();
    }
}

pub fn do_paste(handler: &mut TextHandler) {
    if let Some(cb) = &handler.clipboard {
        if let Some(text) = cb.get_text() {
            handler.editor_mut().insert_str(&text);
        }
    }
}
