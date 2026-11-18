//! `/render [25|50|100]`: the current world as a PNG picture, `/render open`
//! opens the folder.
//!
//! Every tile is drawn from the game's own sprite sheets: items.dat gives
//! each item's `.rttex` file and 32x32 cell, the file comes from the game
//! folder (`%LOCALAPPDATA%\Growtopia\cache\game` first, then `game`).
//! `.rttex`: an optional 32-byte `RTPACK` header with zlib data, then a
//! 100-byte `RTTXTR` header (height, width at 8/12, alpha flag at 28), a
//! 24-byte mipmap header and the pixels, bottom row first.
//!
//! Blocks that join their neighbours ("smart edge") are drawn with their
//! first cell, so the picture is close to the game, not the same. It runs on
//! a thread; [`tick`] reports when the file is written.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::commands::{say, warn};
use super::interceptor::Interceptor;
use crate::items::ItemNames;

const TILE: usize = 32;
const DIR: &str = "renders";
/// Sky behind empty tiles.
const SKY: [u8; 4] = [96, 192, 255, 255];

/// A decoded sprite sheet, top row first, RGBA.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Finished renders (or errors) waiting for [`tick`] to tell the player.
fn done() -> &'static Mutex<Vec<Result<PathBuf, String>>> {
    static DONE: OnceLock<Mutex<Vec<Result<PathBuf, String>>>> = OnceLock::new();
    DONE.get_or_init(Default::default)
}

fn busy() -> &'static Mutex<bool> {
    static BUSY: OnceLock<Mutex<bool>> = OnceLock::new();
    BUSY.get_or_init(Default::default)
}

/// Decodes a `.rttex` file.
pub fn decode_rttex(data: &[u8]) -> Option<Image> {
    let raw;
    let body: &[u8] = if data.starts_with(b"RTPACK") {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(data.get(32..)?).read_to_end(&mut out).ok()?;
        raw = out;
        &raw
    } else {
        data
    };
    if !body.starts_with(b"RTTXTR") {
        return None;
    }
    let int = |at: usize| body.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let (height, width) = (int(8)? as usize, int(12)? as usize);
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return None;
    }
    let pixels = body.get(124..)?;
    let bpp = if *body.get(28)? != 0 { 4 } else { 3 };
    if pixels.len() < width * height * bpp {
        return None;
    }
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        // Stored bottom row first.
        let src = &pixels[(height - 1 - y) * width * bpp..][..width * bpp];
        let dst = &mut rgba[y * width * 4..][..width * 4];
        for x in 0..width {
            let s = &src[x * bpp..x * bpp + bpp];
            dst[x * 4..x * 4 + 3].copy_from_slice(&s[..3]);
            dst[x * 4 + 3] = if bpp == 4 { s[3] } else { 255 };
        }
    }
    Some(Image { width, height, rgba })
}

fn game_dirs() -> Vec<PathBuf> {
    let Some(base) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return Vec::new() };
    vec![base.join(r"Growtopia\cache\game"), base.join(r"Growtopia\game")]
}

/// Sheets are big: decoded once per run.
fn sheet(file: &str) -> Option<Arc<Image>> {
    static SHEETS: OnceLock<Mutex<HashMap<String, Option<Arc<Image>>>>> = OnceLock::new();
    let sheets = SHEETS.get_or_init(Default::default);
    if let Some(s) = sheets.lock().ok()?.get(file) {
        return s.clone();
    }
    let img = game_dirs()
        .iter()
        .find_map(|d| std::fs::read(d.join(file)).ok())
        .and_then(|data| decode_rttex(&data))
        .map(Arc::new);
    if let Ok(mut s) = sheets.lock() {
        s.insert(file.to_string(), img.clone());
    }
    img
}

/// Alpha-blends one item's sprite at tile (tx, ty) of `out` (`w` pixels wide).
fn draw(out: &mut [u8], w: usize, items: &ItemNames, item: u16, tx: usize, ty: usize) {
    let Some(tex) = items.texture(item as u32) else { return };
    let Some(img) = sheet(&tex.file) else { return };
    let (sx, sy) = (tex.x as usize * TILE, tex.y as usize * TILE);
    if sx + TILE > img.width || sy + TILE > img.height {
        return;
    }
    for y in 0..TILE {
        for x in 0..TILE {
            let s = &img.rgba[((sy + y) * img.width + sx + x) * 4..][..4];
            let a = s[3] as u32;
            if a == 0 {
                continue;
            }
            let d = &mut out[((ty * TILE + y) * w + tx * TILE + x) * 4..][..4];
            for c in 0..3 {
                d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a)) / 255) as u8;
            }
            d[3] = 255;
        }
    }
}

/// The world as RGBA pixels: (width, height, pixels).
pub fn render(width: usize, height: usize, tiles: &[(u16, u16)], items: &ItemNames) -> (usize, usize, Vec<u8>) {
    let (w, h) = (width * TILE, height * TILE);
    let mut out: Vec<u8> = SKY.iter().copied().cycle().take(w * h * 4).collect();
    for (i, &(fg, bg)) in tiles.iter().enumerate().take(width * height) {
        let (tx, ty) = (i % width, i / width);
        if bg != 0 {
            draw(&mut out, w, items, bg, tx, ty);
        }
        if fg != 0 {
            draw(&mut out, w, items, fg, tx, ty);
        }
    }
    (w, h, out)
}

