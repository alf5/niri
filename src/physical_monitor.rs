//! Physical monitor abstraction.
//!
//! A [`PhysicalMonitor`] represents one physical display panel (a DRM connector, a winit
//! window, etc.). It owns one or more *virtual outputs* — each a Smithay [`Output`] mapping
//! to an area of the panel.
//!
//! With no `rows` config a monitor has a single virtual output spanning the whole panel, so
//! behavior matches the 1:1 connector-to-output mapping. With `rows > 1` the panel is split
//! into that many equal-height, full-width bands stacked top to bottom.

use niri_config::OutputName;
use smithay::output::{Mode, Output};
use smithay::utils::{Physical, Point, Raw, Rectangle, Size};

use crate::utils::id::IdCounter;

static PHYSICAL_MONITOR_ID_COUNTER: IdCounter = IdCounter::new();

/// Unique identifier for a [`PhysicalMonitor`].
///
/// A copy of this id is stored in each virtual output's `user_data()`, linking the output
/// back to its physical monitor in [`Niri::physical_monitors`](crate::niri::Niri).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysicalMonitorId(u64);

impl PhysicalMonitorId {
    fn next() -> PhysicalMonitorId {
        PhysicalMonitorId(PHYSICAL_MONITOR_ID_COUNTER.next())
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// A physical display panel hosting one or more virtual outputs.
#[derive(Debug)]
pub struct PhysicalMonitor {
    id: PhysicalMonitorId,
    /// Connector/make/model/serial identifying the panel.
    name: OutputName,
    /// Active physical mode (full panel resolution), if known.
    mode: Option<Mode>,
    /// Physical panel size in millimeters.
    physical_size_mm: Size<i32, Raw>,
    /// Virtual outputs hosted on this monitor, in top-to-bottom band order.
    outputs: Vec<Output>,
}

impl PhysicalMonitor {
    /// Creates a physical monitor hosting the given virtual outputs.
    ///
    /// `outputs` must be in top-to-bottom band order and non-empty.
    pub fn new(
        name: OutputName,
        mode: Option<Mode>,
        physical_size_mm: Size<i32, Raw>,
        outputs: Vec<Output>,
    ) -> Self {
        assert!(!outputs.is_empty(), "a physical monitor needs an output");
        Self {
            id: PhysicalMonitorId::next(),
            name,
            mode,
            physical_size_mm,
            outputs,
        }
    }

    /// Creates a physical monitor that hosts a single virtual output spanning the whole panel.
    ///
    /// It derives the panel's identity, mode and size from the output itself. The output must
    /// already have its `OutputName` inserted into `user_data()` (all backends do this before
    /// calling `add_output`).
    pub fn single(output: &Output) -> Self {
        let props = output.physical_properties();
        let name = output
            .user_data()
            .get::<OutputName>()
            .cloned()
            .unwrap_or_else(|| OutputName {
                connector: output.name(),
                make: None,
                model: None,
                serial: None,
            });

        Self::new(
            name,
            output.current_mode(),
            props.size,
            vec![output.clone()],
        )
    }

    pub fn id(&self) -> PhysicalMonitorId {
        self.id
    }

    pub fn name(&self) -> &OutputName {
        &self.name
    }

    pub fn mode(&self) -> Option<Mode> {
        self.mode
    }

    pub fn physical_size_mm(&self) -> Size<i32, Raw> {
        self.physical_size_mm
    }

    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// Returns the monitor's sole virtual output.
    ///
    /// Only valid for an unsplit monitor; callers on the split path use [`Self::outputs`].
    pub fn output(&self) -> &Output {
        debug_assert_eq!(
            self.outputs.len(),
            1,
            "output() is only valid for an unsplit monitor"
        );
        &self.outputs[0]
    }
}

/// Divides a panel of pixel size `size` into `rows` virtual-output rectangles.
///
/// The bands are equal-height, full-width, and stacked top to bottom (niri scrolls
/// horizontally, so only vertical splitting is meaningful). When `size.h` is not divisible
/// by `rows`, the bottom band absorbs the remainder so the bands exactly tile the panel.
///
/// `rows` of 0 or 1 yields a single band covering the whole panel — the default, unsplit
/// monitor. The returned vector always has `rows.max(1)` elements.
pub fn band_rectangles(size: Size<i32, Physical>, rows: u16) -> Vec<Rectangle<i32, Physical>> {
    let rows = i32::from(rows.max(1));
    let band_h = size.h / rows;
    (0..rows)
        .map(|i| {
            let y = i * band_h;
            // The last band absorbs any `size.h % rows` remainder.
            let h = if i == rows - 1 { size.h - y } else { band_h };
            Rectangle::new(Point::from((0, y)), Size::from((size.w, h)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    #[test]
    fn single_band_covers_whole_panel() {
        let size = Size::from((1920, 1080));
        assert_eq!(band_rectangles(size, 0), vec![rect(0, 0, 1920, 1080)]);
        assert_eq!(band_rectangles(size, 1), vec![rect(0, 0, 1920, 1080)]);
    }

    #[test]
    fn even_division() {
        let bands = band_rectangles(Size::from((1920, 1080)), 2);
        assert_eq!(
            bands,
            vec![rect(0, 0, 1920, 540), rect(0, 540, 1920, 540)]
        );
    }

    #[test]
    fn remainder_goes_to_bottom_band() {
        let bands = band_rectangles(Size::from((1000, 1000)), 3);
        // 1000 / 3 = 333, remainder 1 absorbed by the last band.
        assert_eq!(
            bands,
            vec![
                rect(0, 0, 1000, 333),
                rect(0, 333, 1000, 333),
                rect(0, 666, 1000, 334),
            ]
        );
        // Bands exactly tile the panel with no gap or overlap.
        let total: i32 = bands.iter().map(|b| b.size.h).sum();
        assert_eq!(total, 1000);
    }
}
