//! Tests for splitting a physical monitor into several stacked virtual outputs.

use niri_config::{Config, Output as OutputConfig, Outputs};

use super::*;
use crate::niri::RedrawState;

/// Builds a config that splits the given headless monitor into `rows` virtual outputs.
fn config_with_rows(monitor: &str, rows: u16) -> Config {
    Config {
        outputs: Outputs(vec![OutputConfig {
            name: monitor.to_string(),
            rows: Some(rows),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

#[test]
fn rows_splits_monitor_into_stacked_outputs() {
    let mut f = Fixture::with_config(config_with_rows("headless-1", 2));
    f.add_output(1, (1920, 1080));

    let niri = f.niri();

    // The physical monitor hosts two virtual outputs.
    assert_eq!(niri.physical_monitors.len(), 1);
    let monitor = niri.physical_monitors.values().next().unwrap();
    assert_eq!(monitor.outputs().len(), 2);

    // Both virtual outputs are tracked.
    assert_eq!(niri.global_space.outputs().count(), 2);

    // They are full-width, equal-height, and stacked top to bottom.
    let mut geoms: Vec<_> = niri
        .global_space
        .outputs()
        .map(|o| niri.global_space.output_geometry(o).unwrap())
        .collect();
    geoms.sort_by_key(|g| g.loc.y);

    assert_eq!((geoms[0].loc.x, geoms[0].loc.y), (0, 0));
    assert_eq!((geoms[0].size.w, geoms[0].size.h), (1920, 540));
    assert_eq!((geoms[1].loc.x, geoms[1].loc.y), (0, 540));
    assert_eq!((geoms[1].size.w, geoms[1].size.h), (1920, 540));

    // Both virtual outputs resolve back to the same physical monitor.
    let id = monitor.id();
    for output in niri.global_space.outputs() {
        assert_eq!(niri.physical_monitor_for_output(output).unwrap().id(), id);
    }
}

#[test]
fn odd_height_remainder_goes_to_the_bottom_band() {
    let mut f = Fixture::with_config(config_with_rows("headless-1", 3));
    f.add_output(1, (800, 1000));

    let niri = f.niri();
    let mut geoms: Vec<_> = niri
        .global_space
        .outputs()
        .map(|o| niri.global_space.output_geometry(o).unwrap())
        .collect();
    geoms.sort_by_key(|g| g.loc.y);

    // 1000 / 3 = 333, with the remainder absorbed by the bottom band.
    assert_eq!(geoms.iter().map(|g| g.size.h).collect::<Vec<_>>(), [333, 333, 334]);
    // The bands tile the panel exactly, with no gap or overlap.
    assert_eq!(geoms[0].loc.y, 0);
    assert_eq!(geoms[1].loc.y, 333);
    assert_eq!(geoms[2].loc.y, 666);
}

#[test]
fn split_monitor_does_not_shift_other_monitors() {
    // Only headless-1 is split; headless-2 is a normal single-output monitor.
    let mut f = Fixture::with_config(config_with_rows("headless-1", 2));
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let niri = f.niri();
    assert_eq!(niri.physical_monitors.len(), 2);
    assert_eq!(niri.global_space.outputs().count(), 3);

    // headless-2 sits to the right of the split monitor's full width.
    let second = niri
        .global_space
        .outputs()
        .find(|o| o.name() == "headless-2")
        .unwrap();
    let geo = niri.global_space.output_geometry(second).unwrap();
    assert_eq!((geo.loc.x, geo.loc.y), (1920, 0));
    assert_eq!((geo.size.w, geo.size.h), (1280, 720));
}

#[test]
fn split_monitor_redraws_without_panic() {
    let mut f = Fixture::with_config(config_with_rows("headless-1", 2));
    f.add_output(1, (1920, 1080));

    // Queueing a redraw via one band schedules the whole physical monitor.
    let output = f.niri().global_space.outputs().next().cloned().unwrap();
    f.niri().queue_redraw(&output);

    // Redrawing the monitor must walk both virtual outputs without panicking.
    let state = f.niri_state();
    state.niri.redraw_queued_outputs(&mut state.backend);

    // The monitor's shared redraw state has left the Queued state.
    let niri = f.niri();
    assert!(niri.monitor_render_state.values().all(|s| !matches!(
        s.redraw_state,
        RedrawState::Queued | RedrawState::WaitingForEstimatedVBlankAndQueued(_)
    )));
}

#[test]
fn rows_one_keeps_a_single_whole_panel_output() {
    let mut f = Fixture::with_config(config_with_rows("headless-1", 1));
    f.add_output(1, (1920, 1080));

    let niri = f.niri();
    assert_eq!(niri.global_space.outputs().count(), 1);
    let output = niri.global_space.outputs().next().unwrap();
    assert_eq!(output.name(), "headless-1");
    let geo = niri.global_space.output_geometry(output).unwrap();
    assert_eq!((geo.size.w, geo.size.h), (1920, 1080));
}
