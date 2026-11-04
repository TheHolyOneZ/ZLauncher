use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Key, Nonce, XSalsa20Poly1305};
use serde::Serialize;

pub const CHUNK: usize = 262_144;
pub const TAG: u64 = 16;
pub const MAX_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_PER_MESSAGE: usize = 5;
pub const PREVIEW_BYTES: u64 = 25 * 1024 * 1024;
pub const TEXT_PREVIEW: usize = 200 * 1024;
const PREVIEW_SIDE: u32 = 1280;
const MAX_PIXELS_SIDE: u32 = 8192;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;

const IMAGE: &[&str] = &["png", "jpg", "jpeg", "jfif", "gif", "webp", "bmp", "ico", "tif", "tiff", "avif", "heic", "heif"];
const VIDEO: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv", "avi", "ogv"];
const TEXT: &[&str] = &[
    "txt", "md", "markdown", "rst", "log", "csv", "tsv", "json", "jsonc", "json5", "ndjson", "yaml", "yml", "toml", "ini", "cfg", "conf", "properties", "xml", "html", "htm", "css",
    "scss", "sass", "less", "svg", "rs", "py", "pyi", "js", "mjs", "cjs", "jsx", "ts", "tsx", "vue", "svelte", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "cs", "java", "kt", "kts",
    "swift", "go", "rb", "php", "lua", "pl", "pm", "r", "jl", "dart", "scala", "clj", "ex", "exs", "erl", "hs", "ml", "nim", "zig", "sh", "bash", "zsh", "fish", "sql", "graphql",
    "proto", "gradle", "cmake", "mk", "tf", "diff", "patch", "lock",
];
const NAMED_TEXT: &[&str] = &["dockerfile", "makefile", "license", "readme", "changelog", "gitignore"];
const STILL_BRANDS: &[&[u8; 4]] = &[b"avif", b"avis", b"heic", b"heix", b"hevc", b"hevx", b"mif1", b"msf1"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Video,
    Archive,
    Document,
    Text,
}

pub fn clean_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let hidden = |c: char| c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}');
    let name: String = base.chars().filter(|c| !hidden(*c)).map(|c| if "<>:\"|?*".contains(c) { '_' } else { c }).collect();
    let mut name = name.trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']).to_string();
    let stem = name.split('.').next().unwrap_or_default().to_ascii_lowercase();
    let numbered = |p: &str| stem.strip_prefix(p).is_some_and(|d| d.len() == 1 && d.bytes().all(|b| b.is_ascii_digit()));
    if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") || numbered("com") || numbered("lpt") {
        name.insert(0, '_');
    }
    if name.chars().count() > 120 {
        let ext = name.rfind('.').map(|d| name[d..].to_string()).filter(|e| e.chars().count() <= 12).unwrap_or_default();
        let keep = 120 - ext.chars().count();
        name = name.chars().take(keep).collect::<String>() + &ext;
    }
    if name.is_empty() {
        "file".into()
    } else {
        name
    }
}

pub fn kind_of(name: &str) -> Option<Kind> {
    let lower = name.to_lowercase();
    let Some((_, ext)) = lower.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) else {
        return NAMED_TEXT.contains(&lower.as_str()).then_some(Kind::Text);
    };
    match ext {
        "zip" => Some(Kind::Archive),
        "pdf" => Some(Kind::Document),
        e if IMAGE.contains(&e) => Some(Kind::Image),
        e if VIDEO.contains(&e) => Some(Kind::Video),
        e if TEXT.contains(&e) => Some(Kind::Text),
        _ => None,
    }
}

fn at(b: &[u8], offset: usize, magic: &[u8]) -> bool {
    b.get(offset..offset + magic.len()) == Some(magic)
}

fn still_brand(b: &[u8]) -> bool {
    at(b, 4, b"ftyp") && b.get(8..12).is_some_and(|brand| STILL_BRANDS.iter().any(|s| s.as_slice() == brand))
}

fn is_image(b: &[u8]) -> bool {
    at(b, 0, b"\x89PNG\r\n\x1a\n")
        || at(b, 0, b"\xff\xd8\xff")
        || at(b, 0, b"GIF87a")
        || at(b, 0, b"GIF89a")
        || (at(b, 0, b"RIFF") && at(b, 8, b"WEBP"))
        || at(b, 0, b"BM")
        || at(b, 0, b"\0\0\x01\0")
        || at(b, 0, b"II*\0")
        || at(b, 0, b"MM\0*")
        || still_brand(b)
}

fn is_video(b: &[u8]) -> bool {
    (at(b, 4, b"ftyp") && !still_brand(b))
        || [&b"moov"[..], b"mdat", b"wide", b"free", b"skip"].iter().any(|t| at(b, 4, t))
        || at(b, 0, b"\x1a\x45\xdf\xa3")
        || (at(b, 0, b"RIFF") && at(b, 8, b"AVI "))
        || at(b, 0, b"OggS")
}

