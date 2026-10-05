# Open-Meteo response fixtures

Real responses, captured on 2026-09-25 (local time 01:00 in Lisbon, as each file's
`current.time` shows) and committed with the first version of the plugin. The parser
tests in `src/open_meteo.rs` run against them. If Open-Meteo changes a field,
re-capture them with the requests below rather than editing them by hand. (The exact
coordinates of the original capture weren't recorded - responses echo the model's grid
point, not the request - so these are the requests the plugin makes for Lisbon.)

- `forecast.json` - what `forecast_url` builds for Lisbon, metric:

  ```
  https://api.open-meteo.com/v1/forecast?latitude=38.72509&longitude=-9.1498&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,wind_speed_10m&hourly=temperature_2m,weather_code,precipitation_probability,is_day&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset&timezone=auto&forecast_days=7
  ```

- `airquality.json` - what `air_quality_url` builds:

  ```
  https://air-quality-api.open-meteo.com/v1/air-quality?latitude=38.72509&longitude=-9.1498&current=us_aqi,european_aqi,pm2_5,pm10,ozone,nitrogen_dioxide&timezone=auto
  ```

- `geocode.json` - what `geocoding_url` builds for "Lisbon":

  ```
  https://geocoding-api.open-meteo.com/v1/search?name=Lisbon&count=8&language=en&format=json
  ```

- `settings-v0.1.json` - not an API response: settings in the shapes v0.1 saved in
  OpenDeck profiles (fields appear only once the property inspector writes them).
  They must keep loading; see `settings_saved_by_v0_1_still_load`.
