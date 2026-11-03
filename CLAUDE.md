# opendeck-weather

OpenDeck plugin (Rust, `openaction` 2.7) with three actions - Weather, Forecast, Air
Quality - backed by Open-Meteo. User-facing behaviour is in `README.md`; the design
note (layering diagram, decisions) lives in the Obsidian vault at
`personal/projects/opendeck-weather/`, not in this repo.

## Commands

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked                  # no internet needed
cargo build --release --locked && node build.mjs    # bundle in dist/
node --test tests/*.test.mjs         # the property inspector's script and the draft notes (Node only)
```

`scripts/check-binary.sh` is what CI and the release use to check a built binary
starts (and, on macOS, is a signed arm64 Mach-O).

CI runs all of these (also natively on macOS, the `macos` job), plus an aarch64
`cargo check` and `cargo deny check advisories sources` (`deny.toml` allows crates from crates.io
only). Always pass `--locked`. `.nvmrc` is CI's and the release's exact Node version;
bump it by hand (Dependabot doesn't). The release workflow runs
the same advisory scan before it builds, so `advisories` must stay green: to accept an
advisory on purpose, add a `deny.toml` `[advisories] ignore = [{ id = "RUSTSEC-…",
reason = "…" }]` entry with the reason, never leave the job red.

## Layout

`open_meteo` (HTTP + pure `parse_*`; `ipma` the same for Portugal's warnings) ->
`cache` (TTL / stale fallback / backoff) -> `services` (the one shared cache, default
location) -> `views` (pure: data + view state + `now` -> `Card`) -> `card` (draws a
`Card` on a key or a dial).
`view_state` holds each action's UI state and its rules; `tracker` the live
instances; `scheduler` the single refresh/revert loop; `wake` the doorbell that
gets it to look again; `actions/` the generic `CardAction<B>` host plus one small
`Behavior` per action.

Adding an action: see the checklist in the design note (vault,
`personal/projects/opendeck-weather/design`).

## Invariants - don't break these

- **Handlers never await the network.** openaction runs event handlers one at a time;
  anything that can fetch goes through `CardAction::spawn_render` or a spawned task.
- **Every fetch goes through `Services`**, so all actions share one cache.
- **Settings deserialization never fails** (`Settings`/`GlobalSettings` parse per
  field): openaction would otherwise replace the whole settings object - and the
  user's location - with the default. `tests/fixtures/settings-v0.1.json` must keep
  loading.
- **Open-Meteo's CC BY 4.0 attribution** and **IPMA's source line** stay in the
  README and the property inspector (a contract test checks both).
- **IPMA warnings never sit on the render path**: `Services::warning` only peeks at
  the cache and fetches in the background, so a hanging IPMA can't delay a card.
- **Version lives in two places**: `Cargo.toml` and `assets/manifest.json`; bump both
  (`build.mjs` and the release workflow check).
- **No log line or error string carries coordinates or search text**: map
  `reqwest` errors through `http_error`, log setting errors by category
  (`model::field`), and keep openaction's raw inbound payloads out (the
  `logging` module's filter, installed by `main.rs`).
- **Contract tests** tie Rust to the shipped assets (manifest UUIDs, layout keys and
  colors, the property inspector's defaults / `data-action` UUIDs / day options /
  `maxlength`). Change both sides together.
- `assets/propertyInspector/pi.css` is shared byte-for-byte with the sibling
  `opendeck-*` plugins; page-specific CSS goes in `index.html`.
- Icons: `assets/icon-src/*.svg` are generated from `src/glyphs.rs`
  (`cargo test -- --ignored write_icon_sources`), PNGs by `scripts/render-icons.sh`.

## Cache policy

TTL 10 min, retry after a failure 55 s (must stay below the 60 s refresh - a const
assert checks), serve stale data up to 3 h. Views revert 15 s after the last touch.

## Releasing

Tag `vX.Y.Z` -> the release workflow builds Linux x86_64/aarch64 on Ubuntu and the
macOS arm64 binary on a `macos-15` runner, then creates a **draft** release with the bundle,
`SHA256SUMS` and a provenance attestation; its notes start with the README smoke-test
checklist, unticked, under an `UNVERIFIED` line (`scripts/release-body.mjs`, from the
README's `smoke-checklist` markers - keep them). The draft's notes are the one place the
ticked checklist goes; the owner runs it on Linux and publishes with
`scripts/publish-release.sh vX.Y.Z`, which refuses an `UNVERIFIED` draft. The "On a Mac"
items run on a real Mac only until one published release has passed them, then only when
the release touches TLS dependencies, the manifest, `build.mjs`/the bundle layout or
filesystem paths (README "Releasing", step 4). Agents stop at a verified draft.
