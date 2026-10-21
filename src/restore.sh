#!/usr/bin/env bash
# Linuxify restore script. Run it once on your new Linux system, from inside
# the linuxify-migration folder:
#
#   bash restore.sh --dry-run    # show what would happen, change nothing
#   bash restore.sh              # do it (asks before installing apps)
#   bash restore.sh --yes        # don't ask
#
# What it does:
#   1. checks every file against SHA256SUMS
#   2. copies home/ into your home folder, never overwriting existing files
#   3. fixes SSH key permissions and refreshes fonts
#   4. sets up Firefox / LibreWolf / Thunderbird profiles (as extra profiles)
#   5. installs the apps listed in apps.txt (Flatpak first, then native packages)
# A log is written to ~/linuxify-restore.log.

set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DRY=0
YES=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY=1 ;;
    --yes|-y) YES=1 ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "Unknown option: $arg (try --help)"; exit 2 ;;
  esac
done

LOG="$HOME/linuxify-restore.log"
[ "$DRY" = 1 ] && LOG=/dev/null
exec > >(tee -a "$LOG") 2>&1

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
note() { printf '   %s\n' "$*"; }
run() {
  if [ "$DRY" = 1 ]; then
    printf '   [dry-run]'; printf ' %q' "$@"; printf '\n'
  else
    "$@"
  fi
}
ask() {
  [ "$YES" = 1 ] && return 0
  [ "$DRY" = 1 ] && return 0
  local reply
  read -r -p "   $1 [y/N] " reply </dev/tty || return 1
  [[ "$reply" =~ ^[Yy] ]]
}
have() { command -v "$1" >/dev/null 2>&1; }

echo "Linuxify restore - $(date)"
[ "$DRY" = 1 ] && echo "DRY RUN: nothing will be changed."
cd "$HERE" || exit 1

# ---------------------------------------------------------------------------
step "1/5 Checking your files against SHA256SUMS"
if [ ! -f SHA256SUMS ]; then
  echo "   SHA256SUMS is missing; is this the linuxify-migration folder?"; exit 1
fi
TOTAL=$(grep -c . SHA256SUMS || true)
if sha256sum --quiet --check SHA256SUMS; then
  note "All $TOTAL files match."
else
  echo "   Some files are missing or damaged (listed above)."
  ask "Continue anyway with the files that are fine?" || exit 1
fi

# ---------------------------------------------------------------------------
step "2/5 Copying your files into $HOME (existing files are kept)"
copied=0; kept=0
if [ -d home ]; then
  while IFS= read -r -d '' d; do
    [ -d "$HOME/$d" ] || run mkdir -p "$HOME/$d"
  done < <(cd home && find . -mindepth 1 -type d -print0)
  while IFS= read -r -d '' f; do
    if [ -e "$HOME/$f" ]; then
      kept=$((kept + 1))
    else
      run cp -p "home/$f" "$HOME/$f" && copied=$((copied + 1))
    fi
  done < <(cd home && find . -type f -print0)
fi
note "Copied $copied files; $kept already existed and were left alone."

# ---------------------------------------------------------------------------
step "3/5 SSH keys and fonts"
if [ -d "$HOME/.ssh" ]; then
  run chmod 700 "$HOME/.ssh"
  for f in "$HOME"/.ssh/*; do
    [ -f "$f" ] || continue
    case "$(basename "$f")" in
      *.pub|known_hosts*|config|authorized_keys) run chmod 644 "$f" ;;
      *) run chmod 600 "$f" ;;
    esac
  done
  note "SSH folder permissions fixed (private keys 600)."
  if [ -f "$HOME/.ssh/config" ] && grep -qiE '[A-Z]:\\|\\Users\\' "$HOME/.ssh/config"; then
    note "Heads-up: ~/.ssh/config contains Windows paths (like C:\\...). Edit them to Linux paths."
  fi
else
  note "No SSH keys to set up."
fi
if [ -d "$HOME/.local/share/fonts" ] && have fc-cache; then
  run fc-cache -f "$HOME/.local/share/fonts" >/dev/null
  note "Fonts refreshed."
fi

# ---------------------------------------------------------------------------
step "4/5 Browser and email profiles"
# Adds a staged Mozilla-style profile as an extra profile (never replaces one).
# $1 = staged profile dir, $2 = profiles root, $3 = app name, $4 = where to pick it
add_mozilla_profile() {
  local src="$1" root="$2" app="$3"
  local name; name="linuxify-$(basename "$src")"
  if [ -e "$root/$name" ]; then
    note "$app profile '$name' already exists; skipped."; return
  fi
  run mkdir -p "$root"
  run cp -a "$src" "$root/$name"
  if [ "$DRY" = 1 ]; then
    note "[dry-run] would add [$name] to $root/profiles.ini"; return
  fi
  if [ ! -f "$root/profiles.ini" ]; then
    printf '[General]\nStartWithLastProfile=1\nVersion=2\n\n' > "$root/profiles.ini"
  fi
  local n=0
  while grep -q "^\[Profile$n\]" "$root/profiles.ini"; do n=$((n + 1)); done
  printf '\n[Profile%s]\nName=%s\nIsRelative=1\nPath=%s\n' "$n" "$name" "$name" >> "$root/profiles.ini"
  note "$app: added profile '$name'. Open $4 and choose 'Set as default profile' on it."
}
moz_root() { # $1 = flatpak id, $2 = classic dir
  if [ -d "$HOME/.var/app/$1" ]; then echo "$HOME/.var/app/$1/$2"; else echo "$HOME/$2"; fi
}
found=0
for p in browsers/firefox/*/ browsers/librewolf/*/ browsers/waterfox/*/; do
  [ -d "$p" ] || continue
  found=1
  case "$p" in
    browsers/firefox/*) add_mozilla_profile "${p%/}" "$(moz_root org.mozilla.firefox .mozilla/firefox)" Firefox "about:profiles in Firefox" ;;
    browsers/librewolf/*) add_mozilla_profile "${p%/}" "$(moz_root io.gitlab.librewolf-community .librewolf)" LibreWolf "about:profiles in LibreWolf" ;;
    browsers/waterfox/*) add_mozilla_profile "${p%/}" "$HOME/.waterfox" Waterfox "about:profiles in Waterfox" ;;
  esac
done
for p in mail/thunderbird/*/; do
  [ -d "$p" ] || continue
  found=1
  add_mozilla_profile "${p%/}" "$(moz_root org.mozilla.Thunderbird .thunderbird)" Thunderbird "Thunderbird's Profile Manager (thunderbird -P)"
