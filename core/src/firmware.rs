//! Which firmware is where: the version built into a flash image, and the one
//! running on the board.
//!
//! ESP-IDF stamps `version.txt` into the app descriptor (`esp_app_desc_t`) of
//! every image; the board reports it in the `hello` handshake (see
//! [`crate::link`]).

use serde::Serialize;

use crate::profile::BoardKind;

const PARTITION_TABLE: usize = 0x8000;
const PARTITION_MAGIC: [u8; 2] = [0xAA, 0x50];
const APP_DESC_MAGIC: u32 = 0xABCD_5432;
/// `esp_image_header_t` + the first `esp_image_segment_header_t`.
const APP_DESC_OFFSET: usize = 24 + 8;

/// What the app descriptor of a flash image says about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageInfo {
    pub version: String,
    pub project: String,
    pub idf: String,
    /// Build date and time, as `__DATE__ __TIME__` put it.
    pub built: String,
}

/// What the board said about its firmware.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BoardFirmware {
    /// The version its `hello` reported. `kind` is the board's, from the same `hello`.
    Version { kind: BoardKind, version: String, idf: Option<String> },
    /// The ROM finds no bootable app in flash.
    Missing,
}

impl BoardFirmware {
    /// Which kind of board runs it, when there is firmware.
    pub fn kind(&self) -> Option<BoardKind> {
        match self {
            BoardFirmware::Version { kind, .. } => Some(*kind),
            BoardFirmware::Missing => None,
        }
    }
}

/// The app descriptor of `image`, checked to be firmware for a `kind` board:
/// flashing another board's image would leave it with the wrong display and
/// pins. The text says what is wrong.
pub fn check_image(kind: BoardKind, image: &[u8]) -> Result<ImageInfo, String> {
    let info = image_info(image).ok_or_else(|| format!("the {kind} firmware image has no app descriptor"))?;
    match BoardKind::from_image_project(&info.project) {
        Some(k) if k == kind => Ok(info),
        Some(other) => Err(format!("the image for {kind} is {other} firmware: refusing to flash it")),
        None => Err(format!("the image for {kind} is project `{}`, not {kind} firmware: refusing to flash it", info.project)),
    }
}

/// Where the first app partition starts in a merged image (bootloader +
/// partition table + app), from its partition table.
pub fn app_offset(image: &[u8]) -> Option<usize> {
    let table = image.get(PARTITION_TABLE..PARTITION_TABLE + 0xC00)?;
    table
        .chunks_exact(32)
        .take_while(|e| e[..2] == PARTITION_MAGIC)
        .find(|e| e[2] == 0)
        .map(|e| u32::from_le_bytes(e[4..8].try_into().unwrap()) as usize)
}

/// Read the app descriptor of a merged image: the first app partition's.
pub fn image_info(image: &[u8]) -> Option<ImageInfo> {
    let app = app_offset(image)?;
    let desc = image.get(app + APP_DESC_OFFSET..app + APP_DESC_OFFSET + 256)?;
    if u32::from_le_bytes(desc[..4].try_into().unwrap()) != APP_DESC_MAGIC {
        return None;
    }
    let text = |range: std::ops::Range<usize>| {
        let raw = &desc[range];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).into_owned()
    };
    Some(ImageInfo {
        version: text(16..48),
        project: text(48..80),
        built: format!("{} {}", text(96..112), text(80..96)),
        idf: text(112..144),
    })
}

/// `invalid header: 0xffffffff`: the ROM, looping over an erased flash.
pub(crate) fn is_missing_app_log(line: &str) -> bool {
    line.contains("invalid header")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(version: &str) -> Vec<u8> {
        image_of("eye", version)
    }

    fn image_of(project: &str, version: &str) -> Vec<u8> {
        let mut img = vec![0xFF; 0x10000 + 0x200];
        let mut entry = |i: usize, kind: u8, offset: u32| {
            let e = &mut img[PARTITION_TABLE + i * 32..PARTITION_TABLE + i * 32 + 32];
            e[..2].copy_from_slice(&PARTITION_MAGIC);
            e[2] = kind;
            e[4..8].copy_from_slice(&offset.to_le_bytes());
        };
        entry(0, 1, 0x9000);
        entry(1, 0, 0x10000);
        let desc = &mut img[0x10000 + APP_DESC_OFFSET..0x10000 + APP_DESC_OFFSET + 256];
        desc.fill(0);
        desc[..4].copy_from_slice(&APP_DESC_MAGIC.to_le_bytes());
        desc[16..16 + version.len()].copy_from_slice(version.as_bytes());
        desc[48..48 + project.len()].copy_from_slice(project.as_bytes());
        desc[80..88].copy_from_slice(b"16:05:04");
        desc[96..107].copy_from_slice(b"Sep 26 2026");
        desc[112..116].copy_from_slice(b"v6.1");
        img
    }

    #[test]
    fn reads_the_app_descriptor() {
        let info = image_info(&image("0.2.0")).unwrap();
        assert_eq!(info.version, "0.2.0");
        assert_eq!(info.project, "eye");
        assert_eq!(info.idf, "v6.1");
        assert_eq!(info.built, "Sep 26 2026 16:05:04");
        assert_eq!(image_info(&[0; 100]), None);
    }

    #[test]
    fn images_are_checked_against_the_board() {
        let yoru = image_of("yoru", "0.5.3");
        assert_eq!(check_image(BoardKind::Yoru, &yoru).unwrap().version, "0.5.3");
        assert!(check_image(BoardKind::Yoru, &image("1.0.0")).unwrap_err().contains("`eye`"));
        assert!(check_image(BoardKind::Yoru, &[0; 100]).is_err());
    }
}
