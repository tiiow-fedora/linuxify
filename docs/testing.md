# Testing: unit, property and fuzz tests

Linuxify reads a lot of text it doesn't control: `winget` tables, Steam and Mozilla profile files, Chromium
`Local State`, checksum files, JSON data files users can override, whatever a website quiz sends, bytes read back
from firmware, `bcdedit` output. Every one of those readers has three layers of tests.

| Layer | Where | Run with | Needs |
|---|---|---|---|
| Unit and integration tests (examples) | `src/**` `#[cfg(test)]`, `tests/*.rs` | `cargo test` | stable Rust |
| Property tests (`proptest`) | `tests/prop_parsers.rs` | `cargo test --test prop_parsers` | stable Rust |
| Coverage-guided fuzzing (`cargo-fuzz`) | `fuzz/` | `cargo +nightly fuzz run <target>` | nightly Rust, `cargo install cargo-fuzz` |

All three run on Linux and Windows. Nothing here touches disks or the network.

## What the property tests assert

Each parser gets "never panics on arbitrary text", and, where there is a contract, an invariant on whatever it accepts:

- `DistroList`, `IsoList`, `AppMap`: an accepted file satisfies the documented rules (unique ids, https homepages, trait range, checksum host listed in `allowed_hosts`, valid app map). Truncated and spliced copies of the shipped files also run.
- `parse_checksums`: every hash is 64 lowercase hex characters; a GNU-format line round-trips.
- `host_of` (ISO and quiz): never returns a name with userinfo, a port or a path; `https://good:pw@evil/` yields `evil`.
- `parse_winget_table`: arbitrary text, and synthetic real-looking tables with CRLF and truncation.
- `normalize_name`: idempotent, trimmed, single-spaced, no control characters. `classify` on the shipped map never panics.
- `parse_profiles_ini`, `parse_library_folders`, `parse_app_manifest`: arbitrary text, plus round-trips of generated valid files.
- `webquiz`: `parse_result` (size cap, a picked distro must exist), `base64url_decode` against a reference encoder, `resolve_asset` can't leave its root (`..`, `%2e%2e`, backslashes), `safe_to_open_externally` allows only `https:`/`mailto:`, and a local quiz is never allowed to navigate to `file:`, `javascript:` or `data:`.
- `validate_answers`: every complete valid answer set is accepted; an unknown question is refused.
- Firmware: `parse_gpt_entries`, `load_option_description`, `fix_load_option` on random bytes. A rewritten boot option never changes length.
- `parse_bcd_guid`: only a well-formed `{GUID}`, even in localised output.
- `path_within`: reflexive, and `C:\dataX` is not inside `C:\data`.
- `parse_progress`: garbage lines are ignored.

Failures are shrunk to a small input and saved in `tests/prop_parsers.proptest-regressions`. Commit that file; it replays the failure on every run.
Run longer with `PROPTEST_CASES=20000 cargo test --test prop_parsers`.

## The fuzz targets

`fuzz/` is a separate crate (it has its own empty `[workspace]`), so `cargo build` and `cargo test` at the root never see it.

| Target | Functions covered |
|---|---|
| `data_files` | `DistroList::parse`, `IsoList::parse`, `AppMap::parse` (+ `validate`, `lint`) |
| `checksums_and_urls` | `parse_checksums`, `iso::host_of` |
| `winget_table` | `parse_winget_table` |
| `app_matching` | `normalize_name`, `steam_appid_from_key`, `pattern_match_len`, `classify` |
| `userdata_formats` | `parse_profiles_ini`, `parse_library_folders`, `parse_app_manifest`, `parse_chromium_local_state` |
| `webquiz_input` | `parse_result`, `base64url_decode`, `extract_from_url`, `check_url`, `host_of`, `resolve_asset`, `nav_decision`, `safe_to_open_externally`, `QuizSettings` |
| `firmware_bytes` | `parse_gpt_entries`, `load_option_description`, `fix_load_option` |
| `state_files` | `InstallState`, `Manifest`, `ScanReport` JSON, `parse_progress`, `parse_bcd_guid`, `path_within` |
| `hardware_probe` | `Probe` JSON → `bitlocker_volumes`, `detect_rst_vmd`, `wifi_from_raw`, `interpret` |
| `answers_and_vm` | the `--answers` file, `validate_answers`, `Backend::parse` |

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cargo +nightly fuzz list
cargo +nightly fuzz run webquiz_input -- -max_total_time=300
```

Seeds in `fuzz/seeds/<target>/` are committed; copy them into `fuzz/corpus/<target>/` (ignored by git) to start from them, or pass the seed folder as an extra corpus directory:

```bash
cargo +nightly fuzz run data_files fuzz/corpus/data_files fuzz/seeds/data_files
```

A crash is saved to `fuzz/artifacts/<target>/crash-…`. Reproduce and minimise it:

```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-…
cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/crash-…
```

Then add a plain unit test with the minimised input next to the parser, fix the bug, and keep the regression test.

### Parsers that are not fuzzed

`restore.sh` and `remove-windows.sh` are shell, not parsers; they have sandboxed tests in `tests/restore_sh.rs` and `tests/remove_windows_sh.rs`. Windows-only readers (registry, PowerShell calls) can only be exercised through the pure functions they feed, which are covered above.
