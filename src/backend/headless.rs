//! Headless backend for tests.
//!
//! This can eventually grow into a more complete backend if needed, but for now it's missing some
//! crucial parts like dmabufs.

use std::mem;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use niri_config::OutputName;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::native::EGLSurfacelessDisplay;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::element::RenderElementStates;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::utils::Size;
use smithay::wayland::presentation::Refresh;

use super::{IpcOutputMap, OutputId, RenderResult};
use crate::niri::{Niri, RedrawState};
use crate::physical_monitor::{band_rectangles, PhysicalMonitor, PhysicalMonitorId};
use crate::render_helpers::{resources, shaders};
use crate::utils::{get_monotonic_time, logical_output};

pub struct Headless {
    renderer: Option<GlesRenderer>,
    ipc_outputs: Arc<Mutex<IpcOutputMap>>,
}

impl Headless {
    pub fn new() -> Self {
        Self {
            renderer: None,
            ipc_outputs: Default::default(),
        }
    }

    pub fn init(&mut self, _niri: &mut Niri) {}

    pub fn add_renderer(&mut self) -> anyhow::Result<()> {
        if self.renderer.is_some() {
            error!("add_renderer: renderer must not already exist");
            return Ok(());
        }

        let mut renderer = unsafe {
            let display =
                EGLDisplay::new(EGLSurfacelessDisplay).context("error creating EGL display")?;
            let context = EGLContext::new(&display).context("error creating EGL context")?;
            GlesRenderer::new(context).context("error creating renderer")?
        };

        resources::init(&mut renderer);
        shaders::init(&mut renderer);

        self.renderer = Some(renderer);
        Ok(())
    }

    pub fn add_output(&mut self, niri: &mut Niri, n: u8, size: (u16, u16)) {
        let connector = format!("headless-{n}");
        let make = "niri".to_string();
        let model = "headless".to_string();

        let full_size = Size::from((i32::from(size.0), i32::from(size.1)));

        // The physical monitor is identified by the bare connector name. A config block
        // `output "headless-N"` can split it into several stacked virtual outputs via `rows`.
        let monitor_name = OutputName {
            connector: connector.clone(),
            make: Some(make.clone()),
            model: Some(model.clone()),
            serial: Some(n.to_string()),
        };
        let rows = niri
            .config
            .borrow()
            .outputs
            .find(&monitor_name)
            .and_then(|o| o.rows)
            .unwrap_or(1)
            .max(1);

        let bands = band_rectangles(full_size, rows);

        let mut outputs = Vec::with_capacity(bands.len());
        for (i, band) in bands.iter().enumerate() {
            // An unsplit monitor keeps the bare connector name; bands are suffixed so that
            // wl_output names, IPC keys and layout output ids all stay unique.
            let (out_connector, serial) = if rows == 1 {
                (connector.clone(), n.to_string())
            } else {
                (format!("{connector}-{i}"), format!("{n}.{i}"))
            };

            let output = Output::new(
                out_connector.clone(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: make.clone(),
                    model: model.clone(),
                    serial_number: serial.clone(),
                },
            );

            let mode = Mode {
                size: band.size,
                refresh: 60_000,
            };
            output.change_current_state(Some(mode), None, None, None);
            output.set_preferred(mode);

            output.user_data().insert_if_missing(|| OutputName {
                connector: out_connector,
                make: Some(make.clone()),
                model: Some(model.clone()),
                serial: Some(serial),
            });

            let physical_properties = output.physical_properties();
            self.ipc_outputs.lock().unwrap().insert(
                OutputId::next(),
                niri_ipc::Output {
                    name: output.name(),
                    make: physical_properties.make,
                    model: physical_properties.model,
                    serial: None,
                    physical_size: None,
                    modes: vec![niri_ipc::Mode {
                        width: band.size.w.clamp(0, u16::MAX as i32) as u16,
                        height: band.size.h.clamp(0, u16::MAX as i32) as u16,
                        refresh_rate: 60_000,
                        is_preferred: true,
                    }],
                    current_mode: Some(0),
                    is_custom_mode: true,
                    vrr_supported: false,
                    vrr_enabled: false,
                    logical: Some(logical_output(&output)),
                },
            );

            outputs.push(output);
        }

        let monitor = PhysicalMonitor::new(
            monitor_name,
            Some(Mode {
                size: full_size,
                refresh: 60_000,
            }),
            (0, 0).into(),
            outputs,
        );
        niri.add_output(monitor, None, false);
    }

    pub fn seat_name(&self) -> String {
        "headless".to_owned()
    }

    pub fn with_primary_renderer<T>(
        &mut self,
        f: impl FnOnce(&mut GlesRenderer) -> T,
    ) -> Option<T> {
        self.renderer.as_mut().map(f)
    }

    pub fn render(&mut self, niri: &mut Niri, monitor_id: PhysicalMonitorId) -> RenderResult {
        let outputs: Vec<Output> = niri.physical_monitors[&monitor_id].outputs().to_vec();

        // Presentation feedback is per virtual output.
        for output in &outputs {
            let states = RenderElementStates::default();
            let mut presentation_feedbacks = niri.take_presentation_feedbacks(output, &states);
            presentation_feedbacks.presented::<_, smithay::utils::Monotonic>(
                get_monotonic_time(),
                Refresh::Unknown,
                0,
                wp_presentation_feedback::Kind::empty(),
            );
        }

        // The redraw state is shared by all of the monitor's virtual outputs.
        let render_state = niri.monitor_render_state.get_mut(&monitor_id).unwrap();
        match mem::replace(&mut render_state.redraw_state, RedrawState::Idle) {
            RedrawState::Idle => unreachable!(),
            RedrawState::Queued => (),
            RedrawState::WaitingForVBlank { .. } => unreachable!(),
            RedrawState::WaitingForEstimatedVBlank(_) => unreachable!(),
            RedrawState::WaitingForEstimatedVBlankAndQueued(_) => unreachable!(),
        }

        for output in &outputs {
            let output_state = niri.output_state.get_mut(output).unwrap();
            output_state.frame_callback_sequence =
                output_state.frame_callback_sequence.wrapping_add(1);
        }

        // FIXME: request redraw on unfinished animations remain

        RenderResult::Submitted
    }

    pub fn import_dmabuf(&mut self, _dmabuf: &Dmabuf) -> bool {
        unimplemented!()
    }

    pub fn ipc_outputs(&self) -> Arc<Mutex<IpcOutputMap>> {
        self.ipc_outputs.clone()
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}
