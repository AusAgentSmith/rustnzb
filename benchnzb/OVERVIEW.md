  benchnzb/
  ├── Cargo.toml
  ├── Dockerfile          # Rust build + par2/p7zip runtime
  ├── docker-compose.yml  # mock-nntp, sabnzbd, rustnzb, orchestrator
  ├── run.sh              # Entry point
  ├── .gitignore
  ├── configs/
  │   ├── sabnzbd.ini     # Pre-seeded SABnzbd config (known API key, mock NNTP)
  │   └── rustnzb.toml   # Pre-seeded rustnzb config (mock NNTP)
  └── src/
      ├── main.rs         # CLI: run | mock-nntp | regen-charts
      ├── config.rs       # 9 scenarios: 5/10/50 GB x raw/par2/unpack
      ├── yenc.rs         # yEnc encoder (correct CRC32, escaping, line wrapping)
      ├── nzb.rs          # NZB XML generator
      ├── mock_nntp.rs    # Mock NNTP server (on-the-fly yEnc from data files)
      ├── datagen.rs      # Generates test data, par2, 7z archives, NZBs, article index
      ├── runner.rs       # Orchestrator (sequential client runs, metrics collection)
      ├── metrics.rs      # Docker stats collection (CPU/mem/net/disk timeseries)
      ├── docker.rs       # Docker API helpers
      ├── report.rs       # JSON/CSV/summary output
      ├── charts.rs       # SVG chart generation
      └── clients/
          ├── mod.rs
          ├── sabnzbd.rs  # SABnzbd API client (queue/history/stage timing)
          └── rustnzb.rs # rustnzb API client (queue/history/stage timing)

  How it works

  1. run.sh --scenarios quick seeds configs, launches Docker Compose
  2. mock-nntp starts serving articles on port 119 (reads data files, yEnc-encodes on-the-fly)
  3. orchestrator generates random test data + par2/7z + NZB files, reloads mock-nntp index
  4. For each scenario, runs SABnzbd then rustnzb sequentially, collecting Docker stats
  5. Outputs JSON, CSV, summary text, and SVG charts to results/

  Scenarios

  ┌───────────────┬───────┬─────────────────────────────────────┐
  │     Name      │ Size  │            What it tests            │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz5gb_raw     │ 5 GB  │ Pure NNTP download speed            │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz10gb_raw    │ 10 GB │ Pure NNTP download speed            │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz50gb_raw    │ 50 GB │ Pure NNTP download speed            │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz5gb_par2    │ 5 GB  │ Download + par2 repair (5% missing) │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz10gb_par2   │ 10 GB │ Download + par2 repair (5% missing) │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz50gb_par2   │ 50 GB │ Download + par2 repair (5% missing) │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz5gb_unpack  │ 5 GB  │ Download + 7z extraction            │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz10gb_unpack │ 10 GB │ Download + 7z extraction            │
  ├───────────────┼───────┼─────────────────────────────────────┤
  │ sz50gb_unpack │ 50 GB │ Download + 7z extraction            │
  └───────────────┴───────┴─────────────────────────────────────┘