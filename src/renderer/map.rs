use crate::models::{Player, ReplayInfo};
use ab_glyph::{FontRef, PxScale};
use image::{ImageBuffer, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_text_mut};
use std::path::Path;

/// Default map size if no image found
const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 600;

/// Player marker radius
const MARKER_RADIUS: i32 = 20;

/// Embedded font (DejaVu Sans Mono - open source)
const FONT_DATA: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");

/// Position coordinates on the map (normalized 0.0-1.0)
/// Positions are arranged in a circle like clock positions
const POSITION_COORDS: [(f32, f32); 8] = [
    (0.5, 0.15),  // Position 0: Top center (12 o'clock)
    (0.85, 0.25), // Position 1: Top right (1-2 o'clock)
    (0.85, 0.5),  // Position 2: Right (3 o'clock)
    (0.85, 0.75), // Position 3: Bottom right (4-5 o'clock)
    (0.5, 0.85),  // Position 4: Bottom center (6 o'clock)
    (0.15, 0.75), // Position 5: Bottom left (7-8 o'clock)
    (0.15, 0.5),  // Position 6: Left (9 o'clock)
    (0.15, 0.25), // Position 7: Top left (10-11 o'clock)
];

/// Team colors
const TEAM_COLORS: [[u8; 4]; 4] = [
    [65, 105, 225, 255],  // Team 1: Royal Blue
    [220, 20, 60, 255],   // Team 2: Crimson
    [50, 205, 50, 255],   // Team 3: Lime Green
    [255, 215, 0, 255],   // Team 4: Gold
];

/// Render a map visualization with player positions
pub fn render_map(replay: &ReplayInfo, assets_path: &Path) -> Result<Vec<u8>, String> {
    // Try to load the map image
    let mut img = load_map_image(replay, assets_path)?;

    // Load font
    let font = FontRef::try_from_slice(FONT_DATA)
        .map_err(|e| format!("Failed to load font: {}", e))?;

    let scale = PxScale::from(16.0);

    // Draw player markers
    for player in &replay.players {
        draw_player_marker(&mut img, player, &font, scale);
    }

    // Draw legend/info box
    draw_info_box(&mut img, replay, &font, scale);

    // Encode to PNG
    let mut buffer = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buffer);

    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode image: {}", e))?;

    Ok(buffer)
}

/// Load map image or create a default one
fn load_map_image(replay: &ReplayInfo, assets_path: &Path) -> Result<RgbaImage, String> {
    let map_path = assets_path
        .join("maps")
        .join(format!("{}.png", replay.map_name));

    // Also try jpg extension
    let map_path_jpg = assets_path
        .join("maps")
        .join(format!("{}.jpg", replay.map_name));

    if map_path.exists() {
        image::open(&map_path)
            .map(|img| img.to_rgba8())
            .map_err(|e| format!("Failed to load map image: {}", e))
    } else if map_path_jpg.exists() {
        image::open(&map_path_jpg)
            .map(|img| img.to_rgba8())
            .map_err(|e| format!("Failed to load map image: {}", e))
    } else {
        // Create default background
        Ok(create_default_map())
    }
}

/// Create a default map image with a grid pattern
fn create_default_map() -> RgbaImage {
    let mut img = ImageBuffer::from_pixel(DEFAULT_WIDTH, DEFAULT_HEIGHT, Rgba([40, 60, 40, 255]));

    // Draw grid lines
    let grid_color = Rgba([60, 80, 60, 255]);
    let grid_spacing = 50;

    for x in (0..DEFAULT_WIDTH).step_by(grid_spacing) {
        for y in 0..DEFAULT_HEIGHT {
            img.put_pixel(x, y, grid_color);
        }
    }

    for y in (0..DEFAULT_HEIGHT).step_by(grid_spacing) {
        for x in 0..DEFAULT_WIDTH {
            img.put_pixel(x, y, grid_color);
        }
    }

    img
}

/// Draw a player marker at their position
fn draw_player_marker(img: &mut RgbaImage, player: &Player, font: &FontRef, scale: PxScale) {
    let (width, height) = (img.width() as f32, img.height() as f32);

    // Get position coordinates
    let pos_idx = (player.position as usize) % POSITION_COORDS.len();
    let (norm_x, norm_y) = POSITION_COORDS[pos_idx];

    let x = (norm_x * width) as i32;
    let y = (norm_y * height) as i32;

    // Get team color
    let team_color = if player.team > 0 && player.team <= 4 {
        TEAM_COLORS[(player.team - 1) as usize]
    } else {
        [128, 128, 128, 255] // Gray for unknown team
    };

    // Draw outer circle (team color)
    draw_filled_circle_mut(img, (x, y), MARKER_RADIUS, Rgba(team_color));

    // Draw inner circle (faction color)
    let faction_color = player.faction.color();
    draw_filled_circle_mut(
        img,
        (x, y),
        MARKER_RADIUS - 5,
        Rgba([faction_color[0], faction_color[1], faction_color[2], 255]),
    );

    // Draw player name
    let name = if player.name.len() > 12 {
        format!("{}...", &player.name[..10])
    } else {
        player.name.clone()
    };

    draw_text_mut(
        img,
        Rgba([255, 255, 255, 255]),
        x + MARKER_RADIUS + 5,
        y - 8,
        scale,
        font,
        &name,
    );
}

/// Draw info box with game details
fn draw_info_box(img: &mut RgbaImage, replay: &ReplayInfo, font: &FontRef, scale: PxScale) {
    // Draw semi-transparent background
    let box_x = 10;
    let box_y = 10;
    let box_width = 250;
    let line_height = 20;
    let box_height = (30 + replay.players.len() * line_height as usize) as u32;

    // Draw box background
    for x in box_x..box_x + box_width {
        for y in box_y..box_y + box_height {
            if x < img.width() && y < img.height() {
                let pixel = img.get_pixel_mut(x, y);
                pixel[0] = (pixel[0] as u16 / 2) as u8;
                pixel[1] = (pixel[1] as u16 / 2) as u8;
                pixel[2] = (pixel[2] as u16 / 2) as u8;
                pixel[3] = 200;
            }
        }
    }

    // Draw map name
    let map_title = format!("Map: {}", replay.map_name);
    draw_text_mut(
        img,
        Rgba([255, 255, 255, 255]),
        box_x as i32 + 5,
        box_y as i32 + 5,
        scale,
        font,
        &map_title,
    );

    // Draw player list
    for (i, player) in replay.players.iter().enumerate() {
        let y_pos = box_y as i32 + 25 + (i as i32 * line_height as i32);
        let faction_color = player.faction.color();
        let text = format!(
            "{} - {} (Team {})",
            player.name, player.faction, player.team
        );

        draw_text_mut(
            img,
            Rgba([faction_color[0], faction_color[1], faction_color[2], 255]),
            box_x as i32 + 5,
            y_pos,
            scale,
            font,
            &text,
        );
    }
}
