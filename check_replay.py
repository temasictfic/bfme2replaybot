"""
BFME2 Replay Checker — shows teams, players, and winner.

Usage:
    python check_replay.py <replay_file> [replay_file2 ...]
    python check_replay.py C:\\path\\to\\*.BfME2Replay
"""

import struct
import sys
import glob
import os
from datetime import datetime, timezone

MAGIC = b"BFME2RPL"

# Command types
CMD_BUILD_OBJECT = 1049
CMD_BUILD_OBJECT_2 = 1050
CMD_END_GAME = 29
CMD_PLAYER_DEFEATED = 1096

# Sanity limits
MAX_SANE_TIMECODE = 10_000_000
MAX_SANE_PLAYER_NUM = 100
MAX_SANE_ARG_TYPES = 100
MAX_SANE_ARG_COUNT = 50

# Map midpoint for Left/Right side determination
MAP_X_MIDPOINT = 2500.0

# SAGE engine ticks per second
SAGE_TICKS_PER_SECOND = 5

# Argument type sizes (from OpenSAGE)
ARG_SIZES = {
    0x00: 4,   # int32
    0x01: 4,   # float
    0x02: 1,   # bool
    0x03: 4,   # ObjectId
    0x04: 4,   # unknown4
    0x05: 8,   # ScreenPosition
    0x06: 12,  # Vec3
    0x07: 12,  # another 12-byte type
    0x08: 16,  # quaternion/camera
    0x09: 4,   # BFME2-specific
    0x0A: 4,   # 4 bytes
}

FACTIONS = {
    -1: "Random",
    0: "Men",
    1: "Goblins",
    2: "Dwarves",
    3: "Isengard",
    4: "Elves",
    5: "Mordor",
    6: "Angmar",
}

# Building ID ranges for faction detection
BUILDING_FACTIONS = [
    (2622, 2720, "Men"),
    (2577, 2620, "Elves"),
    (2541, 2575, "Dwarves"),
    (2151, 2185, "Goblins"),
    (2060, 2090, "Isengard"),
    (2130, 2150, "Mordor"),
]

# Windows-1254 Turkish character map (selected chars)
TURKISH_MAP = {
    0x80: '\u20AC', 0x8A: '\u015E', 0x8C: '\u0152',
    0x9A: '\u015F', 0x9C: '\u0153', 0x9F: '\u0178',
    0xC7: '\u00C7', 0xD0: '\u011E', 0xDD: '\u0130',
    0xDE: '\u015E', 0xE7: '\u00E7', 0xF0: '\u011F',
    0xFD: '\u0131', 0xFE: '\u015F',
}


def decode_turkish(data: bytes) -> str:
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        pass
    result = []
    for b in data:
        if b in TURKISH_MAP:
            result.append(TURKISH_MAP[b])
        elif b < 0x80:
            result.append(chr(b))
        else:
            result.append(chr(b))
    return "".join(result)


def infer_faction_from_building(bid):
    for lo, hi, name in BUILDING_FACTIONS:
        if lo <= bid <= hi:
            return name
    return None


