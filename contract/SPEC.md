# Wire-format contract: addon, uploader, site

Status: describes the code as it is today. It is not a redesign.
This file replaces `export-format.md`, which many files cite but which never existed (see Gap 1).

Path notation:
- Plain paths (`addon/Core.lua:36`, `uploader/core/src/lib.rs:34`) are in this repo.
- `site:` paths (`site:src/lib/ingest.ts:29`) are in the private site repo.
- Placeholders: "Example Guild", "Testchar", `https://guild.example`, `Player-1234-0ABCDEF1`.

---

## 1. Overview

Three parts:

| Part | Code | Role |
|---|---|---|
| Addon (Lua, in game) | `addon/*.lua` | Captures data, signs `!BF:` export strings, keeps them in SavedVariables (`<Addon>DB.outbox`). Imports site strings by paste. |
| Uploader (Rust core + Tauri app) | `uploader/core/src/`, `uploader/app/` | Reads SavedVariables files, posts export strings and rank names to the site. Signs in with OAuth. Checks for addon/app updates. |
| Site (Next.js) | `site:src/lib/`, `site:src/app/api/` | Verifies strings, stores data. Issues per-character secrets and signed import strings. Publishes latest-release info. |

```
            in game                          desktop                            site
 +---------------------------+     +-------------------------+     +-----------------------------+
 | addon                     |     | uploader                |     | https://guild.example       |
 |  capture -> sign (HMAC)   |     |  read SavedVariables    |     |                             |
 |  outbox in SavedVariables |---->|  extract !BF: strings   |---->| POST /api/uploader/ingest   |
 |                           |     |  read guild rank block  |---->| POST /api/uploader/ranks    |
 |                           |     |  error reports          |---->| POST /api/uploader/logs     |
 |                           |     |  OAuth loopback sign-in |<--->| /api/uploader/oauth/*       |
 |                           |     |  addon update check     |<----| GET  /api/addon/latest      |
 |  "/cmd export" copy box   |---- paste by hand ------------------>| Bank / Link / Calendar forms |
 |  "/cmd import" paste box  |<--- paste by hand ------------------| link-ok, signups (Ed25519)  |
 +---------------------------+     +-------------------------+     | GET /api/uploader/latest    |
                                                                   |   (not called by uploader)  |
                                                                   +-----------------------------+
```

Paths into the site:
- Uploader: `gbank`, `recipes`, `ranks` strings via `/api/uploader/ingest` (`uploader/core/src/lib.rs:29`, `site:src/app/api/uploader/ingest/route.ts:13`).
- Paste forms: `link` (`site:src/app/settings/link/actions.ts:51`), `gbank` (`site:src/app/bank/actions.ts:77`), `signup`, `attendance`, `kills` (`site:src/app/calendar/actions.ts:36,164-165`).
- Site to addon: `link-ok` (`site:src/app/settings/link/actions.ts:90`) and `signups` (`site:src/app/calendar/[id]/page.tsx:59`), pasted with `/cmd import` (`addon/Core.lua:94-107`).

In-game addon messages are not sent to the site. Prefix: addon folder name, upper case, 16 chars max (`addon/Core.lua:5`). Messages: `VER:<x.y.z>` (`addon/Comms.lua:24`), `VREQ` (`addon/Comms.lua:54,68`), `LOC:<zone>:<subzone>:<x>:<y>` with x and y in 0..1000 (`addon/Comms.lua:46`).

---

## 2. Export string format

### 2.1 Shape

`!BF:<type>:v<n>:<body>`

| Part | Rule | Evidence |
|---|---|---|
| `<type>` | `[a-z-]+` | `addon/Codec.lua:179`, `site:src/lib/addon-codec.ts:51` |
| `<n>` | decimal integer. Every producer writes `1` today. | `addon/Core.lua:36,89`, `site:src/lib/addon-codec.ts:76` |
| `<body>` | LibDeflate `EncodeForPrint` of raw deflate (no zlib header) of the JSON envelope | `addon/Codec.lua:176`, `site:src/lib/addon-codec.ts:59,76` |
| Whitespace | Leading/trailing whitespace is trimmed before parsing | `addon/Codec.lua:179`, `site:src/lib/addon-codec.ts:49`, `uploader/core/src/envelope.rs:16` |

### 2.2 Body encoding

