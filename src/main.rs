use std::ops::Add;
use std::time::Duration;

use chrono::{DurationRound, SubsecRound, TimeDelta};
use sctk::compositor::{CompositorHandler, CompositorState, Surface};
use sctk::output::{OutputHandler, OutputState};
use sctk::reexports::calloop::timer::TimeoutAction;
use sctk::reexports::client::protocol::wl_output::WlOutput;
use sctk::reexports::client::protocol::wl_shm::Format;
use sctk::reexports::client::QueueHandle;
use sctk::shell::wlr_layer::{
    Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use sctk::shell::WaylandSurface;
use sctk::shm::slot::{Buffer, SlotPool};
use sctk::shm::{Shm, ShmHandler};
use smithay_client_toolkit as sctk;

use sctk::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    reexports::*, registry_handlers,
};
use sctk::{reexports::client::globals::registry_queue_init, registry::RegistryState};

use calloop::{EventLoop, EventSource};
use calloop_wayland_source::WaylandSource;
use miette::IntoDiagnostic;

mod config;
mod modules;

use config::Config;

fn main() -> miette::Result<()> {
    let config = Config { height: 16 };
    let connection = client::Connection::connect_to_env().into_diagnostic()?;
    let (globals, mut event_queue) = registry_queue_init(&connection).into_diagnostic()?;
    let qh: QueueHandle<AppState> = event_queue.handle();
    let layer_shell = LayerShell::bind(&globals, &qh).into_diagnostic()?;
    let compositor = CompositorState::bind(&globals, &qh).into_diagnostic()?;
    let surface = Surface::new(&compositor, &qh).into_diagnostic()?;
    let layer_surface =
        layer_shell.create_layer_surface(&qh, surface, Layer::Top, None::<String>, None);
    layer_surface.set_size(0, config.height);
    layer_surface.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer_surface.set_exclusive_zone(config.height as i32);
    layer_surface.commit();

    let shm = Shm::bind(&globals, &qh).into_diagnostic()?;
    // At this point, the size of the layer_surface isn't known, so we can't allocate
    // the exact size of SlotPool that we need without doing a roundtrip.
    // 1 is the minimum allocation for SlotPool, but it'll automatically grow larger as needed.
    let pool = SlotPool::new(1, &shm).into_diagnostic()?;

    let mut appstate = AppState {
        config,
        output: OutputState::new(&globals, &qh),
        compositor,
        shm,
        layer_shell,
        layer_surface,
        pool,
        first_configure: true,
        width: 1,
        height: config.height,
        buffer: None,
        current_time: String::new(),
        registry: RegistryState::new(&globals),
    };

    let mut event_loop = EventLoop::try_new().into_diagnostic()?;
    let loop_handle = event_loop.handle();
    WaylandSource::new(connection, event_queue)
        .insert(loop_handle.clone())
        .into_diagnostic()?;
    let timer = calloop::timer::Timer::from_duration(Duration::from_secs(0));
    loop_handle
        .insert_source(timer, {
            |_event, _, data| {
                data.current_time = chrono::Local::now().format("%T").to_string();
                data.draw(&qh);
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
        .run(None, &mut appstate, |data| {})
        .into_diagnostic()?;
    Ok(())
}

#[derive(Debug)]
struct AppState {
    config: Config,
    output: OutputState,
    compositor: CompositorState,
    shm: Shm,
    layer_shell: LayerShell,
    layer_surface: LayerSurface,
    pool: SlotPool,
    first_configure: bool,
    width: u32,
    height: u32,
    buffer: Option<Buffer>,
    current_time: String,
    registry: RegistryState,
}

impl AppState {
    fn draw(&mut self, qh: &QueueHandle<AppState>) {
        let width = self.width as i32;
        let height = self.height as i32;
        let stride = width * 4;

        let buffer = self.buffer.get_or_insert_with(|| {
            self.pool
                .create_buffer(width, height, stride, Format::Xrgb8888)
                .expect("create_buffer")
                .0
        });

        let canvas = match self.pool.canvas(buffer) {
            Some(canvas) => canvas,
            None => {
                // The previous buffer is still in use, allocate new buffer.
                let (second_buffer, canvas) = self
                    .pool
                    .create_buffer(width, height, stride, Format::Xrgb8888)
                    .expect("create_buffer");
                *buffer = second_buffer;
                canvas
            }
        };
        canvas.fill(0);
        self.layer_surface
            .wl_surface()
            .damage_buffer(0, 0, width, height);
        buffer
            .attach_to(self.layer_surface.wl_surface())
            .expect("buffer attach");
        self.layer_surface.commit();
    }
}

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }

    fn new_output(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        output: WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        output: WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        output: WlOutput,
    ) {
    }
}

impl CompositorHandler for AppState {
    fn scale_factor_changed(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        surface: &client::protocol::wl_surface::WlSurface,
        new_factor: i32,
    ) {
        todo!()
    }

    fn transform_changed(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        surface: &client::protocol::wl_surface::WlSurface,
        new_transform: client::protocol::wl_output::Transform,
    ) {
        todo!()
    }

    fn frame(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        surface: &client::protocol::wl_surface::WlSurface,
        time: u32,
    ) {
        self.draw(qh);
    }

    fn surface_enter(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        surface: &client::protocol::wl_surface::WlSurface,
        output: &client::protocol::wl_output::WlOutput,
    ) {
        //TODO
    }

    fn surface_leave(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        surface: &client::protocol::wl_surface::WlSurface,
        output: &client::protocol::wl_output::WlOutput,
    ) {
        todo!()
    }
}

impl ShmHandler for AppState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for AppState {
    fn closed(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        layer: &LayerSurface,
    ) {
        todo!()
    }

    fn configure(
        &mut self,
        conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        self.buffer = None;
        self.width = configure.new_size.0;
        self.height = configure.new_size.1;
        if self.first_configure {
            self.first_configure = false;
            self.draw(qh);
        }
    }
}

delegate_output!(AppState);
delegate_compositor!(AppState);
delegate_shm!(AppState);
delegate_layer!(AppState);
delegate_registry!(AppState);

impl sctk::registry::ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState,];
}