pub fn confirms(kind: Kind, b: &[u8]) -> bool {
    match kind {
        Kind::Image => is_image(b),
        Kind::Video => is_video(b),
        Kind::Archive => at(b, 0, b"PK\x03\x04") || at(b, 0, b"PK\x05\x06"),
        Kind::Document => at(b, 0, b"%PDF-"),
        Kind::Text => !b.contains(&0),
    }
}

pub fn keeps_metadata(kind: Kind, head: &[u8]) -> bool {
    match kind {
        Kind::Video | Kind::Document => true,
        Kind::Image => !(at(head, 0, b"\xff\xd8\xff") || at(head, 0, b"\x89PNG\r\n\x1a\n")),
        Kind::Archive | Kind::Text => false,
    }
}

fn exif_orientation(seg: &[u8]) -> u16 {
    if !at(seg, 4, b"Exif\0\0") || seg.len() < 22 {
        return 0;
    }
    let tiff = 10;
    let little = seg[tiff] == b'I';
    let u16_at = |p: usize| seg.get(p..p + 2).map(|s| if little { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) });
    let u32_at = |p: usize| seg.get(p..p + 4).map(|s| if little { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) });
    let Some(ifd) = u32_at(tiff + 4).map(|o| tiff + o as usize) else { return 0 };
    let Some(count) = u16_at(ifd) else { return 0 };
    (0..count as usize)
        .map(|i| ifd + 2 + i * 12)
        .find(|&e| u16_at(e) == Some(0x0112))
        .and_then(|e| u16_at(e + 8))
        .unwrap_or(0)
}

fn orientation_segment(value: u8) -> Vec<u8> {
    let body = [b'E', b'x', b'i', b'f', 0, 0, b'M', b'M', 0, 0x2a, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, value, 0, 0, 0, 0, 0, 0];
    let size = (body.len() + 2) as u16;
    [&[0xff, 0xe1], &size.to_be_bytes()[..], &body[..]].concat()
}

fn scrub_jpeg(b: Vec<u8>) -> Vec<u8> {
    let mut kept = vec![b[0], b[1]];
    let mut orientation = 0;
    let mut i = 2;
    while i + 4 <= b.len() && b[i] == 0xff {
        let marker = b[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let end = i + 2 + usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        if end > b.len() || end <= i + 2 {
            return b;
        }
        if marker == 0xe1 && orientation == 0 {
            orientation = exif_orientation(&b[i..end]);
        }
        let metadata = marker == 0xfe || ((0xe1..=0xef).contains(&marker) && marker != 0xe2 && marker != 0xee);
        if !metadata {
            kept.extend_from_slice(&b[i..end]);
        }
        i = end;
    }
    if (2..=8).contains(&orientation) {
        let seg = orientation_segment(orientation as u8);
        kept.splice(2..2, seg);
    }
    kept.extend_from_slice(&b[i..]);
    kept
}

fn scrub_png(b: Vec<u8>) -> Vec<u8> {
    let mut kept = b[..8].to_vec();
    let mut i = 8;
    while i + 12 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let Some(end) = i.checked_add(12 + len).filter(|e| *e <= b.len()) else { return b };
        let kind = &b[i + 4..i + 8];
        if ![&b"tEXt"[..], b"zTXt", b"iTXt", b"eXIf", b"tIME"].contains(&kind) {
            kept.extend_from_slice(&b[i..end]);
        }
        let last = kind == b"IEND";
        i = end;
        if last {
            return kept;
        }
    }
    b
}

pub fn scrub(b: Vec<u8>) -> Vec<u8> {
    if at(&b, 0, b"\xff\xd8\xff") {
        scrub_jpeg(b)
    } else if at(&b, 0, b"\x89PNG\r\n\x1a\n") {
        scrub_png(b)
    } else {
        b
    }
}

pub fn chunks_for(size: u64) -> u64 {
    size.div_ceil(CHUNK as u64).max(1)
}

#[derive(Clone)]
pub struct Sealer {
    pub key: [u8; 32],
    pub prefix: [u8; 16],
}

impl Sealer {
    pub fn new(key: [u8; 32], prefix: [u8; 16]) -> Sealer {
        Sealer { key, prefix }
    }

    fn nonce(&self, index: u64) -> Nonce {
        let mut n = [0u8; 24];
        n[..16].copy_from_slice(&self.prefix);
        n[16..20].copy_from_slice(&(index as u32).to_le_bytes());
        n.into()
    }

    fn cipher(&self) -> XSalsa20Poly1305 {
        XSalsa20Poly1305::new(Key::from_slice(&self.key))
    }

    pub fn seal(&self, index: u64, plain: &[u8]) -> Vec<u8> {
        self.cipher().encrypt(&self.nonce(index), plain).expect("encrypting in memory cannot fail")
    }

