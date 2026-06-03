//! Procedurally generated tray icons so the crate needs no image assets.
//!
//! Each [`SyncState`] maps to a filled rounded square in a status colour. The
//! icon doubles as the activity indicator: blue while syncing, yellow for
//! conflicts, red for failures, green when idle.

use tray_icon::Icon;

use crate::status::SyncState;

const SIZE: u32 = 32;

fn color_for(state: SyncState) -> [u8; 3] {
    match state {
        SyncState::Unconfigured => [120, 120, 128], // neutral grey
        SyncState::Idle => [60, 170, 90],           // green
        SyncState::Syncing => [40, 130, 220],       // blue
        SyncState::Conflicts => [220, 170, 40],     // yellow
        SyncState::Error => [210, 70, 60],          // red
    }
}

/// Build a square status icon for the given state.
pub fn icon_for(state: SyncState) -> Icon {
    let [r, g, b] = color_for(state);
    let w = SIZE as i32;
    let radius = 7.0_f32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);

    for y in 0..w {
        for x in 0..w {
            let alpha = rounded_rect_alpha(x, y, w, radius);
            // Subtle vertical gradient so the icon reads as a glyph, not a flat
            // block, on both light and dark menu bars.
            let shade = 1.0 - (y as f32 / w as f32) * 0.18;
            rgba.push((r as f32 * shade) as u8);
            rgba.push((g as f32 * shade) as u8);
            rgba.push((b as f32 * shade) as u8);
            rgba.push(alpha);
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid generated rgba icon")
}

/// Coverage alpha for a pixel inside a rounded square, with 1px anti-aliasing at
/// the corners.
fn rounded_rect_alpha(x: i32, y: i32, size: i32, radius: f32) -> u8 {
    let fx = x as f32 + 0.5;
    let fy = y as f32 + 0.5;
    let s = size as f32;

    // Distance into the nearest corner region; 0 elsewhere.
    let dx = (radius - fx).max(fx - (s - radius)).max(0.0);
    let dy = (radius - fy).max(fy - (s - radius)).max(0.0);
    let dist = (dx * dx + dy * dy).sqrt();

    if dist <= radius - 1.0 {
        255
    } else if dist >= radius {
        0
    } else {
        ((radius - dist) * 255.0) as u8
    }
}