- Alphabet: `a-z A-Z 0-9 ( )`, 64 chars, index = 6-bit value (`site:src/lib/addon-codec.ts:6`, `uploader/core/src/envelope.rs:8`).
- Every 3 bytes become 4 chars, little-endian: `n = b0 + b1*256 + b2*65536`, chars `n&63, n>>6&63, n>>12&63, n>>18&63` (`site:src/lib/addon-codec.ts:34-37`).
- Tail: 1 byte gives 2 chars, 2 bytes give 3 chars (`site:src/lib/addon-codec.ts:38-43`). A 1-char body is invalid (`site:src/lib/addon-codec.ts:14`, `uploader/core/src/envelope.rs:53`).
- Compression: the addon uses `CompressDeflate(..., {level = 9})` (`addon/Codec.lua:176`); the site uses `deflateRawSync(level 9)` (`site:src/lib/addon-codec.ts:76`). Readers only need raw inflate.
- The uploader's extractor only matches `[A-Za-z0-9()]+` after the prefix (`uploader/core/src/lib.rs:34`).

### 2.3 Size limits

| Limit | Value | Where enforced |
|---|---|---|
| MAX_STRING (trimmed string) | 256 KiB | `site:src/lib/addon-codec.ts:8,50` (`too_large`); `uploader/core/src/envelope.rs:9,17` (count only) |
| MAX_DECODED (inflated JSON) | 512 KiB | `site:src/lib/addon-codec.ts:9,59` (`too_large`); `uploader/core/src/envelope.rs:10,90-95` |
| Ingest request body | 2 MiB (by `Content-Length` header) | `site:src/app/api/uploader/ingest/route.ts:18-19` |
| Strings per ingest request | 1..20 | `site:src/app/api/uploader/ingest/route.ts:22`, `uploader/core/src/lib.rs:30` |
| Addon decoder | none | `addon/Codec.lua:178-187` |

### 2.4 Envelope (addon to site)

The JSON is an object (`site:src/lib/addon-codec.ts:61`). Built at `addon/Core.lua:33-35`:

| Field | Type | Meaning | Site check |
|---|---|---|---|
| `id` | string, 32 lower hex | random envelope id (`addon/Codec.lua:167-171`) | `/^[0-9a-f-]{8,64}$/i`, dedupe (`site:src/lib/ingest.ts:32,43`) |
| `t` | integer, Unix seconds | `GetServerTime()` | number; 30 days back to 24 h ahead (`site:src/lib/ingest.ts:41`) |
| `addon` | string | addon version from the `.toc` (`addon/Core.lua:3`) | stored only if `^\d{1,4}\.\d{1,4}\.\d{1,4}$` (`site:src/lib/ingest.ts:47-49`) |
| `char` | object `{guid, name, realm}` | signing character (`addon/Core.lua:20-23`) | `char.guid` string, must be a linked character (`site:src/lib/ingest.ts:32,35`) |
| `p` | object | type payload (section 3) | object (`site:src/lib/ingest.ts:32`) |
| `guild` | object `{name, ranks[], t}` or absent | rank names; on every type except `ranks` (`addon/Core.lua:35`, `addon/Guild.lua:52-57`) | parsed best-effort, never fails the export (`site:src/lib/uploader.ts:72-74`) |
| `sig` | string, 64 lower hex | HMAC (2.5) | `/^[0-9a-f]{64}$/i` (`site:src/lib/ingest.ts:38`) |

### 2.5 Signing

Two schemes. Neither covers the `!BF:` prefix; both cover only JSON.

| Direction | Algorithm | Key | Signed bytes | Field | Verified by |
|---|---|---|---|---|---|
| Addon to site (all types except `link`) | HMAC-SHA256 | per-character 32-byte secret. Issued at link time, sent to the addon as 64 hex in `link-ok`, stored as `chars[guid].secret` (`addon/Core.lua:102`). Site stores it AES-256-GCM sealed (`site:src/lib/addon-codec.ts:105-122`). | UTF-8 canonical JSON of the envelope without `sig` (`addon/Codec.lua:174`) | `sig` at top level of the envelope | site: `site:src/lib/ingest.ts:36-39` (constant-time compare) |
| `link` | none | - | - | no `sig` | site trusts the one-time link code (`site:src/app/settings/link/actions.ts:65-68`) |
| Site to addon (`link-ok`, `signups`) | Ed25519 | site key `ADDON_SIGNING_KEY`, base64 PKCS#8 DER (`site:src/lib/addon-codec.ts:83-84`) | UTF-8 canonical JSON of `{id, t, char:{guid}, p}` (`site:src/lib/addon-codec.ts:99-100`) | `sig`, 128 hex, top level | nobody yet: the addon does not check it (`addon/Core.lua:92-93`) |

The HMAC key is the raw 32 bytes (`ns.fromHex(secretHex)`, `addon/Codec.lua:174`; `openSecret()`, `site:src/lib/ingest.ts:37`). HMAC uses a 64-byte block (`addon/Codec.lua:157-163`).

### 2.6 Canonical JSON

The site does not verify the received bytes. It parses the JSON, removes `sig`, and re-serializes with `canonical()` (`site:src/lib/ingest.ts:36-37`). So the addon's encoder must produce exactly what `canonical()` produces.

