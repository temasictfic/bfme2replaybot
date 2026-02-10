use crate::models::{Player, ReplayInfo, Winner};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use imageproc::drawing::draw_text_mut;
use std::path::Path;

/// Load and prepare a map image from the assets directory (call once at startup)
pub fn load_map_image(map_name: &str, assets_path: &Path) -> Result<RgbImage, String> {
    let map_path_jpg = assets_path.join("maps").join(format!("{}.jpg", map_name));
    let map_path_png = assets_path.join("maps").join(format!("{}.png", map_name));

    let img = if map_path_jpg.exists() {
        image::open(&map_path_jpg)
            .map(|img| img.to_rgb8())
            .map_err(|e| format!("Failed to load map image: {}", e))?
    } else if map_path_png.exists() {
        image::open(&map_path_png)
            .map(|img| img.to_rgb8())
            .map_err(|e| format!("Failed to load map image: {}", e))?
    } else {
        return Err(format!("Map image not found: {}", map_name));
    };

    // Resize to ~1000px for output if larger
    let (w, h) = (img.width(), img.height());
    if w > 1000 || h > 1000 {
        let scale = 1000.0 / w.max(h) as f32;
        let new_w = (w as f32 * scale) as u32;
        let new_h = (h as f32 * scale) as u32;
        Ok(image::imageops::resize(
            &img,
            new_w,
            new_h,
            image::imageops::FilterType::Lanczos3,
        ))
    } else {
        Ok(img)
    }
}

/// Parse font data into a FontArc (call once at startup, then share across renders)
pub fn load_font(font_data: &[u8]) -> Result<FontArc, String> {
    FontArc::try_from_vec(font_data.to_vec()).map_err(|e| format!("Failed to parse font: {}", e))
}

/// Circle center coordinates in pixels on the original 1624x1620 map asset.
/// At render time these are scaled to match the actual (resized) image dimensions.
const MAP_ASSET_WIDTH: f32 = 1624.0;
const MAP_ASSET_HEIGHT: f32 = 1620.0;

// Map position thresholds (game world coordinates)
const MAP_X_MIDPOINT: f32 = 2500.0;
const MAP_Y_TOP_THRESHOLD: f32 = 3000.0;
const MAP_Y_MID_THRESHOLD: f32 = 1500.0;

/// Map positions for player placement
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    TopLeft,
    MidLeft,
    BottomLeft,
    TopRight,
    MidRight,
    BottomRight,
}

impl Position {
    /// Get pixel coordinates on the original map asset for this position
    fn coords(self) -> (f32, f32) {
        match self {
            Position::TopLeft => (272.0, 336.0),
            Position::MidLeft => (198.0, 896.0),
            Position::BottomLeft => (344.0, 1370.0),
            Position::TopRight => (1330.0, 336.0),
            Position::MidRight => (1370.0, 850.0),
            Position::BottomRight => (1314.0, 1420.0),
        }
    }
}

/// Get position from game world coordinates
fn get_position(x: f32, y: f32) -> Position {
    let is_left = x < MAP_X_MIDPOINT;
    if y > MAP_Y_TOP_THRESHOLD {
        if is_left {
            Position::TopLeft
        } else {
            Position::TopRight
        }
    } else if y > MAP_Y_MID_THRESHOLD {
        if is_left {
            Position::MidLeft
        } else {
            Position::MidRight
        }
    } else if is_left {
        Position::BottomLeft
    } else {
        Position::BottomRight
    }
}

/// Draw a semi-transparent rectangle (alpha blending on RGB image)
fn draw_rect_alpha(img: &mut RgbImage, x: i32, y: i32, w: i32, h: i32, color: [u8; 4]) {
    let a = color[3] as f32 / 255.0;
    let inv_a = 1.0 - a;
    let src_r = color[0] as f32 * a;
    let src_g = color[1] as f32 * a;
    let src_b = color[2] as f32 * a;

    for py in y.max(0)..((y + h).min(img.height() as i32)) {
        for px in x.max(0)..((x + w).min(img.width() as i32)) {
            let pixel = img.get_pixel_mut(px as u32, py as u32);
            pixel[0] = (pixel[0] as f32 * inv_a + src_r) as u8;
            pixel[1] = (pixel[1] as f32 * inv_a + src_g) as u8;
            pixel[2] = (pixel[2] as f32 * inv_a + src_b) as u8;
        }
    }
}

