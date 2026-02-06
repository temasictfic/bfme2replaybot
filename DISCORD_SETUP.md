# Discord Bot Setup Guide

## Step 1: Create a Discord Application
1. Go to https://discord.com/developers/applications
2. Click **"New Application"** (top right)
3. Name it "DCReplayBot" (or anything you like)
4. Click **"Create"**

## Step 2: Create the Bot
1. In the left sidebar, click **"Bot"**
2. Click **"Reset Token"** and confirm
3. **Copy the token** - you'll only see it once!

## Step 3: Enable Required Intents
On the same Bot page, scroll down to **"Privileged Gateway Intents"** and enable:
- Message Content Intent (required to read file attachments)

## Step 4: Invite the Bot to Your Server
1. In the left sidebar, click **"OAuth2"** -> **"URL Generator"**
2. Under **Scopes**, check: `bot`
3. Under **Bot Permissions**, check:
   - Read Messages/View Channels
   - Send Messages
   - Attach Files
   - Embed Links
4. Copy the generated URL at the bottom and open it in your browser
5. Select your server and authorize

## Step 5: Configure the Bot
Create a `.env` file in the project root with your token:

```
DISCORD_TOKEN=your_token_here
```

## Step 6: Run the Bot
```powershell
cargo run --release
```

## Usage
The bot only responds when **@mentioned**. Upload a file and tag the bot in the same message:

- `.BfME2Replay` — single replay file (max 5MB)
- `.zip` or `.rar` — archive containing multiple replays (max 25MB)

Example: upload your replay file and type `@DCReplayBot` in the message text.
