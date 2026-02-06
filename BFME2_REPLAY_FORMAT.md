# BFME2 Replay File Format Documentation

## Overview

Battle for Middle-earth II (BFME2) replay files (`.BfME2Replay`) are binary files that record all game events for playback. This document describes the file structure based on reverse engineering and empirical analysis.

**File Extension:** `.BfME2Replay`
**Default Location:** `%APPDATA%\My Battle for Middle-earth(tm) II Files\Replays\`

---

## File Structure

The replay file consists of two main sections:

```
┌─────────────────────────────────────────┐
│              HEADER                      │
│  (Variable length, null-terminated)      │
├─────────────────────────────────────────┤
│              CHUNKS                      │
│  (Sequence of game event records)        │
└─────────────────────────────────────────┘
```

---

## Header Section

### Binary Header (Fixed Offsets)

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0x00 | 8 | bytes | Unknown (possibly magic number/version) |
| 0x08 | 4 | uint32_le | Game start timestamp (Unix epoch) |
| 0x0C | 4 | uint32_le | Game end timestamp (Unix epoch) |
| 0x10+ | variable | ASCII/text | Key-value metadata string |

### Metadata String Format

After the binary header, there's a semicolon-delimited key-value string terminated by a null byte (`0x00`).

**Format:** `KEY=VALUE;KEY=VALUE;...KEY=VALUE\x00`

#### Known Keys

| Key | Description | Example |
|-----|-------------|---------|
| `M` | Map path | `data/maps/official/map wor rhun/map wor rhun.map` |
| `MC` | Map CRC/checksum | `1234567890` |
| `MS` | Map size (bytes) | `123456` |
| `SD` | Random seed | `1234567890` |
| `S` | Player slots data | See Player Slots Format below |

#### Extracting Map Name

```python
m_start = data.find(b"M=") + 2
m_end = data.find(b";", m_start)
map_raw = data[m_start:m_end].decode("ascii")
map_name = map_raw.split("maps/")[1] if "maps/" in map_raw else map_raw
```

### Player Slots Format (S= field)

The `S=` field contains colon-separated slot entries. Each slot can be:
- Empty slot: Single character or empty
- Human player: `H` prefix with comma-separated values
- AI player: `C` prefix (computer)

**Human Player Format:** `Hplayername,uid,port?,type?,color_id,?,faction_id,team_id,?,?,?`

| Index | Description | Example | Notes |
|-------|-------------|---------|-------|
| 0 | Player name (with H prefix) | `HPlayerName` | Remove H prefix to get name |
| 1 | **Player UID** | `1ABB848B` | 8-character hex identifier |
| 2 | Port/Version? | `8094` | Same for all players in game |
| 3 | Game Type? | `TT` | Possibly "Team" game mode |
| 4 | Color ID | `0`-`9` or `-1` | -1 = random color |
| 5 | Unknown | `-1`, `-2` | Possibly handicap or spot setting |
| 6 | Faction ID | `0`-`5` or `-1` | -1 = random faction |
| 7 | Team ID | `0`-`3` or `-1` | 0-3 = teams 1-4, -1 = spectator |
| 8 | Unknown | `0` | Flag/boolean? |
| 9 | Unknown | `1` | Flag/boolean? |
| 10 | Unknown | `0` | Flag/boolean? |

#### Faction ID Mapping

| ID | Faction |
|----|---------|
| 0 | Men (Gondor/Rohan) |
| 1 | Goblins |
| 2 | Dwarves |
| 3 | Isengard |
| 4 | Elves |
| 5 | Mordor |
| -1 | Random |

#### Color ID Mapping

| ID | Color | RGB Value |
|----|-------|-----------|
| 0 | Blue | (70, 91, 156) |
| 1 | Red | (158, 56, 42) |
| 2 | Yellow | (175, 189, 76) |
| 3 | Green | (62, 152, 100) |
| 4 | Orange | (206, 135, 69) |
| 5 | Teal/Cyan | (122, 168, 204) |
| 6 | Purple | (148, 116, 183) |
| 7 | Pink | (204, 159, 188) |
| 8 | Gray | (100, 100, 100) |
| 9 | White | (226, 226, 226) |
| -1 | Random | Assigned by game |


### Text Encoding

Player names may contain non-ASCII characters (e.g., Turkish İ, ş, ğ). Try decoding with:
1. UTF-8
2. Windows-1254 (Turkish)
3. ISO-8859-9 (Latin-5 Turkish)
4. Windows-1252 (Western European)

```python
for encoding in ["utf-8", "windows-1254", "iso-8859-9", "windows-1252"]:
    try:
        players_raw = players_bytes.decode(encoding)
        break
    except UnicodeDecodeError:
        continue
