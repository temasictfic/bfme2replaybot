import struct
from collections import defaultdict
from datetime import datetime

from PIL import Image, ImageDraw, ImageFont

replay_path = r"C:\Users\user\AppData\Roaming\My Battle for Middle-earth(tm) II Files\Replays\ereninyıkılışıgöremedim aq.BfME2Replay"
map_image_path = r"C:\Users\user\Desktop\dcreplaybot\assets\maps\map wor rhun.jpg"
output_path = r"C:\Users\user\Desktop\dcreplaybot\output_map.jpg"

# Faction ID mapping from header (lobby selection)
FACTION_ID_TO_NAME = {
    0: "Men",
    1: "Goblins",
    2: "Dwarves",
    3: "Isengard",
    4: "Elves",
    5: "Mordor",
    -1: "Random",  # Will be inferred from buildings
}

# Improved faction detection using building ID ranges
def detect_faction_from_buildings(buildings):
    """
    Detect faction based on building IDs using range-based detection.
    Returns faction name or None if can't determine.

    Building ID ranges (empirically determined):
    - Men: 2622-2720
    - Elves: 2577-2620
    - Dwarves: 2541-2575
    - Goblins: 2151-2185
    - Isengard: 2060-2090
    - Mordor: 2130-2150
    """
    for bid in buildings:
        # Men: 2622-2720 range
        if 2622 <= bid <= 2720:
            return "Men"
        # Elves: 2577-2620 range
        if 2577 <= bid <= 2620:
            return "Elves"
        # Dwarves: 2541-2575 range
        if 2541 <= bid <= 2575:
            return "Dwarves"
        # Goblins: 2151-2185 range
        if 2151 <= bid <= 2185:
            return "Goblins"
        # Isengard: 2060-2090 range
        if 2060 <= bid <= 2090:
            return "Isengard"
        # Mordor: 2130-2150 range
        if 2130 <= bid <= 2150:
            return "Mordor"
    return None

# In-game player colors (10 colors from BFME2)
# Color ID from header maps to these RGB values
# Mapping based on empirical testing (IDs don't match visual UI order)
PLAYER_COLORS = {
    0: (70, 91, 156),       # Blue
    1: (158, 56, 42),       # Red
    2: (175, 189, 76),      # Yellow
    3: (62, 152, 100),      # Green
    4: (206, 135, 69),      # Orange
    5: (122, 168, 204),     # Teal/Cyan (Light Blue)
    6: (148, 116, 183),     # Purple (was Pink)
    7: (204, 159, 188),     # Pink/Magenta (was Purple)
    8: (100, 100, 100),     # Gray
    9: (226, 226, 226),     # White
    -1: None,               # Random color - will be assigned
}


# Faction colors (used as fallback and for faction text)
FACTION_COLORS = {
    "Men": (30, 144, 255),      # Dodger Blue
    "Elves": (0, 200, 83),      # Green
    "Dwarves": (205, 133, 63),  # Peru/Bronze
    "Isengard": (255, 255, 255),# White
    "Mordor": (139, 0, 0),      # Dark Red
    "Goblins": (85, 107, 47),   # Dark Olive
    "Unknown": (128, 128, 128), # Gray
}

# Circle center coordinates in pixels on the original 1624x1620 map asset.
# At render time these are scaled to match the actual (resized) image dimensions.
MAP_ASSET_WIDTH = 1624
MAP_ASSET_HEIGHT = 1620

POSITION_COORDS = {
    "TOP_LEFT":     (272, 336),
    "MID_LEFT":     (198, 896),
    "BOTTOM_LEFT":  (344, 1370),
    "TOP_RIGHT":    (1330, 336),
    "MID_RIGHT":    (1370, 850),
    "BOTTOM_RIGHT": (1314, 1420),
}


def get_position_name(x, y):
    """Convert game coordinates to position name."""
    side = "LEFT" if x < 2500 else "RIGHT"
    if y > 3000:
        vert = "TOP"
    elif y > 1500:
        vert = "MID"
    else:
        vert = "BOTTOM"
    return f"{vert}_{side}"