/// Every `step`-th pixel in both directions (50 % = 2, 25 % = 4).
fn shrink(w: usize, h: usize, px: &[u8], step: usize) -> (usize, usize, Vec<u8>) {
    if step <= 1 {
        return (w, h, px.to_vec());
    }
    let (nw, nh) = (w / step, h / step);
    let mut out = Vec::with_capacity(nw * nh * 4);
    for y in 0..nh {
        for x in 0..nw {
            out.extend_from_slice(&px[((y * step) * w + x * step) * 4..][..4]);
        }
    }
    (nw, nh, out)
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, v) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *v = c;
        }
        t
    });
    !data.iter().fold(!0u32, |c, &b| table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}

/// RGBA pixels as a PNG file.
pub fn encode_png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for row in rgba.chunks(w * 4).take(h) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = z.write_all(&raw);
    let idat = z.finish().unwrap_or_default();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        png.extend_from_slice(&body);
        png.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &idat);
    chunk(b"IEND", &[]);
    png
}

/// `2026-11-18_15-40-07` in local-ish time (UTC; Windows gives no zone without a crate).
fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

fn folder() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join(DIR)
}

fn save(world: &str, w: usize, h: usize, px: &[u8]) -> Result<PathBuf, String> {
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let safe: String = world.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let path = dir.join(format!("{safe}_{}.png", stamp()));
    std::fs::write(&path, encode_png(w, h, px)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

/// `/render [25|50|100]`, `/render open`.
pub fn render_cmd(ic: &mut Interceptor, args: &[&str]) {
    if args.first().is_some_and(|a| a.eq_ignore_ascii_case("open")) {
        open_folder(&folder());
        return say(ic, format!("Opened the {DIR} folder"));
    }
    let step = match args.first().map(|a| a.trim_end_matches('%')) {
        None | Some("100") => 1,
        Some("50") => 2,
        Some("25") => 4,
        Some(_) => return warn(ic, format!("Usage: {0}render [25|50|100], {0}render open", ic.cfg.command.prefix)),
    };
    let Some(map) = ic.world.as_ref() else { return warn(ic, "No world map yet: enter a world first") };
    if std::mem::replace(&mut *busy().lock().unwrap(), true) {
        return warn(ic, "A render is already running");
    }
    let (width, height, name) = (map.width as usize, map.height as usize, map.name.clone());
    let tiles: Vec<(u16, u16)> = map.tiles.iter().map(|t| (t.fg, t.bg)).collect();
    let items = ic.items.clone();
    say(ic, format!("Rendering `w{name}`` ({width}x{height})..."));
    std::thread::spawn(move || {
        let (w, h, px) = render(width, height, &tiles, &items);
        let (w, h, px) = shrink(w, h, &px, step);
        let result = save(&name, w, h, &px);
        if let Ok(mut d) = done().lock() {
            d.push(result);
        }
        *busy().lock().unwrap() = false;
    });
}

/// Called every bridge loop: tells the player when a render is saved.
pub fn tick(ic: &mut Interceptor) {
    let results: Vec<_> = match done().try_lock() {
        Ok(mut d) if !d.is_empty() => std::mem::take(&mut *d),
        _ => return,
    };
    for r in results {
        match r {
            Ok(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                say(ic, format!("World saved as `w{DIR}/{name}`` ({0}render open shows the folder)", ic.cfg.command.prefix));
            }
            Err(e) => warn(ic, format!("Render failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_well_formed() {
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&png[12..16], b"IHDR");
        assert!(png.ends_with(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]));
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn rttex_rows_are_flipped() {
        // 1x2 RGBA, bottom row (red) stored first.
        let mut d = b"RTTXTR".to_vec();
        d.resize(124, 0);
        d[8..12].copy_from_slice(&2i32.to_le_bytes());
        d[12..16].copy_from_slice(&1i32.to_le_bytes());
        d[28] = 1;
        d.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255]);
        let img = decode_rttex(&d).unwrap();
        assert_eq!((img.width, img.height), (1, 2));
        assert_eq!(&img.rgba[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn stamp_has_a_date() {
        let s = stamp();
        assert_eq!(s.len(), 19);
        assert!(s.starts_with("20"));
    }

    /// Writes the real first tile sheet for a look (`cargo test sheet_preview -- --ignored`).
    #[test]
    #[ignore]
    fn sheet_preview() {
        let img = sheet("tiles_page1.rttex").expect("tiles_page1.rttex");
        let out = std::env::var("PREVIEW").unwrap_or_else(|_| "tiles_page1.png".into());
        let (w, h, px) = shrink(img.width, img.height, &img.rgba, 2);
        std::fs::write(&out, encode_png(w, h, &px)).unwrap();
        // A small world: dirt floor, sky above.
        let items = ItemNames::load();
        let tiles: Vec<(u16, u16)> = (0..40).map(|i| if i >= 20 { (2, 14) } else { (0, 0) }).collect();
        let (w, h, px) = render(10, 4, &tiles, &items);
        std::fs::write(out.replace(".png", "_world.png"), encode_png(w, h, &px)).unwrap();
    }
}
