use std::fmt;

/// Faction identifiers from BFME2 Rise of the Witch King
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Faction {
    Men,
    Elves,
    Dwarves,
    Isengard,
    Mordor,
    Goblins,
    Angmar,
    Random, // Player picked Random, actual faction unknown
    Unknown(u8),
}

impl Faction {
    /// Convert faction ID from replay file to Faction enum
    /// Note: For players who picked "Random" in lobby, this returns their
    /// lobby selection, NOT their actual in-game faction
    pub fn from_id(id: i8) -> Self {
        match id {
            -1 => Faction::Random,
            0 => Faction::Men,
            1 => Faction::Goblins,
            2 => Faction::Dwarves,
            3 => Faction::Isengard,
            4 => Faction::Elves,
            5 => Faction::Mordor,
            6 => Faction::Angmar,
            n if n >= 0 => Faction::Unknown(n as u8),
            _ => Faction::Random,
        }
    }
}

impl fmt::Display for Faction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Faction::Men => write!(f, "Men"),
            Faction::Elves => write!(f, "Elves"),
            Faction::Dwarves => write!(f, "Dwarves"),
            Faction::Isengard => write!(f, "Isengard"),
            Faction::Mordor => write!(f, "Mordor"),
            Faction::Goblins => write!(f, "Goblins"),
            Faction::Angmar => write!(f, "Angmar"),
            Faction::Random => write!(f, "Random"),
            Faction::Unknown(n) => write!(f, "Unknown({})", n),
        }
    }
}

/// Vec2 position on the map (game world coordinates)
#[derive(Debug, Clone, Copy, Default)]
pub struct MapPosition {
    pub x: f32,
    pub y: f32,
}

impl MapPosition {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn is_valid(&self) -> bool {
        self.x != 0.0 || self.y != 0.0
    }
}

/// In-game player colors (10 colors from BFME2)
/// Color ID from header maps to these RGB values
pub const PLAYER_COLORS: [[u8; 3]; 10] = [
    [70, 91, 156],   // 0: Blue
    [158, 56, 42],   // 1: Red
    [175, 189, 76],  // 2: Yellow
    [62, 152, 100],  // 3: Green
    [206, 135, 69],  // 4: Orange
    [122, 168, 204], // 5: Teal/Cyan (Light Blue)
    [148, 116, 183], // 6: Purple
    [204, 159, 188], // 7: Pink/Magenta
    [100, 100, 100], // 8: Gray
    [226, 226, 226], // 9: White
];

/// Player information extracted from replay
#[derive(Debug, Clone)]
pub struct Player {
    pub name: String,
    #[allow(dead_code)]
    pub uid: Option<String>, // 8-char hex UID from header
    pub team: i8,
    pub team_raw: i8, // Original team value from header
    pub slot: u8,     // Slot index in lobby
    pub faction: Faction,
    #[allow(dead_code)]
    pub color_id: i8, // -1 = random
    pub color_rgb: [u8; 3],                // Resolved RGB color
    pub map_position: Option<MapPosition>, // Position on map from first building
    pub actual_faction: Option<Faction>,   // For Random players, their actual faction
}

impl Player {
    /// Create a player with full details
    #[allow(clippy::too_many_arguments)]
    pub fn with_details(
        name: String,
        uid: Option<String>,
        team: i8,
        team_raw: i8,
        slot: u8,
        faction: Faction,
        color_id: i8,
        color_rgb: [u8; 3],
    ) -> Self {
        Self {
            name,
            uid,
            team,
            team_raw,
            slot,
            faction,
            color_id,
            color_rgb,
            map_position: None,
            actual_faction: None,
        }
    }

    /// Get the display faction (actual if known, otherwise selected)
    pub fn display_faction(&self) -> &Faction {
        self.actual_faction.as_ref().unwrap_or(&self.faction)
    }

    /// Get the player's display color RGB
    pub fn display_color(&self) -> [u8; 3] {
        self.color_rgb
    }
}

/// Winning team or result
#[derive(Debug, Clone, PartialEq)]
pub enum Winner {
    LeftTeam,        // Left side team won (certain: EndGame or all-defeated)
    RightTeam,       // Right side team won (certain: EndGame or all-defeated)
    LikelyLeftTeam,  // Left side likely won (majority-defeated heuristic)
    LikelyRightTeam, // Right side likely won (majority-defeated heuristic)
    NotConcluded,    // Game crashed/abandoned - no Order 29 and no full team defeated
    Unknown,         // Could not determine
}

impl Winner {
    /// Display text for the winner
    pub fn display_text(&self) -> &'static str {
        match self {
            Winner::LeftTeam => "Left Team",
            Winner::RightTeam => "Right Team",
            Winner::LikelyLeftTeam => "Left Team (likely)",
            Winner::LikelyRightTeam => "Right Team (likely)",
            Winner::NotConcluded => "Not Concluded",
            Winner::Unknown => "Unknown",
        }
    }
}

/// Spectator (observer) information
#[derive(Debug, Clone)]
pub struct Spectator {
    pub name: String,
}

