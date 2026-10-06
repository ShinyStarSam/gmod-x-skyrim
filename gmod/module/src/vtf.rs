//! Source's texture files (.vtf), decoded to RGBA8 for Skyrim (geometry route step G3b, DESIGN.md
//! §12). The file comes from Garry's Mod's own file system (`file.Read(..., "GAME")`), read at run
//! time; nothing is stored. Header versions 7.0-7.5. Only the first frame, face and slice of the
//! biggest mip level no larger than `max_size` is decoded.
//!
//! Layout (Valve Developer Community "VTF (Valve Texture Format)"): a header (`header_size` bytes),
//! then for 7.0-7.2 the low-resolution thumbnail and the mip levels from the smallest to the
//! largest, each holding every frame, face and slice; from 7.3 a resource table in the header says
//! where the image data starts (tag 0x30 0 0).

pub const FMT_RGBA8888: u32 = 0;
pub const FMT_ABGR8888: u32 = 1;
pub const FMT_RGB888: u32 = 2;
pub const FMT_BGR888: u32 = 3;
pub const FMT_I8: u32 = 5;
pub const FMT_IA88: u32 = 6;
pub const FMT_ARGB8888: u32 = 11;
pub const FMT_BGRA8888: u32 = 12;
pub const FMT_DXT1: u32 = 13;
pub const FMT_DXT3: u32 = 14;
pub const FMT_DXT5: u32 = 15;
pub const FMT_BGRX8888: u32 = 16;
pub const FMT_DXT1_ONEBITALPHA: u32 = 20;

const FLAG_ENVMAP: u32 = 0x4000;

/// A decoded picture: RGBA8, top row first.
#[derive(Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// What the file said, for the log.
    pub format: u32,
    pub mip: u32,
    pub full_width: u32,
    pub full_height: u32,
}