/// Measure text width using actual glyph advance widths from the font
fn measure_text_width(text: &str, font: &FontArc, scale: PxScale) -> i32 {
    let scaled = font.as_scaled(scale);
    text.chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum::<f32>() as i32
}

/// Render a map visualization with player positions
pub fn render_map(
    replay: &ReplayInfo,
    font: &FontArc,
    map_image: &RgbImage,
    filename: &str,
) -> Result<Vec<u8>, String> {
    let mut img = map_image.clone();

    // Font sizes
    let font_large = PxScale::from(24.0);
    let font_small = PxScale::from(20.0);

    // Draw player info at each position (text only, no circles)
    for player in &replay.players {
        draw_player_text(&mut img, player, font, font_large, font_small);
    }

    // Draw centered info (Filename, Date, Duration, Winner)
    draw_center_info(&mut img, replay, font, font_large, filename);

    // Draw spectators if any
    draw_spectators(&mut img, replay, font, font_small);

    // Encode directly to JPEG with quality 85 (already RGB, no conversion needed)
    let mut buffer = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buffer);

    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 85);
    encoder
        .encode(
            &img,
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("Failed to encode image: {}", e))?;

    Ok(buffer)
}

/// Draw player text at their position (center-aligned)
fn draw_player_text(
    img: &mut RgbImage,
    player: &Player,
    font: &FontArc,
    font_large: PxScale,
    font_small: PxScale,
) {
    let (width, height) = (img.width() as f32, img.height() as f32);
    let scale_x = width / MAP_ASSET_WIDTH;
    let scale_y = height / MAP_ASSET_HEIGHT;

    // Get position from map coordinates
    let img_pos = if let Some(pos) = &player.map_position {
        if pos.is_valid() {
            Some(get_position(pos.x, pos.y).coords())
        } else {
            None
        }
    } else {
        None
    };

    let img_pos = match img_pos {
        Some(p) => p,
        None => return, // Skip players without valid positions
    };

    // Circle center in rendered image pixels
    let center_x = (img_pos.0 * scale_x) as i32;
    let center_y = (img_pos.1 * scale_y) as i32;

    // Get player color
    let color = player.display_color();
    let text_color = Rgb([color[0], color[1], color[2]]);

    // Truncate name to 12 chars
    let name: String = player.name.chars().take(12).collect();

    let pad = 3;
    let name_h = 24;
    let faction_h = 20;
    let gap = 2; // gap between name and faction rows
    let total_h = name_h + gap + faction_h;

    // Vertically center the two-line block on circle center
    let block_top = center_y - total_h / 2;

    // --- Name (top row, centered horizontally) ---
    let name_w = measure_text_width(&name, font, font_large);
    let name_x = center_x - name_w / 2;
    let name_y = block_top;

    draw_rect_alpha(
        img,
        name_x - pad,
        name_y - 2,
        name_w + pad * 2,
        name_h + 4,
        [0, 0, 0, 180],
    );

    draw_text_mut(img, text_color, name_x, name_y, font_large, font, &name);

    // --- Faction (bottom row, centered horizontally) ---
    let faction_text = player.display_faction().to_string();
    let faction_w = measure_text_width(&faction_text, font, font_small);
    let faction_x = center_x - faction_w / 2;
    let faction_y = block_top + name_h + gap;

    draw_rect_alpha(
        img,
        faction_x - pad,
        faction_y - 2,
        faction_w + pad * 2,
        faction_h + 4,
        [0, 0, 0, 180],
    );

    draw_text_mut(
        img,
        text_color,
        faction_x,
        faction_y,
        font_small,
        font,
        &faction_text,
    );
}

