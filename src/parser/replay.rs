use crate::models::{
    Faction, MapPosition, PLAYER_COLORS, Player, PlayerBuilder, ReplayError, ReplayInfo, Spectator,
    Winner,
};
use std::collections::{HashMap, HashSet};

const MAGIC: &[u8] = b"BFME2RPL";

// Command types from BFME2 replay format
const CMD_BUILD_OBJECT: u32 = 1049;
const CMD_BUILD_OBJECT_2: u32 = 1050;
const CMD_UNIT_COMMAND: u32 = 1071; // Also has position data
const CMD_END_GAME: u32 = 29;
const CMD_PLAYER_DEFEATED: u32 = 1096;

// Sanity limits for chunk parsing
const MAX_SANE_TIMECODE: u32 = 10_000_000;
const MAX_SANE_PLAYER_NUM: u32 = 100;
const MAX_SANE_ARG_TYPES: usize = 100;
const MAX_SANE_ARG_COUNT: usize = 50;

// Map position threshold (game world coordinates)
const MAP_X_MIDPOINT: f32 = 2500.0;

// SAGE engine tick rate (~5 ticks per second)
const SAGE_TICKS_PER_SECOND: u32 = 5;

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
    /// Raw value of the 5th comma-separated field in the `S=` slot entry.
    /// `-2` means observer/spectator; `-1` means random start position;
    /// `0..7` is a chosen start position. Kept for diagnostic and future use.
    #[allow(dead_code)]
    startpos_raw: i8,
}

/// Result of a single-pass header parse
struct HeaderParseResult {
    map_name: String,
    players: Vec<HeaderPlayer>,
    spectators: Vec<String>,
    occupied_slots: Vec<u8>,
    chunks_start: Option<usize>,
    /// Replay seed (the `SD=` header field). Used to deterministically reproduce
    /// the game's random-color assignment.
    sd: u32,
    /// `(slot_index, color_id)` for each spectator/observer, in slot order.
    /// Needed to accurately simulate PRNG consumption during color assignment
    /// (observers consume one rand(0, num_starts-1) call for Phase 1 StartPos
    /// and one rand(0, num_colors-1) retry loop for Phase 2 Color if their
    /// color_id is -1).
    observer_slots: Vec<(u8, i8)>,
}

/// Parse the header in a single pass: extract map name, players/spectators,
/// and locate the chunks start offset.
/// The binary preamble may contain null bytes before the text section,
/// so we search the full buffer for M= and ;S= markers.
fn parse_header(data: &[u8]) -> Result<HeaderParseResult, ReplayError> {
    // Search full data for map name (text section position is variable)
    let map_name = find_map_name_in(data).ok_or(ReplayError::ParseError(
        "Could not find map name".to_string(),
    ))?;

    // Search full data for players/spectators
    let SlotScan {
        players,
        spectators,
        occupied_slots,
        observer_slots,
    } = find_players_and_spectators_in(data);

    // Find chunks start: first null byte after the ;S= section
    let chunks_start = find_chunks_start(data);

    // Extract the `SD=` seed field (decimal integer terminated by `;` or null).
    let sd = find_header_u32_field(data, b";SD=").unwrap_or(0);

    Ok(HeaderParseResult {
        map_name,
        players,
        spectators,
        occupied_slots,
        chunks_start,
        sd,
        observer_slots,
    })
}

/// Find a header field of the form `;KEY=decimal_digits;` and parse as u32.
fn find_header_u32_field(data: &[u8], marker: &[u8]) -> Option<u32> {
    for i in 0..data.len().saturating_sub(marker.len()) {
        if &data[i..i + marker.len()] != marker {
            continue;
        }
        let mut end = i + marker.len();
        while end < data.len() && data[end].is_ascii_digit() {
            end += 1;
        }
        if end > i + marker.len()
            && let Ok(s) = std::str::from_utf8(&data[i + marker.len()..end])
        {
            return s.parse::<u32>().ok();
        }
    }
    None
}

/// Find where chunks start (first null byte after the ;S= section)
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