    pub fn open(&self, index: u64, sealed: &[u8]) -> Option<Vec<u8>> {
        self.cipher().decrypt(&self.nonce(index), sealed).ok()
    }
}

pub fn preview_image(bytes: &[u8]) -> Result<String, String> {
    let unreadable = |_| "This picture can't be shown. You can still save it.".to_string();
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| unreadable(e.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PIXELS_SIDE);
    limits.max_image_height = Some(MAX_PIXELS_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let picture = reader.decode().map_err(|e| unreadable(e.to_string()))?;
    let small = picture.thumbnail(PREVIEW_SIDE, PREVIEW_SIDE);
    let mut out = Cursor::new(Vec::new());
    let mime = if small.color().has_alpha() {
        small.write_to(&mut out, image::ImageFormat::Png).map_err(|e| unreadable(e.to_string()))?;
        "image/png"
    } else {
        small.to_rgb8().write_to(&mut out, image::ImageFormat::Jpeg).map_err(|e| unreadable(e.to_string()))?;
        "image/jpeg"
    };
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(out.into_inner())))
}

pub fn reserve(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 0..10_000 {
        let candidate = dir.join(if n == 0 { name.to_string() } else { format!("{stem} ({n}){ext}") });
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many files with that name"))
}

pub fn mark_downloaded(path: &Path) {
    #[cfg(windows)]
    {
        let stream = format!("{}:Zone.Identifier", path.display());
        let _ = std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://zsync.eu/ttz/\r\n");
    }
    #[cfg(target_os = "macos")]
    {
        let stamp = format!("0081;{:x};ZLauncher;", zl_store::now());
        let _ = std::process::Command::new("/usr/bin/xattr").args(["-w", "com.apple.quarantine", &stamp]).arg(path).status();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = path;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn names_lose_everything_that_could_mislead_or_escape() {
        assert_eq!(clean_name("../../etc/passwd.txt"), "passwd.txt");
        assert_eq!(clean_name(r"C:\Users\me\secret.rs"), "secret.rs");
        assert_eq!(clean_name("evil\u{202e}gnp.exe"), "evilgnp.exe", "the direction trick shows its real extension");
        assert_eq!(clean_name("report.txt:evil.exe"), "report.txt_evil.exe", "no alternate data stream");
        assert_eq!(clean_name("photo.exe. . "), "photo.exe");
        assert_eq!(clean_name("..."), "file");
        assert_eq!(clean_name("CON.txt"), "_CON.txt");
        assert_eq!(clean_name("com1"), "_com1");
        assert_eq!(clean_name(".bashrc"), "bashrc");
        let long = clean_name(&format!("{}.py", "a".repeat(300)));
        assert!(long.ends_with(".py") && long.chars().count() == 120);
    }

    #[test]
    fn only_listed_kinds_pass() {
        for ok in ["a.PNG", "clip.mp4", "src.zip", "main.rs", "tool.py", "notes.md", "doc.pdf", "Dockerfile", "x.tar.zip"] {
            assert!(kind_of(ok).is_some(), "{ok}");
        }
        for no in ["setup.exe", "a.msi", "run.bat", "run.cmd", "x.ps1", "x.vbs", "x.scr", "x.lnk", "x.jar", "x.appimage", "x.deb", "x.dmg", "x.desktop", "x.reg", "a.png.exe", "a.rar", "a.7z", "a.tar", "noext", "app.hta"] {
            assert_eq!(kind_of(no), None, "{no}");
        }
    }

    #[test]
    fn content_has_to_match_its_name() {
        let png = b"\x89PNG\r\n\x1a\n....";
        let exe = b"MZ\x90\0\x03\0\0\0";
        assert!(confirms(Kind::Image, png));
        assert!(!confirms(Kind::Image, exe), "an exe called .png");
        assert!(!confirms(Kind::Text, exe), "an exe called .txt has NUL bytes");
        assert!(!confirms(Kind::Archive, exe) && confirms(Kind::Archive, b"PK\x03\x04rest"));
        assert!(!confirms(Kind::Document, exe) && confirms(Kind::Document, b"%PDF-1.7"));
        assert!(confirms(Kind::Video, b"\0\0\0\x18ftypisom....") && !confirms(Kind::Video, b"\0\0\0\x18ftypheic...."));
        assert!(confirms(Kind::Image, b"\0\0\0\x18ftypheic...."));
        assert!(confirms(Kind::Video, b"\x1a\x45\xdf\xa3...."));
        assert!(!confirms(Kind::Video, png));
    }

    pub(crate) fn jpeg_with_exif() -> Vec<u8> {
        let tiff: Vec<u8> = [
            &b"II*\0"[..], &8u32.to_le_bytes(), &2u16.to_le_bytes(),
            &0x0112u16.to_le_bytes(), &3u16.to_le_bytes(), &1u32.to_le_bytes(), &6u16.to_le_bytes(), &[0, 0],
            &0x8825u16.to_le_bytes(), &4u16.to_le_bytes(), &1u32.to_le_bytes(), &[9, 9, 9, 9],
            b"GPS 52.5200 13.4050 Berlin",
        ].concat();
        let app1 = [&b"Exif\0\0"[..], &tiff].concat();
        let comment = b"taken at home";
        let jfif = b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0";
        [
            &[0xff, 0xd8][..],
            &[0xff, 0xe0], &((jfif.len() + 2) as u16).to_be_bytes(), jfif,
            &[0xff, 0xe1], &((app1.len() + 2) as u16).to_be_bytes(), &app1,
            &[0xff, 0xfe], &((comment.len() + 2) as u16).to_be_bytes(), comment,
            &[0xff, 0xda, 0, 2], b"IMAGEDATA", &[0xff, 0xd9],
        ].concat()
    }

    #[test]
    fn photos_lose_their_location_but_stay_upright() {
        let clean = scrub(jpeg_with_exif());
        let has = |needle: &[u8]| clean.windows(needle.len()).any(|w| w == needle);
        assert!(!has(b"GPS") && !has(b"Berlin") && !has(b"taken at home"));
        assert!(has(b"JFIF") && has(b"IMAGEDATA"), "everything else stays");
        let at = clean.windows(4).position(|w| w == b"Exif").unwrap() - 4;
        let size = usize::from(u16::from_be_bytes([clean[at + 2], clean[at + 3]]));
        assert_eq!(exif_orientation(&clean[at..at + 2 + size]), 6);
    }

    #[test]
    fn png_text_goes_and_the_picture_survives() {
        let mut img = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(3, 2, image::Rgba([200, 10, 10, 255])).write_to(&mut img, image::ImageFormat::Png).unwrap();
        let png = img.into_inner();
        let text = [&5u32.to_be_bytes()[..], b"tEXt", b"where", &[0, 0, 0, 0]].concat();
        let dirty = [&png[..8], &text, &png[8..]].concat();
        let clean = scrub(dirty);
        assert_eq!(clean, png);
        assert!(preview_image(&clean).unwrap().starts_with("data:image/png;base64,"));
    }

    #[test]
    fn damaged_files_are_left_alone_not_panicked_on() {
        let cut = jpeg_with_exif()[..30].to_vec();
        assert_eq!(scrub(cut.clone()), cut);
        let png = [&b"\x89PNG\r\n\x1a\n"[..], &u32::MAX.to_be_bytes(), b"tEXt"].concat();
        assert_eq!(scrub(png.clone()), png);
        assert!(preview_image(b"\x89PNG\r\n\x1a\nnot really").is_err());
        assert!(preview_image(b"MZ\x90\0").is_err());
    }

    #[test]
    fn a_decompression_bomb_is_refused_before_it_allocates() {
        let mut img = Cursor::new(Vec::new());
        image::GrayImage::new(1, 1).write_to(&mut img, image::ImageFormat::Png).unwrap();
        let mut png = img.into_inner();
        png[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&60_000u32.to_be_bytes());
        assert!(preview_image(&png).is_err());
    }

    #[test]
    fn chunks_open_only_in_place_and_only_with_their_key() {
        let s = Sealer::new([7; 32], [9; 16]);
        let sealed = s.seal(3, b"hello");
        assert_eq!(s.open(3, &sealed).as_deref(), Some(&b"hello"[..]));
        assert_eq!(s.open(2, &sealed), None, "a chunk moved to another position");
        assert_eq!(Sealer::new([8; 32], [9; 16]).open(3, &sealed), None);
        let mut tampered = sealed.clone();
        tampered[20] ^= 1;
        assert_eq!(s.open(3, &tampered), None);
    }

    #[test]
    fn matches_the_websites_secretbox() {
        let s = Sealer::new([7; 32], [9; 16]);
        assert_eq!(STANDARD.encode(s.seal(3, "Hi TheZ — äöü 🚀".as_bytes())), TWEETNACL_CHUNK);
    }

    #[test]
    fn saving_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("zl-ttz-reserve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(reserve(&dir, "a.txt").unwrap().file_name().unwrap(), "a.txt");
        assert_eq!(reserve(&dir, "a.txt").unwrap().file_name().unwrap(), "a (1).txt");
        assert_eq!(reserve(&dir, "Makefile").unwrap().file_name().unwrap(), "Makefile");
        assert_eq!(reserve(&dir, "Makefile").unwrap().file_name().unwrap(), "Makefile (1)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const TWEETNACL_CHUNK: &str = "k7AU+hnlCrGBVY1k62ZFChCPTfPNQfETY1TF0Obm44NdVzQV+tRQ";
}
