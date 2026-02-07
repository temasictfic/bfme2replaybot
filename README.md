# DCReplayBot

A Discord bot that parses **Battle for Middle-earth II: Rise of the Witch King** replay files and renders a visual map summary.

Upload a `.BfME2Replay` file (or a `.zip`/`.rar` archive of replays), @mention the bot, and it responds with a rendered map image showing player positions, factions, colors, game duration, and winner.

Currently supports the **Bfme2 1.00 / Rhun** (3v3) map:

<p align="center">
  <img src="assets/maps/map wor rhun.png" alt="Map Wor Rhun" width="400">
</p>


## Features

- Parses BFME2 replay binary format (header + chunk stream)
- Detects player names, teams, colors, and factions (including random faction inference from building IDs)
- Determines player starting positions from build commands
- Detects game winner via EndGame (Order 29) and PlayerDefeated (Order 1096) events
- Supports Turkish character encoding (Windows-1254)
- Handles `.zip` and `.rar` archives (up to 10 replays per archive)
- Shows spectators/observers on the map
- Health check endpoint for container hosting

## Usage

1. Invite the bot to your Discord server
2. Upload a `.BfME2Replay` file (or archive) to any channel the bot can see
3. @mention the bot in the same message, or reply to a message containing a replay with an @mention
4. The bot responds with a rendered map image

## Setup

### Prerequisites

- Rust 1.88+
- A [Discord bot token](https://discord.com/developers/applications)

### Local Development

```bash
# Clone the repo
git clone https://github.com/temasictfic/dcreplaybot.git
cd dcreplaybot

# Create .env file
echo DISCORD_TOKEN=your_token_here > .env

# Build and run
cargo run
```

### Docker

```bash
docker build -t dcreplaybot .
docker run -e DISCORD_TOKEN=your_token_here dcreplaybot
```

### Deploy to Koyeb

The CI pipeline builds a Docker image and pushes it to GHCR on every push to `main`, then triggers a redeploy on Koyeb.

**Required GitHub secrets:**
| Secret | Description |
|--------|-------------|
| `KOYEB_API_TOKEN` | Koyeb API token for automated redeploy |

**Required Koyeb environment variables:**
| Variable | Description |
|----------|-------------|
| `DISCORD_TOKEN` | Discord bot token |

The bot exposes a health check on the `PORT` environment variable (default `8000`).

## Project Structure

```
src/
  main.rs              # Entry point, health check server
  bot/commands.rs      # Discord event handling, file extraction
  parser/replay.rs     # Binary replay parser (header + chunks)
  renderer/map.rs      # Map image renderer
  models/replay.rs     # Data models (Player, Faction, Winner, etc.)
assets/
  fonts/               # Bundled font (NotoSans-Bold)
  maps/                # Map background images
```

## CI

GitHub Actions runs on every push and PR:

- `cargo check` / `cargo test` / `cargo clippy -- -D warnings` / `cargo fmt --check`
- Docker build + push to GHCR (on `main` only, after all checks pass)
- Koyeb service redeploy (on `main` only, after Docker push)

## Technical Details

For details on the BFME2 replay binary format, see [BFME2_REPLAY_FORMAT.md](BFME2_REPLAY_FORMAT.md).

For winner detection logic and known edge cases, see [WINNER_DETECTION.md](WINNER_DETECTION.md).