def format_duration(duration):
    """Format timedelta to MM:SS or HH:MM:SS format."""
    total_seconds = int(duration.total_seconds())
    hours = total_seconds // 3600
    minutes = (total_seconds % 3600) // 60
    seconds = total_seconds % 60
    if hours > 0:
        return f"{hours}:{minutes:02d}:{seconds:02d}"
    return f"{minutes}:{seconds:02d}"


# Parse replay file
with open(replay_path, "rb") as f:
    data = f.read()

# Parse header timestamps
ts1 = struct.unpack("<I", data[8:12])[0]
ts2 = struct.unpack("<I", data[12:16])[0]
start_time = datetime.fromtimestamp(ts1)
end_time = datetime.fromtimestamp(ts2)
duration = end_time - start_time

# Parse map name
m_start = data.find(b"M=") + 2
m_end = data.find(b";", m_start)
map_raw = data[m_start:m_end].decode("ascii")
map_name = map_raw.split("maps/")[1] if "maps/" in map_raw else map_raw

# Parse players from header
s_start = data.find(b";S=") + 3
s_end = data.find(b"\x00", s_start)
# Try encodings that support Turkish characters (İ, ş, ğ, ü, ö, ç)
players_bytes = data[s_start:s_end]
players_raw = None
for encoding in ["utf-8", "windows-1254", "iso-8859-9", "windows-1252"]:
    try:
        players_raw = players_bytes.decode(encoding)
        break
    except (UnicodeDecodeError, LookupError):
        continue
if players_raw is None:
    players_raw = players_bytes.decode("ascii", errors="replace")

header_players = {}
spectators = []
used_colors = set()
occupied_slots = []  # Slot indices of all non-empty entries (players AND spectators)

for slot_idx, slot in enumerate(players_raw.split(":")):
    if slot.startswith("H") and "," in slot:
        parts = slot.split(",")
        if len(parts) >= 8:
            name = parts[0][1:]
            color_id = int(parts[4]) if parts[4].lstrip("-").isdigit() else -1
            faction_id = int(parts[6]) if parts[6].lstrip("-").isdigit() else -1
            team_raw = int(parts[7]) if parts[7].lstrip("-").isdigit() else -1

            occupied_slots.append(slot_idx)

            if team_raw >= 0:
                # Active player
                header_players[slot_idx] = {
                    "name": name,
                    "color_id": color_id,
                    "faction_id": faction_id,
                    "team_raw": team_raw,
                }
                if color_id >= 0:
                    used_colors.add(color_id)
            else:
                # Obs (team_raw is -1 or similar)
                spectators.append({"name": name})

# Build pn_to_slot: game engine assigns pn=3,4,5,... to each occupied slot in order
pn_to_slot = {i + 3: slot for i, slot in enumerate(occupied_slots)}
slot_to_pn = {slot: pn for pn, slot in pn_to_slot.items()}

# Find chunks_start (first null byte after the ;S= section)
s_marker_pos = data.find(b";S=")
header_end = data.find(b"\x00", s_marker_pos)
chunks_start = header_end + 1

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
    if tc > 10000000 or player > 100 or n_args > 100:
        return None, None
    pos = offset + 13
    arg_sig = []
    for _ in range(n_args):
        if pos + 2 > len(data):
            return None, None
        arg_sig.append((data[pos], data[pos + 1]))
        if data[pos + 1] > 50:
            return None, None
        pos += 2
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


chunks = []
pos = chunks_start
while pos < len(data) - 13:
    next_pos, chunk = parse_chunk(data, pos)
    if chunk:
        chunks.append(chunk)
        pos = next_pos
    else:
        pos += 1

# Find positions and buildings per player
# Track build and unit positions separately (prefer build positions for accuracy)
build_positions = {}   # pn -> first build command position
unit_positions = {}    # pn -> first unit command position
player_buildings = defaultdict(set)
player_last_build_tc = {}  # pn -> last build command timecode
max_timecode = 0
valid_player_slots = set(header_players.keys())