```

---

## Chunk Section

Chunks begin immediately after the null terminator of the header string. Each chunk represents a game event/command.

### Chunk Structure

```
┌────────────────────────────────────────────────────────────┐
│ Timecode │ Order Type │ Player Num │ Arg Count │ Args...   │
│ 4 bytes  │ 4 bytes    │ 4 bytes    │ 1 byte    │ variable  │
└────────────────────────────────────────────────────────────┘
```

| Field | Size | Type | Description |
|-------|------|------|-------------|
| Timecode | 4 | uint32_le | Game tick/frame number |
| Order Type | 4 | uint32_le | Command/event type ID |
| Player Num | 4 | uint32_le | Player number (slot + 3) |
| Arg Count | 1 | uint8 | Number of argument type signatures |
| Arg Signatures | 2 × Arg Count | bytes | Type and count for each argument group |
| Argument Data | variable | bytes | Actual argument values |

### Player Number Mapping

The game engine assigns player numbers **sequentially starting from 3** to each **occupied slot** (any slot with a person — player or spectator). Empty slots (`X`, `O`, or blank entries in the `S=` field) are skipped entirely.

**When there are no empty slots between players**, this simplifies to `player_num = slot_index + 3`. However, when empty slots exist in the middle of the slot list, this formula is **wrong** for all players after the gap.

**Correct approach:** Build a mapping from the header:

```python
occupied_slots = []  # slot indices of non-empty entries (players AND spectators)
for slot_idx, slot_str in enumerate(s_field.split(":")):
    if is_occupied(slot_str):  # not X, O, empty, or ;
        occupied_slots.append(slot_idx)

# Game assigns pn=3 to first occupied slot, pn=4 to second, etc.
pn_to_slot = {i + 3: slot for i, slot in enumerate(occupied_slots)}
```

**Example** — 7 slots with slot 3 empty:
```
Slot 0: PlayerA (team=0)  → pn=3
Slot 1: PlayerB (team=1)  → pn=4
Slot 2: PlayerC (team=1)  → pn=5
Slot 3: [EMPTY X]         → skipped
Slot 4: PlayerD (team=3)  → pn=6  (NOT 7)
Slot 5: PlayerE (team=3)  → pn=7  (NOT 8)
Slot 6: PlayerF (team=3)  → pn=8  (NOT 9)
```

### Argument Type Signatures

Each argument group has a 2-byte signature: `[type_id, count]`

| Type ID | Size (bytes) | Description |
|---------|--------------|-------------|
| 0x00 | 4 | Unsigned 32-bit integer |
| 0x01 | 4 | Signed 32-bit integer |
| 0x02 | 1 | Byte/Boolean |
| 0x03 | 4 | Float |
| 0x04 | 4 | Object ID / Reference |
| 0x05 | 8 | Unknown (possibly 2 floats) |
| 0x06 | 12 | Vec3 (3 floats: x, y, z) - Position |
| 0x07 | 12 | Vec3 (3 floats) - Direction/Normal |
| 0x08 | 16 | Unknown (possibly Vec4 or matrix row) |
| 0x09 | 4 | Unknown |
| 0x0A | 4 | Unknown |

### Known Order Types (Commands)

| Order ID | Name | Description | Arguments |
|----------|------|-------------|-----------|
| 29 | EndGame | Game ends (issued by winning player) | None |
| 1047 | CreateUnit | Train/create a unit | int (unit_type_id), vec3 (position) |
| 1049 | BuildObject | Place a building | int (building_type_id), vec3 (position) |
| 1050 | Unknown Build | Building-related command | int (type_id), vec3 (position) |
| 1071 | Unknown Move | Movement/position command | vec3 (position) |
| 1096 | PlayerDefeated | Player eliminated | Player num in chunk header |

### Chunk Parsing Algorithm

```python
ARG_SIZES = {
    0x00: 4, 0x01: 4, 0x02: 1, 0x03: 4, 0x04: 4,
    0x05: 8, 0x06: 12, 0x07: 12, 0x08: 16, 0x09: 4, 0x0A: 4,
}

