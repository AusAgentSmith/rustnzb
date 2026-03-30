//! PAR2 binary packet parser.
//!
//! Parses PAR2 files according to the PAR 2.0 specification:
//! <http://parchive.sourceforge.net/docs/specifications/parity-volume-spec/article-spec.html>
//!
//! Packet layout (all multi-byte fields are little-endian):
//! ```text
//! Offset  Size  Description
//!   0       8   Magic: "PAR2\x00PKT"
//!   8       8   Packet length (u64, includes header, must be multiple of 4)
//!  16      16   MD5 hash of bytes 32..packet_end
//!  32      16   Recovery Set ID
//!  48      16   Packet Type
//!  64       ?   Body (packet-type specific)
//! ```

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use md5::{Digest, Md5};
use tracing::{debug, trace, warn};

use crate::types::{Id16, Md5Hash, Par2File, Par2FileSet, SliceChecksum};

/// PAR2 packet magic bytes.
const PAR2_MAGIC: &[u8; 8] = b"PAR2\x00PKT";

/// Public constants for use by the recovery module.
pub const MAGIC: &[u8; 8] = PAR2_MAGIC;
pub const HEADER_SIZE: usize = 64;

/// Minimum packet length (header only, no body).
const MIN_PACKET_LEN: u64 = 64;

// Packet type identifiers (16 bytes each).
const TYPE_MAIN: &[u8; 16] = b"PAR 2.0\x00Main\x00\x00\x00\x00";
const TYPE_FILE_DESC: &[u8; 16] = b"PAR 2.0\x00FileDesc";
const TYPE_IFSC: &[u8; 16] = b"PAR 2.0\x00IFSC\x00\x00\x00\x00";
const TYPE_RECOVERY: &[u8; 16] = b"PAR 2.0\x00RecvSlic";
const TYPE_CREATOR: &[u8; 16] = b"PAR 2.0\x00Creator\x00";

/// Errors that can occur while parsing PAR2 files.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("no PAR2 packets found in file")]
    NoPar2Packets,
    #[error("missing Main packet — cannot determine slice size")]
    NoMainPacket,
}

/// Intermediate storage during parsing (packets can arrive in any order).
struct ParseState {
    recovery_set_id: Option<Id16>,
    slice_size: Option<u64>,
    nr_files: Option<u32>,
    /// FileDesc data keyed by File ID.
    file_descs: HashMap<Id16, FileDescData>,
    /// IFSC (slice checksum) data keyed by File ID.
    ifsc_data: HashMap<Id16, Vec<SliceChecksum>>,
    /// Recovery slice count.
    recovery_count: u32,
    /// Creator string.
    creator: Option<String>,
}

struct FileDescData {
    hash: Md5Hash,
    hash_16k: Md5Hash,
    size: u64,
    filename: String,
}

/// Parse a PAR2 file and return the complete file set metadata.
///
/// This reads the entire PAR2 file (typically the index `.par2` file, not the
/// large `.volNNN+NNN.par2` recovery volumes). For recovery volumes, only the
/// header packets are read — the large recovery data is skipped.
pub fn parse_par2_file(path: &Path) -> Result<Par2FileSet, ParseError> {
    let file = std::fs::File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut reader = io::BufReader::new(file);

    parse_par2_reader(&mut reader, file_size)
}

