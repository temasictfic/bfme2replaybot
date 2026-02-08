use crate::models::{
    Faction, MapPosition, PLAYER_COLORS, Player, ReplayError, ReplayInfo, Spectator, Winner,
};
use std::collections::{HashMap, HashSet};

const MAGIC: &[u8] = b"BFME2RPL";

// Command types from BFME2 replay format
const CMD_BUILD_OBJECT: u32 = 1049;
const CMD_BUILD_OBJECT_2: u32 = 1050;
const CMD_UNIT_COMMAND: u32 = 1071; // Also has position data
const CMD_END_GAME: u32 = 29;
const CMD_PLAYER_DEFEATED: u32 = 1096;

// Argument type sizes (from OpenSAGE)
const ARG_SIZES: &[(u8, usize)] = &[
    (0x00, 4),  // int32
    (0x01, 4),  // float
    (0x02, 1),  // bool
    (0x03, 4),  // ObjectId
    (0x04, 4),  // unknown4
    (0x05, 8),  // ScreenPosition
    (0x06, 12), // Vec3
    (0x07, 12), // another 12-byte type
    (0x08, 16), // quaternion/camera
    (0x09, 4),  // BFME2-specific
    (0x0A, 4),  // 4 bytes
];

fn get_arg_size(arg_type: u8) -> usize {
    ARG_SIZES
        .iter()
        .find(|(t, _)| *t == arg_type)
        .map(|(_, s)| *s)
        .unwrap_or(4)
}

/// Parsed chunk from replay
#[derive(Debug)]
struct Chunk {
    time_code: u32,
    order_type: u32,
    player_num: u32,
    args: Vec<ChunkArg>,
}

#[derive(Debug)]
enum ChunkArg {
    Int(u32),
    #[allow(dead_code)]
    Float(f32),
    Vec3(f32, f32, f32),
    Other(()),
}

/// Player data from header parsing
#[derive(Debug)]
struct HeaderPlayer {
    name: String,
    uid: Option<String>,
    color_id: i8,
    faction_id: i8,
    team_raw: i8,
    slot: u8,
}

/// Parse a BFME2 replay file and extract game information
pub fn parse_replay(data: &[u8]) -> Result<ReplayInfo, ReplayError> {
    // Verify magic bytes
    if data.len() < MAGIC.len() + 16 || &data[..MAGIC.len()] != MAGIC {
        return Err(ReplayError::InvalidHeader);
    }

    // Check map name FIRST before any heavy parsing (early exit for unsupported maps)
    let map_name = find_map_name(data).ok_or(ReplayError::ParseError(
        "Could not find map name".to_string(),
    ))?;

    // Filter to only "wor rhun" maps
    if !map_name.to_lowercase().contains("wor rhun") {
        return Err(ReplayError::UnsupportedMap(map_name));
    }

    // Parse timestamps from header (offset 8-16)
    let start_time = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let end_time = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);

    // Find and parse players and spectators
    let (mut header_players, spectators, occupied_slots) = find_players_and_spectators(data);

    if header_players.is_empty() {
        return Err(ReplayError::NoPlayers);
    }

    // Build pn_to_slot: game engine assigns pn=3,4,5,... to each occupied slot in order
    let pn_to_slot: HashMap<u32, u8> = occupied_slots
        .iter()
        .enumerate()
        .map(|(i, &slot)| ((i as u32) + 3, slot))
        .collect();

    // Assign colors to players (including random color assignment)
    assign_player_colors(&mut header_players);

    // Build initial players list
    let mut players = build_players(&header_players);

    // Find header end (chunks start after null terminator)
    let chunks_start = find_chunks_start(data);

    // Parse state for streaming chunk processing
    let mut winner = Winner::Unknown;
    let mut game_crashed = false;
    let mut estimated_duration_secs: Option<u32> = None;

    if let Some(start) = chunks_start {
        // Parse chunks for positions, faction detection, and winner
        let parse_result = parse_and_analyze_chunks(data, start, &header_players, &pn_to_slot);

        // Assign positions and actual factions to players
        for player in &mut players {
            if let Some(build) = parse_result.player_builds.get(&player.slot) {
                player.map_position = Some(build.position);
                if let Some(faction) = build.inferred_faction {
                    player.actual_faction = Some(faction);
                }
            }
        }

        // Determine team sides (Left/Right) based on positions
        let team_sides = determine_team_sides(&players);

        // Determine winner
        winner = determine_winner(&parse_result, &header_players, &team_sides, &pn_to_slot);

        // Check for crashed game
        if !parse_result.has_endgame && parse_result.defeated_players.is_empty() {
            game_crashed = true;
            winner = Winner::NotConcluded;
        }

        // Estimate duration from max chunk timecode (SAGE engine ~5 ticks/sec)
        if parse_result.max_timecode > 0 {
            estimated_duration_secs = Some(parse_result.max_timecode / 5);
        }

        // Remap teams to 1/2 based on side
        remap_teams_by_side(&mut players, &team_sides);
    }

    let spectator_list: Vec<Spectator> = spectators
        .into_iter()
        .map(|name| Spectator { name })
        .collect();

    Ok(ReplayInfo::new(map_name, players)
        .with_times(start_time, end_time)
        .with_winner(winner)
        .with_spectators(spectator_list)
        .with_game_crashed(game_crashed)
        .with_estimated_duration(estimated_duration_secs))
}

