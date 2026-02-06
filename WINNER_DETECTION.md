# Winner Detection in BFME2 Replays

## Overview

BFME2 replay files (.BfME2Replay) contain a binary header followed by a stream of "chunks" (game commands). Winner detection relies on two specific command types embedded in this chunk stream:

- **Order 29 (CMD_END_GAME)**: Issued by the winning player when the game ends normally. Most reliable signal.
- **Order 1096 (CMD_PLAYER_DEFEATED)**: Issued when a player is eliminated (all buildings destroyed). Used as a fallback.

## Detection Algorithm

Winner detection uses three methods in priority order:

### Method 1: EndGame Command (Order 29) — Certain

The player who issues Order 29 is on the winning team. This is the most reliable method.

```
if endgame_player found:
    winner = endgame_player's team
    confidence = certain
```

### Method 2: All Players Defeated — Certain

If every player on one team has an Order 1096 event, that team lost.

```
for each team:
    if all players on team have Order 1096:
        winner = other team
        confidence = certain
```

### Method 3: Majority-Defeated Heuristic — Likely

When no EndGame event exists and defeats are found but no team is fully eliminated, the team with MORE defeated players is the **likely** loser. This is not certain because one skilled player can eliminate multiple enemies before losing.

```
if no endgame AND defeats exist AND no team fully eliminated:
    if team_A_defeats > team_B_defeats:
        likely_winner = team_B
        confidence = likely (not certain)
```

### Crash Detection

If neither Order 29 nor any Order 1096 events are found (even after raw scan), the game is assumed to have crashed or been abandoned. Reported as "Not Concluded".

## Spectator Handling

Spectators (observers) are identified in the replay header by `team_raw == -1`. They are:

1. **Excluded from player lists** during header parsing
2. **Included in occupied slot tracking**: Spectators occupy slots and receive player_num assignments just like players (the game assigns pn=3,4,5,... to all occupied slots in order, skipping empty X/O entries)
3. **Excluded from defeat/endgame processing**: Only player_nums corresponding to header players (team_raw >= 0) are counted for Order 1096 and Order 29 events
4. **Displayed separately** on the map render (labeled "Obs: name")

### Why This Matters

Spectators have player_num values in the chunk stream (assigned sequentially alongside players from occupied slots). Without filtering, a defeat event on a spectator's player_num would be counted as a real defeat, corrupting winner detection. This was observed in the "Last Replay" where a defeat on spectator Gusto's player_num was incorrectly treated as a player defeat.

Note: Spectators must be included when building the `pn_to_slot` mapping (occupied slots list), because skipping them would shift all subsequent player_num assignments and cause the same kind of mapping bug as empty slots.

## Parser Sync Issues and Raw Scan Workaround

### The Problem

The chunk parser processes chunks sequentially: it reads 13 header bytes (timecode + order + player_num + n_arg_types), then the argument signature, then argument data. If a chunk has unexpected data or the parser misinterprets argument sizes, it loses sync with the byte stream. When sync is lost:

- The parser falls back to `pos += 1` (advance by one byte)
- It tries to re-sync by finding a valid-looking chunk header
- Critical events (Order 1096/29) that occur AFTER the sync loss are missed

This was observed in multiple replays:
- **arafharzeminbuilderald2v3**: Parser found 2/3 known defeats
- **soltran**: Parser found 0/2 known defeats
- **3dwarf**: Parser found 0 defeats despite all 3 players on one team losing buildings

### The Raw Scan Solution

After normal chunk parsing, a raw binary scan searches the entire chunk data region for:

- `\x48\x04\x00\x00` (Order 1096 in little-endian)
- `\x1d\x00\x00\x00` (Order 29 in little-endian)

For each match, context is validated:
- **Timecode** (4 bytes before order): must be 0 < tc < 10,000,000
- **Player_num** (4 bytes after order): must be 3 <= pn <= 20
- **n_args** (1 byte after player_num): must be <= 10
- **Player validity**: player_num must correspond to a header player (not spectator)

Results are merged with chunk parser results (deduplication by player_num + event type).

### Limitations

The raw scan can produce false positives if the byte pattern appears in argument data. The context validation (timecode range, player_num range, n_args range) mitigates this but cannot eliminate it entirely. In practice, false positives are rare because the 12-byte context window (tc + order + pn) must all pass validation simultaneously.

## Color Mapping Bug

### Symptom

In the 3dwarf replay, player `mustafaa` and `Gusto` have `color_id=-1` (random) and renders as Green and Orange but should be White and Red respectively.

### Current Algorithm

The current color assignment for random-color players uses a "gap" strategy:
1. Find all contiguous gaps in available colors (0-8, excluding 9/white)
2. Pick the largest gap
3. If gap >= 3 colors, start assigning from the gap's start; otherwise from the gap's end
4. Assign sequentially within the gap

### Suspected Correct Algorithm

