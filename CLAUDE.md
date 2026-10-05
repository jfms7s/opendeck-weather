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
```

CI runs all of these, plus an aarch64 `cargo check`, an MSRV (`rust-version`) check
and `cargo deny check advisories`. Always pass `--locked`.

## Layout

`open_meteo` (HTTP + pure `parse_*`) -> `cache` (TTL / stale fallback / backoff) ->
`services` (the one shared cache, default location) -> `views` (pure: data + view
state + `now` -> `Card`) -> `card` (draws a `Card` on a key or a dial).
`view_state` holds each action's UI state and its rules; `tracker` the live
instances; `scheduler` the single refresh/revert loop; `actions/` the generic
`CardAction<B>` host plus one small `Behavior` per action.

## Invariants - don't break these

- **Handlers never await the network.** openaction runs event handlers one at a time;
  anything that can fetch goes through `CardAction::spawn_render` or a spawned task.
- **Every fetch goes through `Services`**, so all actions share one cache.
- **Settings deserialization never fails** (`Settings`/`GlobalSettings` parse per
  field): openaction would otherwise replace the whole settings object - and the
  user's location - with the default. `tests/fixtures/settings-v0.1.json` must keep
  loading.
- **Open-Meteo's CC BY 4.0 attribution** stays in the README and the property
  inspector.
- **Version lives in two places**: `Cargo.toml` and `assets/manifest.json`; bump both
  (`build.mjs` and the release workflow check).
- **Error strings carry no URLs** (they hold coordinates / search text): map
  `reqwest` errors through `http_error`.
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

Tag `vX.Y.Z` -> the release workflow builds a **draft** release with the bundle,
`SHA256SUMS` and a provenance attestation. Run the README smoke-test checklist and
paste it into the notes before publishing.
