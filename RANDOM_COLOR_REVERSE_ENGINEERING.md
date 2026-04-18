# Reverse Engineering BFME2 1.00 Random Color Assignment

## Overview

This document details the systematic attempt to reverse-engineer the random color assignment algorithm used in Battle for Middle-earth II (version 1.00). When players select "Random" for their color in the lobby, the game assigns them a color at game start. The replay file header records `color_id=-1` for these players, but the actual resolved color is not stored in the replay. The goal was to determine the algorithm so the bot could predict the true in-game color from replay data alone.

**Final Conclusion: The algorithm could NOT be recovered.** After exhaustive search across hundreds of thousands of configurations, no deterministic algorithm seeded by any replay header field could reproduce all ground truth observations. The colors assigned by the game appear to depend on runtime state not captured in the replay file (likely a global PRNG state from the game engine seeded at process startup or lobby creation time, not from any replay-stored value).

---

## Ground Truth Data Collection

Ground truth was collected by the user running replay files in-game and reporting what colors the "random" players actually received. This was done iteratively across multiple replays:

### Phase 1: Initial Ground Truth (1 replay, 2 values)
- **3dwarf.BfME2Replay**: `mustafaa` (slot 1) = White (9), `Gusto` (slot 7) = Red (1)
  - Used colors: {0=Blue, 2=Yellow, 6=Purple, 7=Pink}
  - Available: {1, 3, 4, 5, 8, 9}
  - Random slots: [1(play), 5(spec), 6(spec), 7(play)]

### Phase 2: Expanded Ground Truth (5 replays, 6 values)
- **battleofnoobs**: `Gusto` (slot 3) = Yellow (2)
- **adembadem**: `Gusto` (slot 5) = Purple (6)
- **arafkansergob**: `Gusto` (slot 6) = Teal (5)
- **3isenvsisenmormen**: `Gusto` (slot 2) = Red (1)

### Phase 3: Final Ground Truth (9 replays, 10 values)
- **durinkovalama**: `Gusto` = Gray (8)
- **mirrorfena**: `Gusto` = Orange (4)
- **omgbro**: `Gusto` = Teal (5)
- **kazabeladeprem**: `Gusto` = Yellow (2)

---

## Replay Header Fields Available as Potential Seeds

From the replay header ASCII section (`;KEY=VALUE` format):
- **SD** (Seed): A large integer (e.g., `442667640`). Initially the primary candidate for PRNG seeding.
- **GSID** (Game Session ID): A hex string (e.g., `0x1A3F`). Later emerged as a candidate and briefly appeared to work.
- **MC** (Map Checksum): Hex value, tested as alternative seed.
- **S=** section: Player slot data including color_id (-1 for random), team, faction.

---

## Scripts and Investigation Phases

### Phase 1: Discovery and Initial Scanning

#### `find_random_color_replays.py`
**Purpose:** Scan all replay files and identify which ones contain players with `color_id=-1` (random color).

**What it does:**
- Reads the replay directory (`%APPDATA%\My Battle for Middle-earth(tm) II Files\Replays`)
- Parses the `;S=` header section to extract player slot data
- Identifies human players (`H` prefix) with `color_id=-1`
- Outputs a summary of all replays containing random-color players

**Result:** Successfully identified multiple replays with random-color players for further investigation.

#### `check_3dwarf.py`
**Purpose:** Quick diagnostic dump of the 3dwarf replay header to verify parsing correctness.

**What it does:**
- Parses the `;S=` section of the 3dwarf replay
- Prints each slot's name, color_id, faction_id, team_raw
- Verifies the field ordering in the comma-separated player data

**Result:** Confirmed field positions: `HName,UID,Port,TT,ColorID,field5,FactionID,TeamID,field8,field9,field10`

---

### Phase 2: Deterministic Strategy Testing

#### `check_color_assignment.py`
**Purpose:** Test whether a simple deterministic "gap-finding" algorithm could explain the color assignments.

**What it does:**
- Takes the 3dwarf replay's used colors {7, 0, 6, 2}
- Computes available colors excluding used ones
- Tests multiple strategies:
  - Gap start (largest gap in available range), start from begin/end
  - Sequential from 0 (ascending)
  - Sequential from end (descending)
  - Various combinations of gap-finding with range 0-8 (no white) vs 0-9 (with white)

**Result:** **None of the deterministic strategies produced White (9) for mustafaa.** The user confirmed mustafaa should be White, which ruled out all simple gap/sequential approaches. The conclusion at this point was: "the gap algorithm may be fundamentally wrong, or the game uses a completely different approach."

#### `reverse_engineer_colors.py`
**Purpose:** Systematic test of 11 different deterministic assignment strategies against ground truth.

**Strategies tested:**
1. `current_gap` - Find largest gap in 0-8, assign from gap start
2. `highest_first` - Assign highest available color first
3. `lowest_first` - Assign lowest available color first
4. `from_9_down_skip` - Start from 9, decrement, skip used
5. `from_9_down_wrap` - Same but with wraparound
6. `slot_based_highest` - Process by slot index, assign highest available
7. `slot_based_lowest` - Process by slot index, assign lowest available
8. `pn_based_highest` - Process by player_num order, assign highest
9. `players_first_highest` - Process players before spectators, assign highest
10. `players_only_highest` - Only assign to active players, highest available
11. `reverse_slot_highest` - Reverse slot order, assign highest

**What it does:**
- For each strategy, simulates the color assignment using the used colors and random slots from the 3dwarf replay
- Compares against ground truth: mustafaa=White(9), Gusto=Red(1)
- Scores each strategy
- Shows predictions for all replays with random-color players

