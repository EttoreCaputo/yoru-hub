//! The apps' icons Yoru shows on the board (`set_icon`, firmware 0.20.15):
//! on the clips' card when it uses a service, in the errand's portal, and
//! on the recording mark of a meeting. Found once and kept in the data
//! folder (`icons/`):
//! - an app on this Mac with the service's name (Spotify, ClickUp,
//!   WhatsApp…), its own icon, through AppKit;
//! - the icon a meeting's app gave when it was recorded ([`remember`]);
//! - else the service's site: a few known ones (Google's), or the
//!   `apple-touch-icon.png` of `<name>.com`, with rounded corners.
//!
//! A service with none is asked again after a week.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use base64::Engine;
use image::{RgbaImage, imageops};
use serde_json::json;

use crate::link::Link;

/// The board's icons are this many pixels a side.
pub const SIDE: u32 = 48;
/// The raw bytes in one `set_icon` part (the board takes at most 2048).
const PART: usize = 2048;
const RETRY_NONE: Duration = Duration::from_secs(7 * 24 * 3600);
const WEB_TIMEOUT: Duration = Duration::from_secs(3);
const BOARD_TIMEOUT: Duration = Duration::from_secs(3);

/// Services with no app of their own name and a known icon.
const KNOWN: &[(&str, &str)] = &[
    ("gmail", "https://www.gstatic.com/images/branding/product/2x/gmail_2020q4_96dp.png"),
    ("calendar", "https://www.gstatic.com/images/branding/product/2x/calendar_2020q4_96dp.png"),
    ("drive", "https://www.gstatic.com/images/branding/product/2x/drive_2020q4_96dp.png"),
    ("google meet", "https://www.gstatic.com/images/branding/product/2x/meet_2020q4_96dp.png"),
];

/// Services Yoru draws itself (the web search's globe): no icon.
const OWN: &[&str] = &["web", "search", "memory", "task", "timer", "computer"];

/// What this run has already looked for, so a service without an icon
/// isn't looked for again on every call.
static LOOKED: Mutex<Option<HashMap<String, Option<Vec<u8>>>>> = Mutex::new(None);

fn dir() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join("icons"))
}

/// The file name for `source`: lower case letters and digits.
fn slug(source: &str) -> String {
    source.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The icon for `source`, as the board takes it (RGB565A8, [`SIDE`] pixels),
/// or `None` if it has none.
pub fn for_source(source: &str) -> Option<Vec<u8>> {
    let key = slug(source);
    if key.is_empty() || OWN.contains(&source.trim().to_lowercase().as_str()) {
        return None;
    }
    if let Some(found) = LOOKED.lock().unwrap().get_or_insert_with(HashMap::new).get(&key) {
        return found.clone();
    }
    let found = find(source, &key).and_then(|png| rgb565a8(&png));
    LOOKED.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, found.clone());
    found
}

/// Keep `png` as `source`'s icon (a meeting's app, as macOS gives it).
pub fn remember(source: &str, png: &[u8]) {
    let key = slug(source);
    let (Some(dir), false) = (dir(), key.is_empty()) else { return };
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join(format!("{key}.png")), png);
    let _ = fs::remove_file(dir.join(format!("{key}.none")));
    LOOKED.lock().unwrap().get_or_insert_with(HashMap::new).remove(&key);
}

/// [`remember`] for a PNG in base64, as the recorder's helper gives it.
pub fn remember_base64(source: &str, b64: &str) {
    if let Ok(png) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
        remember(source, &png);
    }
}

fn find(source: &str, key: &str) -> Option<Vec<u8>> {
    let dir = dir();
    if let Some(dir) = &dir {
        if let Ok(png) = fs::read(dir.join(format!("{key}.png"))) {
            return Some(png);
        }
        let none = dir.join(format!("{key}.none"));
        let recent = fs::metadata(&none).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
        if recent.is_some_and(|age| age < RETRY_NONE) {
            return None;
        }
    }
    let png = mac_app(source).or_else(|| web(source, key));
    if let Some(dir) = &dir {
        let _ = fs::create_dir_all(dir);
        let _ = match &png {
            Some(png) => fs::write(dir.join(format!("{key}.png")), png),
            None => fs::write(dir.join(format!("{key}.none")), b""),
        };
    }
    png
}

/// The icon of the app on this Mac named `source`, as a PNG.
#[cfg(target_os = "macos")]
fn mac_app(source: &str) -> Option<Vec<u8>> {
    let out = std::env::temp_dir().join(format!("yoru-icon-{}.png", std::process::id()));
    // AppKit through JavaScript for Automation: no helper needed.
    let script = format!(
        r#"ObjC.import('AppKit');
var ws = $.NSWorkspace.sharedWorkspace;
var path = ws.fullPathForApplication({name});
if (path && !path.isNil()) {{
  var image = ws.iconForFile(path);
  var size = 128;
  var rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(null, size, size, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  $.NSGraphicsContext.saveGraphicsState;
  $.NSGraphicsContext.setCurrentContext($.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep));
  image.drawInRectFromRectOperationFraction($.NSMakeRect(0, 0, size, size), $.NSZeroRect, $.NSCompositingOperationSourceOver, 1);
  $.NSGraphicsContext.restoreGraphicsState;
  rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $()).writeToFileAtomically({out}, true);
}}"#,
        name = serde_json::to_string(source.trim()).ok()?,
        out = serde_json::to_string(&out.to_string_lossy()).ok()?,
    );
    let ok = Command::new("osascript").args(["-l", "JavaScript", "-e", &script]).output().ok()?.status.success();
    let png = ok.then(|| fs::read(&out).ok()).flatten();
    let _ = fs::remove_file(&out);
    png
}

