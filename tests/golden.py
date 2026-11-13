"""Golden export strings for contract/golden/strings (contract/SPEC.md section 7).

The Lua 5.1 run (the game's runtime) writes the .txt strings. The LuaJIT run must decode to the same JSON.
Every string is also decoded and its HMAC checked here, in Python, independently of the Lua code, from the
rules in contract/SPEC.md section 2. Files under contract/golden are add-only: see contract/golden/README.md.
"""
import hashlib, hmac, json, os, zlib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "contract", "golden")
ALPHA = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789()"
TYPES = {"link": 1, "gbank": 1, "recipes": 1, "ranks": 1}
UPLOADER_PROTOCOL = 1

GEN = r'''function(ns)
  E2E_SEED(20261113)
  E2E_IN_GUILD = true
  GuildaddonDB.guild = nil
  GuildaddonDB.outbox = {}
  ns.captureRanks()
  local out = {}
  local tabs = { { tab = 1, name = "Mats: \"ore\"", slots = {
      { link = "|cffffffff|Hitem:2771::::::::60:::::|h[Tin Ore]|h|r", count = 20, texture = 135 },
      { link = "|Hitem:2771::|h", count = 5 }, { link = "|Hitem:13444::|h", count = 1 } } },
    { tab = 2, name = "Épées", slots = {} } }
  local p = ns.bankPayload(tabs, 1234567, function(id) return 100000 + id end)
  out["gbank"] = ns.export("gbank", p)
  out["recipes"] = ns.export("recipes", { profession = "Alchemy", skill = 300,
    recipes = ns.recipeIds({ "|Henchant:17187|h", "spell:3170" }) })
  out["ranks"] = GuildaddonDB.outbox.ranks.s
  E2E_IN_GUILD = false
  out["gbank.no-guild"] = ns.export("gbank", p)
  E2E_IN_GUILD = true
  local copied, showCopy = nil, ns.showCopy
  ns.showCopy = function(_, text) copied = text end
  local key = next(SlashCmdList)
  SlashCmdList[key]("link ABCD2345")
  ns.showCopy = showCopy
  out["link"] = copied
  -- bad-sig: a recipes string whose envelope changed after signing.
  local _, _, j = ns.decodeString(out["recipes"])
  j.addon = "0.0.0-tampered"
  out["bad/bad-sig"] = ns.encodeString("recipes", 1, j, nil)
  out["canonical"] = ns.jsonEncode({ z = 1, n = -42, big = 9007199254740991, yes = true, no = false,
    s = { "Épées", "tab\tnew\nline", "quote\"back\\slash", "ctl\1" } })
  return out
end'''

CANONICAL_INPUT = {"z": 1, "n": -42, "big": 9007199254740991, "yes": True, "no": False,
                   "s": ["Épées", "tab\tnew\nline", "quote\"back\\slash", "ctl\u0001"]}


def make(lua, ns):
    t = lua.eval(GEN)(ns)
    return {k: t[k] for k in list(t)}