/// Parse a BFME2 replay file and extract game information
pub fn parse_replay(data: &[u8]) -> Result<ReplayInfo, ReplayError> {
    // Verify magic bytes
    if data.len() < MAGIC.len() + 16 || &data[..MAGIC.len()] != MAGIC {
        return Err(ReplayError::InvalidHeader);
    }

    // Parse header in a single pass
    let header_result = parse_header(data)?;

    // Filter to only "wor rhun" maps (early exit for unsupported maps)
    if !header_result.map_name.to_lowercase().contains("wor rhun") {
        return Err(ReplayError::UnsupportedMap(header_result.map_name));
    }

    // Parse timestamps from header (offset 8-16)
    let start_time = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let end_time = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);

    let map_name = header_result.map_name;
    let mut header_players = header_result.players;
    let spectators = header_result.spectators;
    let occupied_slots = header_result.occupied_slots;

    if header_players.is_empty() {
        return Err(ReplayError::NoPlayers);
    }

    // Build pn_to_slot: game engine assigns pn=3,4,5,... to each occupied slot in order
    let pn_to_slot: HashMap<u32, u8> = occupied_slots
        .iter()
        .enumerate()
        .map(|(i, &slot)| ((i as u32) + 3, slot))
        .collect();

    // Assign colors to players (resolves random slots by replaying the game's PRNG)
    assign_player_colors(
        &mut header_players,
        header_result.sd,
        &header_result.observer_slots,
    );

    // Build initial players list
    let mut players = build_players(&header_players);

    let chunks_start = header_result.chunks_start;

    // Parse state for streaming chunk processing
    let mut winner = Winner::Unknown;
    let mut game_crashed = false;
    let mut estimated_duration_secs: Option<u32> = None;

    if let Some(start) = chunks_start {
        // Parse chunks for positions, faction detection, and winner
        let parse_result = parse_and_analyze_chunks(data, start, &header_players, &pn_to_slot);

        // Assign positions and actual factions to players
        for player in &mut players {
            if let Some(build) = parse_result.positions.player_builds.get(&player.slot) {
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

        // Check for crashed game (only if winner is still unknown)
        if winner == Winner::Unknown
            && !parse_result.combat.has_endgame
            && parse_result.combat.defeated_players.is_empty()
        {
            game_crashed = true;
            winner = Winner::NotConcluded;
        }

        // Estimate duration from max chunk timecode
        if parse_result.max_timecode > 0 {
            estimated_duration_secs = Some(parse_result.max_timecode / SAGE_TICKS_PER_SECOND);
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

/// Search for "M=" marker and extract map name within a header slice
fn find_map_name_in(header: &[u8]) -> Option<String> {
    let marker = b"M=";

    for i in 0..header.len().saturating_sub(marker.len()) {
        if &header[i..i + marker.len()] == marker {
            let start = i + marker.len();
            let mut end = start;

            while end < header.len() && header[end] != b';' {
                end += 1;
            }

            if end > start {
                let map_path = &header[start..end];
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

/// Output of [`find_players_and_spectators_in`]. `occupied_slots` holds the slot
/// index of every non-empty entry (players AND spectators). `observer_slots`
/// pairs each spectator's slot index with its `color_id`, for the random-color
/// PRNG simulation.
struct SlotScan {
    players: Vec<HeaderPlayer>,
    spectators: Vec<String>,
    occupied_slots: Vec<u8>,
    observer_slots: Vec<(u8, i8)>,
}

/// Find the S= section within a header slice and parse all players and spectators.
fn find_players_and_spectators_in(header: &[u8]) -> SlotScan {
    let mut players = Vec::new();
    let mut spectators = Vec::new();
    let mut occupied_slots = Vec::new();
    let mut observer_slots: Vec<(u8, i8)> = Vec::new();
    let marker = b";S=";

    for i in 0..header.len().saturating_sub(marker.len()) {
        if &header[i..i + marker.len()] == marker {
            let start = i + marker.len();
            let mut end = start;

            while end < header.len() {
                let b = header[end];
                if b == 0 || b == b'\n' || b == b'\r' {
                    break;
                }
                if end + 2 < header.len()
                    && header[end] == b';'
                    && header[end + 1].is_ascii_uppercase()
                    && header[end + 2] == b'='
                {
                    break;
                }
                end += 1;
            }

            if end > start {
                let players_str = decode_with_turkish_fallback(&header[start..end]);

                for (slot_idx, player_str) in players_str.split(':').enumerate() {
                    if let Some(parsed) = parse_player_data(player_str, slot_idx as u8) {
                        occupied_slots.push(slot_idx as u8);
                        if parsed.team_raw >= 0 {
                            players.push(parsed);
                        } else {
                            // Spectator (team_raw is -1)
                            observer_slots.push((slot_idx as u8, parsed.color_id));
                            spectators.push(parsed.name);
                        }
                    }
                }
            }

            break;
        }
    }

    SlotScan {
        players,
        spectators,
        occupied_slots,
        observer_slots,
    }
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
        let mut chars = name.chars();
        chars.next(); // skip 'H'
        name = chars.as_str().to_string();
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

    // Parse startpos_raw (index 5) — -2 means observer, -1 random, 0..7 chosen
    let startpos_raw: i8 = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(-1);

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
        startpos_raw,
    })
}

/// Resolve random-color slots by replaying the game's deterministic RNG stream.
///
/// Mirrors `FUN_00647c1d` → `FUN_006443b6` → (`FUN_00643f62`, `FUN_00643bc4`)
/// in `game.dat`. Verified 9/9 against live replays via Frida runtime trace;
/// see `RANDOM_COLOR_REVERSE_ENGINEERING.md` and `simulate_final.py`.
///
/// `players` contains only non-observer slots (team_raw >= 0) in whatever order
/// the header produced. `observer_slots` gives `(slot_index, color_id)` for
/// each observer so their PRNG consumption is simulated in the correct order.
fn assign_player_colors(players: &mut [HeaderPlayer], sd: u32, observer_slots: &[(u8, i8)]) {
    use super::prng::Bfme2Rand;

    const NUM_COLORS: i32 = 10;
    const NUM_STARTS: i32 = 6; // wor rhun 6-player map

    let mut r = Bfme2Rand::new(sd);
    let mod7 = (sd % 7) as usize;

    // Build slot_index → players[] index for non-observer slots.
    let mut by_slot: [Option<usize>; 8] = [None; 8];
    for (idx, p) in players.iter().enumerate() {
        if (p.slot as usize) < 8 {
            by_slot[p.slot as usize] = Some(idx);
        }
    }

    // Build set of observer slot indices with their color_id for Phase 2 simulation.
    let mut observer_by_slot: [Option<i8>; 8] = [None; 8];
    for &(slot, color) in observer_slots {
        if (slot as usize) < 8 {
            observer_by_slot[slot as usize] = Some(color);
        }
    }

    // --- Phase 1 (StartPos) ---
    // Each observer consumes one rand(0, num_starts-1) call (accepts any taken
    // position; observer overlays a player's start spot).
    for _ in 0..observer_slots.len() {
        let _ = r.logic_random(0, NUM_STARTS - 1);
    }

    // --- Phase 2 (Color + Faction) --- iterate slots 0..7 in order.
    let mut taken: HashSet<i8> = HashSet::new();
    for idx in by_slot.iter().flatten() {
        if players[*idx].color_id >= 0 {
            taken.insert(players[*idx].color_id);
        }
    }

    for slot_idx in 0..8 {
        if let Some(pidx) = by_slot[slot_idx] {
            // Non-observer playing slot: one faction-loop iteration
            // (mod7 warmup rand(0,1) calls + one rand(0,1000) faction pick).
            for _ in 0..mod7 {
                let _ = r.logic_random(0, 1);
            }
            let _ = r.logic_random(0, 1000);

            if players[pidx].color_id == -1 {
                let picked = pick_untaken_color(&mut r, &taken, NUM_COLORS);
                players[pidx].color_id = picked;
                taken.insert(picked);
            }
        } else if let Some(obs_color) = observer_by_slot[slot_idx] {
            // Observer: no faction loop; color retry if color_id == -1.
            if obs_color == -1 {
                let picked = pick_untaken_color(&mut r, &taken, NUM_COLORS);
                taken.insert(picked);
            }
        }
    }
}

fn pick_untaken_color(r: &mut super::prng::Bfme2Rand, taken: &HashSet<i8>, num_colors: i32) -> i8 {
    loop {
        let c = r.logic_random(0, num_colors - 1) as i8;
        if !taken.contains(&c) {
            return c;
        }
    }
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

            PlayerBuilder {
                name: hp.name.clone(),
                uid: hp.uid.clone(),
                team,
                team_raw: hp.team_raw,
                slot: hp.slot,
                faction: Faction::from_id(hp.faction_id),
                color_id: hp.color_id,
                color_rgb,
            }
            .build()
        })
        .collect()
}

/// Build info extracted from commands
#[derive(Debug)]
struct BuildInfo {
    position: MapPosition,
    inferred_faction: Option<Faction>,
}

/// Position and faction data collected per player
struct PositionData {
    player_builds: HashMap<u8, BuildInfo>,
    player_positions: HashMap<u8, MapPosition>,
    player_building_ids: HashMap<u8, HashSet<u32>>,
}

/// Combat/game result data from chunk parsing
struct CombatResult {
    defeated_players: HashSet<u32>,
    endgame_player: Option<u32>,
    endgame_timecode: u32,
    has_endgame: bool,
}

/// Result of chunk parsing and analysis
struct ChunkParseResult {
    positions: PositionData,
    combat: CombatResult,
    max_timecode: u32,
    /// Last command timecode per player_num (for activity-based heuristic)
    player_last_command_tc: HashMap<u32, u32>,
    /// Last BUILD command (CMD_BUILD_OBJECT / CMD_BUILD_OBJECT_2) timecode per player_num.
    /// More reliable than last_command_tc because losing teams still issue sell/demolish
    /// commands near the end, but they stop *building* earlier.
    player_last_build_tc: HashMap<u32, u32>,
}

/// Parse chunks and analyze for positions, factions, and winner
fn parse_and_analyze_chunks(
    data: &[u8],
    start: usize,
    header_players: &[HeaderPlayer],
    pn_to_slot: &HashMap<u32, u8>,
) -> ChunkParseResult {
    let mut result = ChunkParseResult {
        positions: PositionData {
            player_builds: HashMap::new(),
            player_positions: HashMap::new(),
            player_building_ids: HashMap::new(),
        },
        combat: CombatResult {
            defeated_players: HashSet::new(),
            endgame_player: None,
            endgame_timecode: 0,
            has_endgame: false,
        },
        max_timecode: 0,
        player_last_command_tc: HashMap::new(),
        player_last_build_tc: HashMap::new(),
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

            // Track last command timecode per player (for activity-based heuristic)
            // Only track regular gameplay commands, not engine events
            if is_valid_player
                && chunk.order_type != CMD_PLAYER_DEFEATED
                && chunk.order_type != CMD_END_GAME
            {
                result
                    .player_last_command_tc
                    .entry(chunk.player_num)
                    .and_modify(|tc| *tc = (*tc).max(chunk.time_code))
                    .or_insert(chunk.time_code);

                // Track build commands separately (more reliable signal)
                if chunk.order_type == CMD_BUILD_OBJECT || chunk.order_type == CMD_BUILD_OBJECT_2 {
                    result
                        .player_last_build_tc
                        .entry(chunk.player_num)
                        .and_modify(|tc| *tc = (*tc).max(chunk.time_code))
                        .or_insert(chunk.time_code);
                }
            }

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
                        .positions
                        .player_building_ids
                        .entry(slot)
                        .or_default()
                        .insert(bid);
                }
            }

            // Process EndGame command (only from actual players, not spectators)
            // Keep the one with the highest timecode (latest)
            if chunk.order_type == CMD_END_GAME && is_valid_player {
                if !result.combat.has_endgame || chunk.time_code >= result.combat.endgame_timecode {
                    result.combat.endgame_player = Some(chunk.player_num);
                    result.combat.endgame_timecode = chunk.time_code;
                }
                result.combat.has_endgame = true;
            }

            // Process Player Defeated command (only actual players, not spectators)
            if chunk.order_type == CMD_PLAYER_DEFEATED && is_valid_player {
                result.combat.defeated_players.insert(chunk.player_num);
            }

            pos = next_pos;
        } else {
            pos += 1;
        }
    }

    // Merge positions: prefer build positions, fall back to unit positions
    for (slot, pos_data) in &build_positions {
        result.positions.player_positions.insert(*slot, *pos_data);
    }
    for (slot, pos_data) in &unit_positions {
        result
            .positions
            .player_positions
            .entry(*slot)
            .or_insert(*pos_data);
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
    for (slot, position) in &result.positions.player_positions.clone() {
        let buildings = result.positions.player_building_ids.get(slot);
        let inferred_faction = buildings.and_then(detect_faction_from_buildings);

        result.positions.player_builds.insert(
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
    if time_code > MAX_SANE_TIMECODE
        || player_num > MAX_SANE_PLAYER_NUM
        || n_arg_types > MAX_SANE_ARG_TYPES
    {
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
        if arg_count > MAX_SANE_ARG_COUNT {
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

                    let tc_valid = tc > 0 && tc < MAX_SANE_TIMECODE;
                    let pn_valid = (3..=20).contains(&player_num);
                    let nargs_valid = n_args <= 10;

                    if tc_valid
                        && pn_valid
                        && nargs_valid
                        && valid_player_nums.contains(&player_num)
                    {
                        if cmd == CMD_PLAYER_DEFEATED {
                            result.combat.defeated_players.insert(player_num);
                        } else if cmd == CMD_END_GAME {
                            // Keep the latest EndGame by timecode
                            if !result.combat.has_endgame || tc >= result.combat.endgame_timecode {
                                result.combat.endgame_player = Some(player_num);
                                result.combat.endgame_timecode = tc;
                            }
                            result.combat.has_endgame = true;
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
            let side = if pos.x < MAP_X_MIDPOINT {
                "Left"
            } else {
                "Right"
            };
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

/// Convert a side string to a certain Winner variant
fn side_to_winner(side: &str) -> Winner {
    if side == "Left" {
        Winner::LeftTeam
    } else {
        Winner::RightTeam
    }
}

/// Convert a side string to a likely Winner variant
fn side_to_likely_winner(side: &str) -> Winner {
    if side == "Left" {
        Winner::LikelyLeftTeam
    } else {
        Winner::LikelyRightTeam
    }
}

/// Try to determine winner from EndGame command (Order 29)
///
/// If the EndGame player is also in the defeated set, they lost — the other team wins.
/// Otherwise the EndGame player's team is considered the winner.
fn winner_from_endgame(
    combat: &CombatResult,
    header_players: &[HeaderPlayer],
    team_sides: &HashMap<i8, &'static str>,
    pn_to_slot: &HashMap<u32, u8>,
) -> Option<Winner> {
    let endgame_pn = combat.endgame_player?;
    let &endgame_slot = pn_to_slot.get(&endgame_pn)?;
    let hp = header_players.iter().find(|hp| hp.slot == endgame_slot)?;
    let &endgame_side = team_sides.get(&hp.team_raw)?;

    if combat.defeated_players.contains(&endgame_pn) {
        // EndGame player was defeated — their team lost, the other team won
        let other_side = if endgame_side == "Left" {
            "Right"
        } else {
            "Left"
        };
        // Verify the other side actually exists in team_sides
        if team_sides.values().any(|&s| s == other_side) {
            return Some(side_to_winner(other_side));
        }
        return None;
    }

    Some(side_to_winner(endgame_side))
}

/// Try to determine winner from all players on one team being defeated
fn winner_from_full_defeat(
    defeated: &HashSet<u32>,
    team_players: &HashMap<i8, Vec<u32>>,
    team_sides: &HashMap<i8, &'static str>,
) -> Option<Winner> {
    for (team_raw, players_pn) in team_players {
        if players_pn.iter().all(|pn| defeated.contains(pn)) {
            // This team lost, the other team won
            for other_team_raw in team_players.keys() {
                if other_team_raw != team_raw
                    && let Some(&side) = team_sides.get(other_team_raw)
                {
                    return Some(side_to_winner(side));
                }
            }
        }
    }
    None
}

/// Try to determine winner from majority-defeated heuristic
fn winner_from_majority_defeated(
    defeated: &HashSet<u32>,
    team_players: &HashMap<i8, Vec<u32>>,
    team_sides: &HashMap<i8, &'static str>,
) -> Option<Winner> {
    if team_players.len() != 2 {
        return None;
    }
    let teams: Vec<i8> = team_players.keys().cloned().collect();
    let team_a = teams[0];
    let team_b = teams[1];

    let defeats_a = team_players[&team_a]
        .iter()
        .filter(|pn| defeated.contains(pn))
        .count();
    let defeats_b = team_players[&team_b]
        .iter()
        .filter(|pn| defeated.contains(pn))
        .count();

    if defeats_a > defeats_b {
        team_sides.get(&team_b).map(|s| side_to_likely_winner(s))
    } else if defeats_b > defeats_a {
        team_sides.get(&team_a).map(|s| side_to_likely_winner(s))
    } else {
        None
    }
}

/// Try to determine winner from last-activity heuristic.
///
/// If one team stopped building significantly before the other team,
/// the inactive team probably lost. This is useful for observer replays where
/// engine events (EndGame/PlayerDefeated) may not map to real players.
///
/// Uses build commands (CMD_BUILD_OBJECT) as the signal instead of all commands,
/// because losing teams still issue sell/demolish commands near the end of the
/// game, but they stop *constructing* earlier.
///
/// Requirements:
/// - Exactly 2 teams
/// - Both teams must have issued at least one build command
/// - The gap between teams' last build time must be > 5% of max_timecode
fn winner_from_last_activity(
    player_last_build_tc: &HashMap<u32, u32>,
    team_players: &HashMap<i8, Vec<u32>>,
    team_sides: &HashMap<i8, &'static str>,
    max_timecode: u32,
) -> Option<Winner> {
    if team_players.len() != 2 || max_timecode == 0 {
        return None;
    }

    let teams: Vec<i8> = team_players.keys().cloned().collect();

    // Find latest build command timecode for each team
    let team_last_build = |team: &i8| -> Option<u32> {
        team_players[team]
            .iter()
            .filter_map(|pn| player_last_build_tc.get(pn))
            .copied()
            .max()
    };

    let last_a = team_last_build(&teams[0])?;
    let last_b = team_last_build(&teams[1])?;

    // Require a meaningful gap: > 5% of game duration
    let gap_threshold = max_timecode / 20;
    let gap = last_a.abs_diff(last_b);

    if gap <= gap_threshold {
        return None; // Not enough difference to be confident
    }

    if last_a > last_b {
        // Team A was still building later → Team A probably won
        team_sides.get(&teams[0]).map(|s| side_to_likely_winner(s))
    } else {
        // Team B was still building later → Team B probably won
        team_sides.get(&teams[1]).map(|s| side_to_likely_winner(s))
    }
}

/// Determine winner based on game events, using chained strategies
fn determine_winner(
    parse_result: &ChunkParseResult,
    header_players: &[HeaderPlayer],
    team_sides: &HashMap<i8, &'static str>,
    pn_to_slot: &HashMap<u32, u8>,
) -> Winner {
    // Build reverse mapping and team grouping (shared by fallback strategies)
    let slot_to_pn: HashMap<u8, u32> = pn_to_slot.iter().map(|(&pn, &slot)| (slot, pn)).collect();
    let mut team_players: HashMap<i8, Vec<u32>> = HashMap::new();
    for hp in header_players {
        if let Some(&pn) = slot_to_pn.get(&hp.slot) {
            team_players.entry(hp.team_raw).or_default().push(pn);
        }
    }

    winner_from_endgame(&parse_result.combat, header_players, team_sides, pn_to_slot)
        .or_else(|| {
            if parse_result.combat.defeated_players.is_empty() {
                return None;
            }
            winner_from_full_defeat(
                &parse_result.combat.defeated_players,
                &team_players,
                team_sides,
            )
        })
        .or_else(|| {
            if parse_result.combat.defeated_players.is_empty() {
                return None;
            }
            winner_from_majority_defeated(
                &parse_result.combat.defeated_players,
                &team_players,
                team_sides,
            )
        })
        .or_else(|| {
            winner_from_last_activity(
                &parse_result.player_last_build_tc,
                &team_players,
                team_sides,
                parse_result.max_timecode,
            )
        })
        .unwrap_or(Winner::Unknown)
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

    /// Verified against the live 3dwarf replay via Frida trace.
    /// Ground truth: mustafaa (slot 1) resolves to color 9 (White),
    /// Gusto (slot 7) resolves to color 1 (Red).
    #[test]
    fn test_assign_player_colors_3dwarf() {
        fn p(name: &str, slot: u8, color: i8, faction: i8, team: i8) -> HeaderPlayer {
            HeaderPlayer {
                name: name.into(),
                uid: None,
                color_id: color,
                faction_id: faction,
                team_raw: team,
                slot,
                startpos_raw: -1,
            }
        }
        // 3dwarf occupied_slots: 0..7. Slots 5 and 6 are observers.
        let mut players = vec![
            p("ALPHA", 0, 7, 0, 1),
            p("mustafaa", 1, -1, 2, 3),
            p("SuperNova", 2, 0, 1, 3),
            p("C__", 3, 6, -1, 1),
            p("AKINCI", 4, 2, 3, 1),
            p("Gusto", 7, -1, 4, 3),
        ];
        // Observers: slot 5 k$ln$, slot 6 Bullet, both with color_id=-1
        let observers = vec![(5u8, -1i8), (6u8, -1i8)];
        assign_player_colors(&mut players, 442_667_640, &observers);

        let get_color = |name: &str| players.iter().find(|p| p.name == name).unwrap().color_id;
        assert_eq!(
            get_color("mustafaa"),
            9,
            "mustafaa should resolve to White (9)"
        );
        assert_eq!(get_color("Gusto"), 1, "Gusto should resolve to Red (1)");
    }

    #[test]
    fn test_turkish_decode() {
        // Test that Turkish characters are handled
        let turkish_bytes = b"Test\xDD\xFD"; // I with dot, dotless i in Windows-1254
        let decoded = decode_with_turkish_fallback(turkish_bytes);
        assert!(decoded.contains("Test"));
    }

    /// Build a minimal valid replay byte sequence for testing
    fn build_test_replay(map_name: &str, players_str: &str) -> Vec<u8> {
        let mut data = Vec::new();
        // Magic
        data.extend_from_slice(b"BFME2RPL");
        // Start time (4 bytes) + End time (4 bytes)
        data.extend_from_slice(&1700000000u32.to_le_bytes());
        data.extend_from_slice(&1700001000u32.to_le_bytes());
        // Header content
        let header = format!("M=maps/{};S={}", map_name, players_str);
        data.extend_from_slice(header.as_bytes());
        // Null terminator (marks end of header / start of chunks)
        data.push(0);
        data
    }

    #[test]
    fn test_parse_replay_valid_rhun() {
        let data = build_test_replay(
            "map wor rhun",
            "HAlice,12345678,8094,TT,0,-1,0,0,0,1,0:HBob,87654321,8094,TT,1,-1,1,1,0,1,0",
        );
        let result = parse_replay(&data);
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.players.len(), 2);
        assert_eq!(info.players[0].name, "Alice");
        assert_eq!(info.players[1].name, "Bob");
    }

    #[test]
    fn test_parse_replay_unsupported_map() {
        let data = build_test_replay(
            "fords of isen",
            "HAlice,12345678,8094,TT,0,-1,0,0,0,1,0:HBob,87654321,8094,TT,1,-1,1,1,0,1,0",
        );
        let result = parse_replay(&data);
        assert!(result.is_err());
        match result.unwrap_err() {
            ReplayError::UnsupportedMap(name) => assert_eq!(name, "fords of isen"),
            other => panic!("Expected UnsupportedMap, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_replay_corrupt_data() {
        // Too short to even have magic bytes
        let result = parse_replay(&[0u8; 4]);
        assert!(matches!(result, Err(ReplayError::InvalidHeader)));
    }

    #[test]
    fn test_parse_replay_bad_magic() {
        let mut data = vec![0u8; 24];
        data[..8].copy_from_slice(b"NOTMAGIC");
        let result = parse_replay(&data);
        assert!(matches!(result, Err(ReplayError::InvalidHeader)));
    }

    #[test]
    fn test_parse_replay_no_players() {
        let data = build_test_replay("map wor rhun", "X:X:X:X");
        let result = parse_replay(&data);
        assert!(matches!(result, Err(ReplayError::NoPlayers)));
    }

    #[test]
    fn test_char_safe_name_slicing() {
        // Test that H-prefix stripping works with multi-byte characters
        let player = parse_player_data("HTest,12345678,8094,TT,0,-1,0,0,0,1,0", 0).unwrap();
        assert_eq!(player.name, "Test");
    }

    #[test]
    fn test_endgame_defeated_player_means_other_team_wins() {
        // When the EndGame player is also in defeated_players,
        // their team lost — the other team should win.
        let mut defeated = HashSet::new();
        defeated.insert(4u32); // pn=4 is defeated

        let combat = CombatResult {
            defeated_players: defeated,
            endgame_player: Some(4), // same player triggered EndGame
            endgame_timecode: 7000,
            has_endgame: true,
        };

        // pn=4 → slot=1 (Left team, team_raw=0)
        let header_players = vec![
            HeaderPlayer {
                name: "LeftPlayer".to_string(),
                uid: None,
                slot: 1,
                color_id: 0,
                faction_id: 0,
                team_raw: 0,
                startpos_raw: -1,
            },
            HeaderPlayer {
                name: "RightPlayer".to_string(),
                uid: None,
                slot: 2,
                color_id: 1,
                faction_id: 1,
                team_raw: 1,
                startpos_raw: -1,
            },
        ];

        let mut team_sides = HashMap::new();
        team_sides.insert(0i8, "Left");
        team_sides.insert(1i8, "Right");

        let mut pn_to_slot = HashMap::new();
        pn_to_slot.insert(4u32, 1u8);
        pn_to_slot.insert(5u32, 2u8);

        let result = winner_from_endgame(&combat, &header_players, &team_sides, &pn_to_slot);
        // Left player was defeated + triggered EndGame → Right team wins
        assert_eq!(result, Some(Winner::RightTeam));
    }

    #[test]
    fn test_endgame_non_defeated_player_means_their_team_wins() {
        // When the EndGame player is NOT defeated, their team wins (normal case).
        let combat = CombatResult {
            defeated_players: HashSet::new(),
            endgame_player: Some(5), // Right player triggered EndGame, not defeated
            endgame_timecode: 7000,
            has_endgame: true,
        };

        let header_players = vec![
            HeaderPlayer {
                name: "LeftPlayer".to_string(),
                uid: None,
                slot: 1,
                color_id: 0,
                faction_id: 0,
                team_raw: 0,
                startpos_raw: -1,
            },
            HeaderPlayer {
                name: "RightPlayer".to_string(),
                uid: None,
                slot: 2,
                color_id: 1,
                faction_id: 1,
                team_raw: 1,
                startpos_raw: -1,
            },
        ];

        let mut team_sides = HashMap::new();
        team_sides.insert(0i8, "Left");
        team_sides.insert(1i8, "Right");

        let mut pn_to_slot = HashMap::new();
        pn_to_slot.insert(4u32, 1u8);
        pn_to_slot.insert(5u32, 2u8);

        let result = winner_from_endgame(&combat, &header_players, &team_sides, &pn_to_slot);
        // Right player triggered EndGame and was NOT defeated → Right team wins
        assert_eq!(result, Some(Winner::RightTeam));
    }
}
