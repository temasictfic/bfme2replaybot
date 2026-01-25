# AI CLI Discussions - BFME2 Discord Replay Bot

## Project Context
Creating a Discord bot that processes Battle for Middle-earth 2 (.BfME2Replay) replay files and displays game information.

## Replay File Format Analysis (Pre-Discussion)
From our hex analysis of actual replay files:
- **Magic bytes**: `BFME2RPL` (8 bytes)
- **Header**: Contains timestamps, version info, date/time
- **Map info**: Encoded as `M=<id>maps/<mapname>`
- **Game settings**: Key-value pairs (MC, MS, SD, GSID, GT, SI, GR)
- **Player data**: `S=PlayerName,ID,Port,TT,Team,Position,Faction,...` format
  - Multiple players separated by `:`
  - Example: `S=HGusto,1A53EFD5,8094,TT,-1,-1,1,1,0,1,0:HThe_King_,...`

## Information Extractable from Replays
1. Map name
2. Player names
3. Teams/Positions
4. Factions (by ID)
5. Game timestamp
6. Game version

---

## Copilot (GPT-5.2) Discussion

### Query 1: Replay Parsing, Hosting & Visualization

**Question Asked:**
> I'm building a Discord bot to parse Battle for Middle-earth 2 replay files (.BfME2Replay format). From hex analysis: Magic bytes 'BFME2RPL', header has player data in format S=PlayerName,ID,Port,TT,Team,Position,Faction, map info as M=<id>maps/<mapname>. I also need to visualize data - like showing player positions on map images in Discord. Questions: 1) Best language/approach for binary parsing this format? 2) Top 3 free hosting options for Discord bots in 2025? 3) How to visualize player positions on map images for Discord?

**Copilot Response:**

1) **Best approach:** Use a strong binary parser with explicit structs and endian handling. **Rust with `binread`/`nom`** or **Go with `encoding/binary`** are ideal for correctness/speed. Parse header → player records → map string, and add unit tests against known replays. If your bot is Node/Python, consider a small Rust/Go parser CLI/library you call from the bot.

2) **Free hosting in 2025:**
   - **Koyeb Starter** ($0/mo, scale-to-zero) - koyeb.com/pricing
   - **Render Free Web Service** (free tier with idle spin-down) - render.com/docs/free
   - **Railway Free** (30-day $5 credit then $1/mo, so not fully free long-term) - railway.app/pricing

3) **Visualization:** Keep a map image per map, define game-coord bounds → pixel transform, draw player dots/labels with a 2D image lib (node-canvas/Sharp, Python Pillow), output PNG, and send it as a Discord attachment/embed; cache map images and precomputed transforms for speed.

---

## Gemini (Gemini 2.5) Discussion

### Query 1: Architecture, Tech Stack & Free Hosting

**Question Asked:**
> I'm creating a Discord bot to parse Battle for Middle-earth 2 game replay files (.BfME2Replay format). The format has: magic header 'BFME2RPL', player data encoded as S=PlayerName,ID,Port,TT,Team,Position,Faction separated by colons, map info as M=<id>maps/<mapname>. Requirements: 1) Parse binary files to extract player names, teams, map name, factions 2) Host bot for free (high priority) 3) Visualize replay data - show player positions on map images in Discord. What's your recommended tech stack and architecture? What are the most reliable free hosting platforms for Discord bots in 2025?

**Gemini Response:**

### Recommended Tech Stack & Architecture

For parsing binary files and image manipulation, **Python** is the clear winner over Node.js due to its superior libraries for binary data analysis (`struct`) and image processing (`Pillow`).

- **Language:** Python 3.11+
- **Discord Framework:** `discord.py` (Stable, robust, easy to maintain)
- **Image Processing:** `Pillow` (Python Imaging Library) to draw player positions on map files
- **Parsing Logic:** Custom binary scanner. Since there is no public library for `.BfME2Replay`, we will write a parser that scans the binary stream for the specific ASCII signatures (`S=...` and `M=...`)

**Architecture:**
1. **Input:** User uploads a `.BfME2Replay` file to a Discord channel
2. **Processing:** Bot downloads the file into RAM (using `io.BytesIO`) to avoid disk I/O latency
3. **Parsing:** The parser verifies the `BFME2RPL` header and regex-scans the binary content for player/map strings
4. **Visualization:** The bot looks for a matching map image in its `assets/maps/` folder. If found, it plots the starting coordinates. If not, it uses a generic background
5. **Output:** A rich Discord Embed containing the match stats and the generated image

### Reliable Free Hosting (2025)

1. **Oracle Cloud Infrastructure (OCI) "Always Free" (Best Performance):**
   - **Pros:** Offers an ARM Ampere instance with up to **4 OCPUs and 24 GB of RAM** completely free. Vastly more powerful than other free offers.
   - **Cons:** Verification can be picky (credit card required for identity, though not charged).

2. **Google Cloud Platform (GCP) Free Tier:**
   - **Pros:** `e2-micro` instance is free in specific regions (us-west1, us-central1).
   - **Cons:** Very limited CPU/RAM (shared core, 1GB RAM). Requires credit card setup.

3. **Fly.io (Good for low usage):**
   - **Pros:** Very easy deployment (Docker-based).
   - **Cons:** Free allowance is small; if your bot creates many images, you might hit limits.

4. **Bot-Hosting.net / fps.ms (Niche):**
   - **Pros:** Specifically for Discord bots, very easy setup.
   - **Cons:** Freemium hosts often have downtime or require clicking "renew" button weekly.

---

## Summary: Key Insights from AI Discussions

### Tech Stack Consensus
| Aspect | Copilot Recommendation | Gemini Recommendation |
|--------|------------------------|----------------------|
| Language | Rust/Go (performance) or Node/Python with external parser | Python (simplicity) |
| Binary Parsing | binread/nom (Rust) or encoding/binary (Go) | struct + regex (Python) |
| Image Processing | node-canvas/Sharp or Pillow | Pillow |
| Discord Library | (not specified) | discord.py |

### Free Hosting Consensus
| Host | Mentioned By | Notes |
|------|-------------|-------|
| Oracle Cloud (OCI) | Gemini | Best specs (4 OCPU, 24GB RAM) - Always Free |
| Koyeb | Copilot | Scale-to-zero, truly free |
| Render | Copilot | Free tier with spin-down |
| Fly.io | Gemini | Easy Docker deployment |
| GCP Free Tier | Gemini | e2-micro instance |

---

## User Decisions

After presenting options, the user chose:
- **Language**: Rust with winnow (not nom) for both bot and parser
- **Hosting**: Koyeb Starter (scale-to-zero, truly free)
- **Map Images**: User has map images available from game files

---

## Final Tech Stack (Agreed)

| Component | Library | Purpose |
|-----------|---------|---------|
| Parser | winnow | Binary parsing of .BfME2Replay files |
| Discord | poise + serenity | Discord bot framework |
| Image Processing | image + imageproc + ab_glyph | Drawing player positions on maps |
| Async Runtime | tokio | Async runtime for bot |
| Hosting | Koyeb Starter | Free Docker-based deployment |