def parse_chunk(data, offset):
    if offset + 13 > len(data):
        return None, None

    tc = struct.unpack("<I", data[offset:offset + 4])[0]
    order = struct.unpack("<I", data[offset + 4:offset + 8])[0]
    player = struct.unpack("<I", data[offset + 8:offset + 12])[0]
    n_args = data[offset + 12]

    # Sanity checks
    if tc > 10000000 or player > 100 or n_args > 100:
        return None, None

    pos = offset + 13

    # Read argument signatures
    arg_sig = []
    for _ in range(n_args):
        if pos + 2 > len(data):
            return None, None
        arg_type = data[pos]
        arg_count = data[pos + 1]
        if arg_count > 50:
            return None, None
        arg_sig.append((arg_type, arg_count))
        pos += 2

    # Read argument values
    args = []
    for arg_type, arg_count in arg_sig:
        size = ARG_SIZES.get(arg_type, 4)
        for _ in range(arg_count):
            if pos + size > len(data):
                return None, None
            if arg_type == 0x00:
                args.append(("int", struct.unpack("<I", data[pos:pos + size])[0]))
            elif arg_type == 0x06:
                x, y, z = struct.unpack("<fff", data[pos:pos + size])
                args.append(("vec3", (x, y, z)))
            else:
                args.append((f"t{arg_type:02x}", data[pos:pos + size].hex()))
            pos += size

    return pos, {"tc": tc, "order": order, "player": player, "args": args}
```

---

## Algorithms

### Detecting Random Faction Selection

When a player selects "Random" faction (faction_id = -1 in header), the actual faction must be inferred from gameplay data.

**Method:** Analyze building type IDs from BuildObject (1049) commands.

#### Building ID Ranges by Faction (Empirically Determined)

| Faction | Building ID Range |
|---------|-------------------|
| Men | 2622 - 2720 |
| Elves | 2577 - 2620 |
| Dwarves | 2541 - 2575 |
| Goblins | 2151 - 2185 |
| Isengard | 2060 - 2090 |
| Mordor | 2130 - 2150 |

```python
def detect_faction_from_buildings(buildings):
    for bid in buildings:
        if 2622 <= bid <= 2720: return "Men"
        if 2577 <= bid <= 2620: return "Elves"
        if 2541 <= bid <= 2575: return "Dwarves"
        if 2151 <= bid <= 2185: return "Goblins"
        if 2060 <= bid <= 2090: return "Isengard"
        if 2130 <= bid <= 2150: return "Mordor"
    return None
```

**Note:** The `SD=` (seed) value in the header is used by the game's PRNG to determine random faction, but the exact algorithm is not publicly documented. Building-based detection is more reliable.

### Random Color Assignment Algorithm

When a player selects "Random" color (color_id = -1), the game assigns colors using a gap-based algorithm:

1. Find all available colors (0-8, excluding 9/white for random assignment)
2. Identify contiguous gaps of available colors
3. Select the largest gap (prefer higher-numbered gaps when tied)
4. If gap length >= 3: start assigning from the beginning of the gap
5. If gap length < 3: start assigning from the end of the gap
6. Process players in slot order

```python
def find_best_gap(used_colors):
    available = [i for i in range(9) if i not in used_colors]
    if not available:
        return (0, 0, 0)

    # Find contiguous sequences
    gaps = []
    current_start = available[0]
    current_end = available[0]

    for i in range(1, len(available)):
        if available[i] == available[i-1] + 1:
            current_end = available[i]
        else:
            gaps.append((current_start, current_end, current_end - current_start + 1))
            current_start = available[i]
            current_end = available[i]
    gaps.append((current_start, current_end, current_end - current_start + 1))

    # Sort by length (desc), then by end position (desc)
    gaps.sort(key=lambda x: (x[2], x[1]), reverse=True)
    return gaps[0]

# Assignment logic
gap_start, gap_end, gap_len = find_best_gap(used_colors)
start_color = gap_start if gap_len >= 3 else gap_end
```

### Winner Detection Algorithm

**Primary Method:** Look for Order 29 (EndGame) - the player who issues this command is on the winning team.

**Fallback Method:** Check Order 1096 (PlayerDefeated) - if all players from one team are defeated, the other team wins.

```python
winner = None
winning_team_raw = None
defeated_players = set()

