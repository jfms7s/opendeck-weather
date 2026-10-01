mod actions;
mod cache;
mod card;
mod glyphs;
mod model;
mod open_meteo;
mod services;
mod tracker;
mod views;
mod wmo;

use actions::air_quality::AirQualityAction;
use actions::forecast::ForecastAction;
use actions::tick_loop;
use actions::weather::WeatherAction;
use openaction::{OpenActionResult, register_action, run};
use services::Services;
use std::sync::Arc;

#[tokio::main]
async fn main() -> OpenActionResult<()> {
    simplelog::SimpleLogger::init(log::LevelFilter::Info, simplelog::Config::default())
        .expect("logger init");

    // One cache shared by all actions: a Weather key and a Forecast dial for
    // the same place cost a single forecast request per refresh window.
    let services = Arc::new(Services::default());

    let weather = WeatherAction::new(services.clone());
    let forecast = ForecastAction::new(services.clone());
    let air_quality = AirQualityAction::new(services);

    tokio::spawn(tick_loop(weather.clone()));
    tokio::spawn(tick_loop(forecast.clone()));
    tokio::spawn(tick_loop(air_quality.clone()));

    register_action(weather).await;
    register_action(forecast).await;
    register_action(air_quality).await;
    run(std::env::args().collect()).await
}