| Rule | Lua (`addon/Codec.lua`) | TS (`site:src/lib/addon-codec.ts:69-73`) | Agree? |
|---|---|---|---|
| Object key order | `table.sort` byte order (`:36`) | `Object.keys().sort()` UTF-16 order | Yes for ASCII keys (all keys today) |
| Whitespace | none | none | Yes |
| Numbers | integers only, `|v| < 2^53`, `%.0f` (`:25-26`) | `JSON.stringify` | Yes, except `-0` (Lua `-0`, JS `0`) |
| Strings | raw UTF-8; escapes `" \ \b \f \n \r \t`, other `%c` as `\u00xx` (`:13-14`) | `JSON.stringify` | Yes, except DEL (0x7F): Lua escapes it, JS does not (Gap 12) |
| Invalid UTF-8 | passed through | decoded as U+FFFD | No: gives `bad_sig` |
| Arrays | tables marked by `ns.array()`, or `n == #t` with `n > 0` (`:15-20`) | `Array.isArray` | Yes. An empty plain Lua table encodes as `{}`. |
| `null` / `undefined` | never emitted; decoder maps `null` to `nil` (`:97`) | `undefined` keys dropped; `null` kept | Site imports can carry `null` (e.g. `role`, `party`) |
| `\u` escapes in decoder | BMP only, no surrogate pairs (`:63-67`) | full | Only matters for imports |

---

## 3. Type registry

### 3.1 Summary

| Type | Ver | Direction | Producer | Consumer(s) | Site accepts | Notes |
|---|---|---|---|---|---|---|
| `link` | 1 | addon to site (paste) | `addon/Core.lua:83-90` | `site:src/app/settings/link/actions.ts:51` | `{link:[1]}` | unsigned |
| `link-ok` | 1 | site to addon (paste) | `site:src/app/settings/link/actions.ts:90` | `addon/Core.lua:99-103` | - | Ed25519, unchecked |
| `gbank` | 1 | addon to site (uploader or paste) | `addon/Bank.lua:54` | `site:src/lib/uploader.ts:37-51` | `{gbank:[1]}` (`site:src/lib/ingest.ts:29`) | officer roles |
| `recipes` | 1 | addon to site (uploader only) | `addon/Recipes.lua:21` | `site:src/lib/uploader.ts:54-69` | `{recipes:[1]}` | no paste form exists |
| `ranks` | 1 | addon to site (uploader only) | `addon/Guild.lua:72` | `site:src/lib/uploader.ts:77-86` | `{ranks:[1]}` | refused unless source is UPLOADER |
| `signup` | 1 | addon to site (paste) | **none in addon** | `site:src/app/calendar/actions.ts:31-47` | `{signup:[1]}` | accepted, never produced |
| `attendance` | 1 | addon to site (paste) | **none in addon** | `site:src/app/calendar/actions.ts:158-182`, `site:src/lib/raids.ts:67` | `{attendance:[1]}` | accepted, never produced |
| `kills` | 1 | addon to site (paste) | **none in addon** | `site:src/app/calendar/actions.ts:158-182`, `site:src/lib/raids.ts:105` | `{kills:[1]}` | accepted, never produced |
| `signups` | 1 | site to addon (paste) | `site:src/app/calendar/[id]/page.tsx:59-62` | **addon rejects** (`addon/Core.lua:105`) | - | produced, never consumed |

There is no per-type accepted-versions map. Each caller passes `{[type]: [1]}`: `site:src/lib/ingest.ts:29` (all signed types) and `site:src/app/settings/link/actions.ts:51` (`link`). The uploader's extractor also hard-codes `v1` (`uploader/core/src/lib.rs:34`).

### 3.2 Payloads (`p`)

`link` (`addon/Core.lua:88`). Envelope has `id, t, addon, char{guid,name,realm}, p`, no `sig`, no `guild`.

| Field | Type | Site rule |
|---|---|---|
| `code` | string, 8 chars `[A-Z0-9]` (`addon/Core.lua:85`) | upper-cased, must be the user's live code (`site:src/app/settings/link/actions.ts:58,65-68`) |
| `class` | string, class file name e.g. `MAGE` | must be a playable class, else `unsupported_version` message (`:61`) |
| `race` | string | first 20 chars (`:63`) |
| `level` | integer | 1..60, else null (`:62`) |
| `char.guid` | string | `^Player-\d{1,5}-[0-9A-F]{8}$` (`:42,60`) |
| `char.name` | string | `^\p{L}{2,12}$` (`:43,60`) |
| `char.surname` or `char.realm` | string | surname falls back to `realm`; addon sends `realm` (`:56`) |

`link-ok` (`site:src/app/settings/link/actions.ts:90`). Envelope `{id, t, char:{guid}, p, sig}`.