for chunk in chunks:
    if chunk["order"] == 1096:
        defeated_players.add(chunk["player"])
    elif chunk["order"] == 29:
        endgame_player = chunk["player"]
        endgame_slot = pn_to_slot.get(endgame_player)
        if endgame_slot is not None and endgame_slot in header_players:
            winning_team_raw = header_players[endgame_slot]["team_raw"]
        break

# Fallback: check if all players from one team are defeated
if winning_team_raw is None and defeated_players:
    for team_raw, team_players in team_groups.items():
        if all(pn in defeated_players for pn in team_players):
            # This team lost, other team won
            winning_team_raw = other_team_raw
            break
```

### Player Position Detection

Player starting positions are determined from commands that contain Vec3 coordinates. **Build commands (1049/1050) should be preferred** over unit commands (1071), because a player's first unit command can target enemy territory (e.g., sending a scout), while build commands are always at the player's own base.

```
Priority:
1. First BuildObject (1049) or BuildObject2 (1050) with Vec3 → base position
2. First UnitCommand (1071) with Vec3 → fallback only if no build commands
```

**Game Coordinate System:**
- X axis: < 2500 = Left side, >= 2500 = Right side
- Y axis: > 3000 = Top, 1500-3000 = Middle, < 1500 = Bottom

```python
def get_position_name(x, y):
    side = "LEFT" if x < 2500 else "RIGHT"
    if y > 3000:
        vert = "TOP"
    elif y > 1500:
        vert = "MID"
    else:
        vert = "BOTTOM"
    return f"{vert}_{side}"
