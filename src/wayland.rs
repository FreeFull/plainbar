use miette::IntoDiagnostic;
use sctk::reexports::calloop::LoopHandle;
use smithay_client_toolkit as sctk;

use sctk::compositor::{CompositorHandler, CompositorState, Surface};
use sctk::output::{OutputHandler, OutputState};
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::protocol::wl_output::WlOutput;
use sctk::reexports::client::protocol::wl_shm::Format;
use sctk::reexports::client::QueueHandle;
use sctk::shell::wlr_layer::{
    Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use sctk::shell::WaylandSurface;
use sctk::shm::slot::{Buffer, SlotPool};
use sctk::shm::{Shm, ShmHandler};
use sctk::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    reexports::*, registry_handlers,
}; registry::RegistryState};

use crate::config::Config;
use crate::AppState;

#[derive(Debug)]
pub struct WaylandState {
    pub output: OutputState,
    pub _compositor: CompositorState,
    pub shm: Shm,
    pub _layer_shell: LayerShell,
    pub layer_surface: LayerSurface,
    pub pool: SlotPool,
    pub first_configure: bool,
    pub width: u32,
    pub height: u32,
    pub buffer: Option<Buffer>,
    pub registry: RegistryState,
}

impl WaylandState {
    pub fn init(
        config: &Config,
        loop_handle: &LoopHandle<AppState>,
    ) -> miette::Result<(QueueHandle<Self>, Self)> {
        let connection = client::Connection::connect_to_env().into_diagnostic()?;
        let (globals, event_queue) = registry_queue_init(&connection).into_diagnostic()?;
        let qh: QueueHandle<WaylandState> = event_queue.handle();
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

        loop_handle
            .insert_source(
                WaylandSource::new(connection, event_queue),
                |_, queue, data: &mut AppState| queue.dispatch_pending(&mut data.wayland_state),
            )
            .expect("insert wayland source");

        Ok((
            qh.clone(),
            WaylandState {
                output: OutputState::new(&globals, &qh),
                _compositor: compositor,
                shm,
                _layer_shell: layer_shell,
                layer_surface,
                pool,
                first_configure: true,
                width: 1,
                height: config.height,
                buffer: None,
                registry: RegistryState::new(&globals),
            },
        ))
    }

    pub fn draw(&mut self, _qh: &QueueHandle<WaylandState>) {
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

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }

    fn new_output(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _output: WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _output: WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _output: WlOutput,
    ) {
    }
}

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _surface: &client::protocol::wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // TODO!
    }

    fn transform_changed(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _surface: &client::protocol::wl_surface::WlSurface,
        _new_transform: client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        _surface: &client::protocol::wl_surface::WlSurface,
        _time: u32,
    ) {
        self.draw(qh);
    }

    fn surface_enter(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _surface: &client::protocol::wl_surface::WlSurface,
        _output: &client::protocol::wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _surface: &client::protocol::wl_surface::WlSurface,
        _output: &client::protocol::wl_output::WlOutput,
    ) {
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for WaylandState {
    fn closed(
        &mut self,
        _conn: &client::Connection,
        _qh: &client::QueueHandle<Self>,
        _layer: &LayerSurface,
    ) {
    }

    fn configure(
        &mut self,
        _conn: &client::Connection,
        qh: &client::QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
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

delegate_output!(WaylandState);
delegate_compositor!(WaylandState);
delegate_shm!(WaylandState);
delegate_layer!(WaylandState);
delegate_registry!(WaylandState);

impl sctk::registry::ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState,];
}
