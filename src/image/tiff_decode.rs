/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! A small baseline TIFF decoder (iPhone OS supports TIFF in `UIImage`).
//!
//! Supports little- and big-endian files with one image, 8 bits per sample,
//! chunky (contiguous) samples, grayscale / palette / RGB(A), strips that are
//! uncompressed, LZW (TIFF flavour, "early change") or PackBits compressed, and
//! the horizontal-differencing predictor. That covers what apps bundle in
//! practice (e.g. Photoshop / `tiffutil` output).

struct Reader<'a> {
    data: &'a [u8],
    big_endian: bool,
}

impl Reader<'_> {
    fn u16(&self, at: usize) -> Result<u16, String> {
        let b = self.data.get(at..at + 2).ok_or("TIFF: truncated file")?;
        Ok(if self.big_endian {
            u16::from_be_bytes([b[0], b[1]])
        } else {
            u16::from_le_bytes([b[0], b[1]])
        })
    }
    fn u32(&self, at: usize) -> Result<u32, String> {
        let b = self.data.get(at..at + 4).ok_or("TIFF: truncated file")?;
        Ok(if self.big_endian {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        })
    }
}

/// One IFD entry's values, widened to `u32`.
fn entry_values(r: &Reader, entry_at: usize) -> Result<Vec<u32>, String> {
    let field_type = r.u16(entry_at + 2)?;
    let count = r.u32(entry_at + 4)? as usize;
    let size = match field_type {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 => 4,
        _ => return Err(format!("TIFF: unsupported field type {}", field_type)),
    };
    let total = size * count;
    let base = if total <= 4 {
        entry_at + 8
    } else {
        r.u32(entry_at + 8)? as usize
    };
    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let at = base + i * size;
        values.push(match size {
            1 => *r.data.get(at).ok_or("TIFF: truncated file")? as u32,
            2 => r.u16(at)? as u32,
            _ => r.u32(at)?,
        });
    }
    Ok(values)
}

fn lzw_decode(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    fn fresh_table() -> Vec<Vec<u8>> {
        let mut table: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
        table.push(Vec::new()); // 256: clear code
        table.push(Vec::new()); // 257: end of information
        table
    }

    let mut out = Vec::with_capacity(expected);
    let mut table = fresh_table();
    let mut code_size = 9u32;
    let mut previous: Option<Vec<u8>> = None;
    let mut bit_pos = 0usize;
    let total_bits = data.len() * 8;

    while bit_pos + code_size as usize <= total_bits {
        // Codes are packed most-significant-bit first.
        let mut code = 0u32;
        for _ in 0..code_size {
            let byte = data[bit_pos / 8];
            let bit = (byte >> (7 - (bit_pos % 8))) & 1;
            code = (code << 1) | bit as u32;
            bit_pos += 1;
        }

        if code == 257 {
            break;
        }
        if code == 256 {
            table = fresh_table();
            code_size = 9;
            previous = None;
            continue;
        }

        let entry = if (code as usize) < table.len() {
            table[code as usize].clone()
        } else if code as usize == table.len() {
            let Some(ref prev) = previous else {
                return Err("TIFF: corrupt LZW data".to_string());
            };
            let mut e = prev.clone();
            e.push(prev[0]);
            e
        } else {
            return Err("TIFF: corrupt LZW data".to_string());
        };
        out.extend_from_slice(&entry);
        if let Some(prev) = previous {
            let mut new_entry = prev;
            new_entry.push(entry[0]);
            if table.len() < 4096 {
                table.push(new_entry);
            }
        }
        previous = Some(entry);

        // "Early change": widen the codes one entry before it is strictly needed.
        code_size = match table.len() {
            0..=510 => 9,
            511..=1022 => 10,
            1023..=2046 => 11,
            _ => 12,
        };

        if out.len() >= expected {
            break;
        }
    }
    Ok(out)
}

fn packbits_decode(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < data.len() && out.len() < expected {
        let n = data[i] as i8;
        i += 1;
        if n >= 0 {
            let len = n as usize + 1;
            let chunk = data.get(i..i + len).ok_or("TIFF: corrupt PackBits data")?;
            out.extend_from_slice(chunk);
            i += len;
        } else if n != -128 {
            let len = (1 - n as i32) as usize;
            let byte = *data.get(i).ok_or("TIFF: corrupt PackBits data")?;
            out.extend(std::iter::repeat(byte).take(len));
            i += 1;
        }
    }
    Ok(out)
}

pub fn is_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || bytes.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
}