```

---

## What We Can Extract

### Confirmed Extractable Data

| Data | Source | Reliability |
|------|--------|-------------|
| Game start time | Header offset 0x08 | High |
| Game end time | Header offset 0x0C | High |
| Game duration | Calculated | High |
| Map name | Header M= field | High |
| Random seed | Header SD= field | High |
| Player names | Header S= field index 0 | High |
| **Player UID** | Header S= field index 1 | High |
| Lobby faction selection | Header S= field index 6 | High |
| Lobby color selection | Header S= field index 4 | High |
| Team assignments | Header S= field index 7 | High |
| Spectators | Header S= field (team = -1) | High |
| Actual faction (if random) | Building IDs in chunks | High |
| Actual color (if random) | Algorithm + used colors | Medium |
| Player positions | Vec3 from build commands | High |
| Winner team | Order 29 or 1096 analysis | High |
| All game commands | Chunk stream | High |

---

## Unexplored Areas

### Header Section

| Offset/Field | Status | Notes |
|--------------|--------|-------|
| Bytes 0x00-0x07 | Unknown | Possibly magic number, version, or flags |
| `MC=` field | Partially known | Map CRC/checksum - purpose unclear |
| `MS=` field | Partially known | Map size - exact usage unclear |
| Player slot field 1 | **Decoded** | 8-char hex Player UID |
| Player slot field 2 | Unknown | `8094` - same for all (port/version?) |
| Player slot field 3 | Unknown | `TT` - same for all (game type?) |
| Player slot field 5 | Unknown | `-1` or `-2` values (handicap/spot?) |
| Player slot fields 8-10 | Unknown | `0,1,0` pattern - boolean flags? |

### Chunk Section

#### Unknown Order Types

Many order types exist that we haven't fully decoded:

| Potential Category | Notes |
|-------------------|-------|
| Unit commands | Attack, move, patrol, guard, etc. |
| Ability usage | Hero powers, special abilities |
| Resource events | Resource gathering, spending |
| Research commands | Upgrades, technologies |
| Formation commands | Unit formations |
| Waypoint commands | Movement waypoints |
| Camera events | Possibly spectator/replay camera |

#### Unknown Argument Types

| Type ID | Size | Possible Purpose |
|---------|------|------------------|
| 0x01 | 4 | Signed integer (counts, deltas?) |
| 0x02 | 1 | Boolean flags |
| 0x03 | 4 | Float values (speeds, timers?) |
| 0x04 | 4 | Object/entity references |
| 0x05 | 8 | Pair of values (start/end?) |
| 0x07 | 12 | Direction vectors |
| 0x08 | 16 | Extended data (quaternions?) |
| 0x09, 0x0A | 4 | Unknown references |

### Potential Hidden Data

| Speculation | Evidence |
|-------------|----------|
| Post-game statistics | Games typically store kills, units lost, resources gathered |
| Detailed timestamps | Sub-second timing for commands |
| Network/sync data | May contain multiplayer sync information |
| Replay metadata | Version info, game settings |
| Unit selection groups | Ctrl+# hotkey groups |
| Chat messages | In-game chat (if any) |

---

## Assumptions and Limitations

### Confirmed Assumptions

1. **Little-endian encoding** - All multi-byte integers use little-endian byte order
2. **Unix timestamps** - Start/end times are Unix epoch seconds
3. **Player num = sequential from occupied slots** - Game assigns pn=3,4,5,... to each occupied (non-empty) slot in order. Empty slots (X/O) are skipped. See "Player Number Mapping" above.
4. **Building IDs are consistent** - Same buildings have same IDs across games (version-dependent)
5. **Build commands reflect base position** - BuildObject (1049/1050) positions are always at the player's base, unlike unit commands which can target anywhere

### Unverified Assumptions

1. **Building ID ranges** - Determined empirically, may vary by game version/patch
2. **Random color algorithm** - Reverse-engineered behavior, edge cases may differ
3. **Chunk parsing** - May miss some chunk types or have incorrect size mappings
4. **Coordinate system** - Thresholds (2500, 1500, 3000) determined for specific maps

### Known Limitations

1. **Version dependency** - File format may differ between game versions/patches
2. **Mod compatibility** - Modded games may have different building/unit IDs
3. **Incomplete order catalog** - Many order types remain undocumented
4. **No official specification** - All information is reverse-engineered

---

## Tools and References

### External References

- [CnC Replay Readers (eareplay.html)](https://github.com/louisdx/cnc-replayreaders/blob/master/eareplay.html) - EA replay format documentation
- [OpenSAGE ReplayMetadata.cs](https://github.com/OpenSAGE/OpenSAGE/blob/master/src/OpenSage.Game/Data/Rep/ReplayMetadata.cs) - Open-source SAGE engine implementation
- [BFME2 INI Files (ValheruGR)](https://github.com/ValheruGR/BFME2/blob/master/1.00/data/ini/playertemplate.ini) - Game data files with faction definitions
- [OpenSAGE Blog] (https://opensage.github.io/blog/replay-file-parsing) - OpenSAGE replay parsing

### Building ID to Faction Reference

From game INI files, each faction has unique starting buildings:
- Men: `MenFortress`, `MenPorter`
- Elves: `ElvenFortress`, `ElvenPorter`
- Dwarves: `DwarvenFortress`, `DwarvenPorter`
- Isengard: `IsengardFortress`, `IsengardPorter`
- Mordor: `MordorFortress`, `MordorPorter`
- Goblins (Wild): `WildFortress`, `WildPorter`

---

## Future Investigation Areas

1. **Complete order type catalog** - Systematically document all order types
2. **Unit type ID mapping** - Map CreateUnit IDs to unit names
3. **Post-game statistics** - Search for kill counts, resources, etc.
4. **Ability/power usage** - Decode hero ability commands
5. **Chat messages** - Locate any in-game communication
6. **Detailed timing** - Understand timecode units (ticks per second)
7. **Multi-version support** - Document differences between game versions
8. **Compression** - Check if any parts are compressed

---

## Changelog

- **2025-02-07**: Corrected player number mapping and position detection
  - Fixed: `player_num = slot + 3` is only correct when no empty slots exist. Actual mapping is sequential assignment (pn=3,4,5,...) over occupied slots only, skipping empty (X/O) entries.
  - Fixed: Position detection now prefers build command positions (1049/1050) over unit command positions (1071), since unit commands can target enemy territory.
  - Updated winner detection examples to use `pn_to_slot` mapping.
- **2025-01-26**: Initial documentation based on reverse engineering
  - Documented header structure and player data format
  - Documented chunk structure and parsing algorithm
  - Added faction detection from building IDs
  - Added random color assignment algorithm
  - Added winner detection methods
  - Listed unexplored areas and assumptions
