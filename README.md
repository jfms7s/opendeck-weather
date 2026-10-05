# OpenDeck Weather

An [OpenDeck](https://github.com/nekename/OpenDeck) plugin modelled on Elgato's
[Weather](https://marketplace.elgato.com/product/weather-info-5c3b5556-febc-4a3d-939d-469c15057407)
plugin for Stream Deck. It has three actions - **Weather**, **Forecast** and **Air
Quality** - each assignable to a key or a dial. Data comes from
[Open-Meteo](https://open-meteo.com): free, and no API key or account needed.

Runs in OpenDeck on Linux (x86_64 and aarch64) and on macOS with Apple Silicon. The code
has no platform-specific parts; Windows has never been tried, so the manifest doesn't
offer it there.

## Actions

### Weather

Current conditions at your location: an icon (day and night variants), the temperature,
and the sky condition.

- **Key** - press for today's high/low and the "feels like" temperature.
- **Dial** - rotate to scroll the hourly forecast up to 24 hours ahead (time, temperature,
  chance of rain). Press the dial or tap the strip for the details screen: high/low,
  feels-like, wind and humidity.

### Forecast

One day of the 7-day forecast: high/low, condition, and chance of rain.

- **Key** - shows the configured day (tomorrow by default): its icon, high/low, and
  the day with its chance of rain (e.g. `Tomorrow · 70%`; just the day when rain is
  unlikely). Press to step through the following days; after the last day the
  forecast still covers, it wraps back to Today. Put several Forecast keys in a row,
  set to Today, Tomorrow, In 2 days... for a multi-day strip.
- **Dial** - rotate to scroll through the days. Press or tap for sunrise and sunset.

### Air Quality

The current air quality index, colored by category, on the **US AQI** or **European
AQI** scale (your choice).

- **Key** - press to page through PM2.5, PM10, ozone and NO₂ (µg/m³).
- **Dial** - rotate to page through them. Press or tap to jump back to the index.

Any scrolled, paged or details screen returns to its resting screen 15 seconds after
your last press or turn.

## Settings

Every action has the same settings:

- **Location** - type a city or postal code, press Search (or Enter), and pick the right
  match from the list. The search runs through Open-Meteo's geocoding API.
  **Use for all actions** makes it the default location: every action without a
  location of its own uses it, so a row of Forecast keys needs only one search. An
  action with its own location can go back to the default with **Use the default
  instead**.
- **Units** - Metric (°C, km/h) or Imperial (°F, mph).
- **Day** (Forecast only) - which day the key shows when you're not browsing.
- **Index** (Air Quality only) - US or European AQI.

Until a location (its own or the default) is set, an action shows a map pin and
"Set location".

## How it fetches data

- All keys and dials for the same place and units share one cached request. Forecasts
  and air quality are re-fetched at most every 10 minutes. Each action re-renders every
  minute, so the current hour stays up to date.
- If a request fails, the last good data stays on screen for up to 3 hours, and the
  plugin tries again at the next minute's refresh. Data older than 10 minutes is
  marked: its number turns gray and a small clock appears on the icon. A Weather key
  then shows the forecast for the current hour rather than the old reading. With no
  data at all, the key shows a crossed-out cloud and "No data".
- Fetches never hold up the deck: a press or turn always responds at once, even while
  the network is slow or down.
- Times (hours, sunrise, sunset) use the location's local time, not your machine's.

## Installing

Download the latest `.streamDeckPlugin` and `SHA256SUMS` from
[Releases](https://github.com/jfms7s/opendeck-weather/releases), and check them:

```bash
sha256sum -c SHA256SUMS
gh attestation verify opendeck-weather.streamDeckPlugin --repo jfms7s/opendeck-weather
```

On macOS, check the download with `shasum -a 256 -c SHA256SUMS` instead.

Then either double-click it (if your file manager associates the extension with
OpenDeck) or unzip it into OpenDeck's plugin folder and restart OpenDeck (OpenDeck
only loads plugins at startup):

- Linux: `~/.config/opendeck/plugins/`
- macOS: `~/Library/Application Support/opendeck/plugins/`

On macOS, a bundle unzipped by hand (e.g. in Finder) is marked as downloaded and
Gatekeeper refuses to start the binary. Clear the mark once:

```bash
xattr -dr com.apple.quarantine ~/Library/Application\ Support/opendeck/plugins/com.jfms7s.weather.sdPlugin
```

## Manual smoke-test checklist

Run this in a live OpenDeck + Stream Deck session before cutting a release:

- [ ] A new Weather key shows a map pin and "Set location".
- [ ] In the property inspector, searching "Lisbon" lists several matches, including
      "Lisbon, Lisbon District, Portugal" and "Lisbon, Ohio, United States". Picking one
      updates the key within a few seconds.
- [ ] Searching nonsense (`zzzqqq`) shows "No matches".
- [ ] Switching Units to Imperial changes the key to °F.
- [ ] Pressing the Weather key shows high/low and "Feels ..."; about 15s later it goes
      back to the current temperature.
- [ ] On a dial, rotating Weather clockwise steps through the hours (the label shows the
      hour, e.g. `16:00`). It stops at +24h, and rotating back stops at "now". Pressing
      shows the details screen.
- [ ] A Forecast key set to "Tomorrow" shows "Tomorrow" with its chance of rain (e.g.
      `Tomorrow · 70%`) under the high/low. Pressing it steps to the next dates and
      wraps back to Today after the last day.
- [ ] "Use for all actions" on one action's location makes a new, unconfigured action
      show that place; "Use the default instead" on another switches it to the default.
- [ ] On a dial, pressing Forecast shows `↑HH:MM ↓HH:MM` sunrise/sunset.
- [ ] Air Quality shows a number colored by category. Presses page through PM2.5 → PM10
      → Ozone → NO₂ → index. Switching Index to European changes the number and category.
- [ ] With the network down (e.g. `nmcli networking off`), keys keep showing their last
      data. A key given a new, never-fetched location shows "No data". After the network
      comes back, the keys recover within about a minute.
- [ ] With requests hanging instead of failing
      (`sudo iptables -A OUTPUT -p tcp --dport 443 -j DROP`), presses and turns still
      respond at once. After about 10 minutes the values turn gray with a clock badge.
      Remove the rule (`sudo iptables -D OUTPUT -p tcp --dport 443 -j DROP`): the keys
      recover within about a minute.

On a Mac (Apple Silicon), additionally - with the draft release's bundle (Releasing,
step 4):

- [ ] The release bundle installs through OpenDeck and every action above renders.
- [ ] `xattr -l` on the installed `opendeck-weather-aarch64-apple-darwin` shows no
      `com.apple.quarantine`.
- [ ] Weather, Forecast and Air Quality refresh on keys and a dial; location search works.
- [ ] With Wi-Fi off, keys keep their last data and recover after it comes back.

## Development

```bash
cargo fmt --check                            # what CI runs
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked                          # unit tests (no internet needed)
cargo build --release --locked
node build.mjs                               # assembles dist/<uuid>.sdPlugin for this machine
cp -r dist/com.jfms7s.weather.sdPlugin ~/.config/opendeck/plugins/
# (macOS: ~/Library/Application\ Support/opendeck/plugins/)
# restart OpenDeck, then work through the smoke-test checklist above
```

`node build.mjs <triple>...` packages binaries built with `--target <triple>`, and
`node build.mjs --all` every target in the manifest (what a release ships).

The API response fixtures in `tests/fixtures/` are real Open-Meteo responses (see
`tests/fixtures/README.md`). The parser tests run against them.

The action-list icons (`assets/icons/`) are drawn by the same code as the keys: their
SVG sources in `assets/icon-src/` come from `src/glyphs.rs`. After changing a glyph,
run `cargo test -- --ignored write_icon_sources` and `scripts/render-icons.sh`.

## Releasing

1. Bump `version` in `Cargo.toml` and `Version` in `assets/manifest.json` together
   (`node build.mjs` and CI fail if they differ).
2. Run the smoke-test checklist on a local build (`node build.mjs`) and paste it,
   ticked, with the OpenDeck version and device, into the release PR.
3. After merging, tag the merge commit `vX.Y.Z` and push the tag. The release workflow
   tests and builds it and creates a **draft** release with the bundle, `SHA256SUMS`
   and a build-provenance attestation.
4. Install the draft's bundle through OpenDeck on Linux and on a Mac, and run the
   checklist's "On a Mac" items there.
5. Write the notes (including the ticked checklists) and publish the draft.

## Attribution

Weather data by [Open-Meteo.com](https://open-meteo.com/), licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

## License

MIT — see [LICENSE](LICENSE).