/// Search for "M=" marker and extract map name
fn find_map_name(data: &[u8]) -> Option<String> {
    let marker = b"M=";

    for i in 0..data.len().saturating_sub(marker.len()) {
        if &data[i..i + marker.len()] == marker {
            let start = i + marker.len();
            let mut end = start;

            while end < data.len() && data[end] != b';' {
                end += 1;
            }

            if end > start {
                let map_path = &data[start..end];
                if let Ok(path_str) = std::str::from_utf8(map_path) {
                    return extract_map_name_from_path(path_str);
                }
            }
        }
    }
    None
}

/// Extract clean map name from path like "385maps/map wor rhun"
fn extract_map_name_from_path(path: &str) -> Option<String> {
    if let Some(idx) = path.find("maps/") {
        let name = &path[idx + 5..];
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    if !path.is_empty() {
        Some(path.to_string())
    } else {
        None
    }
}

/// Decode bytes using Turkish-compatible encodings
fn decode_with_turkish_fallback(bytes: &[u8]) -> String {
    // Try UTF-8 first
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }

    // Try Windows-1254 (Turkish)
    let mut result = String::with_capacity(bytes.len());
    for &b in bytes {
        let c = match b {
            0x80 => '\u{20AC}', // Euro sign
            0x8A => '\u{015E}', // S with cedilla
            0x8C => '\u{0152}', // OE
            0x9A => '\u{015F}', // s with cedilla
            0x9C => '\u{0153}', // oe
            0x9F => '\u{0178}', // Y with diaeresis
            0xC7 => '\u{00C7}', // C with cedilla
            0xD0 => '\u{011E}', // G with breve
            0xDD => '\u{0130}', // I with dot above (Turkish I)
            0xDE => '\u{015E}', // S with cedilla
            0xE7 => '\u{00E7}', // c with cedilla
            0xF0 => '\u{011F}', // g with breve
            0xFD => '\u{0131}', // dotless i (Turkish i)
            0xFE => '\u{015F}', // s with cedilla
            b if b < 0x80 => b as char,
            b => {
                // For other bytes, try Latin-1 interpretation
                char::from_u32(b as u32).unwrap_or('\u{FFFD}')
            }
        };
        result.push(c);
    }
    result
}

