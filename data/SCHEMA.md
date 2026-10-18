# data/app_map.json schema

`app_map.json` tells Linuxify what happens to each Windows program on Linux.
It is plain JSON so it can be edited by hand. Linuxify reads it at startup
(from `data/` next to the exe, or `--data-dir`), falling back to the copy built
into the exe.

```json
{
  "schema_version": 1,
  "ignore": [
    { "match": ["microsoft visual c++"], "reason": "Windows runtime library; not needed on Linux" }
  ],
  "apps": [
    {
      "id": "firefox",
      "name": "Mozilla Firefox",
      "match": ["mozilla firefox"],
      "category": "browser",
      "status": "native",
      "linux": { "flatpak": "org.mozilla.firefox", "apt": "firefox", "dnf": "firefox" },
      "alternatives": [],
      "notes": "Your profile (bookmarks, passwords, extensions) can be carried over.",
      "link": "https://www.mozilla.org/firefox/"
    }
  ]
}
```

## Fields

| field | required | meaning |
|---|---|---|
| `id` | yes | unique, lowercase, `a-z0-9-` |
| `name` | yes | human name shown in the report |
| `match` | yes | list of match patterns (see below), all lowercase |
| `category` | yes | one of: `browser`, `office`, `creative`, `audio`, `video`, `dev`, `game`, `game-launcher`, `chat`, `media`, `utility`, `security`, `vpn`, `cloud`, `hardware`, `system`, `education`, `finance`, `other` |
| `status` | yes | `native`, `alternative`, `wine_ok`, `blocked`, `unknown` |
| `linux` | no | package names for the native Linux version: any of `flatpak`, `apt`, `dnf`, `snap`, `url` |
| `alternatives` | for `alternative` (optional otherwise) | list of `{ "name", "flatpak"?, "note"? }` |
| `notes` | no | one or two plain-English sentences for a non-expert |
| `link` | no | official homepage, ProtonDB or WineHQ AppDB page |

## Status meanings

- `native` – the same program has an official Linux version.
- `alternative` – no Linux version; list replacements in `alternatives`.
- `wine_ok` – no Linux version, but it is widely reported to run well through Wine or Proton.
- `blocked` – known NOT to work on Linux (e.g. kernel-level anti-cheat that blocks Linux, Adobe Creative Cloud apps).
- `unknown` – not sure. **Use this rather than guessing.**

A program can be both `alternative`-worthy and `wine_ok`; pick the one a
newcomer should try first, and mention the other in `notes`.

## Matching

Each installed program's display name is normalised (lowercased, ™ ® ©
removed, runs of whitespace collapsed to one space, trimmed). A pattern matches
when:

- `"text"` – the normalised name **contains** `text`
- `"^text"` – the normalised name **starts with** `text`
- `"=text"` – the normalised name **equals** `text`

If several entries match, the one with the **longest** matching pattern wins
(so `"microsoft visual studio code"` beats `"microsoft visual studio"`).
`ignore` entries are checked with the same rules and the same longest-wins
contest against `apps`.

Steam games are recognised automatically (registry key `Steam App <id>`), so
unmapped Steam games get a ProtonDB link without needing an entry here.