Testing suggests BFME2 assigns random colors starting from the **highest available color** and working downward. For the 3dwarf case:
- Used colors: {0 (Blue), 1 (Red), 2 (Yellow), 3 (Green), 4 (Orange)}
- Available (high to low): 9 (White), 8 (Gray), 7 (Pink), 6 (Purple), 5 (Teal)
- Expected assignment: White (9) — matches in-game observation
- Current algorithm gives: Green (gap-based) — wrong

This needs further investigation across more replays with random-color players to confirm the "sequential from highest available" hypothesis.

## Known Problem Replays

### arafharzeminbuilderald2v3
- **Known events**: HarzemShah defeated -> ESCOBAR defeated -> MiSKiN defeated -> Pan sold buildings -> game ends
- **Issue**: Chunk parser found only 2/3 defeats (MiSKiN missed due to parser sync loss)
- **Fix**: Raw scan recovers the missing defeat event

### soltran
- **Known events**: The_King_ defeated -> dz_free defeated -> StreetBoy sold buildings -> game ends
- **Issue**: Chunk parser found 0/2 defeats (complete sync loss before defeat events)
- **Fix**: Raw scan recovers both defeat events

### Last Replay
- **Known events**: Felix defeated -> DK about to lose -> game ended abruptly
- **Issue**: Parser reported a defeat on spectator Gusto's player_num instead of Felix
- **Fix**: Spectator exclusion filter prevents counting spectator defeats

### 3dwarf
- **Known events**: C__, AKINCI#, ALPHA all lost buildings (3 players on same team)
- **Issue**: Chunk parser found 0 defeats (complete sync loss)
- **Fix**: Raw scan recovers defeat events
- **Color issue**: mustafaa shows as Green instead of White (color mapping bug)

### Replay 07 (Aki+Pri+Gab vs Dan+Bos+Hil)
- **Known events**: 6 players, slot 3 is empty (X)
- **Issue**: `player_num = slot + 3` formula broke for all players after the empty slot. Players after the gap got wrong positions; last player had no position at all (invisible).
- **Fix**: Build `pn_to_slot` mapping from occupied slots instead of using `slot + 3`.

### Replay 02
- **Known events**: 6 players, slot 1 is empty (X)
- **Issue**: Same empty-slot shift bug as Replay 07, positions shifted by 1 for all players after slot 1.
- **Fix**: Same `pn_to_slot` mapping fix.

### Replay 08
- **Known events**: BOSS's first command is a unit command (1071) targeting enemy territory at (703,195)
- **Issue**: Position detection used first Vec3-providing command regardless of type. Unit commands can target anywhere on the map, so BOSS appeared at the enemy's base position.
- **Fix**: Prefer build command positions (1049/1050) over unit command positions (1071). Build commands are always at the player's own base.

### Replay 03
- **Known events**: BOSS's first command is a unit command targeting enemy territory at (4297,781)
- **Issue**: Same position-priority bug as Replay 08. BOSS rendered at enemy side.
- **Fix**: Same build-position priority fix.

## Architecture

### Files

| File | Role |
|------|------|
| `src/parser/replay.rs` | Main Rust parser: header parsing, chunk parsing, raw scan, winner detection |
| `src/models/replay.rs` | Data models: Player, Winner enum (LeftTeam/RightTeam/Likely*/NotConcluded/Unknown) |
| `src/renderer/map.rs` | Map image renderer (Rust) |
| `render_map.py` | Standalone Python map renderer (mirrors Rust logic) |
| `analyze_winners_final.py` | Batch analysis of all replays with detailed winner detection reporting |
| `investigate_raw_1096.py` | Investigation tool: compares raw scan vs chunk parser results across all replays |

### Winner Enum Values

| Variant | Meaning | Confidence |
|---------|---------|------------|
| `LeftTeam` | Left side team won | Certain (EndGame or all-defeated) |
| `RightTeam` | Right side team won | Certain (EndGame or all-defeated) |
| `LikelyLeftTeam` | Left side likely won | Likely (majority-defeated heuristic) |
| `LikelyRightTeam` | Right side likely won | Likely (majority-defeated heuristic) |
| `NotConcluded` | Game crashed/abandoned | N/A |
| `Unknown` | Could not determine | N/A |

## Further Investigation Needed

1. **Color mapping**: Confirm "sequential from highest available" hypothesis across more replays
2. **Order 1096 arg data**: The defeated player command may carry additional data in its arguments (e.g., which player/team defeated them). Currently unused.
3. **Parser sync root cause**: Investigate which specific chunk types/argument patterns cause the parser to lose sync. The OpenSAGE argument type table may be incomplete for BFME2 Rise of the Witch King.
4. **Sell-all detection**: Some games end when a player sells all their buildings rather than being defeated. This does not generate Order 1096. Currently undetected.
5. **Multiple EndGame events**: Some replays may have multiple Order 29 events (e.g., if multiple players trigger end-game). Currently the one with the highest timecode (latest) is used.
6. **Observer with team number**: Observers can optionally carry a team number (0-3) even though it doesn't affect gameplay. Currently observers are identified by `team_raw < 0` (i.e., -1). An observer with a team number would be misclassified as an active player.
