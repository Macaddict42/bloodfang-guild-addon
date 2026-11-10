# Addon changelog

The release version comes from the `addon-v<version>` tag (see `.github/workflows/addon-release.yml`); nothing in the
source carries it. Pushing the tag publishes the release.

## 0.2.1 (unreleased)
- Slash commands come from the build config: the short command (`ADDON_SLASH`) and the addon name in lower case are both
  registered, generated into `Config.lua` at package time. `/bf` is no longer registered. Help text and every printed
  hint use the short command.
- AddOns list icon: the build converts the guild logo (`ADDON_ICON`, a PNG path or https URL) to a 64x64 TGA and sets
  `## IconTexture` in the TOC. No logo file is in the source.
- `tests/package-check.sh` (run in CI before the build) checks a built zip registers both commands and not `/bf`, and
  ships the icon with the right TOC line.

## 0.2.0 (Nov 9, 2026)
- Reads the guild's rank names in game (`GuildControlGetNumRanks` / `GuildControlGetRankName`, 1-based, Guild Master
  first) on `PLAYER_GUILD_UPDATE` and `GUILD_ROSTER_UPDATE`, and keeps them in SavedVariables as
  `guild = { name, ranks, t }` (`t` = server time when read). Calls are guarded, so a client without them captures nothing.
- Signed `gbank` and `recipes` exports carry that block in their envelope, so the existing uploader sends rank names with
  no change. A separate `!BF:ranks:v1:` string is kept in the outbox (`ranks`) for an uploader that knows that type.
- `/bf ranks` shows the names read.

## 0.1.1
- Last published release.