/// Find the S= section and parse all players and spectators.
/// Returns (players, spectator_names, occupied_slots) where occupied_slots
/// contains the slot index of every non-empty entry (players AND spectators).
fn find_players_and_spectators(data: &[u8]) -> (Vec<HeaderPlayer>, Vec<String>, Vec<u8>) {
    let mut players = Vec::new();
    let mut spectators = Vec::new();
    let mut occupied_slots = Vec::new();
    let marker = b";S=";

    for i in 0..data.len().saturating_sub(marker.len()) {
        if &data[i..i + marker.len()] == marker {
            let start = i + marker.len();
            let mut end = start;

            while end < data.len() {
                let b = data[end];
                if b == 0 || b == b'\n' || b == b'\r' {
                    break;
                }
                if end + 2 < data.len()
                    && data[end] == b';'
                    && data[end + 1].is_ascii_uppercase()
                    && data[end + 2] == b'='
                {
                    break;
                }
                end += 1;
            }

            if end > start {
                let players_str = decode_with_turkish_fallback(&data[start..end]);

                for (slot_idx, player_str) in players_str.split(':').enumerate() {
                    if let Some(parsed) = parse_player_data(player_str, slot_idx as u8) {
                        occupied_slots.push(slot_idx as u8);
                        if parsed.team_raw >= 0 {
                            players.push(parsed);
                        } else {
                            // Spectator (team_raw is -1)
                            spectators.push(parsed.name);
                        }
                    }
                }
            }

            break;
        }
    }

    (players, spectators, occupied_slots)
}

/// Parse player data from a slot string
/// Format: HName,UID,Port,TT,ColorID,field5,FactionID,Team,field8,field9,field10
/// Returns parsed player data if valid
fn parse_player_data(s: &str, slot: u8) -> Option<HeaderPlayer> {
    let s = s.trim();
    if s.is_empty() || s == "X" || s == "O" || s == ";" {
        return None;
    }

    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 8 {
        return None;
    }

    let mut name = parts[0].to_string();
    if name.starts_with('H') && name.len() > 1 {
        name = name[1..].to_string();
    }

    if name.is_empty() {
        return None;
    }

    // Parse UID (index 1) - 8-char hex string
    let uid = if parts.len() > 1 && parts[1].len() == 8 {
        Some(parts[1].to_string())
    } else {
        None
    };

    // Parse color_id (index 4)
    let color_id: i8 = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(-1);

    // Parse faction_id (index 6)
    let faction_id: i8 = parts.get(6).and_then(|s| s.parse().ok()).unwrap_or(-1);

    // Parse team_raw (index 7)
    let team_raw: i8 = parts.get(7).and_then(|s| s.parse().ok()).unwrap_or(-1);

    Some(HeaderPlayer {
        name,
        uid,
        color_id,
        faction_id,
        team_raw,
        slot,
    })
}

/// Assign colors to players, handling random color assignment
fn assign_player_colors(players: &mut [HeaderPlayer]) {
    let mut used_colors: HashSet<i8> = HashSet::new();

    // First pass: collect used colors
    for player in players.iter() {
        if player.color_id >= 0 {
            used_colors.insert(player.color_id);
        }
    }

    // Find the best gap for random color assignment
    let (gap_start, gap_end, gap_len) = find_best_gap(&used_colors);

    // Determine starting color based on gap size
    let mut next_color = if gap_len >= 3 { gap_start } else { gap_end };

    // Second pass: assign random colors
    // Process in slot order (already sorted by slot)
    for player in players.iter_mut() {
        if player.color_id == -1 {
            // Find next available color
            for offset in 0..10 {
                let color_id = ((next_color as i16 + offset) % 10) as i8;
                if !used_colors.contains(&color_id) {
                    player.color_id = color_id;
                    used_colors.insert(color_id);
                    next_color = (color_id + 1) % 10;
                    break;
                }
            }
        }
    }
}

/// Find the largest contiguous gap in available colors (0-8, excluding 9/white)
fn find_best_gap(used: &HashSet<i8>) -> (i8, i8, i8) {
    let available: Vec<i8> = (0..9).filter(|c| !used.contains(c)).collect();
    if available.is_empty() {
        return (0, 0, 0);
    }

    let mut gaps: Vec<(i8, i8, i8)> = Vec::new();
    let mut current_start = available[0];
    let mut current_end = available[0];

    for i in 1..available.len() {
        if available[i] == available[i - 1] + 1 {
            current_end = available[i];
        } else {
            gaps.push((current_start, current_end, current_end - current_start + 1));
            current_start = available[i];
            current_end = available[i];
        }
    }
    gaps.push((current_start, current_end, current_end - current_start + 1));

    // Sort by length (desc), then by end position (desc) for ties
    gaps.sort_by(|a, b| {
        let len_cmp = b.2.cmp(&a.2);
        if len_cmp == std::cmp::Ordering::Equal {
            b.1.cmp(&a.1)
        } else {
            len_cmp
        }
    });

    gaps.first().cloned().unwrap_or((0, 0, 0))
}