/// Decode a TIFF to `(width, height, RGBA8 pixels with premultiplied alpha)`.
pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if !is_tiff(bytes) {
        return Err("TIFF: bad header".to_string());
    }
    let r = Reader {
        data: bytes,
        big_endian: bytes[0] == 0x4D,
    };
    let ifd = r.u32(4)? as usize;
    let entry_count = r.u16(ifd)? as usize;

    let mut width = 0u32;
    let mut height = 0u32;
    let mut bits: Vec<u32> = vec![1];
    let mut compression = 1u32;
    let mut photometric = 1u32;
    let mut strip_offsets: Vec<u32> = Vec::new();
    let mut samples_per_pixel = 1u32;
    let mut rows_per_strip = u32::MAX;
    let mut strip_byte_counts: Vec<u32> = Vec::new();
    let mut planar = 1u32;
    let mut predictor = 1u32;
    let mut color_map: Vec<u32> = Vec::new();
    let mut extra_samples: Vec<u32> = Vec::new();

    for i in 0..entry_count {
        let at = ifd + 2 + i * 12;
        let tag = r.u16(at)?;
        match tag {
            256 => width = *entry_values(&r, at)?.first().ok_or("TIFF: bad width")?,
            257 => height = *entry_values(&r, at)?.first().ok_or("TIFF: bad height")?,
            258 => bits = entry_values(&r, at)?,
            259 => compression = *entry_values(&r, at)?.first().unwrap_or(&1),
            262 => photometric = *entry_values(&r, at)?.first().unwrap_or(&1),
            273 => strip_offsets = entry_values(&r, at)?,
            277 => samples_per_pixel = *entry_values(&r, at)?.first().unwrap_or(&1),
            278 => rows_per_strip = *entry_values(&r, at)?.first().unwrap_or(&u32::MAX),
            279 => strip_byte_counts = entry_values(&r, at)?,
            284 => planar = *entry_values(&r, at)?.first().unwrap_or(&1),
            317 => predictor = *entry_values(&r, at)?.first().unwrap_or(&1),
            320 => color_map = entry_values(&r, at)?,
            338 => extra_samples = entry_values(&r, at)?,
            _ => {}
        }
    }

    if width == 0 || height == 0 {
        return Err("TIFF: missing dimensions".to_string());
    }
    if bits.iter().any(|&b| b != 8) {
        return Err("TIFF: only 8 bits per sample is supported".to_string());
    }
    if planar != 1 {
        return Err("TIFF: planar sample layout is not supported".to_string());
    }
    if strip_offsets.is_empty() || strip_offsets.len() != strip_byte_counts.len() {
        return Err("TIFF: missing or tiled strip data".to_string());
    }
    let spp = samples_per_pixel as usize;
    if !(1..=4).contains(&spp) {
        return Err("TIFF: unsupported number of samples per pixel".to_string());
    }
    let (w, h) = (width as usize, height as usize);
    let row_bytes = w * spp;
    let rows_per_strip = (rows_per_strip as usize).min(h).max(1);

    let mut raw = Vec::with_capacity(row_bytes * h);
    for (strip, (&offset, &count)) in strip_offsets.iter().zip(&strip_byte_counts).enumerate() {
        let rows = rows_per_strip.min(h.saturating_sub(strip * rows_per_strip));
        let expected = rows * row_bytes;
        let strip_data = bytes
            .get(offset as usize..offset as usize + count as usize)
            .ok_or("TIFF: strip is outside the file")?;
        let mut decoded = match compression {
            1 => strip_data.to_vec(),
            5 => lzw_decode(strip_data, expected)?,
            32773 => packbits_decode(strip_data, expected)?,
            other => return Err(format!("TIFF: unsupported compression {}", other)),
        };
        decoded.resize(expected, 0);
        if predictor == 2 {
            for row in decoded.chunks_exact_mut(row_bytes) {
                for i in spp..row_bytes {
                    row[i] = row[i].wrapping_add(row[i - spp]);
                }
            }
        }
        raw.extend_from_slice(&decoded);
    }
    raw.resize(row_bytes * h, 0);

    // Expand to RGBA8 with premultiplied alpha.
    let associated_alpha = extra_samples.first() == Some(&1);
    let mut out = Vec::with_capacity(w * h * 4);
    for px in raw.chunks_exact(spp) {
        let (mut rr, mut gg, mut bb, mut aa) = match (photometric, spp) {
            (3, _) => {
                // Palette: the colour map is 3 tables of 256 16-bit entries.
                let index = px[0] as usize;
                let get = |plane: usize| -> u8 {
                    (color_map.get(plane * 256 + index).copied().unwrap_or(0) >> 8) as u8
                };
                (get(0), get(1), get(2), 255)
            }
            (_, 1) | (_, 2) => {
                let mut g = px[0];
                if photometric == 0 {
                    g = 255 - g;
                }
                (g, g, g, if spp == 2 { px[1] } else { 255 })
            }
            (_, 3) => (px[0], px[1], px[2], 255),
            _ => (px[0], px[1], px[2], px[3]),
        };
        if !associated_alpha && aa != 255 {
            let a = aa as u32;
            rr = ((rr as u32 * a + 127) / 255) as u8;
            gg = ((gg as u32 * a + 127) / 255) as u8;
            bb = ((bb as u32 * a + 127) / 255) as u8;
        }
        if aa == 0 {
            aa = 0;
        }
        out.extend_from_slice(&[rr, gg, bb, aa]);
    }
    Ok((width, height, out))
}