for c in chunks:
    tc = c["tc"]
    if tc > max_timecode:
        max_timecode = tc

    pn = c["player"]
    slot = pn_to_slot.get(pn)
    is_valid = slot is not None and slot in valid_player_slots

    if is_valid:
        # Track build command timecodes
        if c["order"] in [1049, 1050]:
            player_last_build_tc[pn] = max(player_last_build_tc.get(pn, 0), tc)

    if c["order"] in [1049, 1050, 1071]:
        for arg_type, arg_val in c["args"]:
            if arg_type == "vec3":
                if is_valid:
                    if c["order"] in [1049, 1050]:
                        if pn not in build_positions:
                            build_positions[pn] = arg_val
                    else:
                        if pn not in unit_positions:
                            unit_positions[pn] = arg_val
                break
    if c["order"] in [1049, 1050]:
        for arg_type, arg_val in c["args"]:
            if arg_type == "int" and 2000 < arg_val < 3000:
                player_buildings[c["player"]].add(arg_val)

# Merge positions: prefer build positions, fall back to unit positions
player_positions = {}
for pn, pos in build_positions.items():
    player_positions[pn] = pos
for pn, pos in unit_positions.items():
    if pn not in player_positions:
        player_positions[pn] = pos

# Assign random colors to players who didn't choose
# Game assigns colors from the largest gap in available colors
def find_best_gap(used):
    """Find the largest contiguous gap in available colors (0-8, excluding 9/white)
       Returns (start, end, length) of the best gap. Prefers higher IDs when tied."""
    available = [i for i in range(9) if i not in used]  # Exclude 9 (white)
    if not available:
        return (0, 0, 0)

    # Find all contiguous sequences
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

    # Sort by length (desc), then by end position (desc) for ties
    gaps.sort(key=lambda x: (x[2], x[1]), reverse=True)
    return gaps[0]

best_gap = find_best_gap(used_colors)
gap_start, gap_end, gap_len = best_gap

# If gap is large (>= 3 colors), start from the beginning of the gap
# If gap is small (< 3 colors), start from the end of the gap
if gap_len >= 3:
    start_color = gap_start
else:
    start_color = gap_end

for slot, info in sorted(header_players.items()):  # Process in slot order
    if info["color_id"] == -1:
        # Find next available, cycling through
        for offset in range(10):
            color_id = (start_color + offset) % 10
            if color_id not in used_colors:
                info["assigned_color_id"] = color_id
                used_colors.add(color_id)
                start_color = color_id + 1
                break

# Build final player data
final_players = []
for slot, info in sorted(header_players.items()):
    pn = slot_to_pn.get(slot)
    if pn is None:
        continue

    # Get position from chunks
    position = player_positions.get(pn)

    # Determine faction - always use building-based detection
    # (header faction_id can be incorrect due to lobby swaps or other mechanics)
    buildings = player_buildings.get(pn, set())
    actual_faction = detect_faction_from_buildings(buildings)
    if not actual_faction:
        # Fallback to header faction if building detection fails
        lobby_faction = FACTION_ID_TO_NAME.get(info["faction_id"], "Unknown")
        actual_faction = lobby_faction if lobby_faction != "Random" else "Unknown"

    # Determine player color
    if info["color_id"] >= 0 and info["color_id"] in PLAYER_COLORS:
        player_color = PLAYER_COLORS[info["color_id"]]
    elif "assigned_color_id" in info and info["assigned_color_id"] in PLAYER_COLORS:
        player_color = PLAYER_COLORS[info["assigned_color_id"]]
    else:
        # Fallback to faction color
        player_color = FACTION_COLORS.get(actual_faction, (128, 128, 128))

    # Determine position name and image coords
    pos_name = get_position_name(position[0], position[1]) if position else None
    img_pos = POSITION_COORDS.get(pos_name) if pos_name else None

    # Team mapping - map team_raw to team 1 or 2
    # Get unique team values and assign 1 to first, 2 to second
    team_values = sorted(set(p["team_raw"] for p in header_players.values()))
    team_map = {tv: i + 1 for i, tv in enumerate(team_values)}
    team = team_map.get(info["team_raw"], 1)

    final_players.append({
        "name": info["name"],
        "faction": actual_faction,
        "color": player_color,
        "faction_color": FACTION_COLORS.get(actual_faction, (128, 128, 128)),
        "team": team,
        "position_name": pos_name,
        "image_position": img_pos,
    })

