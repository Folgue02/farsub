//! GUI: file and folder icons (built-in drawn badges and icon-package images).

use super::*;

/// Draw a file or folder icon centred at `at` (sidebar, tabs, Goto Anything). Folders are a
/// drawn folder shape (open or closed); files a coloured badge with a short label, both
/// from icons.toml.
pub(super) type IconTextures = HashMap<PathBuf, Option<egui::TextureHandle>>;

/// Draw a package icon image if it loads; returns false to fall back to the drawn icon.
pub(super) fn draw_icon_image(
    p: &Painter,
    tex: &mut IconTextures,
    path: &Path,
    at: Pos2,
    tint: Color32,
) -> bool {
    let t = tex.entry(path.to_path_buf()).or_insert_with(|| {
        let img = image::open(path).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        Some(p.ctx().load_texture(
            format!("icon:{}", path.display()),
            ci,
            egui::TextureOptions::LINEAR,
        ))
    });
    let Some(t) = t else { return false };
    let r = Rect::from_center_size(at, Vec2::splat(16.0));
    p.image(
        t.id(),
        r,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        tint,
    );
    true
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_icon(
    p: &Painter,
    cfg: &crate::config::Config,
    tex: &mut IconTextures,
    at: Pos2,
    name: &str,
    is_dir: bool,
    open: bool,
    alpha: f32,
    bg: Color32,
) {
    if !cfg.editor.file_icons {
        return;
    }
    let fade = |c: Color32| mix(bg, c, alpha);
    let tint = Color32::from_white_alpha((alpha * 255.0) as u8);
    if is_dir {
        let img = if open {
            &cfg.folder_images.1
        } else {
            &cfg.folder_images.0
        };
        if let Some(path) = img
            && draw_icon_image(p, tex, path, at, tint)
        {
            return;
        }
    } else if let Some(path) = cfg.icon_for(name).and_then(|d| d.image.as_ref())
        && draw_icon_image(p, tex, Path::new(path), at, tint)
    {
        return;
    }
    let hex = |s: Option<&str>, d: Color32| {
        s.and_then(crate::config::parse_hex_color)
            .map(|(r, g, b)| Color32::from_rgb(r, g, b))
            .unwrap_or(d)
    };
    if is_dir {
        let col = fade(hex(
            cfg.folder_color.as_deref(),
            Color32::from_rgb(0x8F, 0xA1, 0xB3),
        ));
        let body = Rect::from_center_size(at + Vec2::new(0.0, 1.0), Vec2::new(14.0, 10.0));
        // Tab on the top-left edge.
        p.rect_filled(
            Rect::from_min_size(body.min - Vec2::new(0.0, 2.0), Vec2::new(6.0, 3.0)),
            1.0,
            col,
        );
        if open {
            p.rect_filled(body, 1.5, mix(bg, col, 0.55));
            // The open flap, slanted forward.
            let top = body.top() + 3.5;
            p.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(body.left() + 2.5, top),
                    Pos2::new(body.right() + 1.0, top),
                    Pos2::new(body.right() - 1.5, body.bottom()),
                    Pos2::new(body.left(), body.bottom()),
                ],
                col,
                Stroke::NONE,
            ));
        } else {
            p.rect_filled(body, 1.5, col);
        }
        return;
    }
    let def = cfg.icon_for(name);
    let label = def.map(|d| d.label.as_str()).unwrap_or("");
    let base = hex(
        def.map(|d| d.color.as_str()),
        Color32::from_rgb(0x7D, 0x85, 0x90),
    );
    let col = fade(base);
    if label.is_empty() {
        // Plain document with a folded corner.
        let r = Rect::from_center_size(at, Vec2::new(10.0, 13.0));
        let k = 3.5;
        p.add(egui::Shape::convex_polygon(
            vec![
                r.left_top(),
                Pos2::new(r.right() - k, r.top()),
                Pos2::new(r.right(), r.top() + k),
                r.right_bottom(),
                r.left_bottom(),
            ],
            col,
            Stroke::NONE,
        ));
        return;
    }
    let font = FontId::proportional(if label.chars().count() >= 3 { 7.5 } else { 8.5 });
    let galley = p.layout_no_wrap(label.to_string(), font, Color32::WHITE);
    let w = (galley.size().x + 5.0).max(14.0);
    let r = Rect::from_center_size(at, Vec2::new(w, 13.0));
    p.rect_filled(r, 3.0, col);
    // Dark text on light badges, light text on dark ones.
    let lum = 0.299 * base.r() as f32 + 0.587 * base.g() as f32 + 0.114 * base.b() as f32;
    let ink = if lum > 140.0 {
        Color32::from_rgb(0x1E, 0x1E, 0x1E)
    } else {
        Color32::WHITE
    };
    let pos = r.center() - galley.size() / 2.0;
    // Short labels are drawn twice, a hair apart, for a semi-bold look; three letters
    // would smudge together, so they stay regular.
    if label.chars().count() < 3 {
        p.galley(pos + Vec2::new(0.35, 0.0), galley.clone(), fade(ink));
    }
    p.galley(pos, galley, fade(ink));
}
