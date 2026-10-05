//! Pictures headless: what opens as what, a picture in a file tab from
//! opening to closing, the tabs of last time, the read again when the
//! file changes, an animation's clock, and the view's zoom and drag.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lntrn_app::lntrn_render::{ImageHandle, ImageId};
use lntrn_image::{Animation, Frame, Image};
use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Host, Key, Modifiers, Shell};

use super::view::draw_picture;
use super::*;
use crate::app::{App, Editor, Goto};
use crate::launch::LAUNCHED;
use crate::session::Session;
use crate::settings::Settings;
use crate::watch::IN_DELETE;

/// An empty folder of this test's own, by its real path.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lntrn-code-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

/// A PNG of `w` × `h` with one see-through pixel.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut image = Image::solid(w, h, [10, 20, 30, 255]);
    image.rgba[3] = 0;
    lntrn_image::encode_png(&image)
}

/// Wait for a read to come in from the pool.
fn landed(pictures: &mut Pictures) {
    for _ in 0..3000 {
        if pictures.land() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the read never came in");
}

/// The app on the project `dir`, its first frame drawn.
fn app_on(dir: &Path) -> (App, Shell<App>, Harness) {
    let mut app = App::new(Settings::default(), Session { root: Some(dir.to_path_buf()), ..Session::default() }, Vec::new());
    let mut shell = Shell::new(Editor::Code);
    let mut h = Harness::new(1600.0, 1000.0);
    h.shell_frame(&mut shell, &mut app);
    app.apply_pending(&mut shell);
    (app, shell, h)
}

#[test]
fn a_name_says_what_a_file_opens_as() {
    for name in ["a.png", "b.JPG", "c.jpeg", "d.bmp", "e.qoi", "f.ico", "g.gif", "h.webp", "i.svg"] {
        assert_eq!(opens(Path::new(name)), Opens::Picture, "{name}");
    }
    assert_eq!(opens(Path::new("/x/art.lstudio")), Opens::Studio);
    for name in ["scan.tiff", "photo.HEIC", "layers.psd"] {
        assert_eq!(opens(Path::new(name)), Opens::External, "{name}");
    }
    for name in ["main.rs", "png", "notes.png.md", "Makefile"] {
        assert_eq!(opens(Path::new(name)), Opens::Text, "{name}");
    }
}

#[test]
fn a_picture_opens_in_a_file_tab_and_closes() {
    let dir = temp("picture-tab");
    let file = dir.join("logo.png");
    std::fs::write(&file, png(4, 2)).unwrap();
    let (mut app, mut shell, mut h) = app_on(&dir);
    app.pending_paths.push(file.clone());
    app.apply_pending(&mut shell);
    assert!(app.docs.is_empty(), "not read as text");
    let id = app.focus_picture().expect("the focused tab holds a picture").id;
    assert_eq!((app.tab_path(id), app.tab_title(id)), (Some(file.as_path()), Some("logo.png")));
    assert_eq!(app.status(), "Loading…");
    landed(&mut app.pictures);
    let p = app.focus_picture().unwrap();
    assert_eq!((&p.state, p.width, p.height, p.format, p.translucent, p.animated()), (&State::Ready, 4, 2, "PNG", true, false));
    assert!(app.status().starts_with("4×2 · PNG · "), "{}", app.status());
    // Opened again, it is the same tab.
    app.pending_paths.push(file.clone());
    app.apply_pending(&mut shell);
    assert_eq!(app.pictures.iter().count(), 1);
    // It draws with no texture yet, and Ctrl+W closes it.
    h.shell_frame(&mut shell, &mut app);
    h.key_with(Key::Char('w'), Modifiers::CTRL);
    h.shell_frame(&mut shell, &mut app);
    app.apply_pending(&mut shell);
    assert_eq!(app.pictures.iter().count(), 0);
    assert!(app.focus_doc.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_studio_document_and_a_picture_we_cannot_read_go_to_their_apps() {
    let dir = temp("picture-out");
    let (doc, scan) = (dir.join("art.lstudio"), dir.join("scan.tiff"));
    std::fs::write(&doc, b"LSTUDIO\0").unwrap();
    std::fs::write(&scan, b"II*\0").unwrap();
    let (mut app, mut shell, _) = app_on(&dir);
    app.pending_paths.extend([doc.clone(), scan.clone()]);
    app.apply_pending(&mut shell);
    assert!(app.docs.is_empty() && app.pictures.iter().count() == 0, "neither opens here");
    let launched = LAUNCHED.with(|l| l.borrow().clone());
    assert_eq!(launched, vec![format!("lantern-studio {}", doc.display()), format!("xdg-open {}", scan.display())]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_svg_is_a_picture_until_its_text_is_asked_for() {
    let dir = temp("picture-svg");
    let (icon, other) = (dir.join("icon.svg"), dir.join("other.svg"));
    let source = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\">\n<rect width=\"8\" height=\"4\" fill=\"#f00\"/>\n</svg>\n";
    std::fs::write(&icon, source).unwrap();
    std::fs::write(&other, source).unwrap();
    let (mut app, mut shell, _) = app_on(&dir);
    app.pending_paths.push(icon.clone());
    app.apply_pending(&mut shell);
    landed(&mut app.pictures);
    let p = app.focus_picture().expect("an SVG opens as a picture");
    assert_eq!((&p.state, p.format, p.width, p.translucent), (&State::Ready, "SVG", 2048, true));
    // Its source, asked for: a document beside the picture.
    app.pending_text.push(icon.clone());
    app.apply_pending(&mut shell);
    assert_eq!(app.focus_doc().and_then(|d| d.path.clone()), Some(icon.clone()));
    assert_eq!(app.pictures.iter().count(), 1);
    // A place in the text of one (a search hit) goes to the text.
    app.pending_paths.push(other.clone());
    app.pending_goto = Some((other.clone(), Goto::Span { line: 1, col: 1, len: 4 }));
    app.apply_pending(&mut shell);
    assert_eq!(app.focus_doc().map(|d| d.selected_text()).as_deref(), Some("rect"));
    assert_eq!(app.pictures.iter().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn last_times_tabs_come_back_as_what_they_were() {
    let at = |name: &str| PathBuf::from("/home/x/proj").join(name);
    let open = ["a.rs", "b.png", "c.svg", "d.lstudio", "e.tiff"].iter().map(|n| (at(n), 0, 0)).collect();
    let app = App::new(Settings::default(), Session { open, pictures: vec![at("f.png")], ..Session::default() }, Vec::new());
    assert_eq!(app.pending_paths, vec![at("a.rs"), at("b.png"), at("f.png")], "text, a picture once read as text, a picture");
    assert_eq!(app.pending_text, vec![at("c.svg")], "an SVG's source stays its source");
    assert!(LAUNCHED.with(|l| l.borrow().is_empty()), "coming back starts no other app");
}

#[test]
fn a_picture_follows_its_file() {
    let dir = temp("picture-reload");
    let file = dir.join("logo.png");
    std::fs::write(&file, png(4, 2)).unwrap();
    let mut pictures = Pictures::new();
    pictures.open(DocId(7), file.clone(), None);
    landed(&mut pictures);
    let written = Change { dir: dir.clone(), name: Some("logo.png".into()), mask: IN_CLOSE_WRITE };
    // Saved again, bigger: read again.
    std::fs::write(&file, png(6, 3)).unwrap();
    pictures.changed(&written, None);
    landed(&mut pictures);
    assert_eq!((pictures.get(DocId(7)).unwrap().width, pictures.get(DocId(7)).unwrap().height), (6, 3));
    // Something else in the folder is not its business.
    pictures.changed(&Change { name: Some("other.png".into()), ..written.clone() }, None);
    assert!(!pictures.land());
    // Written as something that is no picture: the old one stays, and says so.
    std::fs::write(&file, b"not a picture").unwrap();
    pictures.changed(&written, None);
    landed(&mut pictures);
    let p = pictures.get(DocId(7)).unwrap();
    assert!(p.state == State::Ready && p.width == 6 && p.stale.is_some());
    assert!(p.info().contains("changed on disk"), "{}", p.info());
    // Deleted.
    std::fs::remove_file(&file).unwrap();
    pictures.changed(&Change { mask: IN_DELETE, ..written }, None);
    assert!(pictures.get(DocId(7)).unwrap().disk_missing);
    // Renamed, alone and with its folder.
    pictures.retarget(&file, &dir.join("mark.png"));
    assert_eq!(pictures.get(DocId(7)).unwrap().title, "mark.png");
    pictures.retarget(&dir, Path::new("/elsewhere"));
    assert_eq!(pictures.get(DocId(7)).unwrap().path, Path::new("/elsewhere/mark.png"));
    pictures.close(DocId(7));
    assert!(!pictures.has(DocId(7)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A picture as if read: `frames` of one pixel, each with its delay.
fn moving(delays: &[u32]) -> Picture {
    let frames = delays.iter().map(|&delay_ms| Frame { image: Image::solid(1, 1, [0; 4]), delay_ms }).collect();
    let mut p = Picture::new(DocId(1), PathBuf::from("/x/spin.gif"));
    p.take(Loaded { frames: Animation { width: 1, height: 1, frames, loops: 0 }, format: "GIF", bytes: 3, translucent: true });
    p
}

#[test]
fn an_animation_keeps_its_own_time() {
    let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1.0e-3);
    // The second frame names no time of its own: a tenth of a second.
    let mut p = moving(&[100, 0, 200]);
    assert!(p.animated() && p.info().contains("3 frames"));
    assert!(near(p.tick(10.0), 0.1) && p.want_frame == 0);
    assert!(near(p.tick(10.1), 0.1) && p.want_frame == 1);
    assert!(near(p.tick(10.2), 0.2) && p.want_frame == 2);
    assert!(near(p.tick(10.3), 0.1) && p.want_frame == 2, "halfway through the long frame");
    // A minute out of sight: it picks up at the next frame, the first again.
    assert!(near(p.tick(70.3), 0.1) && p.want_frame == 0);
    // Paused it stays where it is, however long.
    p.playing = false;
    assert_eq!(p.tick(71.0), None);
    p.playing = true;
    assert!(near(p.tick(99.0), 0.1) && p.want_frame == 0);
    // A still has no clock.
    assert_eq!(moving(&[0]).tick(5.0), None);
}

#[test]
fn the_wheel_zooms_at_the_pointer_and_a_drag_moves() {
    let mut p = moving(&[0]);
    (p.width, p.height) = (4000, 2000);
    p.handle = Some(ImageHandle { id: ImageId(1), width: 4000, height: 2000 });
    let mut h = Harness::new(1200.0, 900.0);
    h.frame(|ui| {
        draw_picture(ui, &mut p);
    });
    let canvas = h.rect_of_label("code").expect("the picture's part of the tab");
    let fit = p.view.shown;
    assert!(p.view.zoom.is_none() && fit < 1.0 && 4000.0 * fit <= canvas.width(), "fitted: {fit}");
    // One notch in with the pointer off to a side: what is under it stays.
    let at = canvas.center() + Vec2::new(200.0, -100.0);
    let under = |p: &Picture| (at - canvas.center() - p.view.pan) * (1.0 / p.view.shown);
    let before = under(&p);
    h.move_to(at);
    h.wheel(1.0);
    h.frame(|ui| {
        draw_picture(ui, &mut p);
    });
    assert!((p.view.shown - fit * 1.2).abs() < 1.0e-9, "{} from {fit}", p.view.shown);
    assert!((under(&p) - before).length() < 1.0e-6);
    // A drag takes the picture along.
    let pan = p.view.pan;
    h.drag(at, at + Vec2::new(30.0, 40.0), 3, |ui| {
        draw_picture(ui, &mut p);
    });
    assert!((p.view.pan - pan - Vec2::new(30.0, 40.0)).length() < 1.0e-6, "{:?} from {pan:?}", p.view.pan);
    // It cannot be dragged out of sight.
    h.drag(at, at + Vec2::new(90_000.0, 0.0), 2, |ui| {
        draw_picture(ui, &mut p);
    });
    assert!(canvas.center().x + p.view.pan.x - 4000.0 * p.view.shown * 0.5 < canvas.max.x, "its left edge is still in the tab");
    // Zoom In and Out step about the middle; Reset fits again.
    let shown = p.view.shown;
    p.view.step(1);
    assert_eq!(p.view.zoom, Some(shown * 1.2));
    p.view.fit();
    h.frame(|ui| {
        draw_picture(ui, &mut p);
    });
    assert!(p.view.zoom.is_none() && p.view.pan == Vec2::ZERO && (p.view.shown - fit).abs() < 1.0e-9);
    // A double click: one pixel for one, and back.
    h.advance(1.0);
    for want in [Some(1.0), None] {
        h.click_at(at, |ui| {
            draw_picture(ui, &mut p);
        });
        h.click_at(at, |ui| {
            draw_picture(ui, &mut p);
        });
        assert_eq!(p.view.zoom, want);
        h.advance(1.0);
    }
}