def parse_header(data: bytes):
    if len(data) < len(MAGIC) + 16 or data[:len(MAGIC)] != MAGIC:
        return None

    start_time = struct.unpack_from("<I", data, 8)[0]
    end_time = struct.unpack_from("<I", data, 12)[0]

    # Find M= marker
    header_end = min(len(data), 8192)
    header_bytes = data[16:header_end]

    map_name = None
    marker = b"M="
    for i in range(len(header_bytes) - len(marker)):
        if header_bytes[i:i+len(marker)] == marker:
            start = i + len(marker)
            end = start
            while end < len(header_bytes) and header_bytes[end] != ord(";"):
                end += 1
            if end > start:
                path_str = header_bytes[start:end].decode("utf-8", errors="replace")
                idx = path_str.find("maps/")
                if idx >= 0:
                    map_name = path_str[idx + 5:]
                elif path_str:
                    map_name = path_str
            break

    # Find ;S= marker for players
    players = []
    spectators = []
    occupied_slots = []
    s_marker = b";S="
    for i in range(len(header_bytes) - len(s_marker)):
        if header_bytes[i:i+len(s_marker)] == s_marker:
            start = i + len(s_marker)
            end = start
            while end < len(header_bytes):
                b = header_bytes[end]
                if b == 0 or b == ord("\n") or b == ord("\r"):
                    break
                # Check for next section marker like ;X=
                if (end + 2 < len(header_bytes) and
                        header_bytes[end] == ord(";") and
                        chr(header_bytes[end+1]).isupper() and
                        header_bytes[end+2] == ord("=")):
                    break
                end += 1
            if end > start:
                players_str = decode_turkish(header_bytes[start:end])
                for slot_idx, pdata in enumerate(players_str.split(":")):
                    parsed = parse_player_data(pdata.strip(), slot_idx)
                    if parsed:
                        occupied_slots.append(slot_idx)
                        if parsed["team_raw"] >= 0:
                            players.append(parsed)
                        else:
                            spectators.append(parsed["name"])
            break

    # Find chunks_start (first null byte after the ;S= section)
    chunks_start = None
    s_marker = b";S="
    for i in range(16, min(len(data), 8192)):
        if data[i:i+len(s_marker)] == s_marker:
            for j in range(i, min(len(data), 8192)):
                if data[j] == 0:
                    chunks_start = j + 1
                    break
            break

    return {
        "map_name": map_name,
        "start_time": start_time,
        "end_time": end_time,
        "players": players,
        "spectators": spectators,
        "occupied_slots": occupied_slots,
        "chunks_start": chunks_start,
    }


def parse_player_data(s: str, slot: int):
    s = s.strip()
    if not s or s in ("X", "O", ";"):
        return None

    parts = s.split(",")
    if len(parts) < 8:
        return None

    name = parts[0]
    if name.startswith("H") and len(name) > 1:
        name = name[1:]
    if not name:
        return None

    uid = parts[1] if len(parts) > 1 and len(parts[1]) == 8 else None

    try:
        color_id = int(parts[4]) if len(parts) > 4 else -1
    except ValueError:
        color_id = -1
    try:
        faction_id = int(parts[6]) if len(parts) > 6 else -1
    except ValueError:
        faction_id = -1
    try:
        team_raw = int(parts[7]) if len(parts) > 7 else -1
    except ValueError:
        team_raw = -1

    return {
        "name": name,
        "uid": uid,
        "slot": slot,
        "color_id": color_id,
        "faction_id": faction_id,
        "team_raw": team_raw,
    }


def parse_chunk(data: bytes, offset: int):
    if offset + 13 > len(data):
        return None

    time_code = struct.unpack_from("<I", data, offset)[0]
    order_type = struct.unpack_from("<I", data, offset + 4)[0]
    player_num = struct.unpack_from("<I", data, offset + 8)[0]
    n_arg_types = data[offset + 12]

    if time_code > MAX_SANE_TIMECODE or player_num > MAX_SANE_PLAYER_NUM or n_arg_types > MAX_SANE_ARG_TYPES:
        return None

    pos = offset + 13

    # Read argument signature
    arg_sig = []
    for _ in range(n_arg_types):
        if pos + 2 > len(data):
            return None
        arg_type = data[pos]
        arg_count = data[pos + 1]
        if arg_count > MAX_SANE_ARG_COUNT:
            return None
        arg_sig.append((arg_type, arg_count))
        pos += 2

    # Read arguments
    args = []
    for arg_type, arg_count in arg_sig:
        size = ARG_SIZES.get(arg_type, 4)
        for _ in range(arg_count):
            if pos + size > len(data):
                return None
            if arg_type == 0x06 and size == 12:  # Vec3
                x, y, z = struct.unpack_from("<fff", data, pos)
                args.append(("vec3", x, y, z))
            elif arg_type == 0x00 and size == 4:  # int32
                v = struct.unpack_from("<I", data, pos)[0]
                args.append(("int", v))
            pos += size

    return pos, {
        "time_code": time_code,
        "order_type": order_type,
        "player_num": player_num,
        "args": args,
    }


