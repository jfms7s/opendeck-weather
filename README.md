# OpenDeck Weather

An [OpenDeck](https://github.com/nekename/OpenDeck) plugin modelled on Elgato's
[Weather](https://marketplace.elgato.com/product/weather-info-5c3b5556-febc-4a3d-939d-469c15057407)
plugin for Stream Deck. It has three actions - **Weather**, **Forecast** and **Air
Quality** - each assignable to a key or a dial. Data comes from
[Open-Meteo](https://open-meteo.com): free, and no API key or account needed.

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

- **Key** - shows the configured day (tomorrow by default). Press to step through the
  next days. Put several Forecast keys in a row, set to Today, Tomorrow, In 2 days... for
  a multi-day strip.
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
- **Units** - Metric (°C, km/h) or Imperial (°F, mph).
- **Day** (Forecast only) - which day the key shows when you're not browsing.
- **Index** (Air Quality only) - US or European AQI.

Until a location is set, an action shows a map pin and "Set location".

## How it fetches data

- All keys and dials for the same place and units share one cached request. Forecasts
  and air quality are re-fetched at most every 10 minutes. Each action re-renders every
  minute, so the current hour stays up to date.
- If a request fails, the last good data stays on screen for up to 3 hours, and the
  plugin waits a minute before trying again. With no data at all, the key shows a
  crossed-out cloud and "No data".
- Times (hours, sunrise, sunset) use the location's local time, not your machine's.

## Installing

Download the latest `.streamDeckPlugin` from
[Releases](https://github.com/jfms7s/opendeck-weather/releases). Then either
double-click it (if your file manager associates the extension with OpenDeck) or unzip it
into `~/.config/opendeck/plugins/` and restart OpenDeck (OpenDeck only loads plugins at
startup).

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
- [ ] A Forecast key set to "Tomorrow" is labelled "Tomorrow". Pressing it steps to the
      next dates and wraps back to Today after the 7th day.
- [ ] On a dial, pressing Forecast shows `↑HH:MM ↓HH:MM` sunrise/sunset.
- [ ] Air Quality shows a number colored by category. Presses page through PM2.5 → PM10
      → Ozone → NO₂ → index. Switching Index to European changes the number and category.
- [ ] With the network down (e.g. `nmcli networking off`), keys keep showing their last
      data. A key given a new, never-fetched location shows "No data". After the network
      comes back, the keys recover within about a minute.

## Development

```bash
cargo test                                   # unit tests (no network needed)
cargo build --release --target <triple>
node build.mjs <triple>                      # assembles dist/<uuid>.sdPlugin
cp -r dist/com.jfms7s.weather.sdPlugin ~/.config/opendeck/plugins/
# restart OpenDeck, then work through the smoke-test checklist above
```

The API response fixtures in `tests/fixtures/` are real Open-Meteo responses. The
parser tests run against them.

## Attribution

Weather data by [Open-Meteo.com](https://open-meteo.com/), licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

## License

MIT — see [LICENSE](LICENSE).
