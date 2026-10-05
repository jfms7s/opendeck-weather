mod actions;
mod cache;
mod card;
mod glyphs;
mod model;
mod open_meteo;
mod palette;
mod scheduler;
mod services;
mod tracker;
mod view_state;
mod views;
mod wmo;

use actions::GlobalSettingsHandler;
use openaction::OpenActionResult;
use openaction::global_events::set_global_event_handler;
use scheduler::Wake;
use services::Services;
use std::sync::Arc;

// Handlers never block (network work is spawned) and the plugin is idle
// almost all the time, so two workers are plenty.
#[tokio::main(worker_threads = 2)]
async fn main() -> OpenActionResult<()> {
    simplelog::SimpleLogger::init(log::LevelFilter::Info, simplelog::Config::default())
        .expect("logger init");

    // One cache shared by all actions: a Weather key and a Forecast dial for
    // the same place cost a single forecast request per refresh window.
    let services = Arc::new(Services::default());
    let wake = Arc::new(Wake::default());

    let actions = actions::register_all(&services, &wake).await;
    tokio::spawn(scheduler::run(actions, wake.clone()));
    set_global_event_handler(Box::leak(Box::new(GlobalSettingsHandler {
        services,
        wake,
    })));

    openaction::run(std::env::args().collect()).await
}