/// Build Player structs from header data
fn build_players(header_players: &[HeaderPlayer]) -> Vec<Player> {
    // Collect unique team values for mapping
    let mut team_raws: Vec<i8> = header_players
        .iter()
        .filter(|p| p.team_raw >= 0)
        .map(|p| p.team_raw)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    team_raws.sort();

    header_players
        .iter()
        .map(|hp| {
            // Map team_raw to 1 or 2
            let team = team_raws
                .iter()
                .position(|&t| t == hp.team_raw)
                .map(|i| (i + 1) as i8)
                .unwrap_or(hp.team_raw + 1);

            // Get color RGB
            let color_rgb = if hp.color_id >= 0 && hp.color_id < 10 {
                PLAYER_COLORS[hp.color_id as usize]
            } else {
                [128, 128, 128]
            };

            Player::with_details(
                hp.name.clone(),
                hp.uid.clone(),
                team,
                hp.team_raw,
                hp.slot,
                Faction::from_id(hp.faction_id),
                hp.color_id,
                color_rgb,
            )
        })
        .collect()
}

/// Find where chunks start (after header null terminator)
fn find_chunks_start(data: &[u8]) -> Option<usize> {
    let s_marker = b";S=";
    for i in 0..data.len().saturating_sub(s_marker.len()) {
        if &data[i..i + s_marker.len()] == s_marker {
            for (j, &byte) in data.iter().enumerate().skip(i) {
                if byte == 0 {
                    return Some(j + 1);
                }
            }
        }
    }
    None
}

/// Build info extracted from commands
#[derive(Debug)]
struct BuildInfo {
    position: MapPosition,
    inferred_faction: Option<Faction>,
}

/// Result of chunk parsing and analysis
struct ChunkParseResult {
    player_builds: HashMap<u8, BuildInfo>,
    player_positions: HashMap<u8, MapPosition>,
    player_building_ids: HashMap<u8, HashSet<u32>>,
    defeated_players: HashSet<u32>,
    endgame_player: Option<u32>,
    endgame_timecode: u32,
    has_endgame: bool,
    max_timecode: u32,
}