| Field | Type | Addon rule |
|---|---|---|
| `guid` | string | not read; `char.guid` must equal the player GUID (`addon/Core.lua:98`) |
| `secret` | string, 64 hex | must be 64 hex, stored lower case (`addon/Core.lua:101-102`) |

`gbank` (`addon/Bank.lua:10-31`).

| Field | Type | Site rule (`site:src/lib/bank.ts:65-83`) |
|---|---|---|
| `tabs` | array, at most 8 | `GBANK_TABS = 8` (`site:src/lib/bank.ts:37`) |
| `tabs[].tab` | integer 1..8 | required |
| `tabs[].name` | string | control chars and `|` removed, 32 chars, default `Tab <n>` |
| `tabs[].items` | array, at most 196 | required |
| `items[].itemId` | integer 1..1e6 | required |
| `items[].count` | integer 1..1e6 | summed per item (addon also sums, `addon/Bank.lua:18`) |
| `items[].icon` | integer file id, or string | optional; `iconKey()` (`site:src/lib/bank.ts:42-48`) |
| `copper` | integer >= 0 | optional (`addon/Bank.lua:29`) |

`recipes` (`addon/Recipes.lua:21`).

| Field | Type | Site rule (`site:src/lib/crafting.ts:15-20`) |
|---|---|---|
| `profession` | string, English name | one of 8 names (`site:src/lib/crafting.ts:12`; addon list `addon/Recipes.lua:4`) |
| `skill` | integer | ignored |
| `recipes` | sorted unique spell ids | at most 1000; non-integers dropped silently |

`ranks` (`addon/Guild.lua:72`, payload is the guild block from `addon/Guild.lua:56`). Envelope has no `guild` field.

| Field | Type | Site rule (`site:src/lib/rank-names.ts:40-48`) |
|---|---|---|
| `name` | string guild name | 1..64 chars, must match configured guild (`site:src/lib/rank-store.ts:36`) |
| `ranks` | array 1..10 strings, GM first | each cleaned, 1..40 chars (`site:src/lib/rank-names.ts:13-35`) |
| `t` | integer Unix seconds | > 0; newer than stored to replace (`site:src/lib/rank-names.ts:109-111`) |

`signup` (site reader only, `site:src/app/calendar/actions.ts:38-43`): `eventId` (must equal page event), `status` (upper-cased, in `STATUSES`), `role` (optional), `note` (140 chars).

`attendance` (site reader only, `site:src/lib/raids.ts:67-80`): `instanceId`, `firstPull` (Unix s, within event +/- 3 h), `members[]` `{guid, online}`, `snapshots[]` `{t, guids[]}`. Signer must be in the list. Ownership not required (`othersOk`, `site:src/app/calendar/actions.ts:165`).

`kills` (site reader only, `site:src/lib/raids.ts:105-121`): `instanceId`, `kills[]` `{encounterId, start, end, success}`, first 50 used.

`signups` (`site:src/app/calendar/[id]/page.tsx:59-62`): `eventId, instanceId, startsAt, raidGroup, roster[] {name, class, role, status, confirmed, party:null, slot:null}`.

---

## 4. Uploader to site HTTP API

All site JSON calls use one agent: 5 s connect, 30 s overall, `User-Agent: guild-uploader/<ver>` (`uploader/core/src/http.rs:201,254-256`). Token format: `upl_` + 43 base64url chars, header `Authorization: Bearer <token>` (`site:src/lib/uploader.ts:19,23`). A token is valid only while its user is a member (`site:src/lib/uploader.ts:26-28`). Rate limits are per clock hour (`site:src/lib/rateLimit.ts:24`).

### 4.1 OAuth sign-in (authorization code + PKCE S256, loopback redirect)

Constants: `client_id=guild-uploader`, `scope=ingest`, transaction 5 min, code 60 s, max 5 tokens per user (`site:src/lib/uploader-oauth.ts:13-18`).

