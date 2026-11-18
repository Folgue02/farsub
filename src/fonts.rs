//! Fonts installed on this computer: the families for Preferences › Select Font, and the
//! file of one (`font_face`) for the editor text.

use std::sync::OnceLock;

/// Every installed font, scanned once (a few hundred ms) on first use.
fn db() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut d = fontdb::Database::new();
        d.load_system_fonts();
        d
    })
}

/// Scan the fonts in the background, so the first use doesn't wait.
pub fn warm_up() {
    std::thread::spawn(|| {
        db();
    });
}

#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    pub name: String,
    /// Every character is the same width (what code is usually set in).
    pub mono: bool,
}

/// Whether egui can draw text with this face: it has glyph outlines (TrueType `glyf` or
/// CFF). Bitmap-only fonts such as macOS's "GB18030 Bitmap" give egui no usable line
/// height (NaN), which broke the editor's layout.
fn has_outlines(data: &[u8], index: u32) -> bool {
    let Ok(face) = ttf_parser::Face::parse(data, index) else {
        return false;
    };
    let raw = face.raw_face();
    face.units_per_em() > 0
        && [b"glyf", b"CFF ", b"CFF2"]
            .iter()
            .any(|t| raw.table(ttf_parser::Tag::from_bytes(t)).is_some())
}

/// Installed font families egui can draw: monospaced ones first, then by name.
pub fn families() -> Vec<Family> {
    let mut map: std::collections::BTreeMap<String, bool> = Default::default();
    for f in db().faces() {
        let Some((name, _)) = f.families.first() else {
            continue;
        };
        // Hidden system fonts on macOS start with a dot.
        if name.starts_with('.') {
            continue;
        }
        if !db().with_face_data(f.id, has_outlines).unwrap_or(false) {
            continue;
        }
        *map.entry(name.clone()).or_default() |= f.monospaced;
    }
    let mut v: Vec<Family> = map
        .into_iter()
        .map(|(name, mono)| Family { name, mono })
        .collect();
    v.sort_by(|a, b| {
        b.mono
            .cmp(&a.mono)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

/// The regular face of `family`: the font file's bytes and the face's index in it.
pub fn load(family: &str) -> Option<(Vec<u8>, u32)> {
    let names = [fontdb::Family::Name(family)];
    let id = db().query(&fontdb::Query {
        families: &names,
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    })?;
    // The query falls back to similar fonts; only an exact family counts.
    let face = db().face(id)?;
    if !face
        .families
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case(family))
    {
        return None;
    }
    db().with_face_data(id, |data, index| {
        has_outlines(data, index).then(|| (data.to_vec(), index))
    })?
}

#[cfg(test)]
mod tests {
    /// Every installed family loads into egui and draws text without panicking (what
    /// previewing fonts in Select Font does). Slow: scans and loads every font.
    #[test]
    #[ignore]
    fn every_installed_font_loads_into_egui() {
        use eframe::egui;
        let mut bad = vec![];
        for f in super::families() {
            let Some((bytes, index)) = super::load(&f.name) else {
                continue;
            };
            let r = std::panic::catch_unwind(|| {
                let ctx = egui::Context::default();
                let mut defs = egui::FontDefinitions::default();
                let mut fd = egui::FontData::from_owned(bytes);
                fd.index = index;
                defs.font_data
                    .insert("x".into(), std::sync::Arc::new(fd));
                defs.families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(0, "x".into());
                ctx.set_fonts(defs);
                for _ in 0..2 {
                    let _ = ctx.run_ui(Default::default(), |ui| {
                        ui.label(egui::RichText::new("fn main() { let x = 1; } ½ → ✓").monospace());
                    });
                }
            });
            if r.is_err() {
                bad.push(f.name.clone());
            }
        }
        assert!(bad.is_empty(), "fonts that crash egui: {bad:?}");
    }

    /// Every family offered has a usable row height and glyph width, as the editor
    /// measures them (a bitmap-only font gave NaN and crashed the editor).
    #[test]
    fn font_metrics_are_finite() {
        use eframe::egui;
        let mut bad = vec![];
        for f in super::families() {
            let Some((bytes, index)) = super::load(&f.name) else {
                continue;
            };
            let ctx = egui::Context::default();
            let mut defs = egui::FontDefinitions::default();
            let mut fd = egui::FontData::from_owned(bytes);
            fd.index = index;
            defs.font_data.insert("x".into(), std::sync::Arc::new(fd));
            defs.families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "x".into());
            ctx.set_fonts(defs);
            let mut m = (0.0, 0.0);
            for _ in 0..2 {
                let mut out = ctx.run_ui(Default::default(), |ui| {
                    let font = egui::FontId::monospace(14.0);
                    m = ui.ctx().fonts_mut(|x| (x.glyph_width(&font, 'M'), x.row_height(&font)));
                });
                out.textures_delta.clear();
            }
            if !(m.0 > 0.0 && m.0.is_finite() && m.1 > 0.0 && m.1.is_finite()) {
                bad.push((f.name.clone(), m));
            }
        }
        assert!(bad.is_empty(), "fonts with unusable metrics: {bad:?}");
    }
}