#[cfg(not(target_os = "macos"))]
fn mac_app(_source: &str) -> Option<Vec<u8>> {
    None
}

/// The service's icon from its site, with rounded corners like an app's.
fn web(source: &str, key: &str) -> Option<Vec<u8>> {
    let low = source.trim().to_lowercase();
    let urls: Vec<String> = match KNOWN.iter().find(|(word, _)| low.contains(word)) {
        Some((_, url)) => vec![url.to_string()],
        None => vec![format!("https://{key}.com/apple-touch-icon.png"), format!("https://www.{key}.com/apple-touch-icon.png")],
    };
    let agent = ureq::Agent::config_builder().timeout_global(Some(WEB_TIMEOUT)).build().new_agent();
    let started = Instant::now();
    for url in urls {
        if started.elapsed() > WEB_TIMEOUT * 2 {
            break;
        }
        let Ok(response) = agent.get(&url).call() else { continue };
        let mut bytes = Vec::new();
        if response.into_body().into_reader().take(1 << 20).read_to_end(&mut bytes).is_err() {
            continue;
        }
        let Ok(image) = image::load_from_memory(&bytes) else { continue };
        let mut rgba = imageops::resize(&image.to_rgba8(), 128, 128, imageops::FilterType::Lanczos3);
        round_corners(&mut rgba, 0.225);
        let mut png = Vec::new();
        if image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
            return Some(png);
        }
    }
    None
}

/// Clear the corners outside a rounded square, `radius` of its side, with a
/// soft edge.
fn round_corners(image: &mut RgbaImage, radius: f32) {
    let (w, h) = image.dimensions();
    let r = radius * w as f32;
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let cx = (x as f32 + 0.5).clamp(r, w as f32 - r);
        let cy = (y as f32 + 0.5).clamp(r, h as f32 - r);
        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
        let cover = (r - d + 0.5).clamp(0.0, 1.0);
        pixel.0[3] = (pixel.0[3] as f32 * cover) as u8;
    }
}

/// `png` as the board draws it: [`SIDE`] pixels, RGB565 little endian, then
/// the alpha plane (LVGL's RGB565A8).
pub fn rgb565a8(png: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(png).ok()?.to_rgba8();
    let small = imageops::resize(&image, SIDE, SIDE, imageops::FilterType::Lanczos3);
    let n = (SIDE * SIDE) as usize;
    let mut out = vec![0u8; n * 3];
    for (i, p) in small.pixels().enumerate() {
        let [r, g, b, a] = p.0;
        let v = ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3);
        out[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        out[n * 2 + i] = a;
    }
    Some(out)
}

/// Make sure the board on `link` has `source`'s icon: asks, and sends it in
/// parts if not. False when there is none, or the board can't take it.
pub fn send(link: &Link, source: &str) -> bool {
    let Some(icon) = for_source(source) else { return false };
    let name: String = source.chars().scan(0, |len, c| {
        *len += c.len_utf8();
        (*len <= 24).then_some(c)
    }).collect();
    let have = link.call_tool("set_icon", json!({"name": name}), BOARD_TIMEOUT);
    match have {
        Ok(r) if !r.is_error => {
            if r.structured_content.as_ref().and_then(|s| s["have"].as_bool()) == Some(true) {
                return true;
            }
        }
        // Firmware before 0.20.15, or the board's busy: no icon then.
        _ => return false,
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    for (i, part) in icon.chunks(PART).enumerate() {
        let args = json!({"name": name, "at": i * PART, "data": b64.encode(part)});
        match link.call_tool("set_icon", args, BOARD_TIMEOUT) {
            Ok(r) if !r.is_error => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Google Meet"), "googlemeet");
        assert_eq!(slug("zoom.us"), "zoomus");
    }

    #[test]
    fn converts_to_the_boards_format() {
        let mut image = RgbaImage::new(96, 96);
        for p in image.pixels_mut() {
            p.0 = [255, 0, 0, 255];
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let icon = rgb565a8(&png).unwrap();
        assert_eq!(icon.len(), (SIDE * SIDE * 3) as usize);
        assert_eq!(&icon[..2], &0xF800u16.to_le_bytes());
        assert_eq!(icon[(SIDE * SIDE * 2) as usize], 255);
    }

    #[test]
    fn rounds_the_corners() {
        let mut image = RgbaImage::from_pixel(100, 100, image::Rgba([9, 9, 9, 255]));
        round_corners(&mut image, 0.25);
        assert_eq!(image.get_pixel(0, 0).0[3], 0);
        assert_eq!(image.get_pixel(50, 50).0[3], 255);
    }
}
