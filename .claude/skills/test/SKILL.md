---
name: test
description: Run the rustnzbd Rust test suite
disable-model-invocation: true
allowed-tools: Bash(cargo *)
user-invocable: true
argument-hint: "[test-name-or-module] [-- --nocapture]"
---

# Run Tests

Run the rustnzbd Rust test suite.

## Usage

- `/test` — Run all tests
- `/test decode` — Run tests matching "decode"
- `/test -- --nocapture` — Run all tests with output visible
- `/test -p nzb-decode` — Run tests in a specific crate
- `/test --test e2e_download_test` — Run a specific integration test

## Crate test targets

| Crate | Key test areas |
|-------|---------------|
| nzb-core | Config parsing, NZB XML parsing, database schema |
| nzb-decode | yEnc decoding, CRC32 verification, file assembly |
| nzb-nntp | NNTP protocol, connection handling |
| nzb-postproc | par2 verification, archive detection |
| nzb-web | API handlers, queue management |
| par2-sys | Embedded binary extraction, execution |

## Integration tests (tests/)

| File | Purpose |
|------|---------|
| e2e_download_test | Full download pipeline |
| nntp_connection_test | NNTP protocol testing |
| e2e_postproc_detection | Post-processing detection |
| e2e_full_pipeline | End-to-end workflow |
| parse_real_nzbs | NZB XML parsing with real files |

## Steps

1. If no arguments, run all tests:
   ```bash
   cargo test --workspace
   ```

2. With arguments, pass directly:
   ```bash
   cargo test $ARGUMENTS
   ```

3. If tests fail:
   - Read the failure output
   - Identify the failing test and source file
   - Fix the code
   - Re-run the specific failing test to confirm
   - Run full suite to check for regressions