done
for b in browsers/google-chrome browsers/microsoft-edge browsers/brave browsers/vivaldi browsers/chromium; do
  [ -d "$b" ] || continue
  found=1
  note "Bookmarks from $(basename "$b") are in $HERE/$b/<profile>/Bookmarks."
  note "  Chrome-family browsers can import them: open the browser once, close it, then copy that"
  note "  file over the new profile's Bookmarks file (e.g. ~/.config/google-chrome/Default/Bookmarks)."
done
[ "$found" = 0 ] && note "No browser or email profiles were staged."
if [ -d apps/obs-studio ]; then
  note "OBS scenes are in $HERE/apps/obs-studio/basic. In OBS: Scene Collection > Import."
fi

# ---------------------------------------------------------------------------
step "5/5 Installing your apps (from apps.txt)"
PM=""
for c in apt-get dnf zypper pacman rpm-ostree; do have "$c" && { PM=$c; break; }; done
note "Package manager: ${PM:-none found}"
mapfile -t FLATPAKS < <(awk '$1=="flatpak"{print $2}' apps.txt 2>/dev/null)
mapfile -t PKGLINES < <(awk '$1=="pkg"{print $2" "$3}' apps.txt 2>/dev/null)
note "${#FLATPAKS[@]} Flatpak apps and ${#PKGLINES[@]} native packages are listed in apps.txt."
failed=()
if [ "${#FLATPAKS[@]}" -gt 0 ] || [ "${#PKGLINES[@]}" -gt 0 ]; then
  if ask "Install them now? (needs internet; you may be asked for your password)"; then
    if [ "${#FLATPAKS[@]}" -gt 0 ]; then
      if ! have flatpak; then
        case "$PM" in
          apt-get) run sudo apt-get install -y flatpak ;;
          dnf) run sudo dnf install -y flatpak ;;
          zypper) run sudo zypper --non-interactive install flatpak ;;
          pacman) run sudo pacman -S --noconfirm --needed flatpak ;;
          *) note "Flatpak is not installed and I don't know how to install it here." ;;
        esac
      fi
      if have flatpak || [ "$DRY" = 1 ]; then
        run flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
        for id in "${FLATPAKS[@]}"; do
          run flatpak install --user -y --noninteractive flathub "$id" || failed+=("flatpak $id")
        done
      fi
    fi
    if [ "${#PKGLINES[@]}" -gt 0 ] && [ "$PM" = apt-get ]; then
      run sudo apt-get update -q || note "Couldn't refresh package lists; trying anyway."
    fi
    for line in "${PKGLINES[@]}"; do
      apt_name=${line%% *}; dnf_name=${line##* }
      case "$PM" in
        apt-get) [ "$apt_name" != "-" ] && { run sudo apt-get install -y "$apt_name" || failed+=("apt $apt_name"); } ;;
        dnf) [ "$dnf_name" != "-" ] && { run sudo dnf install -y "$dnf_name" || failed+=("dnf $dnf_name"); } ;;
        zypper) [ "$dnf_name" != "-" ] && { run sudo zypper --non-interactive install "$dnf_name" || failed+=("zypper $dnf_name"); } ;;
        pacman) [ "$dnf_name" != "-" ] && { run sudo pacman -S --noconfirm --needed "$dnf_name" || failed+=("pacman $dnf_name"); } ;;
        *) failed+=("$line (no supported package manager)") ;;
      esac
    done
  else
    note "Skipped. You can run this script again later; it won't copy files twice."
  fi
fi

# ---------------------------------------------------------------------------
step "Done"
if [ "${#failed[@]}" -gt 0 ]; then
  note "These installs didn't work (install them from the Software app instead):"
  for f in "${failed[@]}"; do note "  - $f"; done
fi
note "Things Linuxify could not copy are listed in $HERE/flagged.txt"
[ "$DRY" = 0 ] && note "This log: $LOG"
exit 0
