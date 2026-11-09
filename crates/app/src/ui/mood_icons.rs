//! Small, theme-independent line icons drawn at device resolution.
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::HDC};

pub(crate) fn kind(id: &str, title: &str) -> &'static str {
    let text = format!("{id} {title}").to_lowercase();
    for (kind, terms) in [
        ("happy", &["happy", "mutlu"][..]),
        ("workout", &["workout", "sport", "motivat"][..]),
        ("chill", &["chill", "relax", "rahat"][..]),
        ("focus", &["focus", "odak"][..]),
        ("party", &["party", "parti"][..]),
        ("sad", &["sad", "hüzün", "melanchol"][..]),
        ("romance", &["romance", "love", "aşk"][..]),
        ("sleep", &["sleep", "uyku"][..]),
        ("discover", &["discover", "keşif"][..]),
    ] {
        if terms.iter().any(|term| text.contains(term)) {
            return kind;
        }
    }
    "personal"
}

pub(crate) fn color(kind: &str) -> u32 {
    let (r, g, b) = match kind {
        "happy" | "party" => (244, 199, 69),
        "workout" => (255, 112, 63),
        "chill" => (196, 89, 225),
        "focus" => (111, 126, 255),
        "sad" => (104, 151, 245),
        "romance" | "discover" => (231, 125, 227),
        "sleep" => (160, 137, 240),
        _ => (208, 151, 255),
    };
    r | (g << 8) | (b << 16)
}

pub(crate) unsafe fn draw(dc: HDC, rect: RECT, kind: &str, color: u32) {
    let size = (rect.right - rect.left).min(rect.bottom - rect.top).max(1) as f32;
    let scale = size / 24.;
    let x = rect.left as f32;
    let y = rect.top as f32;
    let ink = crate::drawing::argb(color, 255);
    let line = |points: &[(f32, f32)]| {
        for segment in points.windows(2) {
            crate::drawing::line(
                dc,
                x + segment[0].0 * scale,
                y + segment[0].1 * scale,
                x + segment[1].0 * scale,
                y + segment[1].1 * scale,
                ink,
                1.25 * scale,
            );
        }
    };
    let ring = |cx: f32, cy: f32, radius: f32| {
        let r = RECT {
            left: (x + (cx - radius) * scale) as i32,
            top: (y + (cy - radius) * scale) as i32,
            right: (x + (cx + radius) * scale) as i32,
            bottom: (y + (cy + radius) * scale) as i32,
        };
        crate::drawing::stroke(dc, r, ink, radius * scale, 1.25 * scale);
    };
    match kind {
        "happy" | "sad" => {
            ring(12., 12., 8.);
            line(&[(8., 9.), (9., 8.), (10., 9.)]);
            line(&[(14., 9.), (15., 8.), (16., 9.)]);
            if kind == "happy" {
                line(&[(8., 14.), (10., 16.), (14., 16.), (16., 14.)]);
            } else {
                line(&[(8., 16.), (10., 14.), (14., 14.), (16., 16.)]);
            }
        }
        "workout" => {
            line(&[
                (5., 4.),
                (8., 3.),
                (11., 10.),
                (18., 14.),
                (21., 15.),
                (21., 19.),
                (17., 21.),
                (3., 14.),
                (4., 8.),
                (5., 4.),
            ]);
            line(&[(4., 11.), (11., 15.), (18., 18.), (21., 17.)]);
            line(&[(10., 9.), (13., 8.)]);
            line(&[(12., 11.), (15., 10.)]);
        }
        "chill" => {
            line(&[(12., 9.), (11., 15.), (11., 22.)]);
            line(&[
                (12., 9.),
                (7., 7.),
                (3., 9.),
                (2., 12.),
                (7., 10.),
                (12., 9.),
            ]);
            line(&[
                (12., 9.),
                (17., 7.),
                (21., 10.),
                (22., 13.),
                (17., 10.),
                (12., 9.),
            ]);
            line(&[(12., 9.), (9., 3.), (5., 2.), (6., 5.), (12., 9.)]);
            line(&[(12., 9.), (15., 3.), (19., 3.), (17., 6.), (12., 9.)]);
        }
        "focus" => {
            line(&[
                (12., 20.),
                (8., 15.),
                (9., 8.),
                (12., 3.),
                (15., 8.),
                (16., 15.),
                (12., 20.),
            ]);
            line(&[(12., 20.), (5., 17.), (3., 8.), (8., 11.)]);
            line(&[(12., 20.), (19., 17.), (21., 8.), (16., 11.)]);
            line(&[(12., 20.), (5., 21.), (1., 17.), (5., 17.)]);
            line(&[(12., 20.), (19., 21.), (23., 17.), (19., 17.)]);
        }
        "party" => {
            line(&[(3., 21.), (7., 8.), (16., 17.), (3., 21.)]);
            line(&[(6., 13.), (11., 18.)]);
            line(&[(8., 10.), (15., 17.)]);
            line(&[(13., 10.), (16., 8.), (15., 5.), (18., 3.)]);
            line(&[(16., 13.), (19., 11.), (22., 12.)]);
            line(&[(10., 6.), (11., 2.)]);
            line(&[(20., 5.), (22., 4.)]);
        }
        "romance" => {
            line(&[
                (12., 20.),
                (3., 12.),
                (2., 8.),
                (4., 4.),
                (8., 4.),
                (12., 8.),
                (16., 4.),
                (20., 4.),
                (22., 8.),
                (21., 12.),
                (12., 20.),
            ]);
            line(&[(1., 19.), (23., 8.)]);
            line(&[(19., 7.), (23., 8.), (22., 12.)]);
        }
        "sleep" => {
            line(&[
                (17., 3.),
                (12., 4.),
                (8., 8.),
                (8., 13.),
                (11., 17.),
                (17., 18.),
                (21., 15.),
                (19., 20.),
                (14., 22.),
                (8., 20.),
                (4., 15.),
                (4., 9.),
                (8., 4.),
                (13., 2.),
                (17., 3.),
            ]);
            line(&[(16., 5.), (21., 5.), (16., 10.), (21., 10.)]);
        }
        _ => {
            line(&[(3., 21.), (16., 8.), (19., 11.), (6., 24.)]);
            for (cx, cy) in [(6., 5.), (18., 3.), (21., 17.)] {
                line(&[(cx - 2., cy), (cx + 2., cy)]);
                line(&[(cx, cy - 2.), (cx, cy + 2.)]);
            }
        }
    }
}