def raw_scan_critical_events(data: bytes, chunks_start: int, valid_pns: set):
    """Fallback raw byte scan for Order 1096/29 patterns the chunk parser may miss."""
    defeated = set()
    endgame_player = None
    endgame_timecode = 0
    has_endgame = False

    if len(data) < chunks_start + 8:
        return defeated, endgame_player, endgame_timecode, has_endgame

    end = len(data) - 3
    i = chunks_start
    while i < end:
        b = data[i]
        cmd = None
        if b == 0x48 and data[i+1:i+4] == b'\x04\x00\x00':
            cmd = CMD_PLAYER_DEFEATED
        elif b == 0x1d and data[i+1:i+4] == b'\x00\x00\x00':
            cmd = CMD_END_GAME

        if cmd is not None and i >= chunks_start + 4:
            chunk_offset = i - 4
            if chunk_offset + 13 <= len(data):
                tc = struct.unpack_from("<I", data, chunk_offset)[0]
                pn = struct.unpack_from("<I", data, chunk_offset + 8)[0]
                n_args = data[chunk_offset + 12]

                if 0 < tc < MAX_SANE_TIMECODE and 3 <= pn <= 20 and n_args <= 10 and pn in valid_pns:
                    if cmd == CMD_PLAYER_DEFEATED:
                        defeated.add(pn)
                    elif cmd == CMD_END_GAME:
                        if not has_endgame or tc >= endgame_timecode:
                            endgame_player = pn
                            endgame_timecode = tc
                        has_endgame = True
        i += 1

    return defeated, endgame_player, endgame_timecode, has_endgame


def parse_chunks(data: bytes, start: int, header_players, pn_to_slot):
    defeated_players = set()
    endgame_player = None
    endgame_timecode = 0
    has_endgame = False
    max_timecode = 0
    player_positions = {}  # slot -> (x, y)
    player_building_ids = {}  # slot -> set of building IDs
    player_last_build_tc = {}  # pn -> last build command timecode

    valid_slots = {hp["slot"] for hp in header_players}
    build_positions = {}
    unit_positions = {}

    pos = start
    while pos < len(data):
        result = parse_chunk(data, pos)
        if result is None:
            pos += 1
            continue

        next_pos, chunk = result
        tc = chunk["time_code"]
        pn = chunk["player_num"]
        otype = chunk["order_type"]

        if tc > max_timecode:
            max_timecode = tc

        slot = pn_to_slot.get(pn)
        is_valid = slot is not None and slot in valid_slots

        if is_valid and slot is not None:
            # Track last build command timecode per player
            if otype not in (CMD_PLAYER_DEFEATED, CMD_END_GAME):
                if otype in (CMD_BUILD_OBJECT, CMD_BUILD_OBJECT_2):
                    player_last_build_tc[pn] = max(player_last_build_tc.get(pn, 0), tc)

            # Extract position from Vec3 args
            for arg in chunk["args"]:
                if arg[0] == "vec3":
                    x, y = arg[1], arg[2]
                    if 0 < x < 5000 and 0 < y < 5000:
                        if otype in (CMD_BUILD_OBJECT, CMD_BUILD_OBJECT_2):
                            if slot not in build_positions:
                                build_positions[slot] = (x, y)
                        else:
                            if slot not in unit_positions:
                                unit_positions[slot] = (x, y)
                        break

            # Extract building IDs
            if otype in (CMD_BUILD_OBJECT, CMD_BUILD_OBJECT_2):
                for arg in chunk["args"]:
                    if arg[0] == "int" and 2000 < arg[1] < 3000:
                        player_building_ids.setdefault(slot, set()).add(arg[1])
                        break

        # EndGame
        if otype == CMD_END_GAME and is_valid:
            if not has_endgame or tc >= endgame_timecode:
                endgame_player = pn
                endgame_timecode = tc
            has_endgame = True

        # PlayerDefeated
        if otype == CMD_PLAYER_DEFEATED and is_valid:
            defeated_players.add(pn)

        pos = next_pos

    # Raw scan fallback
    valid_pns = {pn for pn, sl in pn_to_slot.items() if sl in valid_slots}
    raw_defeated, raw_eg_player, raw_eg_tc, raw_has_eg = raw_scan_critical_events(
        data, start, valid_pns
    )
    defeated_players.update(raw_defeated)
    if raw_has_eg and (not has_endgame or raw_eg_tc >= endgame_timecode):
        endgame_player = raw_eg_player
        endgame_timecode = raw_eg_tc
        has_endgame = True

    # Merge positions: prefer build positions
    for slot, pos_xy in build_positions.items():
        player_positions[slot] = pos_xy
    for slot, pos_xy in unit_positions.items():
        if slot not in player_positions:
            player_positions[slot] = pos_xy

    # Detect actual factions from buildings
    actual_factions = {}
    for slot, bids in player_building_ids.items():
        for bid in bids:
            f = infer_faction_from_building(bid)
            if f:
                actual_factions[slot] = f
                break

    return {
        "defeated_players": defeated_players,
        "endgame_player": endgame_player,
        "endgame_timecode": endgame_timecode,
        "has_endgame": has_endgame,
        "max_timecode": max_timecode,
        "player_positions": player_positions,
        "actual_factions": actual_factions,
        "player_last_build_tc": player_last_build_tc,
    }