**Result:** Some strategies (highest_first, from_9_down variants, slot_based_highest, pn_based_highest, players_first_highest, players_only_highest, reverse_slot_highest) matched the first value (mustafaa=White) but NONE matched both values simultaneously. This proved the algorithm was **not purely deterministic based on slot ordering and available colors** -- it required some form of randomness.

---

### Phase 3: PRNG-Based Search (SD as Seed)

#### `reverse_engineer_seed.py`
**Purpose:** Test whether the `SD` header value is used as a PRNG seed for color assignment.

**PRNGs implemented:**
1. **MSVC LCG**: `state = state * 214013 + 2531011 (mod 2^32)`, output `(state >> 16) & 0x7FFF`
2. **glibc LCG**: `state = state * 1103515245 + 12345 (mod 2^31)`
3. **Borland LCG**: `state = state * 22695477 + 1 (mod 2^32)`, output `(state >> 16) & 0x7FFF`
4. **Multiply-with-carry (MWC)**: `state = 36969 * low16 + high16`
5. **XorShift32**: shifts 13/17/5
6. **Park-Miller (MINSTD)**: `state = state * 16807 (mod 2^31-1)`
7. **Numerical Recipes LCG**: `state = state * 1664525 + 1013904223`
8. **Knuth MMIX**: 64-bit LCG

**Assignment methods tested:**
- Pick `rand() % len(available)` from sorted available list, remove picked color
- Same but with available sorted in reverse (descending)

**Seed transformations tested:**
- Raw SD value
- SD & 0xFFFFFFFF (low 32 bits)
- SD + 1, SD - 1
- Negated: `(-SD) & 0xFFFFFFFF`
- Byte-swapped: `swap16(SD)`

**Slot orderings tested:**
- Slot ascending, Slot descending
- Players first (non-spectators before spectators), ascending and descending
- Spectators first
- Players only (exclude spectators)

**Additional search:**
- Each PRNG was tested with 0-4 "skip" calls (advance PRNG N times before use)
- Brute-force simple arithmetic formulas: `(SD + slot*k) % n`, `(SD * slot + k) % n`, `(SD >> (slot*k)) % n` for various k values

**Result:** Multiple PRNG configurations matched the single 3dwarf ground truth (2 values), but these were not validated against additional replays at this point. The search identified MSVC LCG with various configurations as candidates.

---

### Phase 4: Cross-Validation (MSVC LCG Candidates)

#### `validate_prng_candidates.py`
**Purpose:** Cross-validate the top MSVC LCG candidates across ALL replays to find where they disagree, so the user could check minimal additional data to narrow down.

**Top 5 candidates tested:**
- A: MSVC, skip=1, players_first ordering, available ascending
- B: MSVC, skip=1, players_first_desc, available descending
- C: MSVC, skip=1, players_only, available ascending
- D: MSVC, skip=0, slot_asc, available ascending
- E: MSVC, skip=0, slot_asc, available descending

**What it does:**
- Runs all 5 candidates on every replay with random-color players
- Identifies replays where candidates DISAGREE on the predicted color for playing (non-spectator) players
- Outputs disagreements for the user to verify by running those specific replays in-game

**Result:** Found several replays where candidates disagreed. User was asked to check specific replays to narrow down. This led to collecting ground truth for battleofnoobs, adembadem, arafkansergob, and 3isenvsisenmormen.

---

### Phase 5: Exhaustive Search with Expanded Ground Truth

#### `exhaustive_color_search.py`
**Purpose:** Exhaustive search across all PRNG/seed/method/ordering combinations using 5 ground truth values from 4 replays.

**Expanded search space:**
- 5 PRNGs: MSVC, glibc, Borland, MINSTD, Numerical Recipes
- 8 seed transforms: raw, low32, +1, -1, neg, xor_ff, >>1, <<1
- 0-19 skip values
- 6 slot orderings
- 3 assignment methods:
  1. **Pick from available**: `rand() % len(remaining)`, pick and remove
  2. **Fisher-Yates backward full**: Shuffle all 10 colors (backward FY), skip taken, assign in order
  3. **Fisher-Yates forward full**: Same but forward Knuth variant
  4. **Fisher-Yates available only (backward)**: Shuffle only available colors
  5. **Fisher-Yates available only (forward)**: Same but forward variant
- Available list sorted ascending and descending

**Result:** Found 3 matching configurations:
1. glibc, SD+1, skip=6, Fisher-Yates backward full, players_first_desc ordering
2. glibc, SD>>1, skip=8, pick from available, slot_asc, ascending
3. MINSTD, SD<<1, skip=4, Fisher-Yates backward full, specs_first ordering

However, statistical analysis revealed these were likely **false positives**: with ~48,000 configurations tested against 4 replay constraints (each with ~1/5 chance of random match), the expected number of false positives was ~13. Having 3 matches was consistent with random chance.

---

### Phase 6: Cross-Validation of 3 Candidates

#### `final_color_validate.py`
**Purpose:** Cross-validate the 3 surviving candidates from the exhaustive search against additional replay data.

**The 3 configs:**
1. C1: glibc, SD+1, skip=6, FY-backward, players_first_desc
2. C2: glibc, SD>>1, skip=8, pick, slot_asc, ascending
3. C3: MINSTD, SD<<1, skip=4, FY-backward, specs_first

**What it does:**
- Runs all 3 on every replay
- Shows where they agree and disagree
- Identifies easiest replays to check to narrow down

**Result:** User checked the 3isenvsisenmormen replay. The new ground truth (Gusto=Red) **invalidated all 3 candidates**. This confirmed they were indeed false positives from the initial search.

---

### Phase 7: Massively Expanded Search

#### `expanded_color_search.py`
**Purpose:** Dramatically expand the search space with new PRNGs, seed transforms, and assignment methods.