1. Preflight: `GET /api/uploader/oauth/authorize` with no query and `Accept: application/json`. Expect `400 {"error":"invalid_request"}` (`site:src/app/api/uploader/oauth/authorize/route.ts:16-18`). Any other answer is `NotDeployed`; transport failure is `Network` (`uploader/core/src/browser_auth.rs:220-245`).
2. Uploader binds `127.0.0.1:0`, makes a 32-byte verifier and 32-byte state, base64url (`uploader/core/src/browser_auth.rs:479-482`).
3. Browser opens `authorize?response_type=code&client_id&redirect_uri=http://127.0.0.1:<port>/callback&state&code_challenge&code_challenge_method=S256&scope=ingest[&prompt=login]` (`uploader/core/src/browser_auth.rs:484-491`).
4. Site validates (`site:src/lib/uploader-oauth.ts:143-160`): no duplicate keys; only `prompt=login` extra; challenge 43 base64url; state `[A-Za-z0-9_-]{22,128}`; redirect exactly `http://127.0.0.1:<1024-65535>/callback` (`site:src/lib/oauth-form-action.ts:10-21`).
5. No session: `302` to `/signin?callbackUrl=<authorize path>` (`authorize/route.ts:23-27`). With `prompt=login`: a switch page that posts to `/switch` (`authorize/route.ts:29-36`).
6. Consent page posts `txn`, `csrfToken`, `decision=approve|deny` to `/consent` (`site:src/lib/uploader-oauth.ts:242`). A `upl_txn` cookie binds the browser.
7. Approve: `302 {redirect_uri}?code=&state=`. Deny: `302 {redirect_uri}?error=access_denied&state=` (`site:src/lib/uploader-oauth.ts:369,383`).
8. Uploader checks `state` in constant time; up to 8 bad callbacks are ignored, then `BadResponse` (`uploader/core/src/browser_auth.rs:377-383`). `access_denied` is `Denied`; a token in the callback fails closed (`:374-376,385-387`).
9. `POST /api/uploader/oauth/token` JSON `{grant_type:"authorization_code", client_id, code, redirect_uri, code_verifier, state}` (`uploader/core/src/browser_auth.rs:404-418`). Site checks verifier shape `[A-Za-z0-9._~-]{43,128}`, PKCE, state, redirect, one use (`site:src/lib/uploader-oauth.ts:412-428`). At 5 tokens it revokes the oldest desktop token (`:431-437`).
10. Whole flow times out after 5 min (`uploader/app/src-tauri/src/lib.rs:233`).

### 4.2 Endpoint table

| Endpoint | Method | Auth | Request | Responses (status, body) | Uploader reaction |
|---|---|---|---|---|---|
| `/api/uploader/oauth/authorize` | GET | site session cookie | query (4.1) | 400 `{error:"invalid_request"}` (JSON, empty or bad query); 429 `{error:"rate_limited"}` (30/h per IP); 302 to sign-in; 200 consent/switch HTML; 400 HTML `invalid`; 403 HTML `forbidden` (`authorize/route.ts:16-45`) | preflight: only 400 `invalid_request` is OK; 429 counts as `NotDeployed` (`browser_auth.rs:236-239`) |
| `/api/uploader/oauth/consent` | POST form | session + `upl_txn` cookie + csrf; same origin | `txn, csrfToken, decision` | 302 to loopback; 400 HTML `invalid`; 400 HTML `expired`; 403 HTML `forbidden`; 429 HTML `invalid` (20/h per user) (`consent/route.ts:12-34`) | browser only; uploader waits for the callback |
| `/api/uploader/oauth/switch` | POST form | session + cookie + csrf; same origin | `txn, csrfToken, decision=switch` | sign-out then redirect to sign-in; 400 HTML `invalid`/`expired`; 429 (`switch/route.ts:15-33`) | browser only |
| `/api/uploader/oauth/token` | POST JSON | none (no cookies) | 4.1 step 9 | 200 `{token, token_type:"Bearer"}`; 400 `{error:"invalid_grant"}`; 403 `{error:"forbidden"}`; 429 `{error:"rate_limited"}` (60/h per IP) (`token/route.ts:16-24`) | 400 `InvalidGrant`, 403 `Forbidden`, 404 `NotDeployed`, other (incl. 429) `BadResponse` (`browser_auth.rs:436-443`); token must start `upl_`, <= 512 chars (`:452-457`) |
| `/api/uploader/ingest` | POST JSON | Bearer token | `{strings: string[1..20]}` | 200 `{results:[{type, ok, error?}]}` in input order; 401 `{error:"bad_token"}`; 429 `{error:"rate_limited"}` (120/h per token); 413 `{error:"too_large"}`; 400 `{error:"bad_request"}` (`ingest/route.ts:15-29`) | 200 parsed; 401 `bad_token` drops the saved token; other 401 shown; 429 `RateLimited`; other codes shown redacted (`uploader/core/src/lib.rs:112-129`). Network and 429 retried up to 3 times, 200-1000 ms pause (`uploader/app/src-tauri/src/upload_gate.rs:167-173`) |
| `/api/uploader/ranks` | POST JSON | Bearer token | `{guild, t, ranks[]}` | 200 `{stored:true}` (stored or unchanged); 200 `{stored:false}` (older); 400 `{error}` = `bad_request`, `other_guild`, `bad_time`, `bad_ranks`, `invalid`; 401 `bad_token`; 403 `not_on_roster`/`not_officer`; 413 `too_large` (16 KiB); 429 `rate_limited` (60/h) (`site:src/app/api/uploader/ranks/route.ts:12-17`, `site:src/lib/rank-sync.ts:19-31`, `site:src/lib/rank-names.ts:117-126`) | 200 final; 401 `bad_token` stops the pass; 403/400 final refusal, not resent for that block; 404/429/other retry next pass (`uploader/core/src/ranks.rs:397-422`, `lib.rs:203-213`) |
| `/api/uploader/logs` | POST JSON | Bearer optional | `{errorCode, message, appVersion, os, addonVersion?, log}` (log <= 64 KiB, body <= 80 KiB) | 201 `{ok, id}`; 400 `bad_request`, `bad_error_code`, `bad_app_version`, `bad_os`, `bad_addon_version`, `bad_log`; 413 `too_large`/`log_too_large`; 429 `rate_limited` (20/h token; 3/h per IP and 60/h total anonymous) (`site:src/lib/uploader-logs.ts:14-35`, `site:src/lib/uploader-log-rules.ts:5-50`) | any status is final, never retried; one report per code per hour (`uploader/core/src/report.rs:77-107`). Codes: `addon_check`, `addon_install`, `upload`, `app_update_check`, `app_update_install` (`report.rs:15-21`) |
| `/api/addon/latest` | GET | none | - | 200 `{version, tag, publishedAt, assets:[{name, sizeBytes, sha256, url}]}`, cached 300 s; 404 `{error:"No release published yet"}` (`site:src/app/api/addon/latest/route.ts:8-9`) | needs a `<Addon>-*.zip` asset with 64-hex sha256 and https URL, else falls back to GitHub `addon-v*` + `SHA256SUMS` (`uploader/core/src/update.rs:65-127`) |
| `/api/uploader/latest` | GET | none | - | same shape for `uploader-app-v*`; 404 until published (`site:src/app/api/uploader/latest/route.ts:8-9`) | not called. App updates read GitHub `uploader-app-v*` directly (`uploader/core/src/update.rs:487-494`, `uploader/app/src-tauri/src/app_update.rs:159`) |