def determine_team_sides(players, positions):
    team_sides = {}
    for p in players:
        pos = positions.get(p["slot"])
        if pos and p["team_raw"] not in team_sides:
            x = pos[0]
            if x > 0 and x < 5000:
                team_sides[p["team_raw"]] = "Left" if x < MAP_X_MIDPOINT else "Right"
    return team_sides


def determine_winner(combat, header_players, team_sides, pn_to_slot):
    # Build team -> [player_nums] mapping
    slot_to_pn = {sl: pn for pn, sl in pn_to_slot.items()}
    team_players = {}
    for hp in header_players:
        pn = slot_to_pn.get(hp["slot"])
        if pn is not None:
            team_players.setdefault(hp["team_raw"], []).append(pn)

    defeated = combat["defeated_players"]

    # Strategy 1: EndGame command
    if combat["endgame_player"] is not None:
        eg_pn = combat["endgame_player"]
        eg_slot = pn_to_slot.get(eg_pn)
        if eg_slot is not None:
            hp = next((p for p in header_players if p["slot"] == eg_slot), None)
            if hp and hp["team_raw"] in team_sides:
                eg_side = team_sides[hp["team_raw"]]
                if eg_pn in defeated:
                    # EndGame player was defeated — their team lost
                    other_side = "Right" if eg_side == "Left" else "Left"
                    if other_side in team_sides.values():
                        return other_side + " Team"
                else:
                    return eg_side + " Team"

    # Strategy 2: Full team defeat
    if defeated:
        for team_raw, pns in team_players.items():
            if all(pn in defeated for pn in pns):
                # This team lost, find the other team
                for other_raw in team_players:
                    if other_raw != team_raw and other_raw in team_sides:
                        return team_sides[other_raw] + " Team"

    # Strategy 3: Majority defeated heuristic
    if defeated and len(team_players) == 2:
        teams = list(team_players.keys())
        defeats_a = sum(1 for pn in team_players[teams[0]] if pn in defeated)
        defeats_b = sum(1 for pn in team_players[teams[1]] if pn in defeated)
        if defeats_a > defeats_b and teams[1] in team_sides:
            return team_sides[teams[1]] + " Team (likely)"
        elif defeats_b > defeats_a and teams[0] in team_sides:
            return team_sides[teams[0]] + " Team (likely)"

    # Strategy 4: Last-build-activity heuristic
    # The team that stopped building first (>5% gap) probably lost.
    # Losing teams still issue sell/demolish commands, but stop constructing earlier.
    max_tc = combat.get("max_timecode", 0)
    player_last_build_tc = combat.get("player_last_build_tc", {})
    if len(team_players) == 2 and max_tc > 0 and player_last_build_tc:
        teams = list(team_players.keys())

        def team_last_build(team):
            builds = [player_last_build_tc.get(pn) for pn in team_players[team]]
            builds = [b for b in builds if b is not None]
            return max(builds) if builds else None

        last_a = team_last_build(teams[0])
        last_b = team_last_build(teams[1])
        if last_a is not None and last_b is not None:
            gap_threshold = max_tc // 20  # 5% of game duration
            gap = abs(last_a - last_b)
            if gap > gap_threshold:
                if last_a > last_b and teams[0] in team_sides:
                    return team_sides[teams[0]] + " Team (likely)"
                elif last_b > last_a and teams[1] in team_sides:
                    return team_sides[teams[1]] + " Team (likely)"

    # Check for crashed/abandoned game
    if not combat["has_endgame"] and not defeated:
        return "Not Concluded"

    return "Unknown"


