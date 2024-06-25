use std::time::Duration;

use smithay_client_toolkit::reexports::calloop;

use calloop::{timer::TimeoutAction, EventLoop};
use chrono::{DurationRound, TimeDelta};
use miette::IntoDiagnostic;

mod config;
mod modules;
mod wayland;

use config::Config;
use wayland::WaylandState;

fn main() -> miette::Result<()> {
    let mut event_loop = EventLoop::try_new().into_diagnostic()?;
    let loop_handle = event_loop.handle();

    let config = Config { height: 16 };
    let (qh, wayland_state) = WaylandState::init(&config, &loop_handle)?;

    let mut app_state = AppState {
        _config: config,
        current_time: String::new(),
        wayland_state,
    };

    let timer = calloop::timer::Timer::from_duration(Duration::from_secs(0));
    loop_handle
        .insert_source(timer, {
            move |_event, _, data| {
                data.current_time = chrono::Local::now().format("%T").to_string();
                data.wayland_state.draw(&qh);
                let mut time = chrono::Local::now()
                    .duration_trunc(TimeDelta::seconds(1))
                    .unwrap();
                time += Duration::from_secs(1);
                // Clamped, in case the time jumped a lot between here and the previous calculation
                let time_delta = (time - chrono::Local::now())
                    .clamp(TimeDelta::seconds(0), TimeDelta::seconds(2));
                TimeoutAction::ToDuration(time_delta.to_std().unwrap_or(Duration::from_secs(1)))
            }
        })
        .expect("insert timer");
    event_loop
        .run(None, &mut app_state, |_| {})
        .into_diagnostic()?;
    Ok(())
}

#[derive(Debug)]
struct AppState {
    _config: Config,
    current_time: String,
    wayland_state: WaylandState,
}
