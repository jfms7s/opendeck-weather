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
and the sky condition, plus when rain next starts or stops in the coming 12 hours, from
the hourly forecast: `Rain at 16:00` when it is dry now, `Rain until 18:00` when it is
raining, `Dry next 12h` when every hour is forecast dry. An hour counts as wet when its
forecast is rain, drizzle, snow, showers or thunderstorms, or its chance of rain is at
least 50%. When the forecast doesn't reach far enough to tell (no chance given for an
hour, or rain all 12 hours), the card shows the location name and condition as before.

- **Key** - the rain line replaces the condition only when rain starts or stops within
  12 hours, so a dry day still shows the sky. Press for today's high/low and the "feels
  like" temperature.
- **Dial** - the rain line replaces the location name under the condition. Rotate to
  scroll the hourly forecast up to 24 hours ahead (time, temperature, chance of rain).
  Press the dial or tap the strip for the details screen: high/low, feels-like, wind
  and humidity.

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

Run this in a live OpenDeck + Stream Deck session on Linux, and the Mac list on a Mac,
before publishing a release. Record the result per item: pass, fail or **not run** (with
why). Each release's draft notes start with this list, unticked, under an `UNVERIFIED`
line: tick or annotate each item there, fill in the run details and delete the
`UNVERIFIED` line before publishing. The full run (date, OpenDeck version, platform,
Stream Deck model, per-item notes) goes in that release's smoke log,
`releases/vX.Y.Z-smoke.md` in the maintainer's project notes (kept outside this
repository), which the release notes link.

<!-- smoke-checklist:start (scripts/release-body.mjs copies everything up to the end marker into each draft release) -->

- [ ] A new Weather key shows a map pin and "Set location".
- [ ] In the property inspector, searching "Lisbon" lists several matches, including
      "Lisbon, Lisbon District, Portugal" and "Lisbon, Ohio, United States". Picking one
      updates the key within a few seconds.
- [ ] Searching nonsense (`zzzqqq`) shows "No matches".
- [ ] Switching Units to Imperial changes the key to °F.
- [ ] At rest, a Weather dial shows `Rain at HH:MM`, `Rain until HH:MM` or `Dry next 12h`
      under the condition, matching the hourly forecast's chance of rain when rotated;
      a Weather key shows the same `Rain at` / `Rain until` line instead of the
      condition only when rain starts or stops within 12 hours.
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
      (`sudo iptables -A OUTPUT -p tcp --dport 443 -j DROP`), after about 10 minutes
      the values turn gray with a clock badge. With the rule still in place, press keys
      and turn dials repeatedly over about a minute: every refresh now hangs, and
      presses and turns respond at once even then. Remove the rule
      (`sudo iptables -D OUTPUT -p tcp --dport 443 -j DROP`): the keys recover within
      about a minute.

On a Mac (Apple Silicon), additionally - with the draft release's bundle (Releasing,
step 4):

- [ ] The release bundle installs through OpenDeck and every action above renders.
- [ ] `xattr -l` on the installed `opendeck-weather-aarch64-apple-darwin` shows no
      `com.apple.quarantine`.
- [ ] Weather, Forecast and Air Quality refresh on keys and a dial; location search works.
- [ ] With Wi-Fi off, keys keep their last data and recover after it comes back.

<!-- smoke-checklist:end -->

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
2. Open a `chore: release X.Y.Z` PR with that bump and merge it. The smoke test runs
   on the draft's bundle (step 4), not in the PR.
3. Tag the merge commit `vX.Y.Z` and push the tag. The release workflow tests and builds
   it and creates a **draft** release with the bundle, `SHA256SUMS` and a
   build-provenance attestation. The draft's notes start with the smoke-test checklist
   above, unticked, under an `UNVERIFIED` line (`scripts/release-body.mjs`), followed by
   the generated change list.
4. Install the draft's bundle through OpenDeck on Linux and on a Mac, run the checklist,
   write the release's smoke log, tick or annotate every item in the draft's notes, link
   the log and delete the `UNVERIFIED` line. The release notes are the record; a draft
   whose notes still say `UNVERIFIED` is not published.
5. Publish with `scripts/publish-release.sh vX.Y.Z`, which refuses while the notes say
   `UNVERIFIED` or while GitHub's immutable releases setting is off. Ansible installs the
   latest *published* release, so publishing is what deploys it. Published releases are
   immutable: the assets and the tag are frozen, so a broken release is fixed with a new
   version, never by replacing an asset
   (`gh api repos/jfms7s/opendeck-weather/immutable-releases` must report
   `"enabled":true`; a release published while it is off stays mutable).

## Attribution

Weather data by [Open-Meteo.com](https://open-meteo.com/), licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

## License

MIT — see [LICENSE](LICENSE).