# Determine which team is on which side (LEFT or RIGHT) based on player positions
# Build mapping from team_raw to side
team_to_side = {}
for slot, info in header_players.items():
    pn = slot_to_pn.get(slot)
    if pn is None:
        continue
    position = player_positions.get(pn)
    if position:
        x = position[0]
        side = "Left" if x < 2500 else "Right"
        team_raw = info["team_raw"]
        if team_raw not in team_to_side:
            team_to_side[team_raw] = side

# Collect game events from chunks
# Order 29 = EndGame, Order 1096 = PlayerDefeated
defeated_players = set()
endgame_player = None
endgame_timecode = 0
has_endgame = False

# Build set of valid player_nums
valid_player_nums = set()
for pn, slot in pn_to_slot.items():
    if slot in header_players:
        valid_player_nums.add(pn)

for c in chunks:
    pn = c["player"]
    if pn not in valid_player_nums:
        continue
    if c["order"] == 1096:
        defeated_players.add(pn)
    elif c["order"] == 29:
        # Keep the latest EndGame by timecode
        if not has_endgame or c["tc"] >= endgame_timecode:
            endgame_player = pn
            endgame_timecode = c["tc"]
        has_endgame = True


def raw_scan_for_critical_events(data, chunks_start, valid_pns):
    """
    Raw binary scan fallback for Order 1096 (PlayerDefeated) and Order 29 (EndGame).
    The chunk parser can lose sync and miss events. This scans raw bytes for the order
    patterns and validates context (timecode, player_num) to recover missed events.
    """
    found_defeats = set()
    found_endgame_pn = None
    found_endgame_tc = 0
    found_has_endgame = False

    if len(data) < chunks_start + 8:
        return found_defeats, found_endgame_pn, found_endgame_tc, found_has_endgame

    end = len(data) - 3
    i = chunks_start
    while i < end:
        b = data[i]
        cmd = None
        if b == 0x48 and data[i+1:i+4] == b'\x04\x00\x00':
            cmd = 1096
        elif b == 0x1d and data[i+1:i+4] == b'\x00\x00\x00':
            cmd = 29

        if cmd is not None and i >= chunks_start + 4:
            chunk_offset = i - 4
            if chunk_offset + 13 <= len(data):
                tc = struct.unpack_from("<I", data, chunk_offset)[0]
                player_num = struct.unpack_from("<I", data, chunk_offset + 8)[0]
                n_args = data[chunk_offset + 12]

                if 0 < tc < 10_000_000 and 3 <= player_num <= 20 and n_args <= 10 and player_num in valid_pns:
                    if cmd == 1096:
                        found_defeats.add(player_num)
                    elif cmd == 29:
                        if not found_has_endgame or tc >= found_endgame_tc:
                            found_endgame_pn = player_num
                            found_endgame_tc = tc
                        found_has_endgame = True
        i += 1

    return found_defeats, found_endgame_pn, found_endgame_tc, found_has_endgame


# Raw scan fallback: recover events the chunk parser may have missed
raw_defeats, raw_eg_pn, raw_eg_tc, raw_has_eg = raw_scan_for_critical_events(
    data, chunks_start, valid_player_nums
)
defeated_players.update(raw_defeats)
if raw_has_eg and (not has_endgame or raw_eg_tc >= endgame_timecode):
    endgame_player = raw_eg_pn
    endgame_timecode = raw_eg_tc
    has_endgame = True

# Determine winner using chained strategies (matching Rust bot logic)
winner = "Unknown"

# Build team -> [player_nums] mapping
team_players = {}
for slot, info in header_players.items():
    pn = slot_to_pn.get(slot)
    if pn is not None:
        team_players.setdefault(info["team_raw"], []).append(pn)