/// Parse PAR2 packets from any `Read + Seek` source.
pub fn parse_par2_reader<R: Read + Seek>(
    reader: &mut R,
    file_size: u64,
) -> Result<Par2FileSet, ParseError> {
    let mut state = ParseState {
        recovery_set_id: None,
        slice_size: None,
        nr_files: None,
        file_descs: HashMap::new(),
        ifsc_data: HashMap::new(),
        recovery_count: 0,
        creator: None,
    };

    let mut magic_buf = [0u8; 8];
    let mut packets_parsed = 0u32;

    loop {
        let pos = reader.stream_position()?;
        if pos >= file_size {
            break;
        }

        // Read magic
        if reader.read_exact(&mut magic_buf).is_err() {
            break;
        }

        if magic_buf != *PAR2_MAGIC {
            // Not at a packet boundary — try to find the next one.
            // This handles trailing garbage or alignment issues.
            if let Some(next_pos) = scan_for_magic(reader, file_size)? {
                reader.seek(SeekFrom::Start(next_pos))?;
                continue;
            }
            break;
        }

        // Read packet length
        let mut len_buf = [0u8; 8];
        if reader.read_exact(&mut len_buf).is_err() {
            break;
        }
        let packet_len = u64::from_le_bytes(len_buf);

        // Validate length
        if packet_len < MIN_PACKET_LEN || packet_len % 4 != 0 {
            warn!(packet_len, pos, "invalid PAR2 packet length, skipping");
            continue;
        }

        // Don't read absurdly large packets into memory (recovery slices
        // can be many megabytes). We only need the type to count them.
        let body_len = packet_len - 16; // everything after magic + length + md5
        if body_len > 10 * 1024 * 1024 {
            // Large packet — likely a recovery slice. Read just the type.
            let mut md5_buf = [0u8; 16];
            reader.read_exact(&mut md5_buf)?;

            let mut type_header = [0u8; 32]; // recovery_set_id + type
            reader.read_exact(&mut type_header)?;
            let packet_type = &type_header[16..32];

            if packet_type == TYPE_RECOVERY {
                state.recovery_count += 1;
                if state.recovery_set_id.is_none() {
                    let mut id = [0u8; 16];
                    id.copy_from_slice(&type_header[..16]);
                    state.recovery_set_id = Some(id);
                }
            }

            // Skip the rest
            let remaining = packet_len - 64;
            reader.seek(SeekFrom::Current(remaining as i64))?;
            packets_parsed += 1;
            continue;
        }

        // Read MD5 hash of packet body
        let mut stored_md5 = [0u8; 16];
        reader.read_exact(&mut stored_md5)?;

        // Read the rest of the packet (recovery_set_id + type + body)
        let data_len = (packet_len - 32) as usize;
        let mut data = vec![0u8; data_len];
        if reader.read_exact(&mut data).is_err() {
            break;
        }

        // Verify packet MD5
        let computed_md5: [u8; 16] = Md5::digest(&data).into();
        if computed_md5 != stored_md5 {
            warn!(pos, "PAR2 packet MD5 mismatch, skipping");
            continue;
        }

        // Extract recovery set ID and packet type
        let mut set_id = [0u8; 16];
        set_id.copy_from_slice(&data[..16]);
        if state.recovery_set_id.is_none() {
            state.recovery_set_id = Some(set_id);
        }

        let packet_type = &data[16..32];

        // Dispatch by type
        if packet_type == TYPE_FILE_DESC {
            parse_file_desc(&data, &mut state);
        } else if packet_type == TYPE_IFSC {
            parse_ifsc(&data, packet_len, &mut state);
        } else if packet_type == TYPE_MAIN {
            parse_main(&data, &mut state);
        } else if packet_type == TYPE_RECOVERY {
            state.recovery_count += 1;
        } else if packet_type == TYPE_CREATOR {
            parse_creator(&data, &mut state);
        }

        packets_parsed += 1;

        // Early exit optimisation: once we have all file descs and IFSCs, we
        // can stop (avoids reading huge recovery volumes in concatenated files).
        if let Some(nr) = state.nr_files {
            if state.file_descs.len() == nr as usize
                && state.ifsc_data.len() == nr as usize
                && state.slice_size.is_some()
            {
                // If the file is large, stop early like SABnzbd does.
                if file_size > 10 * 1024 * 1024 {
                    debug!(
                        packets_parsed,
                        "parsed all file metadata, stopping early on large file"
                    );
                    break;
                }
            }
        }
    }

    if packets_parsed == 0 {
        return Err(ParseError::NoPar2Packets);
    }

    let slice_size = state.slice_size.ok_or(ParseError::NoMainPacket)?;
    let recovery_set_id = state.recovery_set_id.unwrap_or([0u8; 16]);

    // Assemble Par2File entries by joining FileDesc + IFSC data on File ID
    let mut files = HashMap::new();
    for (file_id, desc) in state.file_descs {
        let slices = state.ifsc_data.remove(&file_id).unwrap_or_default();
        files.insert(
            file_id,
            Par2File {
                file_id,
                hash: desc.hash,
                hash_16k: desc.hash_16k,
                size: desc.size,
                filename: desc.filename,
                slices,
            },
        );
    }

    debug!(
        files = files.len(),
        recovery_blocks = state.recovery_count,
        slice_size,
        creator = state.creator.as_deref().unwrap_or("unknown"),
        "PAR2 file parsed"
    );

    Ok(Par2FileSet {
        recovery_set_id,
        slice_size,
        files,
        recovery_block_count: state.recovery_count,
        creator: state.creator,
    })
}

