use crate::models::{Faction, Player, ReplayInfo};

const MAGIC: &[u8] = b"BFME2RPL";

/// Parse a BFME2 replay file and extract game information
pub fn parse_replay(data: &[u8]) -> Result<ReplayInfo, String> {
    // Verify magic bytes
    if data.len() < MAGIC.len() || &data[..MAGIC.len()] != MAGIC {
        return Err("Invalid replay file: missing BFME2RPL header".to_string());
    }

    // Find and parse map name
    let map_name = find_map_name(data).ok_or("Could not find map name in replay")?;

    // Find and parse players
    let players = find_players(data);

    if players.is_empty() {
        return Err("No players found in replay".to_string());
    }

    Ok(ReplayInfo::new(map_name, players))
}

/// Search for "M=" marker and extract map name
fn find_map_name(data: &[u8]) -> Option<String> {
    // Look for "M=" followed by map path
    let marker = b"M=";

    for i in 0..data.len().saturating_sub(marker.len()) {
        if &data[i..i + marker.len()] == marker {
            // Found marker, now extract until semicolon
            let start = i + marker.len();
            let mut end = start;

            while end < data.len() && data[end] != b';' && data[end] != 0 {
                end += 1;
            }

            if end > start {
                let map_path = &data[start..end];
                // Extract just the map name from the path
                // Format: <id>maps/<mapname> or similar
                if let Ok(path_str) = std::str::from_utf8(map_path) {
                    return extract_map_name_from_path(path_str);
                }
            }
        }
    }
    None
}

/// Extract clean map name from path like "123maps/map wor rhun"
fn extract_map_name_from_path(path: &str) -> Option<String> {
    // Try to find "maps/" and take everything after
    if let Some(idx) = path.find("maps/") {
        let name = &path[idx + 5..];
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    // Fallback: just use the whole thing
    if !path.is_empty() {
        Some(path.to_string())
    } else {
        None
    }
}

/// Search for "S=" markers and extract player information
fn find_players(data: &[u8]) -> Vec<Player> {
    let mut players = Vec::new();
    let marker = b"S=";

    let mut i = 0;
    while i < data.len().saturating_sub(marker.len()) {
        if &data[i..i + marker.len()] == marker {
            // Found player marker
            let start = i + marker.len();

            // Find the end of the player data (look for next S= or end of valid data)
            let mut end = start;
            while end < data.len() {
                // Player data ends at colon or null or when we hit another marker
                if data[end] == b':' || data[end] == 0 {
                    break;
                }
                // Check for next S= marker
                if end + 2 < data.len() && &data[end..end + 2] == b"S=" {
                    break;
                }
                end += 1;
            }

            if end > start {
                if let Ok(player_str) = std::str::from_utf8(&data[start..end]) {
                    if let Some(player) = parse_player_string(player_str) {
                        players.push(player);
                    }
                }
            }

            i = end;
        } else {
            i += 1;
        }
    }

    players
}

/// Parse a player string in format: Name,ID,Port,TT,Team,Position,Faction,...
fn parse_player_string(s: &str) -> Option<Player> {
    let parts: Vec<&str> = s.split(',').collect();

    // Need at least: name, id, port, tt, team, position, faction
    if parts.len() < 7 {
        return None;
    }

    let name = parts[0].to_string();
    if name.is_empty() {
        return None;
    }

    // Parse team (index 4)
    let team: i8 = parts.get(4)?.parse().ok()?;

    // Parse position (index 5)
    let position: u8 = parts.get(5)?.parse().ok()?;

    // Parse faction (index 6)
    let faction_id: u8 = parts.get(6)?.parse().ok()?;
    let faction = Faction::from_id(faction_id);

    Some(Player::new(name, team, position, faction))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_map_name() {
        assert_eq!(
            extract_map_name_from_path("123maps/map wor rhun"),
            Some("map wor rhun".to_string())
        );
        assert_eq!(
            extract_map_name_from_path("maps/fords of isen"),
            Some("fords of isen".to_string())
        );
    }

    #[test]
    fn test_parse_player_string() {
        let player = parse_player_string("PlayerName,12345,1234,0,1,3,2").unwrap();
        assert_eq!(player.name, "PlayerName");
        assert_eq!(player.team, 1);
        assert_eq!(player.position, 3);
        assert!(matches!(player.faction, Faction::Isengard));
    }
}