# Strategy 1: EndGame command
if endgame_player is not None:
    eg_slot = pn_to_slot.get(endgame_player)
    if eg_slot is not None and eg_slot in header_players:
        eg_team_raw = header_players[eg_slot]["team_raw"]
        eg_side = team_to_side.get(eg_team_raw)
        if eg_side is not None:
            if endgame_player in defeated_players:
                # EndGame player was defeated — their team LOST, the other team won
                other_side = "Right" if eg_side == "Left" else "Left"
                if other_side in team_to_side.values():
                    winner = f"{other_side} Team"
            else:
                winner = f"{eg_side} Team"

# Strategy 2: Full team defeat
if winner == "Unknown" and defeated_players:
    for team_raw, players_pn in team_players.items():
        if all(pn in defeated_players for pn in players_pn):
            # This team lost, find the other team
            for other_team_raw in team_players:
                if other_team_raw != team_raw and other_team_raw in team_to_side:
                    winner = f"{team_to_side[other_team_raw]} Team"
                    break
            break

# Strategy 3: Majority-defeated heuristic
if winner == "Unknown" and defeated_players and len(team_players) == 2:
    teams = list(team_players.keys())
    defeats_a = sum(1 for pn in team_players[teams[0]] if pn in defeated_players)
    defeats_b = sum(1 for pn in team_players[teams[1]] if pn in defeated_players)
    if defeats_a > defeats_b and teams[1] in team_to_side:
        winner = f"{team_to_side[teams[1]]} Team (likely)"
    elif defeats_b > defeats_a and teams[0] in team_to_side:
        winner = f"{team_to_side[teams[0]]} Team (likely)"

# Strategy 4: Last-build-activity heuristic
# The team that stopped building first (>5% gap) probably lost.
if winner == "Unknown" and len(team_players) == 2 and max_timecode > 0 and player_last_build_tc:
    teams = list(team_players.keys())

    def team_last_build(team):
        builds = [player_last_build_tc.get(pn) for pn in team_players[team]]
        builds = [b for b in builds if b is not None]
        return max(builds) if builds else None

    last_a = team_last_build(teams[0])
    last_b = team_last_build(teams[1])
    if last_a is not None and last_b is not None:
        gap_threshold = max_timecode // 20  # 5% of game duration
        gap = abs(last_a - last_b)
        if gap > gap_threshold:
            if last_a > last_b and teams[0] in team_to_side:
                winner = f"{team_to_side[teams[0]]} Team (likely)"
            elif last_b > last_a and teams[1] in team_to_side:
                winner = f"{team_to_side[teams[1]]} Team (likely)"

# Crashed/abandoned game detection
if winner == "Unknown" and not has_endgame and not defeated_players:
    winner = "Not Concluded"

# Load and render map image
print(f"Loading map image: {map_image_path}")
img = Image.open(map_image_path).convert("RGB")
width, height = img.size
print(f"Image size: {width}x{height}")

draw = ImageDraw.Draw(img)

# Try to load fonts with Turkish/Unicode support
# Segoe UI has excellent Unicode support including Turkish characters (İ, ş, ğ, etc.)
font_large = None
font = None
font_small = None

for font_name in ["segoeui.ttf", "tahoma.ttf", "arial.ttf"]:
    try:
        font_large = ImageFont.truetype(font_name, 28)
        font = ImageFont.truetype(font_name, 24)
        font_small = ImageFont.truetype(font_name, 20)
        break
    except:
        continue

if font is None:
    font_large = ImageFont.load_default()
    font = font_large
    font_small = font_large

# Scale factors from asset coordinates to rendered image coordinates
scale_x = width / MAP_ASSET_WIDTH
scale_y = height / MAP_ASSET_HEIGHT