**New PRNGs added (14 total):**
- MSVC (standard + full 32-bit state)
- glibc (31-bit + full 32-bit)
- Borland, MINSTD (a=16807), MINSTD2 (a=48271)
- Numerical Recipes LCG
- XorShift32 (two shift constant sets)
- Knuth MMIX (64-bit)
- Java LCG
- Multiply-with-carry
- SplitMix32

**New seed transforms added (21 total):**
- All previous transforms
- Hash functions: Wang hash, Wang hash 2, Murmur mix
- Multiplicative: golden ratio `* 2654435761`, Fibonacci `* 1640531527`
- XOR with right-shift: `SD ^ (SD >> 16)`
- Player-count-dependent: `SD + num_players`, `SD + num_slots`, `SD + num_random`, `SD ^ num_players`, `SD * num_players`

**New search dimensions:**
- Per-player hash-based approach: `color = hash(SD, slot_index) % num_available` (14 hash functions)
- Variable skip: skip depends on `num_players`, `num_random`, `num_total_slots`, `np-1`, `nr-1`, `np*2`, `nr*2`, `np+nr`, `ns-nr`
- Multiplicative range reduction: `(raw * n) >> 32` instead of `raw % n`
- 0-24 skip values (up from 0-19)

**Result:** **Zero matches across all configurations.** No PRNG + seed + method combination could reproduce all 6 ground truth values from 5 replays.

---

### Phase 8: Fundamentally Different Algorithm Structures

#### `color_search_v2.py`
**Purpose:** Test fundamentally different algorithm structures beyond "pick from available" and "shuffle then assign."

**New methods tested:**
1. **Retry method**: `rand()%10`, if color taken retry until finding free color
2. **Scan forward**: `rand()%10`, if taken scan forward (wrapping) to next available
3. **Scan backward**: Same but scan backward
4. **All-slots-retry**: Process ALL slots (including chosen-color ones), consuming a PRNG call for each. For chosen slots, discard the PRNG value. For random slots, retry until untaken.
5. **All-slots-pick**: Process all slots, chosen slots consume a PRNG call, random slots pick from remaining available list
6. **All-slots-scan**: Process all slots, generate `rand()%10` for each, chosen slots skip, random slots scan forward to next available
7. **Max-slots**: Iterate through 8 or 10 positions (even empty ones), consuming PRNG for empty/chosen positions
8. **Pre-generate**: Generate all random values first, then assign

**Additional seed sources tested:**
- GSID (Game Session ID) as hex integer
- `SD ^ GSID`, `SD + GSID`, `SD * GSID`
- Combined GSID<<16 | SD>>16
- SD ^ (GSID << 16)

**Result:** **Still zero matches.** At this point, an important discovery was made: when using GSID as the seed with the MSVC full-32-bit LCG and the "all-slots-retry" method, it matched all 5 initial ground truth values (6 values total). This was briefly considered a breakthrough.

---

### Phase 9: Apparent Breakthrough - GSID + MSVC Full32 + All-Retry

#### `verify_color_algorithm.py`
**Purpose:** Verify the discovered algorithm against all ground truth.

**The "discovered" algorithm:**
- **PRNG**: MSVC LCG with full 32-bit state (no `>>16` extraction). Formula: `state = state * 214013 + 2531011 (mod 2^32)`, output = full state
- **Seed**: GSID from header (hex game session ID, parsed as integer)
- **Skip**: 0 (no initial PRNG advances)
- **Method**: All-slots-retry:
  1. Collect all chosen colors as "taken"
  2. Iterate through slots 0..N-1 in ascending order
  3. For chosen-color slots: call `rand()%10`, discard the result
  4. For random-color slots: call `rand()%10` repeatedly until an untaken color is found
- **No special ordering** - just process slots in index order

