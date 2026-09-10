//! WI-151: the PAR2 fixture emits IFSC slice checksums that `rust_par2`
//! accepts. A correct file verifies intact through those per-slice checksums,
//! and a single corrupted slice is pinpointed by its block index — which only
//! works if the fixture's per-slice MD5/CRC32 are byte-exact.

mod support;

use std::fs;

use support::par2_fixture::Par2Fixture;

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn intact_file_verifies_through_slice_checksums() {
    let dir = tempfile::tempdir().unwrap();
    // 10_000 bytes at the fixture's 4096-byte slice size -> 3 slices, the last
    // one partial (zero-padded for hashing).
    let data = payload(10_000);
    let name = "movie.mkv";
    Par2Fixture::new()
        .add_file(name, &data)
        .write_index(&dir.path().join("recovery.par2"));
    fs::write(dir.path().join(name), &data).unwrap();

    let set = rust_par2::parse(&dir.path().join("recovery.par2")).unwrap();
    let file = set.files.values().next().unwrap();
    assert_eq!(file.slices.len(), 3, "three slices including one partial");

    let result = rust_par2::verify(&set, dir.path());
    assert!(result.all_correct(), "intact fixture must verify: {result}");
}

#[test]
fn corrupt_slice_is_located_by_its_block_index() {
    let dir = tempfile::tempdir().unwrap();
    let data = payload(10_000);
    let name = "movie.mkv";
    Par2Fixture::new()
        .add_file(name, &data)
        .write_index(&dir.path().join("recovery.par2"));

    // Flip a byte inside slice index 1 (bytes 4096..8192).
    let mut corrupt = data.clone();
    corrupt[5000] ^= 0xff;
    fs::write(dir.path().join(name), &corrupt).unwrap();

    let set = rust_par2::parse(&dir.path().join("recovery.par2")).unwrap();
    let result = rust_par2::verify(&set, dir.path());

    assert_eq!(result.damaged.len(), 1, "one damaged file: {result}");
    assert_eq!(
        result.damaged[0].damaged_block_indices,
        vec![1],
        "IFSC checksums must pinpoint the corrupted slice"
    );
}