### 4.3 Per-string ingest errors

`results[].error` values, from `site:src/lib/ingest.ts:8-21` and `site:src/lib/uploader.ts:34`:

| Code | Cause | Uploader treats as done? | UI sentence (`uploader/app/ui/upload-lines.js:63-75`) |
|---|---|---|---|
| (ok) | accepted | yes | "accepted" |
| `duplicate` | envelope id seen | yes (`uploader/core/src/lib.rs:102`) | counted as accepted (`upload-lines.js:34`) |
| `forbidden` | role check failed (`uploader.ts:38,55,78,83`) | no, resent next pass | "The site didn't accept the guild bank scan." / "...this." |
| `bad_envelope` | missing fields, or payload parse failed | no | "The site couldn't use this scan." |
| `unknown_type` | type not in ingest map (`ingest/route.ts:25-27`) | no | "The site doesn't take this kind of scan yet." |
| `bad_prefix`, `unsupported_version`, `decode_failed`, `too_large`, `not_linked`, `bad_sig`, `stale`, `not_allowed` | `site:src/lib/ingest.ts:28-43` | no | generic "The site didn't accept this." |
| `site_refused` | never sent by the site | - | mapped anyway (`upload-lines.js:69`) |

Every refusal adds "This sign-in is still saved." (`upload-lines.js:77-81`). Only `rejected the uploader token` text drops the token (`upload-lines.js:93-95`, `uploader/core/src/lib.rs:89`). Sign-in error sentences come from Rust `AuthError` (`uploader/core/src/browser_auth.rs:79-88`).

The uploader remembers handled strings by SHA-256 of the full string (`uploader/core/src/lib.rs:51,223,228`). The site dedupes by envelope `id` (`site:src/lib/ingest.ts:43`).

---

## 5. Versioning and compatibility

### 5.1 As implemented today

| Situation | Addon | Uploader | Site |
|---|---|---|---|
| Unknown type | import: "aren't supported" (`addon/Core.lua:105`) | not extracted (`lib.rs:34`); not counted (`envelope.rs:38-42`) | `unknown_type` (`addon-codec.ts:54`; `ingest/route.ts:27`) |
| Version other than 1 | import: `v > 1` refused (`addon/Core.lua:97`) | `v2+` silently skipped by extractor regex (`lib.rs:34`) | `unsupported_version` (`addon-codec.ts:55`) |
| Old addon version | - | - | Not refused. `addon` string stored per ingest (`ingest.ts:48-50`); `AddonOutdated` banner if older than latest release (`ingest.ts:55-59`, `site:src/components/AddonOutdated.tsx:6-10`) |
| Addon update | in-game `VER` warning after 2 newer senders (`addon/Comms.lua:27-36`) | site then GitHub, sha256 checked (`update.rs:80-127,234-244`) | `/api/addon/latest` |
| App update | - | semver compare, deferred version skipped (`app_update.rs:44-55`) | `/api/uploader/latest` exists, unused |
| Stored version | - | - | always `version: 1` (`ingest.ts:50`, `site:src/app/calendar/actions.ts:178`) |

### 5.2 Rules going forward

