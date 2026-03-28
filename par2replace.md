# PAR2 Rewrite Assessment

## Current State

The codebase shells out to an external `par2` binary — there is no third-party Rust PAR2 library. The integration in `crates/nzb-postproc/src/par2.rs` (237 lines) runs `par2 verify` / `par2 repair` via `tokio::process::Command` and parses stdout with regex. This is exactly what SABnzbd, NZBGet, and every other Usenet client does.

## Why There's No Rust PAR2 Crate

Zero exist on crates.io. The reason is the critical blocker:

**GF(2^16) polynomial incompatibility** — PAR2 mandates polynomial `0x1100B`. The two mature Rust Reed-Solomon crates (`reed-solomon-simd` uses `0x1002D`, `reed-solomon-erasure` uses a different field construction) are not compatible. Data encoded with one polynomial cannot be decoded with another. You'd have to implement GF(2^16) arithmetic from scratch.

## What a Pure Rust Implementation Requires

| Component | Effort | Complexity |
|---|---|---|
| GF(2^16) arithmetic (polynomial 0x1100B) | 1-2 weeks | Log/antilog tables, multiply/divide/power |
| Reed-Solomon codec | 1-2 weeks | Vandermonde matrix, Gaussian elimination over GF(2^16) |
| PAR2 file format parser | 1-2 weeks | 7+ packet types, 56-byte headers, MD5 validation |
| Verify workflow | 1-2 weeks | Sliding-window CRC32 scanner, block matching, damage assessment |
| Repair workflow | 1-2 weeks | Matrix inversion, block reconstruction, file reassembly |
| SIMD optimization (optional but 5-10x perf) | 1-2 weeks | PSHUFB-based GF multiply, SIMD MD5/CRC32 |

**Total: ~4,000-7,000 lines of Rust, 6-10 weeks.** The reference C++ implementation (`par2cmdline`) is 64 source files / ~10K lines.

## Three Options

| Option | Effort | Benefit |
|---|---|---|
| **A. Pure Rust rewrite** | 6-10 weeks | No external dependency, full control, ecosystem contribution (first Rust PAR2 crate) |
| **B. Keep shelling out** (current) | 0 | Already works, battle-tested binary |
| **C. FFI to libpar2 (C++)** | 2-3 weeks | Eliminates stdout parsing, gets C++ performance, but adds build complexity |

## Performance Assessment

### Why the current binary is already near-optimal

`par2cmdline-turbo` (the modern fork most distros ship) uses:

- Hand-tuned SIMD for GF(2^16) multiplication — SSE2, SSSE3, AVX2, AVX512, NEON, SVE2 via the ParPar backend
- SIMD-accelerated MD5 and CRC32 (multi-buffer)
- 20+ years of profiling and optimization by the parchive community

The bottleneck in PAR2 is GF(2^16) matrix multiplication over potentially gigabytes of data. This is pure number crunching — the kind of workload where hand-written SIMD intrinsics in C/C++ and hand-written SIMD intrinsics in Rust compile to identical machine instructions. There's no overhead from the shell-out (it's called once per job, runs for seconds to minutes).

### Where Rust could theoretically help

- **Eliminating the process spawn** — saves ~5ms on a job that takes seconds. Negligible.
- **Async I/O integration** — reading PAR2 packets while downloading. Marginal gain, high complexity.
- **Memory safety** — real benefit, but not a performance benefit.

### Where Rust would likely be slower initially

A naive Rust implementation using log-table GF multiplication (no SIMD) would be 5-10x slower than par2cmdline-turbo. Matching its performance means porting the same SIMD intrinsics — at which point you're writing the same code in a different language for zero net gain.

### Bottom line

The PAR2 compute kernel is the same math regardless of language. The C++ implementations have had two decades of SIMD optimization. A Rust rewrite would be a large effort to arrive at parity (pun intended), not superiority.

If the goal is removing the external binary dependency, FFI to libpar2 (2-3 weeks) gives you that without the performance risk. If it's about Rust ecosystem contribution, it's a worthy project — but it's motivated by portability/safety, not speed.

## Test Harness Approach (if proceeding)

### 1. Golden test corpus

Create a `tests/fixtures/par2/` directory with real PAR2 sets:

- Intact files (verify -> AllCorrect)
- Files with 1 damaged block (verify -> RepairPossible, repair -> success)
- Files missing recovery blocks (verify -> RepairNotPossible)
- Renamed files (tests sliding-window detection)
- Multiple file sets, different block sizes

### 2. Dual-execution harness

For each test case, run both:

- `par2cmdline verify/repair` (the reference)
- The new Rust implementation

Then assert identical results: same status, same blocks_needed/available counts, same repaired file checksums (byte-for-byte).

### 3. Low-level unit tests

- GF(2^16) multiply/divide against known values from the spec
- RS encode -> decode round-trip
- Packet parsing against hex dumps from real PAR2 files
- CRC32 rolling window against par2cmdline's output

## References

- [PAR 2.0 Specification](https://parchive.github.io/doc/Parity%20Volume%20Set%20Specification%20v2.0.html)
- [par2cmdline (official C++ reference)](https://github.com/Parchive/par2cmdline)
- [par2cmdline-turbo (SIMD-optimized fork)](https://github.com/animetosho/par2cmdline-turbo)
- [ParPar (high-performance PAR2 create)](https://github.com/animetosho/ParPar)
- [reed-solomon-simd crate (GF polynomial 0x1002D, incompatible)](https://github.com/AndersTrier/reed-solomon-simd)
- [reed-solomon-erasure crate](https://github.com/rust-rse/reed-solomon-erasure)