// ---------------------------------------------------------------------------
// Packet body parsers
// ---------------------------------------------------------------------------

/// Parse a FileDesc packet body.
///
/// Layout (offsets relative to `data`, which starts at recovery_set_id):
/// ```text
///  0..16   Recovery Set ID (already extracted)
/// 16..32   Packet Type (already matched)
/// 32..48   File ID
/// 48..64   Full-file MD5 hash
/// 64..80   First-16K MD5 hash
/// 80..88   File size (u64 LE)
/// 88..     Filename (null-terminated, padded to multiple of 4)
/// ```
fn parse_file_desc(data: &[u8], state: &mut ParseState) {
    if data.len() < 88 {
        warn!("FileDesc packet too short ({} bytes)", data.len());
        return;
    }

    let mut file_id = [0u8; 16];
    file_id.copy_from_slice(&data[32..48]);

    // Skip duplicates
    if state.file_descs.contains_key(&file_id) {
        return;
    }

    let mut hash = [0u8; 16];
    hash.copy_from_slice(&data[48..64]);

    let mut hash_16k = [0u8; 16];
    hash_16k.copy_from_slice(&data[64..80]);

    let size = u64::from_le_bytes(data[80..88].try_into().unwrap());

    // Filename: everything after offset 88, strip null padding
    let name_bytes = &data[88..];
    let name_end = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_bytes.len());
    let filename = String::from_utf8_lossy(&name_bytes[..name_end]).into_owned();

    trace!(filename, size, "parsed FileDesc");

    state.file_descs.insert(
        file_id,
        FileDescData {
            hash,
            hash_16k,
            size,
            filename,
        },
    );
}

/// Parse an IFSC (Input File Slice Checksum) packet.
///
/// Layout:
/// ```text
///  0..16   Recovery Set ID
/// 16..32   Packet Type
/// 32..48   File ID
/// 48..     Pairs of (MD5[16] + CRC32[4]) for each slice
/// ```
fn parse_ifsc(data: &[u8], packet_len: u64, state: &mut ParseState) {
    if data.len() < 48 {
        warn!("IFSC packet too short ({} bytes)", data.len());
        return;
    }

    let mut file_id = [0u8; 16];
    file_id.copy_from_slice(&data[32..48]);

    // Skip duplicates
    if state.ifsc_data.contains_key(&file_id) {
        return;
    }

    let body_len = (packet_len - 64) as usize; // body after 64-byte header
    let checksum_data = &data[48..];
    let num_slices = (body_len - 16) / 20; // subtract File ID, 20 bytes per slice

    let mut slices = Vec::with_capacity(num_slices);
    for i in 0..num_slices {
        let offset = i * 20;
        if offset + 20 > checksum_data.len() {
            break;
        }

        let mut md5 = [0u8; 16];
        md5.copy_from_slice(&checksum_data[offset..offset + 16]);
        let crc32 = u32::from_le_bytes(checksum_data[offset + 16..offset + 20].try_into().unwrap());

        slices.push(SliceChecksum { md5, crc32 });
    }

    trace!(slices = slices.len(), "parsed IFSC");

    state.ifsc_data.insert(file_id, slices);
}

/// Parse the Main packet.
///
/// Layout:
/// ```text
///  0..16   Recovery Set ID
/// 16..32   Packet Type
/// 32..40   Slice size (u64 LE)
/// 40..44   Number of files in recovery set (u32 LE)
/// 44..     File IDs (16 bytes each)
/// ```
fn parse_main(data: &[u8], state: &mut ParseState) {
    if data.len() < 44 {
        warn!("Main packet too short ({} bytes)", data.len());
        return;
    }

    let slice_size = u64::from_le_bytes(data[32..40].try_into().unwrap());
    let nr_files = u32::from_le_bytes(data[40..44].try_into().unwrap());

    trace!(slice_size, nr_files, "parsed Main");

    state.slice_size = Some(slice_size);
    state.nr_files = Some(nr_files);
}

