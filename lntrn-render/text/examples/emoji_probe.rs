//! Diagnostic: shape + rasterize a few glyphs through the exact same
//! cosmic-text/swash stack the DE uses, printing which font each glyph
//! resolves to and whether swash produces any pixels for it.
//! Run: cargo run -p lntrn-text --example emoji_probe

use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, SwashCache};

fn main() {
    let mut fs = lntrn_text::lantern_font_system();
    let mut cache = SwashCache::new();

    for s in [
        "A", "❤", "🔥", "✨", "🎉", "🐧",
        // Smileys across Unicode versions
        "😀", "😂", "😊", "🙂", "😍", "😎", "🤔", "🥰", "🥺", "🤯",
        "🥲", "🫠", "🫡", "🫨", "🙂‍↕", "🫩",
    ] {
        let mut buf = Buffer::new(&mut fs, Metrics::new(32.0, 38.0));
        buf.set_size(&mut fs, Some(400.0), Some(40.0));
        buf.set_text(
            &mut fs,
            s,
            &Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
            None,
        );
        buf.shape_until_scroll(&mut fs, false);

        let mut any = false;
        let runs: Vec<_> = buf
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().cloned().collect::<Vec<_>>())
            .collect();
        for g in runs {
            any = true;
            let face_name = fs
                .db()
                .face(g.font_id)
                .map(|f| {
                    f.families
                        .first()
                        .map(|(n, _)| n.clone())
                        .unwrap_or_else(|| "?".into())
                })
                .unwrap_or_else(|| "<no face>".into());
            let pg = g.physical((0.0, 0.0), 1.0);
            match cache.get_image(&mut fs, pg.cache_key) {
                Some(img) => println!(
                    "{s}  glyph_id={} font=\"{face_name}\"  content={:?}  {}x{}",
                    g.glyph_id, img.content, img.placement.width, img.placement.height
                ),
                None => println!(
                    "{s}  glyph_id={} font=\"{face_name}\"  NO IMAGE (rasterize failed)",
                    g.glyph_id
                ),
            }
        }
        if !any {
            println!("{s}  -> no glyphs at all (shaping produced nothing)");
        }
    }
}