/// Parse chunks and analyze for positions, factions, and winner
fn parse_and_analyze_chunks(
    data: &[u8],
    start: usize,
    header_players: &[HeaderPlayer],
    pn_to_slot: &HashMap<u32, u8>,
) -> ChunkParseResult {
    let mut result = ChunkParseResult {
        player_builds: HashMap::new(),
        player_positions: HashMap::new(),
        player_building_ids: HashMap::new(),
        defeated_players: HashSet::new(),
        endgame_player: None,
        endgame_timecode: 0,
        has_endgame: false,
        max_timecode: 0,
    };

    // Separate position tracking: build commands vs unit commands
    let mut build_positions: HashMap<u8, MapPosition> = HashMap::new();
    let mut unit_positions: HashMap<u8, MapPosition> = HashMap::new();

    let mut pos = start;

    while pos < data.len().saturating_sub(13) {
        if let Some((next_pos, chunk)) = parse_chunk(data, pos) {
            result.max_timecode = result.max_timecode.max(chunk.time_code);

            // Map player_num to slot using pn_to_slot (handles empty slot gaps)
            let slot = match pn_to_slot.get(&chunk.player_num) {
                Some(&s) => s,
                None => {
                    pos = next_pos;
                    continue;
                }
            };
            let is_valid_player = header_players.iter().any(|hp| hp.slot == slot);

            // Process position-providing commands (1049, 1050, 1071)
            if is_valid_player
                && (chunk.order_type == CMD_BUILD_OBJECT
                    || chunk.order_type == CMD_BUILD_OBJECT_2
                    || chunk.order_type == CMD_UNIT_COMMAND)
            {
                // Extract position from chunk
                if let Some(pos_data) = extract_position(&chunk) {
                    // Track build and unit positions separately (prefer build later)
                    if chunk.order_type == CMD_BUILD_OBJECT
                        || chunk.order_type == CMD_BUILD_OBJECT_2
                    {
                        build_positions.entry(slot).or_insert(pos_data);
                    } else {
                        unit_positions.entry(slot).or_insert(pos_data);
                    }
                }

                // Extract building ID for faction detection (only from build commands)
                if (chunk.order_type == CMD_BUILD_OBJECT || chunk.order_type == CMD_BUILD_OBJECT_2)
                    && let Some(bid) = extract_building_id(&chunk)
                {
                    result
                        .player_building_ids
                        .entry(slot)
                        .or_default()
                        .insert(bid);
                }
            }

            // Process EndGame command (only from actual players, not spectators)
            // Keep the one with the highest timecode (latest)
            if chunk.order_type == CMD_END_GAME && is_valid_player {
                if !result.has_endgame || chunk.time_code >= result.endgame_timecode {
                    result.endgame_player = Some(chunk.player_num);
                    result.endgame_timecode = chunk.time_code;
                }
                result.has_endgame = true;
            }

            // Process Player Defeated command (only actual players, not spectators)
            if chunk.order_type == CMD_PLAYER_DEFEATED && is_valid_player {
                result.defeated_players.insert(chunk.player_num);
            }

            pos = next_pos;
        } else {
            pos += 1;
        }
    }

    // Merge positions: prefer build positions, fall back to unit positions
    for (slot, pos_data) in &build_positions {
        result.player_positions.insert(*slot, *pos_data);
    }
    for (slot, pos_data) in &unit_positions {
        result.player_positions.entry(*slot).or_insert(*pos_data);
    }

    // Raw binary scan fallback: scan for Order 1096/29 patterns that the chunk
    // parser may have missed due to sync issues.
    // Only include pns that map to actual players (not spectators) to stay
    // consistent with the chunk parser's is_valid_player filter.
    let valid_player_nums: HashSet<u32> = pn_to_slot
        .iter()
        .filter(|&(_, &slot)| header_players.iter().any(|hp| hp.slot == slot))
        .map(|(&pn, _)| pn)
        .collect();
    raw_scan_for_critical_events(data, start, &valid_player_nums, &mut result);

    // Build player_builds from positions and building IDs
    for (slot, position) in &result.player_positions {
        let buildings = result.player_building_ids.get(slot);
        let inferred_faction = buildings.and_then(detect_faction_from_buildings);

        result.player_builds.insert(
            *slot,
            BuildInfo {
                position: *position,
                inferred_faction,
            },
        );
    }

    result
}

/// Extract position (Vec3) from a chunk
fn extract_position(chunk: &Chunk) -> Option<MapPosition> {
    for arg in &chunk.args {
        if let ChunkArg::Vec3(x, y, _z) = arg {
            return Some(MapPosition::new(*x, *y));
        }
    }
    None
}

/// Extract building ID from a chunk
fn extract_building_id(chunk: &Chunk) -> Option<u32> {
    for arg in &chunk.args {
        if let ChunkArg::Int(v) = arg
            && *v > 2000
            && *v < 3000
        {
            return Some(*v);
        }
    }
    None
}

/// Detect faction from a set of building IDs
fn detect_faction_from_buildings(buildings: &HashSet<u32>) -> Option<Faction> {
    for &bid in buildings {
        if let Some(faction) = infer_faction_from_building(bid) {
            return Some(faction);
        }
    }
    None
}

