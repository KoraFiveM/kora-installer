//! The tray icons, drawn in code: a round badge in the status colour with a pale centre, anti-aliased. No image
//! files or image crate are needed, and tests can check the pixels.

use crate::tray::IconKind;

/// Brand lime, kept for the app icon; the status colours are the usual traffic lights, dimmed for dark trays.
pub fn rgb(kind: IconKind) -> [u8; 3] {
    match kind {
        IconKind::Green => [0x3F, 0xC1, 0x5A],
        IconKind::Amber => [0xF0, 0xA8, 0x1C],
        IconKind::Red => [0xE0, 0x3E, 0x3E],
        IconKind::Grey => [0x8A, 0x90, 0x96],
    }
}

/// RGBA pixels (`size` x `size`, row by row) of the badge.
pub fn status_icon(kind: IconKind, size: u32) -> Vec<u8> {
    let [r, g, b] = rgb(kind);
    let c = size as f32 / 2.0;
    let radius = c - 0.5;
    let inner = radius * 0.38;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = (((x as f32 + 0.5) - c).powi(2) + ((y as f32 + 0.5) - c).powi(2)).sqrt();
            // 1 inside the disc, fading over one pixel at its edge.
            let cover = (radius - d + 0.5).clamp(0.0, 1.0);
            // The pale centre (a lighter dot), also soft at its edge.
            let dot = (inner - d + 0.5).clamp(0.0, 1.0);
            let mix = |base: u8| (base as f32 * (1.0 - dot * 0.75) + 255.0 * dot * 0.75).round() as u8;
            out.extend_from_slice(&[mix(r), mix(g), mix(b), (cover * 255.0).round() as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(v: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [v[i], v[i + 1], v[i + 2], v[i + 3]]
    }

    #[test]
    fn a_badge_is_round_coloured_and_transparent_at_the_corners() {
        let s = 32;
        for k in [IconKind::Green, IconKind::Amber, IconKind::Red, IconKind::Grey] {
            let v = status_icon(k, s);
            assert_eq!(v.len(), (s * s * 4) as usize);
            assert_eq!(px(&v, s, 0, 0)[3], 0, "corner is transparent");
            let edge = px(&v, s, 16, 4);
            assert_eq!(edge[3], 255, "the disc is solid");
            assert_eq!([edge[0], edge[1], edge[2]], rgb(k), "{k:?}");
            let mid = px(&v, s, 16, 16);
            assert!(mid[0] > rgb(k)[0] || mid[1] > rgb(k)[1], "pale centre");
        }
    }

    #[test]
    fn the_four_states_look_different() {
        let all: Vec<_> = [IconKind::Green, IconKind::Amber, IconKind::Red, IconKind::Grey].iter().map(|k| rgb(*k)).collect();
        for i in 0..4 {
            for j in i + 1..4 {
                assert_ne!(all[i], all[j]);
            }
        }
    }
}