/// Parse a Creator packet.
fn parse_creator(data: &[u8], state: &mut ParseState) {
    if data.len() <= 32 {
        return;
    }
    let creator_bytes = &data[32..];
    let end = creator_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(creator_bytes.len());
    let creator = String::from_utf8_lossy(&creator_bytes[..end]).into_owned();
    debug!(creator, "PAR2 creator");
    state.creator = Some(creator);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Scan forward to find the next PAR2_MAGIC occurrence.
fn scan_for_magic<R: Read + Seek>(reader: &mut R, file_size: u64) -> io::Result<Option<u64>> {
    let start = reader.stream_position()?;
    // Read in chunks to find the magic
    let mut buf = [0u8; 4096];
    let mut search_pos = start;

    while search_pos < file_size {
        reader.seek(SeekFrom::Start(search_pos))?;
        let n = reader.read(&mut buf)?;
        if n < 8 {
            return Ok(None);
        }
        for i in 0..n.saturating_sub(7) {
            if &buf[i..i + 8] == PAR2_MAGIC {
                return Ok(Some(search_pos + i as u64));
            }
        }
        // Overlap by 7 to catch magic spanning chunk boundaries
        search_pos += (n - 7) as u64;
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Test parsing the real PAR2 file from SABnzbd test data.
    #[test]
    fn test_parse_par2test() {
        let path = Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic/par2test.par2");
        if !path.exists() {
            eprintln!("Skipping test: {path:?} not found");
            return;
        }

        let set = parse_par2_file(path).unwrap();

        // Should have 6 files
        assert_eq!(set.files.len(), 6, "expected 6 files in par2 set");

        // Slice size should be 100000 (0x186A0)
        assert_eq!(set.slice_size, 100000, "expected slice_size = 100000");

        // Creator should be QuickPar 0.9
        assert_eq!(
            set.creator.as_deref(),
            Some("QuickPar 0.9"),
            "expected creator = QuickPar 0.9"
        );

        // No recovery blocks in the index file
        assert_eq!(set.recovery_block_count, 0);

        // Check that all expected filenames are present
        let filenames: Vec<&str> = set.files.values().map(|f| f.filename.as_str()).collect();
        for i in 1..=6 {
            let expected = format!("par2test.part{i}.rar");
            assert!(
                filenames.contains(&expected.as_str()),
                "missing file: {expected}"
            );
        }

        // Check file sizes
        for f in set.files.values() {
            if f.filename == "par2test.part6.rar" {
                // Last part is smaller
                assert!(f.size < 100000, "part6 should be smaller than slice_size");
            } else {
                assert_eq!(f.size, 102400, "{} should be 102400 bytes", f.filename);
            }
        }

        // Each file should have IFSC slice data
        for f in set.files.values() {
            assert!(
                !f.slices.is_empty(),
                "{} should have slice checksums",
                f.filename
            );
        }
    }

    /// Test parsing the basic_16k par2 file.
    #[test]
    fn test_parse_basic_16k() {
        let path = Path::new("/home/sprooty/sabnzbd/tests/data/par2file/basic_16k.par2");
        if !path.exists() {
            eprintln!("Skipping test: {path:?} not found");
            return;
        }

        let set = parse_par2_file(path).unwrap();
        assert!(!set.files.is_empty(), "should parse at least one file");
        assert!(set.slice_size > 0, "slice_size should be > 0");
    }

    /// Test that parsing a non-PAR2 file returns an error.
    #[test]
    fn test_parse_non_par2() {
        let path =
            Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic/par2test.part2.rar");
        if !path.exists() {
            eprintln!("Skipping test: {path:?} not found");
            return;
        }

        let result = parse_par2_file(path);
        assert!(result.is_err(), "parsing a RAR file should fail");
    }

    /// Test parsing a recovery volume (should count recovery blocks).
    #[test]
    fn test_parse_recovery_volume() {
        let path =
            Path::new("/home/sprooty/sabnzbd/tests/data/par2repair/basic/par2test.vol0+1.par2");
        if !path.exists() {
            eprintln!("Skipping test: {path:?} not found");
            return;
        }

        let set = parse_par2_file(path).unwrap();
        assert!(
            set.recovery_block_count >= 1,
            "recovery volume should have at least 1 recovery block"
        );
    }
}