1. Changing an existing type's payload, envelope, signing or encoding requires `v<n+1>`. Adding an optional field that old readers ignore does not.
2. The site must accept the current and the previous version of every type it reads. Replace the hard-coded `[1]` with one accepted-versions map.
3. The uploader must extract every version the site accepts (today it matches only `v1`).
4. Golden fixtures live under `contract/golden/` in this repo. They are add-only. Never edit an existing `v<n>` file; a change is a new `v<n+1>` file.
5. This repo is the single home of this contract. The site vendors the fixtures under `scripts/tests/fixtures/contract/` with a `SOURCE` file recording the addon commit SHA (`site:docs/workflow/01-decisions.md:10-13`).
6. A new type is added to this file, the registry, the site's accepted map, and (if uploaded) `uploader/core/src/lib.rs:29,34` in the same change.

---

## 6. Gaps and inconsistencies

1. **`export-format.md` does not exist.** Cited at `addon/Codec.lua:1`, `addon/Core.lua:5,26,82`, `addon/Bank.lua:1`, `addon/Recipes.lua:1`, `addon/Comms.lua:1`, `uploader/core/src/envelope.rs:2`, `site:src/lib/addon-codec.ts:5,68`, `site:src/lib/ingest.ts:24`, `site:src/lib/crafting.ts:14`, `site:src/lib/raids.ts:62`, `site:prisma/schema.prisma:322`, `site:docs/workflow/01-decisions.md:12`. Point them at `contract/SPEC.md`.
2. **No cross-language signature test.** `tests/run.py:1` says it writes for `verify.mjs`, which exists in neither repo. It writes `/tmp/addon-out.json` (`tests/run.py:97`) that nothing reads. Nothing proves the site verifies a Lua HMAC. The site test only round-trips TS (`site:scripts/tests/addon-codec.test.ts:9-17`).
3. **Import signatures are not verified.** The addon skips the Ed25519 check (`addon/Core.lua:92-93`) and ships no public key. Any pasted `link-ok` with the right GUID replaces the secret (`addon/Core.lua:98-102`).
4. **`signups` produced, never consumed.** Site builds it (`site:src/app/calendar/[id]/page.tsx:59`); addon refuses it (`addon/Core.lua:105`).
5. **`signup`, `attendance`, `kills` accepted, never produced.** No producer in `addon/`. Their payloads are defined only by site readers (`site:src/app/calendar/actions.ts:38`, `site:src/lib/raids.ts:67,105`). UI labels also list `loot`, which no part produces or accepts (`uploader/app/ui/upload-lines.js:13`).
6. **Version 1 hard-coded in five places.** `addon/Core.lua:36`, `site:src/lib/ingest.ts:29,50`, `site:src/app/calendar/actions.ts:178`, `uploader/core/src/lib.rs:34`. A `v2` string is silently ignored by the uploader, with no message.
7. **Duplicated constants.** ALPHA, MAX_STRING, MAX_DECODED in `site:src/lib/addon-codec.ts:6-9` and `uploader/core/src/envelope.rs:8-10`. Type list in `uploader/core/src/lib.rs:29`, the regex at `lib.rs:34`, and `site:src/app/api/uploader/ingest/route.ts:13`. The Lua decoder has no size limits (`addon/Codec.lua:178-187`).
8. **Rank-name limits disagree.** Addon: 160 bytes (`addon/Guild.lua:23`). Uploader: 160 bytes, guild 96 bytes (`uploader/core/src/ranks.rs:20-21`). Site envelope path: 40 chars, guild 64 (`site:src/lib/rank-names.ts:10,44`). `/ranks` endpoint: 32 chars (`site:src/lib/rank-names.ts:114`). A 33-40 char name is accepted in a `ranks` string but refused by `/ranks`.
9. **Two paths for rank names, one unsigned.** `/api/uploader/ranks` takes a block the uploader reads from SavedVariables, with no HMAC (`uploader/core/src/lib.rs:198-216`). The signed `ranks` string goes through ingest. Both end in `storeRankNames` (`site:src/lib/rank-sync.ts:26`, `site:src/lib/uploader.ts:81`).
10. **`/ranks` response shape drift.** Site sends `{stored: boolean}` (`site:src/lib/rank-sync.ts:27-28`); uploader tests use `{"stored":"stored"}` (`uploader/core/src/lib.rs:337`). The parser accepts both (`uploader/core/src/ranks.rs:403-408`).
11. **UI error map drift.** UI maps `site_refused` which the site never sends (`uploader/app/ui/upload-lines.js:69`). `bad_sig`, `not_linked`, `stale`, `unsupported_version` all show one generic line (`upload-lines.js:74`); the site's helpful `INGEST_ERR` text (`site:src/lib/ingest.ts:9-21`) never reaches the uploader.
12. **Canonical JSON edge cases.** DEL (0x7F) is escaped by Lua (`addon/Codec.lua:14`) but not by `JSON.stringify` (`site:src/lib/addon-codec.ts:72`), so a tab name containing it fails with `bad_sig`. Invalid UTF-8 and `-0` also break. The Lua decoder has no surrogate pairs (`addon/Codec.lua:63-67`).
13. **`link` identity field mismatch.** The addon sends `char.realm` (`addon/Core.lua:20-23`); the site wants `char.surname` and falls back to `realm` (`site:src/app/settings/link/actions.ts:56`). A bad class gives the "too old or too new" message (`:61`).
14. **Fields sent but unused or unvalidated.** `recipes.skill` is ignored (`site:src/lib/crafting.ts:15-20`). Bad recipe ids are dropped, not refused (`:18`). `char.name`/`realm` in signed exports are never read. `envelope.addon` is optional in the TS type (`site:src/lib/ingest.ts:7`).
15. **Addon text points to a missing paste box.** `/cmd export` says paste on the "Bank or Crafting page" (`addon/Core.lua:119`). No recipes paste form exists; the crafting page says nothing needs pasting (`site:src/app/crafting/page.tsx:156`).
16. **Stale uploader comment.** `uploader/core/src/browser_auth.rs:7-11` says the site routes 404 and are "not deployed". They exist (`site:src/app/api/uploader/oauth/*/route.ts`).
17. **429 not handled on sign-in.** Preflight 429 becomes `NotDeployed` (`browser_auth.rs:236-239`); token 429 becomes `BadResponse` (`:442`). Ranks 413 becomes a retry (`ranks.rs:420`) and will repeat each pass.
18. **Ingest size check trusts the header.** `Content-Length` is checked but `req.json()` reads the full body (`site:src/app/api/uploader/ingest/route.ts:18-20`). `/logs` reads with a cap (`site:src/lib/uploader-log-rules.ts:53-68`).
19. **`pasteRaid` guesses the type.** Anything not starting `!BF:kills:` is treated as `attendance` (`site:src/app/calendar/actions.ts:164`).
20. **`/api/uploader/latest` is unused** by the uploader (section 4.2). Its contract has no client.

