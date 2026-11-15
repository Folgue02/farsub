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

/// Installed font families: monospaced ones first, then by name.
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
    db().with_face_data(id, |data, index| (data.to_vec(), index))
}
