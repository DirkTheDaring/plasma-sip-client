# Testing Strategy & Quality Assurance — Plasma SIP

## Principles
1. **No Fake Results:** Every test recorded in `docs/TEST_RESULTS.md` must be executed.
2. **Pure Rust Testing:** Unit and protocol integration tests run in memory without external C library dependencies.
3. **Deterministic Timers:** Network and transaction timers support simulated clocks for repeatable testing.
4. **Synthetic Test Fixtures:** SIP registrars and endpoints are provided for deterministic automated test execution.
5. **Hardware Tests vs Automated CI:** Physical FRITZ!Box tests are recorded separately and not faked if hardware is absent.

## Mandatory Quality Gates
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
```
