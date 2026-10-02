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

/// The paths a `lntrn-file-manager --pick... --pick-print0` wrote: each one
/// its exact bytes followed by a NUL. A path is bytes, not text: it may
/// not be valid UTF-8, and it may hold a line break or blanks at its ends,
/// which reading lines and trimming them would mangle. (Output without a
/// NUL comes from a file manager older than the flag: one path per line.)
fn picked_paths(stdout: &[u8]) -> Vec<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let separator = if stdout.contains(&0) { 0 } else { b'\n' };
    stdout
        .split(|byte| *byte == separator)
        .filter(|part| !part.is_empty())
        .map(|part| std::path::PathBuf::from(std::ffi::OsString::from_vec(part.to_vec())))
        .collect()
}

/// Run the lntrn-file-manager picker with `args` and wait for it. `None`:
/// cancelled, or it could not be started.
fn pick(args: &[&str]) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("lntrn-file-manager")
        .arg("--pick-print0")
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    picked_paths(&out.stdout).into_iter().next()
}

/// Open a file via the lntrn-file-manager picker. Loads it into a new tab.
pub fn open_file_dialog(handler: &mut TextHandler) {
    if let Some(path) = pick(&["--pick", "--title", "Open File"]) {
        let mut e = Editor::new();
        let _ = e.load_file(path);
        handler.tabs.push(e);
        handler.active_tab = handler.tabs.len() - 1;
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
    let picked = pick(&[
        "--pick-save",
        "--title",
        "Save As",
        "--save-name",
        &suggested,
    ]);
    if let Some(path) = picked {
        handler.editor_mut().file_path = Some(path);
        let _ = handler.editor_mut().save_file();
    }
}

/// Export the active editor's content as a `.docx` file via the picker.
pub fn export_docx_dialog(handler: &mut TextHandler) {
    let default_name = format!("{}.docx", filename_stem(handler));
    let picked = pick(&[
        "--pick-save",
        "--title",
        "Export as .docx",
        "--save-name",
        &default_name,
    ]);
    if let Some(path) = picked {
        if let Err(e) = export_docx(handler.editor(), &path) {
            eprintln!("[lntrn-notepad] docx export error: {e}");
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