/// Infer faction from building type ID
/// Building ID ranges from render_map.py:
/// - Men: 2622-2720
/// - Elves: 2577-2620
/// - Dwarves: 2541-2575
/// - Goblins: 2151-2185
/// - Isengard: 2060-2090
/// - Mordor: 2130-2150
fn infer_faction_from_building(building_type: u32) -> Option<Faction> {
    match building_type {
        2622..=2720 => Some(Faction::Men),
        2577..=2620 => Some(Faction::Elves),
        2541..=2575 => Some(Faction::Dwarves),
        2151..=2185 => Some(Faction::Goblins),
        2060..=2090 => Some(Faction::Isengard),
        2130..=2150 => Some(Faction::Mordor),
        _ => None,
    }
}

/// Parse a single chunk from the data
fn parse_chunk(data: &[u8], offset: usize) -> Option<(usize, Chunk)> {
    if offset + 13 > data.len() {
        return None;
    }

    let time_code = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    let order_type = u32::from_le_bytes([
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]);
    let player_num = u32::from_le_bytes([
        data[offset + 8],
        data[offset + 9],
        data[offset + 10],
        data[offset + 11],
    ]);
    let n_arg_types = data[offset + 12] as usize;

    // Sanity checks
    if time_code > 10_000_000 || player_num > 100 || n_arg_types > 100 {
        return None;
    }

    let mut pos = offset + 13;

    // Read argument signature
    let mut arg_sig = Vec::new();
    for _ in 0..n_arg_types {
        if pos + 2 > data.len() {
            return None;
        }
        let arg_type = data[pos];
        let arg_count = data[pos + 1] as usize;
        if arg_count > 50 {
            return None;
        }
        arg_sig.push((arg_type, arg_count));
        pos += 2;
    }

    // Read arguments
    let mut args = Vec::new();
    for (arg_type, arg_count) in arg_sig {
        let size = get_arg_size(arg_type);
        for _ in 0..arg_count {
            if pos + size > data.len() {
                return None;
            }
            let arg_data = &data[pos..pos + size];

            let arg = match arg_type {
                0x06 => {
                    // Vec3
                    let x =
                        f32::from_le_bytes([arg_data[0], arg_data[1], arg_data[2], arg_data[3]]);
                    let y =
                        f32::from_le_bytes([arg_data[4], arg_data[5], arg_data[6], arg_data[7]]);
                    let z =
                        f32::from_le_bytes([arg_data[8], arg_data[9], arg_data[10], arg_data[11]]);
                    ChunkArg::Vec3(x, y, z)
                }
                0x00 => {
                    let v =
                        u32::from_le_bytes([arg_data[0], arg_data[1], arg_data[2], arg_data[3]]);
                    ChunkArg::Int(v)
                }
                0x01 => {
                    let v =
                        f32::from_le_bytes([arg_data[0], arg_data[1], arg_data[2], arg_data[3]]);
                    ChunkArg::Float(v)
                }
                _ => ChunkArg::Other(()),
            };
            args.push(arg);

            pos += size;
        }
    }

    Some((
        pos,
        Chunk {
            time_code,
            order_type,
            player_num,
            args,
        },
    ))
}

