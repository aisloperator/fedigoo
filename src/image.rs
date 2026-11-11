//! Decoding images into textures, working around a GTK crash.
//!
//! A PNG can carry a `cICP` chunk naming its colour space by numbers
//! (ITU-T H.273).  GTK 4.18's PNG loader segfaults, instead of failing or
//! ignoring the chunk, when the numbers name a colour space it doesn't
//! support, such as "unspecified" (2), which Mastodon's video previews have
//! (issue #28).  So before GDK sees a PNG, a `cICP` chunk it can't handle is
//! taken out; the image then decodes as sRGB, as in any viewer that ignores
//! the chunk.  The values kept are the ones GTK 4.18.6 was seen to decode.

use gtk::{gdk, glib};

/// Colour primaries GDK handles: BT.709/sRGB, BT.601 (two), BT.2020, Display P3.
const PRIMARIES: [u8; 5] = [1, 5, 6, 9, 12];
/// Transfer functions GDK handles: BT.709, gamma 2.2, gamma 2.8, BT.601,
/// linear, sRGB, BT.2020 (two), PQ, HLG.
const TRANSFERS: [u8; 10] = [1, 4, 5, 6, 8, 13, 14, 15, 16, 18];
/// Full range; GDK also crashed on narrow range (0).
const FULL_RANGE: u8 = 1;

/// Decodes an image (PNG, JPEG, TIFF, or whatever gdk-pixbuf can load).
pub fn texture_from_bytes(bytes: &glib::Bytes) -> Result<gdk::Texture, glib::Error> {
    match without_unsupported_cicp(bytes) {
        Some(fixed) => gdk::Texture::from_bytes(&glib::Bytes::from_owned(fixed)),
        None => gdk::Texture::from_bytes(bytes),
    }
}

/// If `data` is a PNG with a `cICP` chunk that GDK can't handle, a copy
/// without that chunk; otherwise None.
fn without_unsupported_cicp(data: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIGNATURE) {
        return None;
    }
    let mut at = SIGNATURE.len();
    // cICP comes before the image data, so only the chunks before IDAT matter.
    while let Some(header) = data.get(at..at + 8) {
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = &header[4..8];
        // Length, type, data and CRC.
        let end = at.checked_add(12)?.checked_add(len)?;
        let body = data.get(at + 8..end - 4)?;
        match kind {
            b"IDAT" | b"IEND" => return None,
            b"cICP" if !supported(body) => {
                let mut fixed = Vec::with_capacity(data.len() - (end - at));
                fixed.extend_from_slice(&data[..at]);
                fixed.extend_from_slice(data.get(end..)?);
                return Some(fixed);
            }
            _ => {}
        }
        at = end;
    }
    None
}

/// Whether GDK can decode a PNG whose `cICP` chunk holds `body`: colour
/// primaries, transfer function, matrix (always 0 for RGB), full-range flag.
fn supported(body: &[u8]) -> bool {
    match body {
        [primaries, transfer, _matrix, range] => {
            PRIMARIES.contains(primaries) && TRANSFERS.contains(transfer) && *range == FULL_RANGE
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::*;

    fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = (body.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(kind);
        c.extend_from_slice(body);
        // The CRC isn't checked here.
        c.extend_from_slice(&[0; 4]);
        c
    }

    fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
        for c in chunks {
            p.extend_from_slice(c);
        }
        p
    }

    #[test]
    fn removes_an_unspecified_colour_space() {
        // As in the video preview of issue #28: primaries "unspecified".
        let ihdr = chunk(b"IHDR", &[0; 13]);
        let rest = [
            chunk(b"gAMA", &[0, 0, 0xb1, 0x8f]),
            chunk(b"IDAT", &[1, 2, 3]),
        ];
        let with = png(&[
            ihdr.clone(),
            chunk(b"cICP", &[2, 1, 0, 1]),
            rest[0].clone(),
            rest[1].clone(),
        ]);
        let without = png(&[ihdr, rest[0].clone(), rest[1].clone()]);
        assert_eq!(without_unsupported_cicp(&with), Some(without));
    }

    #[test]
    fn removes_other_unsupported_values() {
        let bodies: [&[u8]; 4] = [&[1, 2, 0, 1], &[3, 1, 0, 1], &[1, 1, 0, 0], &[1, 1, 0]];
        for body in bodies {
            let p = png(&[chunk(b"IHDR", &[0; 13]), chunk(b"cICP", body)]);
            assert!(without_unsupported_cicp(&p).is_some(), "{body:?}");
        }
    }

    #[test]
    fn keeps_what_gdk_supports() {
        for body in [[1, 13, 0, 1], [9, 16, 0, 1], [12, 13, 0, 1], [1, 8, 0, 1]] {
            let p = png(&[chunk(b"IHDR", &[0; 13]), chunk(b"cICP", &body)]);
            assert_eq!(without_unsupported_cicp(&p), None, "{body:?}");
        }
    }

    #[test]
    fn leaves_other_data_alone() {
        assert_eq!(without_unsupported_cicp(b"\xff\xd8\xff\xe0 a JPEG"), None);
        assert_eq!(without_unsupported_cicp(b""), None);
        // A cICP after the image data isn't read by decoders.
        let late = png(&[chunk(b"IDAT", &[1]), chunk(b"cICP", &[2, 2, 0, 1])]);
        assert_eq!(without_unsupported_cicp(&late), None);
        // Truncated or with a length past the end: left for GDK to reject.
        let mut cut = png(&[chunk(b"IHDR", &[0; 13]), chunk(b"cICP", &[2, 1, 0, 1])]);
        cut.truncate(cut.len() - 6);
        assert_eq!(without_unsupported_cicp(&cut), None);
        let huge = png(&[[0xff, 0xff, 0xff, 0xff]
            .iter()
            .chain(b"cICP")
            .copied()
            .collect()]);
        assert_eq!(without_unsupported_cicp(&huge), None);
    }

    /// A 1×1 red PNG with the cICP chunk of issue #28.  GTK 4.18.6 crashes
    /// decoding it as it is.
    const RED_UNSPECIFIED: &str = "89504e470d0a1a0a0000000d494844520000000100000001080200\
        0000907753de00000004634943500201000187c66db80000000c49444154789c63f8cfc000000301\
        0100c9fe92ef0000000049454e44ae426082";

    #[test]
    fn gdk_decodes_it_after() {
        let hex: Vec<u8> = RED_UNSPECIFIED
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        let data: Vec<u8> = hex
            .chunks(2)
            .map(|h| u8::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
            .collect();
        let t = texture_from_bytes(&glib::Bytes::from_owned(data)).expect("decodes");
        assert_eq!((t.width(), t.height()), (1, 1));
        let mut pixel = [0u8; 4];
        t.download(&mut pixel, 4);
        // Cairo's ARGB32 in memory order: blue, green, red, alpha.
        assert_eq!(pixel, [0, 0, 0xff, 0xff]);
    }
}