def format_duration(secs):
    if secs is None:
        return "N/A"
    h, remainder = divmod(secs, 3600)
    m, s = divmod(remainder, 60)
    if h > 0:
        return f"{h}h {m}m {s}s"
    return f"{m}m {s}s"


def format_timestamp(ts):
    if ts == 0:
        return "N/A"
    try:
        return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    except (OSError, ValueError):
        return "N/A"


def analyze_replay(filepath):
    with open(filepath, "rb") as f:
        data = f.read()

    header = parse_header(data)
    if header is None:
        print(f"  ERROR: Invalid replay file (bad header)")
        return

    map_name = header["map_name"] or "Unknown"
    players = header["players"]

    if not players:
        print(f"  ERROR: No players found")
        return

    # Build pn_to_slot
    pn_to_slot = {}
    for i, slot in enumerate(header["occupied_slots"]):
        pn_to_slot[i + 3] = slot

    # Duration from header timestamps
    duration = None
    if header["start_time"] and header["end_time"] and header["end_time"] > header["start_time"]:
        duration = header["end_time"] - header["start_time"]

    # Parse chunks
    winner = "Unknown"
    combat = None
    positions = {}
    actual_factions = {}

    if header["chunks_start"] and header["chunks_start"] < len(data):
        combat = parse_chunks(data, header["chunks_start"], players, pn_to_slot)
        positions = combat["player_positions"]
        actual_factions = combat["actual_factions"]

        team_sides = determine_team_sides(players, positions)
        winner = determine_winner(combat, players, team_sides, pn_to_slot)

        # Use chunk-based duration as fallback if header timestamps unavailable
        if duration is None and combat["max_timecode"] > 0:
            duration = combat["max_timecode"] // SAGE_TICKS_PER_SECOND

    # Group players by team
    teams = {}
    for p in players:
        teams.setdefault(p["team_raw"], []).append(p)

    # Determine sides for display
    team_side_map = determine_team_sides(players, positions) if positions else {}

    # Print results
    print(f"  Map:      {map_name}")
    print(f"  Date:     {format_timestamp(header['start_time'])}")
    print(f"  Duration: {format_duration(duration)}")
    print(f"  Winner:   {winner}")
    print()

    for team_raw in sorted(teams.keys()):
        side = team_side_map.get(team_raw, "?")
        print(f"  [{side} Team]")
        for p in teams[team_raw]:
            faction_id = p["faction_id"]
            lobby_faction = FACTIONS.get(faction_id, f"Unknown({faction_id})")
            actual = actual_factions.get(p["slot"])
            if actual and actual != lobby_faction:
                faction_str = f"{actual} (picked {lobby_faction})"
            else:
                faction_str = lobby_faction if lobby_faction != "Random" else (actual or "Random")

            # Check if defeated
            pn = None
            for k, v in pn_to_slot.items():
                if v == p["slot"]:
                    pn = k
                    break
            defeated_mark = " [DEFEATED]" if (combat and pn in combat["defeated_players"]) else ""

            print(f"    {p['name']:<20s}  {faction_str}{defeated_mark}")
        print()

    if header["spectators"]:
        print(f"  [Spectators]")
        for name in header["spectators"]:
            print(f"    {name}")
        print()


def main():
    if len(sys.argv) < 2:
        print("Usage: python check_replay.py <replay_file> [replay_file2 ...]")
        print("       python check_replay.py C:\\path\\to\\*.BfME2Replay")
        sys.exit(1)

    # Expand globs on Windows
    files = []
    for arg in sys.argv[1:]:
        if "*" in arg or "?" in arg:
            files.extend(glob.glob(arg))
        else:
            files.append(arg)

    if not files:
        print("No files found.")
        sys.exit(1)

    for i, filepath in enumerate(files):
        if not os.path.isfile(filepath):
            print(f"File not found: {filepath}")
            continue

        name = os.path.basename(filepath)
        print(f"{'=' * 60}")
        print(f"  {name}")
        print(f"{'=' * 60}")
        analyze_replay(filepath)

        if i < len(files) - 1:
            print()


if __name__ == "__main__":
    main()