/// Draw centered info (Filename, Date, Duration, Winner)
fn draw_center_info(
    img: &mut RgbImage,
    replay: &ReplayInfo,
    font: &FontArc,
    scale: PxScale,
    filename: &str,
) {
    let (width, height) = (img.width() as i32, img.height() as i32);
    let center_x = width / 2;
    let center_y = height / 2;

    // Format filename: strip extension (case-insensitive), cap at 30 chars
    let display_name = match filename.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("BfME2Replay") => stem,
        _ => filename,
    };
    let display_name: String = display_name.chars().take(30).collect();

    // Format info text
    let date_text = format!("Date: {}", replay.start_date_formatted());
    let duration_text = format!("Duration: {}", replay.duration_formatted());

    // Build info lines
    let mut info_lines: Vec<(&str, Rgb<u8>)> = vec![
        (&display_name, Rgb([255, 255, 255])),
        (&date_text, Rgb([200, 200, 200])),
        (&duration_text, Rgb([200, 200, 200])),
    ];

    // Only show winner if known
    let winner_text = if replay.game_crashed {
        Some(("Winner: Not Concluded".to_string(), Rgb([200, 100, 100])))
    } else if replay.winner == Winner::LikelyLeftTeam || replay.winner == Winner::LikelyRightTeam {
        Some((
            format!("Winner: {}", replay.winner.display_text()),
            Rgb([255, 200, 80]),
        ))
    } else if replay.winner != Winner::Unknown {
        Some((
            format!("Winner: {}", replay.winner.display_text()),
            Rgb([255, 215, 0]),
        ))
    } else {
        None
    };
    if let Some((ref text, color)) = winner_text {
        info_lines.push((text, color));
    }

    let line_height = 28;
    let total_height = (info_lines.len() as i32) * line_height;
    let start_y = center_y - total_height / 2;

    // Calculate max width for background using accurate measurement
    let max_width = info_lines
        .iter()
        .map(|(text, _)| measure_text_width(text, font, scale))
        .max()
        .unwrap_or(0);

    // Draw background rectangle
    let padding = 10;
    draw_rect_alpha(
        img,
        center_x - max_width / 2 - padding,
        start_y - padding,
        max_width + padding * 2,
        total_height + padding * 2,
        [0, 0, 0, 160],
    );

    // Draw info text (centered)
    for (i, (text, color)) in info_lines.iter().enumerate() {
        let text_w = measure_text_width(text, font, scale);
        let text_x = center_x - text_w / 2;
        let text_y = start_y + (i as i32) * line_height;
        draw_text_mut(img, *color, text_x, text_y, scale, font, text);
    }
}

/// Draw spectators above and below center
fn draw_spectators(img: &mut RgbImage, replay: &ReplayInfo, font: &FontArc, scale: PxScale) {
    if replay.spectators.is_empty() {
        return;
    }

    let (width, height) = (img.width() as i32, img.height() as i32);
    let center_x = width / 2;
    let spectator_color = Rgb([180, 180, 180]);

    // First spectator near top
    {
        let spec_y = (height as f32 * 0.08) as i32;
        let spec_text = format!("Obs: {}", replay.spectators[0].name);
        let spec_w = measure_text_width(&spec_text, font, scale);
        let spec_x = center_x - spec_w / 2;

        draw_rect_alpha(img, spec_x - 3, spec_y - 2, spec_w + 6, 24, [0, 0, 0, 160]);
        draw_text_mut(
            img,
            spectator_color,
            spec_x,
            spec_y,
            scale,
            font,
            &spec_text,
        );
    }

    // Second spectator near bottom
    if replay.spectators.len() >= 2 {
        let spec_y = (height as f32 * 0.92) as i32;
        let spec_text = format!("Obs: {}", replay.spectators[1].name);
        let spec_w = measure_text_width(&spec_text, font, scale);
        let spec_x = center_x - spec_w / 2;

        draw_rect_alpha(img, spec_x - 3, spec_y - 2, spec_w + 6, 24, [0, 0, 0, 160]);
        draw_text_mut(
            img,
            spectator_color,
            spec_x,
            spec_y,
            scale,
            font,
            &spec_text,
        );
    }
}