def decode_for_print(body):
    out = bytearray()
    for i in range(0, len(body), 4):
        v = 0
        chunk = body[i:i + 4]
        for j, c in enumerate(chunk):
            k = ALPHA.find(c)
            if k < 0:
                raise ValueError("character outside the alphabet")
            v |= k << (6 * j)
        for _ in range(len(chunk) * 6 // 8):
            out.append(v & 255)
            v >>= 8
    return bytes(out)


def decode(s):
    """SPEC 2.1-2.2: !BF:<type>:v<n>:<EncodeForPrint(raw deflate(canonical JSON))>."""
    assert s.startswith("!BF:"), s[:20]
    _, ty, v, body = s.split(":", 3)
    text = zlib.decompress(decode_for_print(body), -15).decode("utf-8")
    return ty, int(v[1:]), text, json.loads(text)


def canonical(v):
    """SPEC 2.6, for the characters the fixtures use (no DEL, no lone surrogates)."""
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def build(made, secret):
    """Return {relative path: file text} for everything under contract/golden/strings."""
    a, b = made["lua51"], made["luajit"]
    files, errors = {}, []
    for name in ("link", "gbank", "gbank.no-guild", "recipes", "ranks"):
        ty = name.split(".")[0]
        ty_, v, text, env = decode(a[name])
        _, _, _, env_jit = decode(b[name])
        if env != env_jit:
            errors.append(f"{name}: Lua 5.1 and LuaJIT decode to different JSON")
        if ty_ != ty or v != TYPES[ty]:
            errors.append(f"{name}: header says {ty_} v{v}")
        if text != canonical(env):
            errors.append(f"{name}: body is not canonical JSON")
        if ty != "link":
            body = {k: x for k, x in env.items() if k != "sig"}
            want = hmac.new(bytes.fromhex(secret), canonical(body).encode(), hashlib.sha256).hexdigest()
            if env.get("sig") != want:
                errors.append(f"{name}: HMAC does not verify in Python")
        files[f"strings/{name}.v{v}.txt"] = a[name] + "\n"
        files[f"strings/{name}.v{v}.json"] = json.dumps(env, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    if a["canonical"] != canonical(CANONICAL_INPUT):
        errors.append("canonical: Lua and Python canonical JSON differ")
    files["strings/canonical.v1.json"] = json.dumps({"input": CANONICAL_INPUT, "canonical": a["canonical"]},
                                                    indent=2, ensure_ascii=False) + "\n"
    files["strings/secret.v1.hex"] = secret + "\n"
    gb = a["gbank"]
    bad = {
        "bad-prefix": ("hello", "bad_prefix"),
        "unknown-type": (gb.replace("!BF:gbank:", "!BF:nope:", 1), "unknown_type"),
        "unsupported-version": (gb.replace(":v1:", ":v9:", 1), "unsupported_version"),
        "decode-failed": ("!BF:gbank:v1:" + "~" * 40, "decode_failed"),
        "bad-sig": (a["bad/bad-sig"], "bad_sig"),
    }
    expected = {}
    for name, (s, err) in bad.items():
        files[f"strings/bad/{name}.txt"] = s + "\n"
        expected[f"{name}.txt"] = err
    expected["too-large"] = {"rule": "'!BF:gbank:v1:' followed by 262144 'a' characters (longer than MAX_STRING)",
                             "error": "too_large"}
    files["strings/bad/expected.json"] = json.dumps(expected, indent=2, sort_keys=True) + "\n"
    return files, errors


def manifest(files):
    """MANIFEST.json over every file under contract/golden (strings from here, http from cargo)."""
    entries = {}
    for dirpath, _, names in os.walk(ROOT):
        for n in names:
            rel = os.path.relpath(os.path.join(dirpath, n), ROOT).replace(os.sep, "/")
            if rel in ("MANIFEST.json", "README.md") or rel.startswith("strings/"):
                continue
            entries[rel] = open(os.path.join(dirpath, n), "rb").read()
    for rel, text in files.items():
        entries[rel] = text.encode()
    return json.dumps({
        "exportVersions": {k: [v] for k, v in sorted(TYPES.items())},
        "uploaderProtocol": UPLOADER_PROTOCOL,
        "generators": {"strings/": "python tests/run.py --write-golden",
                       "http/*.request.json": "WRITE_GOLDEN=1 cargo test -p guild-uploader-core golden",
                       "http/*.response.json": "written by hand from contract/SPEC.md section 4.2; checked by the site"},
        "pending": ["link-ok, signup, attendance, kills, signups strings: produced or read only by the site; added by a site PR",
                    "oauth-* http pairs: A5", "stale and bad-envelope strings: need the site's rules"],
        "files": {rel: hashlib.sha256(data).hexdigest() for rel, data in sorted(entries.items())},
    }, indent=2) + "\n"


def check_or_write(made, secret, write=False):
    files, errors = build(made, secret)
    for e in errors:
        print("golden:", e)
    if errors:
        return 1
    files["MANIFEST.json"] = manifest(files)
    if write:
        for rel, text in files.items():
            p = os.path.join(ROOT, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w", encoding="utf-8", newline="\n") as f:
                f.write(text)
        print(f"golden: wrote {len(files)} files under contract/golden")
        return 0
    bad = []
    for rel, text in sorted(files.items()):
        p = os.path.join(ROOT, rel)
        if not os.path.exists(p):
            bad.append(f"missing {rel}")
        elif open(p, encoding="utf-8").read() != text:
            bad.append(f"differs {rel}")
    for b in bad:
        print("golden:", b)
    if bad:
        print("golden: output differs from contract/golden. A changed encoding needs a new version (contract/SPEC.md 5.2);"
              " regenerate only for new files: python tests/run.py --write-golden")
        return 1
    print(f"golden: {len(files)} files match")
    return 0
