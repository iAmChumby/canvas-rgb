//! `Thumb` and the detail picture, after `app/web/src/components/Thumb.tsx`,
//! `EffectPreview.tsx` and `lib/thumbs.ts`.
//!
//! The daemon answers `GET /api/v1/effects/{id}/thumbnail` with binary data:
//! u16 LE width, u16 LE height, u16 LE frame count, then frames x width x
//! height x 3 RGB bytes (64x40, 24 frames at 12 fps). One request per effect
//! per session; a failed load is cached as null and never asked again.
//!
//! The loop runs one frame every 1000/12 ms. This port draws frame 0 only:
//! the React capture screenshots a React loop at whatever frame its timers
//! landed on, and a GPUI loop at its own frames, so the two could never be
//! compared pixel for pixel; a fixed frame makes the comparison honest and
//! the capture's settle probe (two identical frames in a row) can finish.
//! The gallery's non-selected tiles and the chooser's rows already sit on
//! frame 0 in React (the loop runs only for the selected or hovered tile).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::{Bounds, Context, Entity, IntoElement, canvas, div, point, prelude::*, px, size};

use crate::theme::{Theme, TokenRadius};

/// One effect's loop: `w` x `h`, `frames` frames, RGB, frame after frame.
#[derive(Debug, Clone)]
pub struct Thumb {
    pub w: u16,
    pub h: u16,
    #[allow(dead_code)]
    pub frames: u16,
    pub data: Vec<u8>,
}

/// Reads the thumbnail header and checks the body is the size it declares
/// (`parseThumbnail`). None when the bytes are not a loop.
pub fn parse_thumbnail(buf: &[u8]) -> Option<Thumb> {
    if buf.len() < 6 {
        return None;
    }
    let w = u16::from_le_bytes([buf[0], buf[1]]);
    let h = u16::from_le_bytes([buf[2], buf[3]]);
    let frames = u16::from_le_bytes([buf[4], buf[5]]);
    if w == 0 || h == 0 || frames == 0 {
        return None;
    }
    let need = w as usize * h as usize * 3 * frames as usize;
    if buf.len() - 6 < need {
        return None;
    }
    Some(Thumb { w, h, frames, data: buf[6..6 + need].to_vec() })
}