# Draw player info at each position (text centered on circle center)
for p in final_players:
    if not p["image_position"]:
        continue

    # Scale asset pixel coords to rendered image coords
    cx = int(p["image_position"][0] * scale_x)
    cy = int(p["image_position"][1] * scale_y)

    # Draw player name centered on circle
    name = p["name"][:12]  # Truncate if too long
    name_bbox = draw.textbbox((cx, cy - 12), name, font=font, anchor="mm")
    draw.rectangle(
        [name_bbox[0] - 3, name_bbox[1] - 2, name_bbox[2] + 3, name_bbox[3] + 2],
        fill=(0, 0, 0, 180),
    )
    draw.text((cx, cy - 12), name, fill=p["color"], font=font, anchor="mm")

    # Draw faction below name (centered)
    faction_text = p["faction"]
    faction_bbox = draw.textbbox((cx, cy + 12), faction_text, font=font_small, anchor="mm")
    draw.rectangle(
        [faction_bbox[0] - 3, faction_bbox[1] - 2, faction_bbox[2] + 3, faction_bbox[3] + 2],
        fill=(0, 0, 0, 180),
    )
    draw.text(
        (cx, cy + 12),
        faction_text,
        fill=p["color"],
        font=font_small,
        anchor="mm",
    )

# Draw centered info (Start Date, Duration, Winner)
center_x = width // 2
center_y = height // 2

# Format info text
date_text = start_time.strftime("%Y-%m-%d %H:%M")
duration_text = format_duration(duration)

# Draw info with semi-transparent background
info_lines = [
    (f"Date: {date_text}", (255, 255, 255)),
    (f"Duration: {duration_text}", (200, 200, 200)),
]
if winner != "Unknown":
    info_lines.append((f"Winner: {winner}", (255, 215, 0)))  # Gold color for winner

line_height = 24
total_height = len(info_lines) * line_height
start_y = center_y - total_height // 2

# Calculate max width for background
max_width = 0
for text, _ in info_lines:
    bbox = draw.textbbox((0, 0), text, font=font)
    text_width = bbox[2] - bbox[0]
    max_width = max(max_width, text_width)

# Draw background rectangle
padding = 10
bg_rect = [
    center_x - max_width // 2 - padding,
    start_y - padding,
    center_x + max_width // 2 + padding,
    start_y + total_height + padding,
]
draw.rectangle(bg_rect, fill=(0, 0, 0, 160))

# Draw info text
for i, (text, color) in enumerate(info_lines):
    y_pos = start_y + i * line_height
    draw.text((center_x, y_pos), text, fill=color, font=font, anchor="mt")

# Draw spectators above and below center
if spectators:
    spectator_font = font_small

    # First spectator above center (0, 1) -> top half
    if len(spectators) >= 1:
        spec_y = int(height * 0.1)  # Near top
        spec_text = f"Obs: {spectators[0]['name']}"
        bbox = draw.textbbox((center_x, spec_y), spec_text, font=spectator_font, anchor="mt")
        draw.rectangle(
            [bbox[0] - 3, bbox[1] - 2, bbox[2] + 3, bbox[3] + 2], fill=(0, 0, 0, 160)
        )
        draw.text((center_x, spec_y), spec_text, fill=(180, 180, 180), font=spectator_font, anchor="mt")

    # Second spectator below center (0, -1) -> bottom half
    if len(spectators) >= 2:
        spec_y = int(height * 0.9)  # Near bottom
        spec_text = f"Obs: {spectators[1]['name']}"
        bbox = draw.textbbox((center_x, spec_y), spec_text, font=spectator_font, anchor="mt")
        draw.rectangle(
            [bbox[0] - 3, bbox[1] - 2, bbox[2] + 3, bbox[3] + 2], fill=(0, 0, 0, 160)
        )
        draw.text((center_x, spec_y), spec_text, fill=(180, 180, 180), font=spectator_font, anchor="mt")

# Save the image
img.save(output_path)
print(f"\nSaved rendered map to: {output_path}")

# Print summary
print(f"\n=== Game Result ===")
print(f"Winner: {winner}")
print("\n=== Player Summary ===")
for p in final_players:
    print(f"  {p['name']}: {p['faction']} - Team {p['team']} - {p['position_name']}")
    print(f"    Color: RGB{p['color']}")
if spectators:
    print("\n=== Obss ===")
    for s in spectators:
        print(f"  {s['name']}")