/// Bytes one image of this format and size takes in the file (every format, so the levels and
/// frames before the one we want can be skipped). None for an unknown format.
pub fn image_bytes(format: u32, w: u32, h: u32) -> Option<usize> {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    Some(match format {
        13 | 20 => blocks * 8,              // DXT1, DXT1_ONEBITALPHA
        14 | 15 => blocks * 16,             // DXT3, DXT5
        0 | 1 | 11 | 12 | 16 | 23 | 26 => w * h * 4,
        2 | 3 | 9 | 10 => w * h * 3,
        4 | 6 | 17 | 18 | 19 | 21 | 22 => w * h * 2,
        5 | 7 | 8 => w * h,
        24 | 25 => w * h * 8,
        _ => return None,
    })
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Decodes the biggest mip level that is at most `max_size` wide and high.
pub fn decode(file: &[u8], max_size: u32) -> Result<Image, String> {
    if file.get(0..4) != Some(b"VTF\0") {
        return Err("not a VTF file".into());
    }
    let short = || "VTF header cut short".to_string();
    let (major, minor) = (u32_at(file, 4).ok_or_else(short)?, u32_at(file, 8).ok_or_else(short)?);
    if major != 7 || minor > 5 {
        return Err(format!("VTF version {major}.{minor} not supported"));
    }
    let header_size = u32_at(file, 12).ok_or_else(short)? as usize;
    let (w, h) = (u16_at(file, 16).ok_or_else(short)? as u32, u16_at(file, 18).ok_or_else(short)? as u32);
    let flags = u32_at(file, 20).ok_or_else(short)?;
    let frames = (u16_at(file, 24).ok_or_else(short)? as usize).max(1);
    let format = u32_at(file, 52).ok_or_else(short)?;
    let mips = (*file.get(56).ok_or_else(short)? as u32).max(1);
    let low_format = u32_at(file, 57).ok_or_else(short)?;
    let (low_w, low_h) = (*file.get(61).ok_or_else(short)? as u32, *file.get(62).ok_or_else(short)? as u32);
    let depth = if minor >= 2 { (u16_at(file, 63).ok_or_else(short)? as usize).max(1) } else { 1 };
    if w == 0 || h == 0 {
        return Err("VTF has no size".into());
    }
    if flags & FLAG_ENVMAP != 0 {
        return Err("VTF is a cube map (not supported)".into());
    }
    let faces = 1usize;

    // Where the mip levels start.
    let data_start = if minor >= 3 {
        let count = u32_at(file, 68).ok_or_else(short)? as usize;
        let mut found = None;
        for k in 0..count.min(32) {
            let at = 80 + k * 8;
            let tag = file.get(at..at + 3).ok_or_else(short)?;
            if tag == [0x30, 0, 0] {
                found = Some(u32_at(file, at + 4).ok_or_else(short)? as usize);
            }
        }
        found.ok_or("VTF has no image resource")?
    } else {
        let low = if low_format == u32::MAX || low_w == 0 || low_h == 0 {
            0
        } else {
            image_bytes(low_format, low_w, low_h).ok_or(format!("VTF thumbnail format {low_format} unknown"))?
        };
        header_size + low
    };

    // The level to use: the first (biggest) that fits in max_size.
    let mut mip = 0;
    while mip + 1 < mips && ((w >> mip) > max_size || (h >> mip) > max_size) {
        mip += 1;
    }
    let level = |m: u32| ((w >> m).max(1), (h >> m).max(1));
    let mut at = data_start;
    for m in (mip + 1..mips).rev() {
        let (lw, lh) = level(m);
        at += image_bytes(format, lw, lh).ok_or(format!("VTF format {format} unknown"))? * frames * faces * depth;
    }
    let (lw, lh) = level(mip);
    let n = image_bytes(format, lw, lh).ok_or(format!("VTF format {format} unknown"))?;
    let px = file.get(at..at + n).ok_or(format!("VTF data cut short (level {mip} at {at}, {n} bytes, file {} bytes)", file.len()))?;
    let rgba = to_rgba(format, lw, lh, px)?;
    Ok(Image { width: lw, height: lh, rgba, format, mip, full_width: w, full_height: h })
}

fn to_rgba(format: u32, w: u32, h: u32, px: &[u8]) -> Result<Vec<u8>, String> {
    let count = (w * h) as usize;
    let mut out = vec![0u8; count * 4];
    let per = |stride: usize, f: &dyn Fn(&[u8]) -> [u8; 4], out: &mut Vec<u8>| {
        for i in 0..count {
            out[i * 4..i * 4 + 4].copy_from_slice(&f(&px[i * stride..i * stride + stride]));
        }
    };
    match format {
        FMT_RGBA8888 => out.copy_from_slice(&px[..count * 4]),
        FMT_ABGR8888 => per(4, &|p| [p[3], p[2], p[1], p[0]], &mut out),
        FMT_ARGB8888 => per(4, &|p| [p[1], p[2], p[3], p[0]], &mut out),
        FMT_BGRA8888 => per(4, &|p| [p[2], p[1], p[0], p[3]], &mut out),
        FMT_BGRX8888 => per(4, &|p| [p[2], p[1], p[0], 255], &mut out),
        FMT_RGB888 => per(3, &|p| [p[0], p[1], p[2], 255], &mut out),
        FMT_BGR888 => per(3, &|p| [p[2], p[1], p[0], 255], &mut out),
        FMT_I8 => per(1, &|p| [p[0], p[0], p[0], 255], &mut out),
        FMT_IA88 => per(2, &|p| [p[0], p[0], p[0], p[1]], &mut out),
        FMT_DXT1 | FMT_DXT1_ONEBITALPHA => decode_blocks(w, h, px, 8, &mut out, |b, o| bc1(b, o, true)),
        FMT_DXT3 => decode_blocks(w, h, px, 16, &mut out, |b, o| {
            bc1(&b[8..16], o, false);
            for i in 0..16 {
                let nib = (b[i / 2] >> ((i % 2) * 4)) & 0xF;
                o[i][3] = nib * 17;
            }
        }),
        FMT_DXT5 => decode_blocks(w, h, px, 16, &mut out, |b, o| {
            bc1(&b[8..16], o, false);
            let (a0, a1) = (b[0] as u32, b[1] as u32);
            let mut alpha = [0u32; 8];
            alpha[0] = a0;
            alpha[1] = a1;
            if a0 > a1 {
                for k in 1..7 {
                    alpha[k + 1] = ((7 - k as u32) * a0 + k as u32 * a1) / 7;
                }
            } else {
                for k in 1..5 {
                    alpha[k + 1] = ((5 - k as u32) * a0 + k as u32 * a1) / 5;
                }
                alpha[6] = 0;
                alpha[7] = 255;
            }
            let bits = b[2..8].iter().enumerate().fold(0u64, |acc, (i, &v)| acc | (v as u64) << (8 * i));
            for i in 0..16 {
                o[i][3] = alpha[((bits >> (3 * i)) & 7) as usize] as u8;
            }
        }),
        _ => return Err(format!("VTF format {format} not decoded")),
    }
    Ok(out)
}

fn rgb565(c: u16) -> [u32; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [(r as u32 * 255 + 15) / 31, (g as u32 * 255 + 31) / 63, (b as u32 * 255 + 15) / 31]
}

/// One BC1 colour block into 16 pixels. `alpha_mode`: DXT1 may use 3 colours + transparent;
/// the colour half of DXT3 / DXT5 always uses 4 colours.
fn bc1(b: &[u8], o: &mut [[u8; 4]; 16], alpha_mode: bool) {
    let (c0, c1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mut pal = [[0u32; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || !alpha_mode {
        for k in 0..3 {
            pal[2][k] = (2 * p0[k] + p1[k]) / 3;
            pal[3][k] = (p0[k] + 2 * p1[k]) / 3;
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for k in 0..3 {
            pal[2][k] = (p0[k] + p1[k]) / 2;
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for i in 0..16 {
        let p = pal[((idx >> (2 * i)) & 3) as usize];
        o[i] = [p[0] as u8, p[1] as u8, p[2] as u8, p[3] as u8];
    }
}

fn decode_blocks(w: u32, h: u32, px: &[u8], block_bytes: usize, out: &mut [u8], f: impl Fn(&[u8], &mut [[u8; 4]; 16])) {
    let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
    let mut texels = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let at = (by * bw + bx) * block_bytes;
            f(&px[at..at + block_bytes], &mut texels);
            for i in 0..16 {
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                if x < w as usize && y < h as usize {
                    let o = (y * w as usize + x) * 4;
                    out[o..o + 4].copy_from_slice(&texels[i]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 7.2 header (80 bytes) or 7.3 with one image resource (88 bytes, data at 88).
    fn header(minor: u32, w: u16, h: u16, format: u32, mips: u8, frames: u16) -> Vec<u8> {
        let size: u32 = if minor >= 3 { 88 } else { 80 };
        let mut v = Vec::new();
        v.extend_from_slice(b"VTF\0");
        v.extend_from_slice(&7u32.to_le_bytes());
        v.extend_from_slice(&minor.to_le_bytes());
        v.extend_from_slice(&size.to_le_bytes());
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // flags
        v.extend_from_slice(&frames.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes()); // first frame
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&[0; 12]); // reflectivity
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&1f32.to_le_bytes()); // bumpmap scale
        v.extend_from_slice(&format.to_le_bytes()); // 52
        v.push(mips); // 56
        v.extend_from_slice(&u32::MAX.to_le_bytes()); // 57: no thumbnail
        v.push(0); // 61
        v.push(0); // 62
        v.extend_from_slice(&1u16.to_le_bytes()); // 63: depth
        v.extend_from_slice(&[0; 3]); // 65
        v.extend_from_slice(&(if minor >= 3 { 1u32 } else { 0 }).to_le_bytes()); // 68: resources
        v.extend_from_slice(&[0; 8]); // 72
        assert_eq!(v.len(), 80);
        if minor >= 3 {
            v.extend_from_slice(&[0x30, 0, 0, 0]);
            v.extend_from_slice(&88u32.to_le_bytes());
        }
        v
    }

    /// A DXT1 block of one solid 565 colour (c0 = c1: index 0 = c0 everywhere).
    fn solid_bc1(c: u16) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&c.to_le_bytes());
        b[2..4].copy_from_slice(&c.to_le_bytes());
        b
    }

    #[test]
    fn dxt1_levels_in_file_order() {
        // 8x8, 4 levels stored smallest first: 1x1 blue, 2x2 green, 4x4 white, 8x8 red.
        for minor in [2, 3] {
            let mut f = header(minor, 8, 8, FMT_DXT1, 4, 1);
            f.extend_from_slice(&solid_bc1(0x001F)); // 1x1
            f.extend_from_slice(&solid_bc1(0x07E0)); // 2x2
            f.extend_from_slice(&solid_bc1(0xFFFF)); // 4x4
            for _ in 0..4 {
                f.extend_from_slice(&solid_bc1(0xF800)); // 8x8: 4 blocks
            }
            let img = decode(&f, 1024).unwrap();
            assert_eq!((img.width, img.height, img.mip), (8, 8, 0));
            assert!(img.rgba.chunks(4).all(|p| p == [255, 0, 0, 255]), "7.{minor}: top level red");
            let small = decode(&f, 4).unwrap();
            assert_eq!((small.width, small.mip), (4, 1));
            assert!(small.rgba.chunks(4).all(|p| p == [255, 255, 255, 255]), "level 1 white");
            assert_eq!(decode(&f, 1).unwrap().rgba, vec![0, 0, 255, 255], "level 3 blue");
        }
    }

    #[test]
    fn dxt1_transparent_and_dxt5_alpha() {
        // DXT1 with c0 <= c1: index 3 is transparent black.
        let mut f = header(2, 4, 4, FMT_DXT1, 1, 1);
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&0x0000u16.to_le_bytes());
        b[2..4].copy_from_slice(&0xFFFFu16.to_le_bytes());
        b[4..8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // every index 3
        f.extend_from_slice(&b);
        assert!(decode(&f, 1024).unwrap().rgba.chunks(4).all(|p| p == [0, 0, 0, 0]));

        // DXT5: alpha 255 / 0 with every index 1 (= 0), colour block solid green.
        let mut f = header(2, 4, 4, FMT_DXT5, 1, 1);
        let mut blk = [0u8; 16];
        blk[0] = 255;
        blk[1] = 0;
        let bits: u64 = (0..16).fold(0, |acc, i| acc | 1 << (3 * i));
        blk[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
        blk[8..16].copy_from_slice(&solid_bc1(0x07E0));
        f.extend_from_slice(&blk);
        assert!(decode(&f, 1024).unwrap().rgba.chunks(4).all(|p| p == [0, 255, 0, 0]));
        // ... and index 0 (= 255) on pixel 0 only.
        blk[2..8].copy_from_slice(&(bits & !7).to_le_bytes()[..6]);
        let mut f2 = header(2, 4, 4, FMT_DXT5, 1, 1);
        f2.extend_from_slice(&blk);
        let px = decode(&f2, 1024).unwrap().rgba;
        assert_eq!(&px[0..4], &[0, 255, 0, 255]);
        assert_eq!(&px[4..8], &[0, 255, 0, 0]);
    }

    #[test]
    fn plain_formats_and_mip_choice() {
        // 2048x2 BGR888, 2 levels, 2 frames: too wide, so level 1 (1024x1) is decoded; frame 0.
        let mut f = header(2, 2048, 2, FMT_BGR888, 2, 2);
        for _frame in 0..2 {
            f.extend(std::iter::repeat([30u8, 20, 10]).take(1024).flatten()); // level 1
        }
        for _frame in 0..2 {
            f.extend(std::iter::repeat([1u8, 2, 3]).take(4096).flatten()); // level 0
        }
        let img = decode(&f, 1024).unwrap();
        assert_eq!((img.width, img.height, img.mip, img.full_width), (1024, 1, 1, 2048));
        assert!(img.rgba.chunks(4).all(|p| p == [10, 20, 30, 255]));
        let big = decode(&f, 4096).unwrap();
        assert_eq!((big.width, big.height), (2048, 2));
        assert!(big.rgba.chunks(4).all(|p| p == [3, 2, 1, 255]));

        let mut f = header(2, 1, 1, FMT_BGRA8888, 1, 1);
        f.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(decode(&f, 1024).unwrap().rgba, vec![3, 2, 1, 4]);
        let mut f = header(2, 1, 1, FMT_IA88, 1, 1);
        f.extend_from_slice(&[9, 7]);
        assert_eq!(decode(&f, 1024).unwrap().rgba, vec![9, 9, 9, 7]);
    }

    #[test]
    fn broken_files_are_refused() {
        assert!(decode(b"PNG", 1024).is_err());
        let f = header(2, 8, 8, FMT_DXT1, 1, 1); // no data
        assert!(decode(&f, 1024).unwrap_err().contains("cut short"));
        let mut f = header(2, 4, 4, 24, 1, 1); // RGBA16161616F: sized but not decoded
        f.extend_from_slice(&[0; 128]);
        assert!(decode(&f, 1024).unwrap_err().contains("not decoded"));
    }
}