/// Reads a base64 payload (the daemon sends the preview's loop as base64).
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, c) in ALPHABET.iter().enumerate() {
        lookup[*c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 255 {
            return None;
        }
        acc = (acc << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

static CACHE: Mutex<Option<HashMap<String, Option<Arc<Thumb>>>>> = Mutex::new(None);

/// The picture for `id` once it has arrived: the outer `None` means it has
/// not been asked for yet, `Some(None)` a failed load (cached, never asked
/// again).
pub fn cached(id: &str) -> Option<Option<Arc<Thumb>>> {
    CACHE.lock().ok().and_then(|guard| guard.as_ref().and_then(|m| m.get(id).cloned()))
}

/// Stores the picture (or the failure) for `id`.
pub fn store(id: &str, thumb: Option<Thumb>) {
    if let Ok(mut guard) = CACHE.lock() {
        guard
            .get_or_insert_with(HashMap::new)
            .insert(id.to_string(), thumb.map(Arc::new));
    }
}

/// Percent-encodes one path segment, as React's
/// `encodeURIComponent(id)` does (`routes.effectThumbnail`).
/// Stack ids are `stack:Sunset glow`: the raw `:` and the space made the
/// request line invalid and the tile kept "···" while React drew the
/// composed stack picture (effects library with the seeded stack).
pub fn encode_segment(id: &str) -> String {
    const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    let mut out = String::with_capacity(id.len());
    for &b in id.as_bytes() {
        if UNRESERVED.contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// Fetches one effect's thumbnail, off the UI thread, and puts it in the
/// session cache when it lands, telling the view the picture is in.
pub fn fetch<V: 'static>(
    id: String,
    view: Entity<V>,
    cx: &mut Context<V>,
    on_loaded: impl Fn(&mut V, &mut Context<V>, &str) + 'static,
) {
    let path = format!("/api/v1/effects/{}/thumbnail", encode_segment(&id));
    let bg = cx.background_executor().spawn(async move {
        let client = crate::client::DaemonClient::new();
        let result = client.get_bytes(&path);
        (id, result)
    });
    let view_clone = view.clone();
    cx.spawn(async move |_view, cx| {
        let (id, result) = bg.await;
        let thumb = match result {
            Ok(bytes) => parse_thumbnail(&bytes),
            Err(_) => None,
        };
        store(&id, thumb);
        let _ = view_clone.update(cx, |this, cx| on_loaded(this, cx, &id));
    })
    .detach();
}

/// The "···" a tile wears until its picture arrives (or never, when the load
/// failed): mute mono dots centred on the sunk tile.
fn pending_label(t: &Theme) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .font_family(crate::fonts::MONO_FAMILY)
        .text_color(Theme::color(t.mute))
        .child("···")
}

/// A thumbnail box `w` x `h` px: the loop's frame 0 painted with pixelated
/// quads, or the pending label. `thumb` is the cached answer (see
/// [`cached`]); `None` (not asked yet) and `Some(None)` (failed) both draw
/// the pending label, exactly as React's tile keeps "···" when the load
/// failed.
pub fn thumb(
    t: &Theme,
    id: &str,
    w: f32,
    h: f32,
    thumb: Option<Option<Arc<Thumb>>>,
    bordered: bool,
) -> impl IntoElement {
    let _ = id;
    let border = if bordered {
        div().border_1().border_color(Theme::color(t.line_2))
    } else {
        div()
    };
    let _ = border;
    let mut el = div()
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .r_sm()
        .overflow_hidden()
        .bg(Theme::color(t.sunk));
    if bordered {
        el = el.border_1().border_color(Theme::color(t.line_2));
    }
    match thumb {
        Some(Some(data)) => {
            let data = data.clone();
            let canvas_el = canvas(
                move |_, _, _| {},
                move |bounds, _, window, _| {
                    paint_frame(&data, bounds, window, 0);
                },
            )
            .absolute()
            .inset_0();
            el.child(canvas_el)
        }
        _ => el.child(pending_label(t)),
    }
}

/// Paints frame `frame` of the loop into `bounds`, sampling bilinearly the
/// way the browser scales the 64x40 bitmap (React's thumb canvas is scaled
/// with smoothing: a flat block's colour sits between two source pixels).
/// Consecutive equal pixels paint as one rect.
pub fn paint_frame(data: &Thumb, bounds: Bounds<gpui::Pixels>, window: &mut gpui::Window, frame: usize) {
    let bw = f32::from(bounds.size.width);
    let bh = f32::from(bounds.size.height);
    if data.w == 0 || data.h == 0 || bw <= 0.0 || bh <= 0.0 {
        return;
    }
    let frame_start = frame * data.w as usize * data.h as usize * 3;
    let sample = |sx: f32, sy: f32| -> (u8, u8, u8) {
        // Nearest texel, the way the browser scales these tiles (measured:
        // on the Rain tile the nearest scan reads 19.9% against the smooth
        // scan's 27.1%, and 12.26% for the whole 1440x900 pair against
        // 14.79%). Frame 0 is the frozen frame: React loops only the
        // selected tile, which this frame keeps comparable.
        let ix = ((sx * data.w as f32).floor() as usize).min(data.w as usize - 1);
        let iy = ((sy * data.h as f32).floor() as usize).min(data.h as usize - 1);
        let i = frame_start + (iy * data.w as usize + ix) * 3;
        (data.data[i], data.data[i + 1], data.data[i + 2])
    };
    let dst_w = bw.ceil() as usize;
    let dst_h = bh.ceil() as usize;
    for dy in 0..dst_h {
        let sy = (dy as f32 + 0.5) / bh;
        let mut dx = 0;
        while dx < dst_w {
            let rgb = sample((dx as f32 + 0.5) / bw, sy);
            // Extend the run over equal pixels.
            let mut end = dx + 1;
            while end < dst_w && sample((end as f32 + 0.5) / bw, sy) == rgb {
                end += 1;
            }
            let color = gpui::Rgba {
                r: rgb.0 as f32 / 255.0,
                g: rgb.1 as f32 / 255.0,
                b: rgb.2 as f32 / 255.0,
                a: 1.0,
            };
            window.paint_quad(gpui::fill(
                Bounds {
                    origin: point(bounds.origin.x + px(dx as f32), bounds.origin.y + px(dy as f32)),
                    size: size(px((end - dx) as f32), px(1.0)),
                },
                color,
            ));
            dx = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_needs_its_whole_body() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&64u16.to_le_bytes());
        buf.extend_from_slice(&40u16.to_le_bytes());
        buf.extend_from_slice(&24u16.to_le_bytes());
        buf.resize(6 + 64 * 40 * 3 * 24, 7u8);
        let thumb = parse_thumbnail(&buf).expect("a full loop parses");
        assert_eq!((thumb.w, thumb.h, thumb.frames), (64, 40, 24));
        assert_eq!(thumb.data.len(), 64 * 40 * 3 * 24);
        // One byte short is not a loop.
        buf.pop();
        assert!(parse_thumbnail(&buf).is_none());
        assert!(parse_thumbnail(&[]).is_none());
        assert!(parse_thumbnail(&[1, 0, 1, 0, 0, 0]).is_none());
    }

    #[test]
    fn a_base64_payload_reads_back() {
        assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello".to_vec());
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello".to_vec());
        assert_eq!(decode_base64("AAECAw==").unwrap(), vec![0, 1, 2, 3]);
        assert!(decode_base64("!!!").is_none());
    }

    #[test]
    fn a_stack_id_is_one_encoded_segment() {
        // React `routes.effectThumbnail` encodes the whole id: `stack:Sunset glow`
        // reaches the daemon as one segment, not with a raw `:` and a space.
        assert_eq!(encode_segment("stack:Sunset glow"), "stack%3ASunset%20glow");
        assert_eq!(encode_segment("solid"), "solid");
        assert_eq!(encode_segment("html:my-effect"), "html%3Amy-effect");
    }

    #[test]
    fn frame_zero_sits_at_the_front_of_the_data() {        // The loop's first pixel is the first three bytes (React `blit`
        // offsets the frame by `frame * w * h * 3`).
        let mut buf = Vec::new();
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
        // Frame 0: red, green. Frame 1: blue, white.
        buf.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
        let thumb = parse_thumbnail(&buf).expect("a small loop parses");
        assert_eq!(&thumb.data[0..3], &[255, 0, 0]);
        assert_eq!(&thumb.data[6..9], &[0, 0, 255]);
    }
}
