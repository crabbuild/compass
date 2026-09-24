//! Shared presentation palette for every Compass renderer.
//!
//! One palette, one source of truth: the viewer model, the HTML/SVG exports, and
//! the Obsidian export all color a community by its index with the same hue, so
//! the same graph looks the same wherever it is opened.
//!
//! The twelve hues sit in a narrow lightness band (L* 54–60) with moderate
//! chroma, so no community shouts and the map reads as one system. Every entry
//! carries at least 4.4:1 contrast against near-black text and 3.1:1 against
//! white, which keeps labels legible on light and dark canvases without
//! inventing a second palette. Indices alternate hue families so that the two
//! communities most likely to sit side by side never share an adjacent hue.

/// Community fill colors, ordered for maximum separation between neighbours.
pub const COMMUNITY_COLORS: [&str; 12] = [
    "#2788C1", // blue
    "#C4814C", // amber
    "#169483", // cyan
    "#C56775", // rose
    "#8C9047", // moss
    "#8176BA", // plum
    "#48956E", // teal
    "#C9705E", // coral
    "#0093A6", // azure
    "#B1699E", // magenta
    "#AE8C46", // sand
    "#679459", // jade
];

/// The fill color for a community. Index wraps, so a repository with more
/// communities than colors still gets a deterministic, evenly spread hue.
#[must_use]
pub fn community_color(community: usize) -> &'static str {
    COMMUNITY_COLORS[community % COMMUNITY_COLORS.len()]
}

/// A deeper companion to the fill, so bubbles and tiles keep a crisp edge on
/// both a light and a dark canvas.
#[must_use]
pub fn community_border(community: usize) -> String {
    shade(community_color(community), 0.78)
}

/// Multiply an `#RRGGBB` color toward black by `factor` (0.0–1.0). Input that is
/// not a hex color is returned unchanged rather than guessed at.
#[must_use]
pub fn shade(hex: &str, factor: f32) -> String {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return hex.to_owned();
    }
    let channel = |range: std::ops::Range<usize>| {
        let value = u8::from_str_radix(&digits[range], 16).unwrap_or_default();
        let scaled = (f32::from(value) * factor.clamp(0.0, 1.0)).round();
        scaled.clamp(0.0, 255.0) as u8
    };
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(0..2),
        channel(2..4),
        channel(4..6)
    )
}

#[cfg(test)]
mod tests {
    use super::{COMMUNITY_COLORS, community_border, community_color, shade};

    #[test]
    fn palette_entries_are_hex_and_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for color in COMMUNITY_COLORS {
            assert!(
                color.len() == 7
                    && color.starts_with('#')
                    && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit()),
                "community color {color} is not #RRGGBB"
            );
            assert!(seen.insert(color), "community color {color} repeats");
        }
    }

    #[test]
    fn community_colors_wrap_deterministically() {
        assert_eq!(community_color(0), COMMUNITY_COLORS[0]);
        assert_eq!(community_color(COMMUNITY_COLORS.len()), COMMUNITY_COLORS[0]);
        assert_eq!(community_color(COMMUNITY_COLORS.len() + 3), COMMUNITY_COLORS[3]);
    }

    #[test]
    fn borders_are_deeper_than_their_fill() {
        for index in 0..COMMUNITY_COLORS.len() {
            let fill = community_color(index);
            let border = community_border(index);
            assert_eq!(border.len(), 7);
            assert_ne!(border, fill.to_owned());
            assert!(perceived_brightness(&border) < perceived_brightness(fill));
        }
    }

    #[test]
    fn shade_ignores_non_hex_input() {
        assert_eq!(shade("rebeccapurple", 0.5), "rebeccapurple");
        assert_eq!(shade("#12345", 0.5), "#12345");
        assert_eq!(shade("#FF8000", 0.5), "#804000");
        assert_eq!(shade("#FFFFFF", 0.0), "#000000");
    }

    fn perceived_brightness(hex: &str) -> f64 {
        let digits = hex.trim_start_matches('#');
        let channel = |range: std::ops::Range<usize>| {
            u8::from_str_radix(&digits[range], 16).unwrap_or_default() as f64
        };
        0.2126 * channel(0..2) + 0.7152 * channel(2..4) + 0.0722 * channel(4..6)
    }
}