---

## 7. Golden fixtures (planned)

Generated in task A2: see `contract/golden/README.md` for how, and `contract/golden/MANIFEST.json` for the inventory and the files still pending (site-produced types, OAuth pairs, `stale` and `bad-envelope`). Strings use a fixed test secret (`11` x 32), `Player-1234-0ABCDEF1`, "Testchar", "Example Guild", fixed `t`.

Export strings (`contract/golden/strings/`), each a `.txt` (the string) plus `.json` (decoded envelope):

- `link.v1.txt`, `link.v1.json`
- `link-ok.v1.txt`, `link-ok.v1.json` (signed with a test Ed25519 key; `link-ok.v1.pub.hex`)
- `gbank.v1.txt`, `gbank.v1.json`
- `gbank.v1.no-guild.txt`, `gbank.v1.no-guild.json`
- `recipes.v1.txt`, `recipes.v1.json`
- `ranks.v1.txt`, `ranks.v1.json`
- `signup.v1.txt`, `signup.v1.json`
- `attendance.v1.txt`, `attendance.v1.json`
- `kills.v1.txt`, `kills.v1.json`
- `signups.v1.txt`, `signups.v1.json`
- `canonical.v1.json` (input object and expected canonical text, including non-ASCII and control chars)
- `secret.v1.hex` (the test HMAC secret)

Negative strings (`contract/golden/strings/bad/`):

- `bad-prefix.txt`, `unknown-type.txt`, `unsupported-version.txt`, `decode-failed.txt`, `too-large.txt`, `bad-envelope.txt`, `bad-sig.txt`, `stale.txt`

HTTP pairs (`contract/golden/http/<endpoint>/<case>.request.json` and `.response.json`):

- `oauth-authorize/`: `preflight`, `bad-query`, `rate-limited`, `no-session-redirect`
- `oauth-consent/`: `approve`, `deny`, `expired`, `invalid`, `forbidden`
- `oauth-switch/`: `switch`, `invalid`
- `oauth-token/`: `ok`, `invalid-grant`, `forbidden`, `rate-limited`
- `ingest/`: `ok-mixed-results`, `duplicate`, `forbidden-gbank`, `unknown-type`, `bad-token`, `rate-limited`, `too-large`, `bad-request`
- `ranks/`: `stored`, `older`, `other-guild`, `bad-ranks`, `bad-time`, `not-officer`, `not-on-roster`, `bad-token`, `rate-limited`
- `logs/`: `stored-token`, `stored-anonymous`, `bad-error-code`, `log-too-large`, `rate-limited`
- `addon-latest/`: `ok`, `not-published`
- `uploader-latest/`: `ok`, `not-published`