/// Raw binary scan for critical events (Order 1096 = PlayerDefeated, Order 29 = EndGame).
/// The chunk parser can lose sync and miss events. This scans raw bytes for the order
/// patterns and validates context (timecode, player_num) to recover missed events.
///
/// Single-pass O(n) scanner: iterates each byte once, checking for first-byte matches
/// of each pattern then verifying remaining bytes.
fn raw_scan_for_critical_events(
    data: &[u8],
    chunks_start: usize,
    valid_player_nums: &HashSet<u32>,
    result: &mut ChunkParseResult,
) {
    // Pattern first bytes for quick check
    const DEFEATED_FIRST: u8 = 0x48; // 1096 LE first byte
    const ENDGAME_FIRST: u8 = 0x1d; // 29 LE first byte
    const DEFEATED_REST: [u8; 3] = [0x04, 0x00, 0x00];
    const ENDGAME_REST: [u8; 3] = [0x00, 0x00, 0x00];

    // The order field is at chunk_offset + 4, so we need at least 4 bytes before
    // the match and 13 bytes total from chunk_offset
    if data.len() < chunks_start + 8 {
        return;
    }

    let end = data.len() - 3; // need 4 bytes for pattern match
    let mut i = chunks_start;
    while i < end {
        let b = data[i];

        let cmd = if b == DEFEATED_FIRST && data[i + 1..i + 4] == DEFEATED_REST {
            Some(CMD_PLAYER_DEFEATED)
        } else if b == ENDGAME_FIRST && data[i + 1..i + 4] == ENDGAME_REST {
            Some(CMD_END_GAME)
        } else {
            None
        };

        if let Some(cmd) = cmd {
            // The order field is at chunk_offset + 4, so chunk_offset = i - 4
            if i >= chunks_start + 4 {
                let chunk_offset = i - 4;
                if chunk_offset + 13 <= data.len() {
                    let tc = u32::from_le_bytes([
                        data[chunk_offset],
                        data[chunk_offset + 1],
                        data[chunk_offset + 2],
                        data[chunk_offset + 3],
                    ]);
                    let player_num = u32::from_le_bytes([
                        data[chunk_offset + 8],
                        data[chunk_offset + 9],
                        data[chunk_offset + 10],
                        data[chunk_offset + 11],
                    ]);
                    let n_args = data[chunk_offset + 12] as u32;

                    let tc_valid = tc > 0 && tc < 10_000_000;
                    let pn_valid = (3..=20).contains(&player_num);
                    let nargs_valid = n_args <= 10;

                    if tc_valid
                        && pn_valid
                        && nargs_valid
                        && valid_player_nums.contains(&player_num)
                    {
                        if cmd == CMD_PLAYER_DEFEATED {
                            result.defeated_players.insert(player_num);
                        } else if cmd == CMD_END_GAME {
                            // Keep the latest EndGame by timecode
                            if !result.has_endgame || tc >= result.endgame_timecode {
                                result.endgame_player = Some(player_num);
                                result.endgame_timecode = tc;
                            }
                            result.has_endgame = true;
                        }
                    }
                }
            }
        }

        i += 1;
    }
}

/// Determine which team is on which side based on player positions
fn determine_team_sides(players: &[Player]) -> HashMap<i8, &'static str> {
    let mut team_sides: HashMap<i8, &'static str> = HashMap::new();

    for player in players {
        if let Some(pos) = &player.map_position
            && pos.is_valid()
            && !team_sides.contains_key(&player.team_raw)
        {
            let side = if pos.x < 2500.0 { "Left" } else { "Right" };
            team_sides.insert(player.team_raw, side);
        }
    }

    team_sides
}

/// Remap team numbers based on side (Left = 1, Right = 2)
fn remap_teams_by_side(players: &mut [Player], team_sides: &HashMap<i8, &'static str>) {
    for player in players.iter_mut() {
        if let Some(&side) = team_sides.get(&player.team_raw) {
            player.team = if side == "Left" { 1 } else { 2 };
        }
    }
}

