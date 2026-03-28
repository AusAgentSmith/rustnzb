use crc32fast::Hasher;

const LINE_WIDTH: usize = 128;

/// yEnc encode a raw data block. Returns (encoded_body_with_headers, crc32).
pub fn encode_article(
    raw: &[u8],
    filename: &str,
    part: u32,
    total_parts: u32,
    file_offset: u64,
    total_file_size: u64,
) -> (Vec<u8>, u32) {
    let mut hasher = Hasher::new();
    hasher.update(raw);
    let crc = hasher.finalize();

    let mut out = Vec::with_capacity(raw.len() * 11 / 10 + 256);

    // =ybegin header
    if total_parts > 1 {
        out.extend_from_slice(
            format!(
                "=ybegin part={part} line={LINE_WIDTH} size={total_file_size} name={filename}\r\n"
            )
            .as_bytes(),
        );
        let begin = file_offset + 1;
        let end = file_offset + raw.len() as u64;
        out.extend_from_slice(format!("=ypart begin={begin} end={end}\r\n").as_bytes());
    } else {
        out.extend_from_slice(
            format!("=ybegin line={LINE_WIDTH} size={total_file_size} name={filename}\r\n")
                .as_bytes(),
        );
    }

    // Encode body
    let mut line_pos: usize = 0;
    for &byte in raw {
        let encoded = byte.wrapping_add(42);

        // Escape critical bytes, plus TAB/SPACE/DOT at line start
        let escape = matches!(encoded, 0x00 | 0x0A | 0x0D | 0x3D)
            || (line_pos == 0 && matches!(encoded, 0x09 | 0x20 | 0x2E));

        if escape {
            out.push(b'=');
            out.push(encoded.wrapping_add(64));
            line_pos += 2;
        } else {
            out.push(encoded);
            line_pos += 1;
        }

        if line_pos >= LINE_WIDTH {
            out.extend_from_slice(b"\r\n");
            line_pos = 0;
        }
    }
    if line_pos > 0 {
        out.extend_from_slice(b"\r\n");
    }

    // =yend footer
    if total_parts > 1 {
        out.extend_from_slice(
            format!("=yend size={} pcrc32={crc:08X}\r\n", raw.len()).as_bytes(),
        );
    } else {
        out.extend_from_slice(
            format!("=yend size={} crc32={crc:08X}\r\n", raw.len()).as_bytes(),
        );
    }

    (out, crc)
}