/// Replay parsing error types
#[derive(Debug, Clone)]
pub enum ReplayError {
    InvalidHeader,
    UnsupportedMap(String),
    NoPlayers,
    ParseError(String),
    RenderError(String),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::InvalidHeader => write!(f, "Invalid replay file: missing BFME2RPL header"),
            ReplayError::UnsupportedMap(name) => write!(f, "Unsupported map: {}", name),
            ReplayError::NoPlayers => write!(f, "No players found in replay"),
            ReplayError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            ReplayError::RenderError(msg) => write!(f, "Render error: {}", msg),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Complete replay information
#[derive(Debug, Clone)]
pub struct ReplayInfo {
    #[allow(dead_code)]
    pub map_name: String,
    pub players: Vec<Player>,
    pub spectators: Vec<Spectator>,
    pub start_time: Option<u32>, // Unix timestamp
    pub end_time: Option<u32>,   // Unix timestamp
    pub winner: Winner,
    pub game_crashed: bool, // No Order 29 and no full team defeated
    pub estimated_duration_secs: Option<u32>, // From max chunk timecode / 5
}

impl ReplayInfo {
    pub fn new(map_name: String, players: Vec<Player>) -> Self {
        Self {
            map_name,
            players,
            spectators: Vec::new(),
            start_time: None,
            end_time: None,
            winner: Winner::Unknown,
            game_crashed: false,
            estimated_duration_secs: None,
        }
    }

    pub fn with_times(mut self, start: u32, end: u32) -> Self {
        self.start_time = Some(start);
        self.end_time = Some(end);
        self
    }

    pub fn with_winner(mut self, winner: Winner) -> Self {
        self.winner = winner;
        self
    }

    pub fn with_spectators(mut self, spectators: Vec<Spectator>) -> Self {
        self.spectators = spectators;
        self
    }

    pub fn with_game_crashed(mut self, crashed: bool) -> Self {
        self.game_crashed = crashed;
        self
    }

    pub fn with_estimated_duration(mut self, secs: Option<u32>) -> Self {
        self.estimated_duration_secs = secs;
        self
    }

    /// Get game duration in seconds
    pub fn duration_seconds(&self) -> Option<u32> {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) if end > start => Some(end - start),
            _ => self.estimated_duration_secs,
        }
    }

    /// Whether the displayed duration is an estimate (from chunk timecodes)
    pub fn is_duration_estimated(&self) -> bool {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) if end > start => false,
            _ => self.estimated_duration_secs.is_some(),
        }
    }

    /// Format duration as "MM:SS" or "HH:MM:SS", prefixed with "~" if estimated
    pub fn duration_formatted(&self) -> String {
        match self.duration_seconds() {
            Some(total_secs) => {
                let hours = total_secs / 3600;
                let mins = (total_secs % 3600) / 60;
                let secs = total_secs % 60;
                let prefix = if self.is_duration_estimated() {
                    "~"
                } else {
                    ""
                };
                if hours > 0 {
                    format!("{}{}:{:02}:{:02}", prefix, hours, mins, secs)
                } else {
                    format!("{}{}:{:02}", prefix, mins, secs)
                }
            }
            None => "Unknown".to_string(),
        }
    }

    /// Get formatted start date as YYYY-MM-DD HH:MM
    pub fn start_date_formatted(&self) -> String {
        match self.start_time {
            Some(ts) => {
                // Convert Unix timestamp to date string
                let secs = ts as i64;
                // Simple date formatting without external crate
                // Using basic calculation (not accounting for leap seconds, etc.)
                let days_since_epoch = secs / 86400;
                let time_of_day = secs % 86400;
                let hours = time_of_day / 3600;
                let minutes = (time_of_day % 3600) / 60;

                // Calculate year/month/day from days since 1970
                let (year, month, day) = days_to_ymd(days_since_epoch as i32);

                format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}",
                    year, month, day, hours, minutes
                )
            }
            None => "Unknown".to_string(),
        }
    }
}

/// Convert days since Unix epoch to year/month/day
fn days_to_ymd(days: i32) -> (i32, u32, u32) {
    // Days since 1970-01-01
    let mut remaining = days;
    let mut year = 1970;

    // Find the year
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    // Find the month and day
    let days_in_months: [i32; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    for &days_in_month in &days_in_months {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }

    let day = (remaining + 1) as u32;
    (year, month, day)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_replay() -> ReplayInfo {
        ReplayInfo::new("map wor rhun".to_string(), vec![])
    }

    #[test]
    fn test_normal_game_duration() {
        let info = make_replay().with_times(1000, 1817);
        assert_eq!(info.duration_seconds(), Some(817));
        assert!(!info.is_duration_estimated());
        assert_eq!(info.duration_formatted(), "13:37");
    }

    #[test]
    fn test_crashed_game_estimated_duration() {
        // Crashed game: end == start, but we have chunk timecode estimate
        let info = make_replay()
            .with_times(1000, 1000)
            .with_estimated_duration(Some(780));
        assert_eq!(info.duration_seconds(), Some(780));
        assert!(info.is_duration_estimated());
        assert_eq!(info.duration_formatted(), "~13:00");
    }

    #[test]
    fn test_crashed_game_no_chunks() {
        // Crashed game with no chunks at all
        let info = make_replay().with_times(1000, 1000);
        assert_eq!(info.duration_seconds(), None);
        assert!(!info.is_duration_estimated());
        assert_eq!(info.duration_formatted(), "Unknown");
    }

    #[test]
    fn test_normal_game_ignores_estimate() {
        // Normal game should use header duration even if estimate is present
        let info = make_replay()
            .with_times(1000, 1817)
            .with_estimated_duration(Some(780));
        assert_eq!(info.duration_seconds(), Some(817));
        assert!(!info.is_duration_estimated());
        assert_eq!(info.duration_formatted(), "13:37");
    }

    #[test]
    fn test_estimated_duration_with_hours() {
        let info = make_replay()
            .with_times(1000, 1000)
            .with_estimated_duration(Some(3661));
        assert_eq!(info.duration_formatted(), "~1:01:01");
    }
}
