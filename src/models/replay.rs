use std::fmt;

/// Faction identifiers from BFME2
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Faction {
    Mordor,
    Gondor,
    Isengard,
    Rohan,
    Dwarves,
    Elves,
    Goblins,
    Angmar,
    Unknown(u8),
}

impl Faction {
    pub fn from_id(id: u8) -> Self {
        match id {
            0 => Faction::Mordor,
            1 => Faction::Gondor,
            2 => Faction::Isengard,
            3 => Faction::Rohan,
            4 => Faction::Dwarves,
            5 => Faction::Elves,
            6 => Faction::Goblins,
            7 => Faction::Angmar,
            n => Faction::Unknown(n),
        }
    }

    /// Returns the faction's display color as RGB
    pub fn color(&self) -> [u8; 3] {
        match self {
            Faction::Mordor => [139, 0, 0],      // Dark red
            Faction::Gondor => [192, 192, 192],  // Silver
            Faction::Isengard => [64, 64, 64],   // Dark gray
            Faction::Rohan => [218, 165, 32],    // Goldenrod
            Faction::Dwarves => [139, 69, 19],   // Saddle brown
            Faction::Elves => [0, 128, 0],       // Green
            Faction::Goblins => [85, 107, 47],   // Dark olive green
            Faction::Angmar => [75, 0, 130],     // Indigo
            Faction::Unknown(_) => [128, 128, 128], // Gray
        }
    }
}

impl fmt::Display for Faction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Faction::Mordor => write!(f, "Mordor"),
            Faction::Gondor => write!(f, "Gondor"),
            Faction::Isengard => write!(f, "Isengard"),
            Faction::Rohan => write!(f, "Rohan"),
            Faction::Dwarves => write!(f, "Dwarves"),
            Faction::Elves => write!(f, "Elves"),
            Faction::Goblins => write!(f, "Goblins"),
            Faction::Angmar => write!(f, "Angmar"),
            Faction::Unknown(n) => write!(f, "Unknown({})", n),
        }
    }
}

/// Player information extracted from replay
#[derive(Debug, Clone)]
pub struct Player {
    pub name: String,
    pub team: i8,
    pub position: u8,
    pub faction: Faction,
}

impl Player {
    pub fn new(name: String, team: i8, position: u8, faction: Faction) -> Self {
        Self {
            name,
            team,
            position,
            faction,
        }
    }
}

/// Complete replay information
#[derive(Debug, Clone)]
pub struct ReplayInfo {
    pub map_name: String,
    pub players: Vec<Player>,
    pub timestamp: Option<String>,
}

impl ReplayInfo {
    pub fn new(map_name: String, players: Vec<Player>) -> Self {
        Self {
            map_name,
            players,
            timestamp: None,
        }
    }

    pub fn with_timestamp(mut self, timestamp: String) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
}
