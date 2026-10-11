# Dependency audit

Audited on 2026-10-11 against the working-tree lockfile (including its existing uncommitted updates), using crates.io metadata and the freshly fetched RustSec advisory database.

## Findings and remediation

- **RUSTSEC-2026-0285:** `rustls` 0.23.43, pulled in by `ureq`, accepted TLS 1.3 handshake messages across encryption-level boundaries. Updated to patched `rustls` 0.23.45 and `ureq` 3.4.2. [Advisory](https://rustsec.org/advisories/RUSTSEC-2026-0285).
- **RUSTSEC-2021-0139, RUSTSEC-2024-0375, RUSTSEC-2021-0145:** unmaintained `ansi_term` and unmaintained/unsound `atty`, through `ksni` → `dbus-codegen` → `clap`. Migrated `ksni` 0.2.2 to 0.3.6's blocking API; these packages and the old libdbus build dependency are no longer in the graph.
- Removed the unused direct `thiserror` dependency. Maintained `thiserror` remains transitively required.
- Replaced `fs2` with `std::fs::File` locking; regression coverage verifies exclusive locking and unlocking.
- Updated paired GTK/libadwaita bindings to 0.11.5/0.9.2, `directories` to 6.0.0, `nix` to 0.31.3, `toml` to 1.1.8, and `which` to 8.0.6. Refreshed compatible transitive dependencies. `whisper-rs` 0.16.0 was already current.

## Compatibility and ongoing checks

- Rust **1.92+** is now required by the GTK bindings and declared in `Cargo.toml`.
- Native runtime minimums remain **GTK 4.12 / libadwaita 1.4**; newer native-library features were not enabled.
- Removed libdbus development packages from build instructions and workflows. A session D-Bus remains necessary for desktop tray integration; CI installs `dbus` for the isolated tray test.
- Security CI now uses `cargo audit --deny warnings`, without advisory exemptions.

## Validation

```sh
cargo check --locked --all-targets
cargo test --locked --all-targets
dbus-run-session -- cargo test --locked tray::tests::spawn_reports_missing_tray_support -- --ignored
cargo audit --deny warnings
cargo machete
cargo update --dry-run --verbose
cargo fmt --all -- --check
git diff --check
```

RustSec scan: **zero vulnerabilities or advisory warnings**. Unused-dependency scan: **none**. Dependency-update dry run: **no further updates** within declared requirements. Added regression tests for config serialization, file locking, and missing tray support.

This is a Rust dependency/advisory audit, not a guarantee of security. It does not audit system-library packages, bundled whisper.cpp C/C++ code, or downloaded model files. GUI appearance and successful tray registration still require desktop smoke testing. Checks were run with Rust 1.98.1; the declared minimum toolchain was not separately exercised. Clippy was unavailable in this environment.
