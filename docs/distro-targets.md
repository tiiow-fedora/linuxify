# Distro targets: package mapping and post-install notes

This document describes the `distros/` layer. It maps every program in the
catalog (`data/app_map.json`) to a package on five install targets, and carries
per-target setup notes. It consists of **new data files and tests only**. No
existing source file is changed.

Targets: **Fedora**, **Arch**, **Debian**, **Ubuntu**, **openSUSE**.

## Why a separate layer

`data/app_map.json` records `apt`, `dnf`, `flatpak`, `snap` and `url` hints per
program. That is enough for the Mint/Ubuntu/Fedora path, but it has no Arch
(`pacman`/AUR) or openSUSE (`zypper`) names, it cannot say *which extra
repository* a package needs, and `apt` is shared by Debian and Ubuntu even though
they differ (`firefox-esr`, `docker.io`, snaps). Rather than widen the catalog
schema, the layer is keyed by catalog `id` and kept in its own files, so the
catalog stays the single source of truth for *what a program is* and this layer
answers *what to type on distro X*.

## Files

| path | purpose |
|---|---|
| `distros/targets.json` | the five targets: package manager, install/refresh command templates, third-party repos (with how to enable them), and post-install notes |
| `distros/packages/*.json` | shards mapping native catalog programs (those with a `linux` block) to a package per target |
| `distros/alternatives/*.json` | optional shards mapping catalog *alternatives* (the replacement apps suggested for programs with no Linux version) to a package per target |
| `tests/distro_targets.rs` | static validation, described below |

Shards exist only to keep files reviewable and to allow parallel authoring.
They are merged by app id; a duplicate id across shards is a test failure.

## Mapping entry

```json
"git": {
  "fedora":   {"source": "repo",        "pkg": "git",            "confidence": "high"},
  "arch":     {"source": "repo",        "pkg": "git",            "confidence": "high"},
  "debian":   {"source": "repo",        "pkg": "git",            "confidence": "high"},
  "ubuntu":   {"source": "repo",        "pkg": "git",            "confidence": "high"},
  "opensuse": {"source": "repo",        "pkg": "git",            "confidence": "high"},
  "notes": ["Optional app specific post-install sentence."]
}
```

Alternatives are keyed `"<app id>::<alternative name>"` exactly as they appear
in the catalog.

### `source`

| source | meaning | needs |
|---|---|---|
| `repo` | the distro's own repositories | optional `needs` (e.g. Arch `multilib`) |
| `rpmfusion`, `packman` | Fedora / openSUSE third-party multimedia repos | `needs` required |
| `vendor-repo` | the vendor's own APT/RPM repository | `needs` required |
| `copr`, `ppa`, `obs` | community build services | `repo` required |
| `aur` | Arch User Repository | none |
| `snap` | Ubuntu snap | none |
| `flatpak` | Flathub application id | `pkg` is the Flathub id |
| `download` | an official https download page, no package | `url` required |
| `none` | not installable on this target | `reason` required |

### `confidence`

* `high`: the name is the long-standing package name on that distro.
* `medium`: believed correct, but names drift between releases.
* `low`: best guess; a human should check before relying on it.

**Important.** The mappings were authored without network access to any package
index, so none of them has been checked against a live repository. The
`confidence` field is the author's honest estimate, and the tests below only
prove that the data is well-formed and consistent, **not** that a package exists.
Before the installer acts on a mapping, a runtime check (`dnf info`, `pacman -Si`,
`apt-cache policy`, `zypper info`) should confirm it and fall back to Flatpak.

## Invariants enforced by the tests

`cargo test --test distro_targets` (no network, no system access) checks:

1. `targets.json` is well-formed: exactly the five targets, command templates
   contain `{pkgs}`, repo ids are unique and each has `enable` and `note`.
2. Post-install notes: ≥3 per target, globally unique ids prefixed by target,
   full sentences, and every `when` condition (`always`, `category:<c>`,
   `app:<id>`) refers to a category or app that exists in the catalog.
3. **Coverage.** Every catalog program with a `linux` block has an entry, with
   all five targets, and no entry refers to an unknown program.
4. Each entry is well-formed for its source: package-name charset, Flathub id
   shape (reverse DNS, ≥3 components), `https` URLs, `repo` shape for
   copr/ppa/obs, `reason` for `none`, `needs` referencing repos that the target
   actually defines, and only sources allowed for that target.
5. **Agreement with the catalog.** A `repo` package on Fedora/Debian/Ubuntu
   equals the catalog's `dnf`/`apt` hint unless the entry says why not with
   `catalog_override`; a `flatpak` source equals the catalog's Flathub id; a
   target is never `none` when the catalog already offers a Flatpak or native
   package for it.
6. Every program is installable on at least one target.
7. **Alternatives coverage.** Every `alternatives` entry in the catalog (606 across
   all programs) has a `distros/alternatives` mapping that obeys the same entry
   rules, and the same alternative name suggested for several programs maps to the
   same package everywhere (unless only one of them has a catalog Flathub id).

`cargo test --test distro_targets -- --nocapture coverage_report` prints
coverage counts by source.

## Post-install notes

`targets.json` has a list of notes per target with a `when` selector.
A front end shows the notes whose selector matches any program being installed
(`always` notes are shown once per target). Typical content: enabling RPM
Fusion, the Arch `multilib` repo and AUR caveats, Debian `non-free-firmware`,
Ubuntu's snap-backed `firefox`, Packman on openSUSE, NVIDIA driver pointers,
and adding Flathub (it is not enabled by default except on Fedora, and even there
the remote is filtered).

## Design choices

* **Keyed by catalog id, never by display name.** The catalog already owns name
  matching; this layer cannot drift from it, and tests fail if it does.
* **Every target gets an explicit entry**, even `none` with a reason. A missing
  key is a test failure, so "not yet looked at" cannot be confused with "not
  available".
* **Flatpak is the universal fallback**, but only where the catalog already names
  a Flathub id. This layer never invents Flathub ids.
* **Provenance is explicit.** Anything outside the distro's official repos
  (`aur`, `copr`, `ppa`, `obs`, `vendor-repo`, `rpmfusion`, `packman`) is labelled
  so a UI can warn before enabling an extra source.
* **Pure data.** No Rust source under `src/` is touched, so it merges cleanly
  with parallel work on the core and CI.

## Not done / follow-ups

* Wiring the layer into `src/` (a loader that selects the target from the
  chosen distro in `data/distros.json`, builds an install command from
  `install_cmd`, and displays notes). Deliberately left out: this change adds
  new files only.
* Live verification of names against Fedora, Arch, Debian, Ubuntu and openSUSE
  indices (for example Repology) from a networked CI job.
* Derivatives: the mapping targets the five bases. Mint, Pop!_OS, Zorin and
  other Ubuntu/Debian derivatives can reuse `ubuntu`/`debian`; Manjaro and
  EndeavourOS can reuse `arch`, noting Manjaro's delayed repos.
