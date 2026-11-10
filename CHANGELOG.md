# Addon changelog

The release version comes from the `addon-v<version>` tag (see `.github/workflows/addon-release.yml`); nothing in the
source carries it. Pushing the tag publishes the release.

## 0.2.0 (unreleased)
- Reads the guild's rank names in game (`GuildControlGetNumRanks` / `GuildControlGetRankName`, 1-based, Guild Master
  first) on `PLAYER_GUILD_UPDATE` and `GUILD_ROSTER_UPDATE`, and keeps them in SavedVariables as
  `guild = { name, ranks, t }` (`t` = server time when read). Calls are guarded, so a client without them captures nothing.
- Signed `gbank` and `recipes` exports carry that block in their envelope, so the existing uploader sends rank names with
  no change. A separate `!BF:ranks:v1:` string is kept in the outbox (`ranks`) for an uploader that knows that type.
- `/bf ranks` shows the names read.

## 0.1.1
- Last published release.