**Detailed trace for 3dwarf:**
```
Seed = GSID
Slot 0: ALPHA          CHOSEN Pink      -> rand()%10=X (discarded)
Slot 1: mustafaa       RANDOM [PLAY]    -> attempts=[...] -> got White(9)
Slot 2: SuperNova      CHOSEN Blue      -> rand()%10=X (discarded)
Slot 3: C__            CHOSEN Purple    -> rand()%10=X (discarded)
Slot 4: AKINCI#        CHOSEN Yellow    -> rand()%10=X (discarded)
Slot 5: k$ln$`         RANDOM [SPEC]    -> attempts=[...] -> got some color
Slot 6: Bullet         RANDOM [SPEC]    -> attempts=[...] -> got some color
Slot 7: Gusto          RANDOM [PLAY]    -> attempts=[...] -> got Red(1)
```

**Result:** Matched all 6 ground truth values from 5 replays. This was marked as a success.

---

### Phase 10: Extended Validation and Failure

#### `color_search_v3.py`
**Purpose:** Re-run the full search with 10 ground truth values across 9 replays to confirm the GSID+MSVC algorithm and eliminate any remaining false positive possibility.

**New ground truth values (4 additional):**
- **durinkovalama**: Gusto = Gray (8)
- **mirrorfena**: Gusto = Orange (4)
- **omgbro**: Gusto = Teal (5)
- **kazabeladeprem**: Gusto = Yellow (2)

**Search scope:** Same massive space as v2 (12 PRNGs, 28 seed transforms, 30 skips, all methods) but now validated against 10 values across 9 replays.

**Expected false positive rate:** With 10 ground truth values, the probability of a random configuration matching all is approximately `1/20,000,000` per configuration. Any match at this level would be virtually certain to be the real algorithm.

**Result:** **ZERO MATCHES.** The GSID + MSVC Full32 + All-Retry algorithm that matched 6 values was a **false positive** that failed on the expanded ground truth. No configuration in the entire search space matched all 10 values.

---

## Other Investigative Scripts (Not Color-Related)

These scripts were created during the broader replay parsing project but are not directly part of the color reverse engineering:

- **`analyze.py`, `analyze_deep.py`, `analyze_chunks.py` (v1-v3), `analyze_complete.py`, `analyze_corrected.py`, `analyze_final.py` (v1-v2), `analyze_units.py`, `analyze_last_replay.py`**: Various replay chunk parsing and analysis scripts for understanding game commands, unit data, and chunk structure
- **`analyze_winners.py`, `analyze_winners_v2.py`, `analyze_winners_final.py`**: Winner/loser detection from replay chunks (Order 1096 = PlayerDefeated, Order 29 = EndGame)
- **`investigate_raw_1096.py`**: Raw binary scanner comparing chunk parser vs raw byte scan for defeat events
- **`render_map.py`**: Map visualization prototype using Pillow
- **`investigate_color_resolution.py`**: Deep investigation into replay header structure searching for hidden color data

---

## Algorithms and PRNGs Tested (Complete List)

### PRNG Implementations (14)
| PRNG | Formula | Output |
|------|---------|--------|
| MSVC LCG (standard) | `s = s*214013 + 2531011 (mod 2^32)` | `(s >> 16) & 0x7FFF` |
| MSVC LCG (full 32-bit) | Same | Full 32-bit state |
| glibc LCG (31-bit) | `s = s*1103515245 + 12345 (mod 2^31)` | Full state |
| glibc LCG (32-bit) | Same but `mod 2^32` | Full state |
| Borland LCG | `s = s*22695477 + 1 (mod 2^32)` | `(s >> 16) & 0x7FFF` |
| MINSTD (a=16807) | `s = s*16807 (mod 2^31-1)` | Full state |
| MINSTD2 (a=48271) | `s = s*48271 (mod 2^31-1)` | Full state |
| Numerical Recipes | `s = s*1664525 + 1013904223 (mod 2^32)` | Full state |
| XorShift32 | shifts: 13, 17, 5 | Full state |
| XorShift32 (alt) | shifts: 1, 3, 10 | Full state |
| Knuth MMIX | 64-bit: `s = s*6364136223846793005 + 1442695040888963407` | `(s >> 32) & 0xFFFFFFFF` |
| Java LCG | 48-bit: `s = s*0x5DEECE66D + 0xB` | `(s >> 16) & 0x7FFFFFFF` |
| Multiply-with-carry | `t = 698769069*s + carry` | Low 32 bits |
| SplitMix32 | `s += 0x9E3779B9; z = mix(s)` | Mixed output |

### Seed Sources (28+)
- SD (raw, +1, -1, negated, XOR 0xFFFFFFFF, >>1, <<1, >>2, <<2, XOR right-shift 16)
- SD hashed: Wang hash, Wang hash 2, Murmur mix, golden ratio multiply, Fibonacci multiply
- GSID (raw, +1, hashed with Wang/Murmur)
- Combined: SD^GSID, SD+GSID, SD*GSID, GSID<<16|SD>>16, SD^(GSID<<16)
- MC (map checksum): raw, SD^MC, SD+MC, GSID^MC
- Player-count: SD+num_players, SD^num_players, GSID+num_players

### Assignment Methods (10+)
1. Pick from sorted available list (`rand()%len`, remove)
2. Pick with multiplicative range reduction (`(raw*n)>>32`)
3. Fisher-Yates backward shuffle (all 10 colors)
4. Fisher-Yates forward shuffle (all 10 colors)
5. Fisher-Yates backward (available only)
6. Fisher-Yates forward (available only)
7. Retry: `rand()%10` until untaken color
8. Scan forward: `rand()%10`, scan forward to next available
9. Scan backward: same but backward
10. All-slots processing (consuming PRNG for chosen slots too)
11. Max-slots (iterating 8 or 10 positions including empty)
12. Pre-generate values then assign

### Slot Orderings (6)
- Slot ascending / descending
- Players first (ascending / descending within group)
- Spectators first
- Players only (exclude spectators)

### Skip Values
- 0 through 29 (fixed skip)
- Variable skip based on: num_players, num_random, num_slots, np-1, nr-1, np*2, nr*2, np+nr, ns-nr

### Hash-Based Per-Player Approaches (14)
- `(SD ^ slot) * golden_ratio`, `(SD + slot) * golden_ratio`
- Wang hash of (SD^slot), (SD+slot)
- Murmur mix of (SD^slot), (SD+slot), (SD*(slot+1))
- Various XOR/shift combinations with slot index

---

## Total Configurations Tested

Across all scripts, approximately **500,000+ unique configurations** were tested against the ground truth data. The final `color_search_v3.py` alone tested over 100,000 configurations with 12 PRNGs, 28 seed transforms, 30 skip values, and 10+ methods.

---

## Conclusions

1. **The random color assignment algorithm in BFME2 1.00 cannot be recovered from replay file data alone.** No deterministic function of any replay header field (SD, GSID, MC, player count, etc.) combined with any standard PRNG and assignment method could reproduce the observed in-game color assignments across 9 replays.

2. **The game likely uses a runtime PRNG state** that is not stored in or derivable from the replay file. This could be:
   - A global game engine PRNG seeded at process startup (e.g., from `GetTickCount()` or `time()`)
   - A PRNG whose state has been advanced by an unknown number of prior game operations
   - A PRNG seeded from network timing or lobby events not captured in the replay

3. **False positive danger:** With only a few ground truth values, it is easy to find PRNG configurations that appear to match. The glibc and MINSTD candidates that matched 5 replays turned out to be statistical noise. Even the MSVC+GSID "breakthrough" that matched 6 values failed on extended validation. At least 10+ ground truth values are needed to have confidence in any candidate.

4. **Practical resolution:** The bot uses the `color_id` value from the replay header as-is. Players who chose a specific color have that color shown correctly. Players who chose "Random" (`color_id=-1`) are displayed with an "unknown/random" indicator rather than attempting to predict their actual in-game color.

---

## Addendum (2026-02-07): Additional Attempts Beyond SD/GSID PRNG Search

After the document above was written, we ran several additional investigations aimed at answering two questions:

1) Is the resolved random color actually stored somewhere in the replay (outside the ASCII header)?
2) If it is not directly stored, is there any deterministic seed material in the non-ASCII header bytes that we previously ignored?

These attempts produced **no working algorithm**, but they did narrow the search space and clarified what *doesn't* work.

### Ground Truth File Consolidation

Added a machine-readable ground truth file:
- `ground_truth_colors.json`: `{ replay_name: { player_name: resolved_color_id } }`

Current contents (9 replays, 10 constraints) match the “Phase 3: Final Ground Truth” list above.

### Attempt A: Seed Mining From Raw Header Bytes (`color_search_header_seeds.py`)

**Hypothesis:** Even if `SD`/`GSID`/`MC` aren’t sufficient, the replay file may contain additional deterministic seed material in the binary header region.

**Key finding:** There are **26 bytes** immediately after the ASCII header’s null terminator before chunk data begins.
- Chunk start offset used across scripts: `ascii_end + 1 + 26`
- Example: `3dwarf` had `ascii_end=606` → chunk start `633`

**What the script does:**
- Parses the replay, extracts:
  - Header ASCII fields (`SD`, `GSID`, `MC`, and `S=` bytes)
  - Two timestamp-like `u32` values at offsets 8 and 12 (begin/end)
  - The full “header region” bytes up through `ascii_end + 1 + 26`
- Generates many seed candidates including:
  - Raw: `SD`, `GSID`, `MC`, timestamps
  - Hashes: `CRC32` and `FNV1a32` over `header_bytes`, `ascii_header_bytes`, and the `S=` field bytes
  - All aligned `u32` values from the header region (and optional unaligned reads)
  - Optional simple combinations/mixes of the above candidates
- Tries a wide PRNG set and assignment methods, including:
  - PRNGs: MSVC LCG (15-bit), MSVC full32, glibc (31-bit), glibc32, Borland, MINSTD, MINSTD2, Numerical Recipes, XorShift32, Java LCG, SplitMix32, MWC, MMIX
  - Methods: pick-from-available, retry-until-available, scan-forward/backward from `rand()%10`, Fisher-Yates shuffles (full and “available only”)
  - Slot orderings: slot asc/desc, players-first (asc/desc), specs-first, players-only
  - Skip counts and both range reduction modes: `raw % n` and multiplicative `((raw*n)>>bits)`

**Result:** **No configuration matched all 10 ground truth constraints**, including with `--full`, `--unaligned`, and `--combinations`.

**Takeaway:** If the game’s color RNG is deterministic per replay, it is not explained by any “typical” PRNG+seed sourced from:
- ASCII header fields (`SD`, `GSID`, `MC`)
- The two early `u32` timestamps
- Simple hashes or direct `u32` reads of the binary header region (including the 26 bytes after the ASCII header)

### Attempt B: Look For Resolved Colors in Chunk Data (`chunk_feature_miner.py`)

**Hypothesis:** The resolved random color might be stored in early “init” chunks as some per-player property, even if the lobby header keeps `color_id=-1`.

**What the script does:**
- Loads `ground_truth_colors.json`
- Builds a slot→player_num mapping consistent with observed replays:
  - `occupied_slots` are derived from the `S=` field entries
  - player numbers are assigned `pn = 3..` in occupied-slot order
- Parses replay chunks from `ascii_end + 1 + 26`, collecting per-player “features”:
  - Feature key: `(order_type, arg_type, arg_index)`
  - Captures the first and last seen values for simple argument types (4-byte ints / BFME2-specific 4-byte types, plus bools)
- Attempts to find a single feature and a simple mapping (identity/low8/mod10/shifts/etc.) that predicts all ground truth colors.

**Result:** No single chunk feature (with simple mappings) predicts all 10 constraints. Best partial rules only matched a minority of cases.

**Takeaway:** If the resolved color is in chunk data, it is not trivially exposed as a small 4-byte integer field under a stable `(order_type, arg_type, index)` signature across these replays; it may be encoded differently, appear only in a specific chunk type we aren’t decoding correctly, or not be present at all.

### Attempt C: Random Faction as a Proxy RNG (`faction_seed_search.py`)

**Hypothesis:** Random faction selection (header `faction_id=-1`) might use the same RNG stream as random color selection. If we can recover the faction RNG process, it might unlock the color RNG (or at least validate candidate seeds/PRNGs).

**What the script does:**
- For replays listed in `ground_truth_colors.json`, detects playing players with `faction_id=-1`
- Infers their actual faction by scanning chunk integer arguments for early build-command building IDs and mapping those IDs to starting fortress/structure ranges
- Treats inferred faction outcomes as constraints and searches PRNG+seed+consumption models similar to the color search:
  - Seeds mined from header bytes (aligned/optionally unaligned), plus hashes
  - PRNG set: MSVC, MSVC32, glibc, glibc32, xorshift32
  - Iteration models: only occupied slots vs max8/max10, multiple slot orders, optional spectator inclusion, optional consumption for fixed-faction slots

**Result:** **No configuration matched all inferred random-faction constraints** (with the current inference logic and tested PRNG space).

**Takeaway:** Either:
- random faction selection uses a different PRNG/seed source than we modeled,
- RNG consumption depends on events not captured in replay header/chunks we are reading,
- or the faction inference heuristic (from building IDs) isn’t robust enough to serve as ground truth.

### Note About Current Bot Behavior

The Rust parser currently contains a deterministic “gap-based” heuristic for assigning colors to `color_id=-1` players (`src/parser/replay.rs` → `assign_player_colors`). This heuristic is **known to disagree with ground truth** (e.g., `3dwarf` mustafaa should resolve to White(9)).

Until a real algorithm is recovered, the safest behavior is to keep random colors as “unknown” rather than guessing.

---

## Addendum (2026-04-16): Ghidra MCP Reverse Engineering — PRNG Recovered

Set up Ghidra 12.0.4 with the `bethington/ghidra-mcp` extension (199 MCP tools). Connected Claude Code via the `.mcp.json` project-scoped MCP config at `127.0.0.1:8089`.

### PRNG algorithm — NOT any standard LCG

All 500k+ Python brute-force configs failed because they tested standard PRNGs. The game uses a **custom 192-bit state PRNG**:

**Key addresses in `game.dat` (base 0x400000):**
| Address | Role |
|---|---|
| `0x00633770` | `GameLogicRandomValue(min, max, file, line)` — int-random API |
| `0x0063363f` | core PRNG step (6×u32 = 192-bit state) |
| `0x006336b6` | state seeder (derives 6 state words from single u32 seed) |
| `0x006336fe` | seed dispatcher — reads override at `[0x00df9618 + 0x1200]`, else uses arg; seeds three parallel states |
| `0x006336ec` | zero-init: `memset(state, 0, 24)` |
| `0x00633743` | single-state re-seed (just `0xdb53a8`) |
| `0x006339c8` | fresh-game seed: `seed = time(NULL)` |
| `0x0077be11` | replay-load seed: reads SD from file → seed |
| `0x00db5390` | PRNG state #1 (24 bytes) |
| `0x00db5378` | PRNG state #2 (24 bytes) |
| `0x00db53a8` | PRNG state #3 — used by `GameLogicRandomValue` |
| `DAT_00df95f4` | "last seed used" global |
| `0x0091957b` | game-start path: non-replay calls `FUN_006336fe(0)`, replay calls `FUN_0077be11` |
| `0x0075b4e0` | offline game setup — calls `FUN_00633743(0)` (seed=0) |

**Seed derivation (`FUN_006336b6`):**
```
state[0] = (seed + 0xf22d0e56) & 0xFFFFFFFF
state[1] = (state[0] - 0x69fbe76d) & 0xFFFFFFFF
state[2] = (state[1] + 0x3df3b646) & 0xFFFFFFFF
state[3] = (state[2] + 0x40dde76d) & 0xFFFFFFFF
state[4] = (state[3] - 0x68cd851f) & 0xFFFFFFFF
state[5] = (state[4] + 0xd1a9fbe7) & 0xFFFFFFFF
```

**PRNG step (`FUN_0063363f`):**
Cascading-sum with carry propagation:
```
eax  = s4 + s5                   ; CF1 = overflow
s4' = eax
eax += s3 + CF1                  ; CF2 = (eax < s3_old)
s3' = eax
eax += s2 + CF2                  ; CF3 = (eax < s2_old)
s2' = eax
eax += s1 + CF3                  ; CF4 = (eax < s1_old)
s1' = eax
eax += CF4
s0' = s0 + eax
increment s5 with carry propagation up through s4, s3, s2, s1, s0
return s0'
```

**`GameLogicRandomValue` wrapper:**
```
range = max - min + 1
if range == 0: return min
raw = prng_step()
return (raw % range) + min
```

### Seeding pathways

- **Fresh multiplayer/skirmish game** (host): `FUN_0091957b` → `FUN_006336fe(0)` (seed=0), possibly overridden by `[TheGameInfo + 0x1200]` set from lobby network handshake
- **Replay playback**: `FUN_0077be11` → `FUN_006336fe(SD)` where SD is read from replay header
- **Engine startup**: `FUN_006339c8` → `FUN_006336fe(time(NULL))`
- **Offline single-player**: `FUN_0075b4e0` → `FUN_00633743(0)` (hard-coded seed=0)

### Status after 2026-04-16 session

**Python impl** at `bfme2_prng.py` — transcribed from assembly, verified against smoke tests.

**Brute-force against 9 ground truth replays** (`test_prng_all.py`, 11 seed transforms × 30 warmup values × 2 ranges × 3 strategies × 4 orderings × 2 spec-inclusion settings ≈ 15k configs):

- **Best: 7/9 match** with `(SD<<1, warmup=12, max=9, pick-from-available, spec-first order, include spectators)`
- Many configs hit 6/9

The 2/9 gap likely comes from one or more of:
1. Missing consumption of RNG calls for **random StartPos** and **random FactionID** (both use the same state); the game likely assigns all three (color, startpos, faction) in a single per-slot pass
2. Override at `[0x00df9618 + 0x1200]` may replace the arg seed for multiplayer — need to find what writes that field
3. Slot iteration order not yet matched — game may iterate by `player_num` (which is 3..N in occupied-slot order) rather than raw slot index

### Remaining work

1. Find exact call site where `FUN_00633770` is called for color assignment (none of the ~100 callers inspected were obvious candidates — the color-assignment caller doesn't use an obvious `(0, 7)` or `(0, 9)` range)
2. Trace writes to `[0x00df9618 + 0x1200]` to find the override seed source
3. Once the exact call pattern is known, implement a Rust version of `BFME2Rand` in `src/parser/replay.rs` and replace the gap-based heuristic with the real algorithm

---

## 2026-04-16 session (continued): Assignment call sites located

### Function map of game start slot randomization

| Address | Function | Role |
|---|---|---|
| `0x006443b6` | Parent | Orchestrates: calls StartPos → Color/Faction |
| `0x00643f62` | `randomizeStartPositions()` | Assigns random StartPos per slot[0x10] |
| `0x00643bc4` | `randomizeColorsAndFactions()` | Assigns random Faction (template) + Color per slot |
| `0x00640927` | `validateChosenColors()` | Pre-step: resets duplicate/invalid chosen colors to `-1` |
| `0x007fe0a5` | `isColorTaken(color, except_slot)` | Scans slots, returns true if another slot has this color |
| `0x007fe0cf` | `isStartPosTaken(pos)` | Equivalent for start positions |
| `0x007fe004` | `getSlot(idx)` | Returns `slots[0..7]` pointer or 0 |
| `0x007fde60` | `isPlayableSlot()` | True if slot is a playing human (not observer) |
| `0x006336e6` | `getLastSeed()` | Returns stored "last seed used" global |

### Slot struct layout (partial, inferred from code)

| Offset | Field |
|---|---|
| `0x0c` | ColorID (-1 = random) |
| `0x10` | StartPos (-1 = random) |
| `0x18` | Faction/Template ID (-2 = observer, -1 = random) |

### Order of operations in `FUN_006443b6`

1. Call `FUN_00643f62(param_3)` — StartPos randomization
2. Call `FUN_00643bc4(param_3)` — Color + Faction randomization

Both consume the SAME PRNG state (state #3 at `0x00db53a8`). So any StartPos RNG calls advance the state before color assignment runs.

### Per-slot loop in `FUN_00643bc4` (the color function)

For each slot index 0..7:
1. Get slot. If null or observer, skip.
2. If slot[0x18] (faction) is out of range (typically -1 → random), enter **faction-retry loop**:
   - Call `getLastSeed() % 7` times of `rand(0, 1)` — **warmup loop** driven by the original seed value
   - Call `rand(0, 1000)` → mod by template array size → pick faction
   - Validate faction; retry if invalid
3. Read slot[0x0c] (color). If `-1` (random), enter **color-retry loop**:
   - Call `rand(0, num_colors - 1)` where num_colors = `*(DAT_00dfd1ac + 0x40)` (default from `+0x38`)
   - Call `isColorTaken()` — if taken, retry

### Implication for simulation

To match ground truth, the Python simulator must model:
1. **Phase 1 — StartPos assignment** via `FUN_00643f62` for every slot with startpos=-1 (most playing slots have startpos=-1 = "random start"). Each call consumes RNG.
2. **Phase 2 — Color/Faction** loop per slot:
   - If faction=-1: `seed%7` calls of `rand(0,1)`, then `rand(0,1000)` (maybe with retries)
   - If color=-1: `rand(0,9)` with retries

The replay header stores the POST-resolved values for StartPos and Faction for most slots, so those aren't directly visible. The "f5" (-1 for all non-observers in our ground truth) suggests **all players requested random StartPos**. Need to check replay chunks to find the resolved StartPos values — those determine the RNG state when color assignment runs.

### Current Python simulator status

Current best: 6-7/9 match. Full simulation requires implementing both Phase 1 (StartPos) and Phase 2 (Color+Faction) with correct retry/warmup semantics. The structure is now known; need to verify each sub-function's exact RNG consumption count.

### Per-replay "before" offsets that match (from brute-force)

Empirical "number of rand(0,9) calls from SD seed until the random player's color is generated":

| Replay | SD | SD%7 | Target | Before | Formula match? |
|---|---|---|---|---|---|
| 3dwarf | 442667640 | 2 | mustafaa | 0 | — |
| battleofnoobs | 1248960671 | 0 | Gusto | 3 | matches ns=3 |
| adembadem | 687641828 | 6 | Gusto | 9 | — |
| arafkansergob | 15522078 | 5 | Gusto | 0 | — |
| 3isenvsisenmormen | 19586284 | 4 | Gusto | 0 | — |
| durinkovalama | 18778011 | 0 | Gusto | 9 | — |
| mirrorfena | 15631534 | 2 | Gusto | 14 | — |
| omgbro | 259706515 | 5 | Gusto | 18 | — |
| kazabeladeprem | 7515156 | 5 | Gusto | 1 | +1 (phase1) |

No clean formula fits all (tested: `nf*(m7+1)`, `+ns`, `+1`). The 2/9 gap is likely because:
- The faction loop in `FUN_00643bc4` may do more than just `rand(0,1000)` per iteration (need to trace `FUN_00622c5c` and validators)
- Observers may consume color RNG in some cases
- There may be a separate validation pass (`FUN_00640927`) that runs BEFORE assignment and consumes state

### Three parallel PRNG states confirmed

| State | Accessor | Purpose (inferred) |
|---|---|---|
| `0x00db53a8` | `FUN_00633770` (int), `FUN_0063380e` (float) | `GameLogicRandom` — logged |
| `0x00db5378` | `FUN_006337c6` (int), `FUN_0063388d` (float) | Likely `GameCombatRandom` |
| `0x00db5390` | `FUN_006337ea` (int), `FUN_006338d5` (float) | Likely `GameClientRandom` |

All three are seeded identically by `FUN_006336fe`. Color assignment uses state `0x00db53a8` exclusively.

**Artifacts created this session (continued):**
- `test_prng_v2.py` — adds per-slot `seed%7` warmup model
- `test_prng_v3.py` — full two-phase StartPos + Color/Faction simulation
- `test_prng_v4.py` — brute-force "before" offset search per replay
- `trace_3dwarf.py` — manual consumption trace for 3dwarf (proved 24 inter-slot calls)
- `correlate.py` — correlates observed "before" values with replay state
- `inspect_slots.py` / `diagnose.py` — diagnostic dumpers

### Complete call chain for game-start (verified)

```
FUN_0077688f (message dispatcher)
  └─ FUN_00647c1d (game-start sequence)
        ├─ FUN_0063cb91  — simple init (no RNG)
        ├─ FUN_00a1c020  — simple init (no RNG)
        ├─ FUN_00640927  — validate chosen colors (no RNG)
        └─ FUN_006443b6  — assignment orchestrator
              ├─ FUN_00643f62  — StartPos (Phase 1) — uses state 0xdb53a8
              └─ FUN_00643bc4  — Color+Faction (Phase 2) — uses state 0xdb53a8