/// Determine winner based on game events
fn determine_winner(
    parse_result: &ChunkParseResult,
    header_players: &[HeaderPlayer],
    team_sides: &HashMap<i8, &'static str>,
    pn_to_slot: &HashMap<u32, u8>,
) -> Winner {
    // Primary: Order 29 = EndGame command indicates winner's team
    if let Some(endgame_pn) = parse_result.endgame_player {
        let endgame_slot = match pn_to_slot.get(&endgame_pn) {
            Some(&s) => s,
            None => return Winner::Unknown,
        };
        if let Some(hp) = header_players.iter().find(|hp| hp.slot == endgame_slot)
            && let Some(&side) = team_sides.get(&hp.team_raw)
        {
            return if side == "Left" {
                Winner::LeftTeam
            } else {
                Winner::RightTeam
            };
        }
    }

    // Build reverse mapping: slot_to_pn
    let slot_to_pn: HashMap<u8, u32> = pn_to_slot.iter().map(|(&pn, &slot)| (slot, pn)).collect();

    // Group players by team (used by both fallback methods)
    let mut team_players: HashMap<i8, Vec<u32>> = HashMap::new();
    for hp in header_players {
        if let Some(&pn) = slot_to_pn.get(&hp.slot) {
            team_players.entry(hp.team_raw).or_default().push(pn);
        }
    }

    if !parse_result.defeated_players.is_empty() {
        // Fallback 1: Check if all players from one team are defeated
        for (team_raw, players_pn) in &team_players {
            let all_defeated = players_pn
                .iter()
                .all(|pn| parse_result.defeated_players.contains(pn));
            if all_defeated {
                // This team lost, the other team won
                for other_team_raw in team_players.keys() {
                    if other_team_raw != team_raw
                        && let Some(&side) = team_sides.get(other_team_raw)
                    {
                        return if side == "Left" {
                            Winner::LeftTeam
                        } else {
                            Winner::RightTeam
                        };
                    }
                }
            }
        }

        // Fallback 2: Majority-defeated heuristic
        // The team with MORE defeated players is the likely loser.
        // Not certain - one player CAN take out multiple enemies.
        if team_players.len() == 2 {
            let teams: Vec<i8> = team_players.keys().cloned().collect();
            let team_a = teams[0];
            let team_b = teams[1];

            let defeats_a = team_players[&team_a]
                .iter()
                .filter(|pn| parse_result.defeated_players.contains(pn))
                .count();
            let defeats_b = team_players[&team_b]
                .iter()
                .filter(|pn| parse_result.defeated_players.contains(pn))
                .count();

            if defeats_a > defeats_b {
                // Team A has more defeats -> Team B likely won
                if let Some(&side) = team_sides.get(&team_b) {
                    return if side == "Left" {
                        Winner::LikelyLeftTeam
                    } else {
                        Winner::LikelyRightTeam
                    };
                }
            } else if defeats_b > defeats_a {
                // Team B has more defeats -> Team A likely won
                if let Some(&side) = team_sides.get(&team_a) {
                    return if side == "Left" {
                        Winner::LikelyLeftTeam
                    } else {
                        Winner::LikelyRightTeam
                    };
                }
            }
        }
    }

    Winner::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_map_name() {
        assert_eq!(
            extract_map_name_from_path("385maps/map wor rhun"),
            Some("map wor rhun".to_string())
        );
        assert_eq!(
            extract_map_name_from_path("maps/fords of isen"),
            Some("fords of isen".to_string())
        );
    }

    #[test]
    fn test_parse_player_data() {
        let player = parse_player_data("HGusto,1A53EFD5,8094,TT,2,-1,1,1,0,1,0", 0).unwrap();
        assert_eq!(player.name, "Gusto");
        assert_eq!(player.uid, Some("1A53EFD5".to_string()));
        assert_eq!(player.color_id, 2);
        assert_eq!(player.faction_id, 1);
        assert_eq!(player.team_raw, 1);
    }

    #[test]
    fn test_skip_empty_slot() {
        assert!(parse_player_data("X", 0).is_none());
        assert!(parse_player_data("O", 0).is_none());
    }

    #[test]
    fn test_infer_faction_from_building() {
        assert_eq!(infer_faction_from_building(2650), Some(Faction::Men));
        assert_eq!(infer_faction_from_building(2600), Some(Faction::Elves));
        assert_eq!(infer_faction_from_building(2550), Some(Faction::Dwarves));
        assert_eq!(infer_faction_from_building(2160), Some(Faction::Goblins));
        assert_eq!(infer_faction_from_building(2070), Some(Faction::Isengard));
        assert_eq!(infer_faction_from_building(2140), Some(Faction::Mordor));
    }

    #[test]
    fn test_find_best_gap() {
        let mut used = HashSet::new();
        used.insert(0);
        used.insert(1);
        // Available: 2,3,4,5,6,7,8 - gap from 2 to 8
        let (start, end, len) = find_best_gap(&used);
        assert_eq!(start, 2);
        assert_eq!(end, 8);
        assert_eq!(len, 7);
    }

    #[test]
    fn test_turkish_decode() {
        // Test that Turkish characters are handled
        let turkish_bytes = b"Test\xDD\xFD"; // I with dot, dotless i in Windows-1254
        let decoded = decode_with_turkish_fallback(turkish_bytes);
        assert!(decoded.contains("Test"));
    }
}