```

Only 3 PRNG call sites in `FUN_00643bc4`:
- `0x00643ceb` — `rand(0, 1)` warmup inside faction-retry loop
- `0x00643e6c` — `rand(0, 1000)` faction pick
- `0x00643f00` — `rand(0, num_colors-1)` color pick

### Observation on Phase 1 RNG consumption

For some replays (3dwarf, arafkansergob, 3isenvsisenmormen), `before=0` matches ground truth — meaning **Phase 1 consumes zero RNG calls** for these games. This is unexpected given the decompile shows a rand path. Likely explanations:

1. **StartPos may be pre-resolved** (e.g. during replay load via `FUN_0077be11` from chunks, or network handshake in multiplayer) — by the time `FUN_00643f62` runs, slot[0x10] may already have valid values, and the distance-based path (no RNG) is taken instead of the rand path.
2. **The flag `*(EBP-0x11)`** gets set to 1 in the pre-loop if any slot has a valid startpos, sending ALL random-startpos slots to the distance path. If at least one slot had a pre-assigned position, no RNG is consumed.

Looking at replay header's `f5` field — it shows -1 for all playing slots. But this may just reflect lobby intent, not the actual value at assignment time. The resolved values likely come from chunks.

### Suggested next steps to hit 9/9

1. **Parse chunk data** for StartPos resolution events and check whether slot[0x10] is pre-set for all slots before color assignment (likely YES, which would zero-out Phase 1 RNG)
2. **Model Phase 2 faction retry counts** — in `FUN_00643bc4`, the `rand(0, 1000) % N` faction pick retries on invalid templates. Need to know the valid-template rate (probably ~3/6 factions are "main" playable)
3. **Run the game under a debugger** (e.g. x64dbg) with a breakpoint at `0x00633770` to log actual call sequence, bypassing all of this reverse engineering

**Artifacts created this session:**
- `bfme2_prng.py` — verified Python implementation
- `test_prng_all.py` — brute-force harness
- `test_prng_gt.py` — per-replay ground truth tester
- `inspect_slots.py` — slot-data dumper
- `diagnose.py` — per-replay PRNG stream diagnostic

---

## Addendum (2026-04-18): Live-match verification from `bfme2-factionforge`

The `bfme2-factionforge` sibling project verified the PRNG model end-to-end
against running BFME2 lobbies. Two live ground-truth traces were captured
(`SD=0x0F25` and `SD=0x19C0`), both producing 6/6 faction matches with the
`FACTION_POOL[rand(0,1000) % 6]` algorithm. Notes relevant to this document:

### Phase-1 RNG consumption is SITUATIONAL, not "one per observer"

The section above ("Suggested next steps to hit 9/9") already observed that
some replays (3dwarf, arafkansergob, 3isenvsisenmormen) need `Phase 1 = 0`
to match ground truth. Live-match Ghidra tracing confirms the mechanism:
`FUN_00643f62` has three loops, and the RNG consumption depends on:

1. Whether any slots have concrete `start_pos`. If yes, the first pass sets
   an "anyConcrete" flag and the second loop's random-start slots take a
   distance-based (no RNG) path.
2. Whether observer slots pass `FUN_007fde60`:
   ```c
   int t = *(int*)(slot + 0x04);
   return (t == 2 || t == 3 || t == 4 || t == 5 || t == 6) &&
          *(char*)(slot + 0x1A4) != 0;
   ```
   If either condition fails, the observer loop skips the `rand(0, num_starts-1)`
   call.

For the user's live 6-player wor-rhun lobbies with **manually-picked start
positions**, Phase 1 consumes ZERO RNG. For the 9 replay files in this
project, the rolls-per-observer heuristic happens to match because those
replays all have random start positions plus observers that pass the check.

### Observer-vs-empty slot disambiguation

Memory representation of empty slots (`template_id == -2`, `start_pos == -2`)
is IDENTICAL to true observer slots. The ASCII header's kind prefix
(`H`/`C`/`O`/`X`) is the authoritative source. A naive
`is_observer = template_id == -2 || start_pos == -2` mis-classifies empty
slots as observers, which inflates the observer count and (via the same
assumption above) mis-counts phase-1 rolls.

Both `bfme2-rfr` and `bfme2-factionforge` were updated with a `SlotKind`
enum + header-based classification. dcreplaybot parses replay headers so
it already has the kind info per-slot; worth verifying the observer-slot
detection doesn't include empty-slot entries when building the
`observer_slots` arg to `assign_player_colors_and_factions`.

### End-to-end verification table

| Seed (SD)  | Slot config                                   | Algorithm output | Actual | Match |
|-----------:|:----------------------------------------------|:-----------------|:-------|:-----:|
| `0x0F25`   | 6 playing (concrete colors 5,1,-,-,4,0), 2 obs| Isengard/Mordor/Goblins/Men/Men/Dwarves | same | 6/6  |
| `0x19C0`   | 6 playing (concrete 5,4,-,-,1,-), 2 obs       | Mordor/Isengard/Goblins/Elves/Men/Dwarves | same | 6/6 |

Both cases used Phase-1 = 0 (not "one per observer"). The
`FACTION_POOL[roll % 6]` pool ordering is verified correct: both faction
and color results match exactly when the algorithm uses this mapping.

See `bfme2-factionforge/SD_RESEARCH.md` for the full trace and the ghidra
dump of `FUN_00643f62` and `FUN_00643bc4`.
