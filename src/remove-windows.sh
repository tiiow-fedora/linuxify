#!/usr/bin/env bash
# Linuxify: remove Windows from a dual-boot PC, AFTER your files are safely in Linux.
#
#   bash remove-windows.sh            # check everything and show the plan (changes nothing)
#   bash remove-windows.sh --really   # do it, after you type the confirmation phrase
#
# It refuses unless:
#   - it's running on an installed Linux (not a live USB session),
#   - every file restored into your home folder matches the checksum taken on Windows
#     (file count + SHA-256),
#   - exactly one Windows installation is found, and none of its partitions is mounted
#     or holds this migration folder.
# It deletes only: the NTFS partition containing Windows\System32, the Microsoft
# reserved and Windows recovery partitions, and the LNXINST installer partition,
# all on that same disk. The EFI partition, Linux partitions and other data
# partitions are never touched. Windows' boot files on the EFI partition are
# renamed (EFI/Microsoft -> EFI/Microsoft.removed-by-linuxify), not deleted.

set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REALLY=0
ACCEPT_CHANGED=0
for arg in "$@"; do
  case "$arg" in
    --really) REALLY=1 ;;
    --accept-changed) ACCEPT_CHANGED=1 ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "Unknown option: $arg (try --help)"; exit 2 ;;
  esac
done
MSR=e3c9e316-0b5c-4db8-817d-f92df00215ae
WINRE=de94bba4-06d1-4d40-a16a-bfd50179d6ac
ESP=c12a7328-f81f-11d2-ba4b-00a0c93ec93b

refuse() { printf '\nREFUSED: %s\nNothing was changed.\n' "$*"; exit 1; }
step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
note() { printf '   %s\n' "$*"; }

[ "$(id -u)" = 0 ] && refuse "run this as your normal user (it uses sudo when needed), so it can check YOUR home folder."
for t in lsblk sfdisk sha256sum mount; do command -v "$t" >/dev/null || refuse "missing tool: $t"; done

# ---------------------------------------------------------------------------
step "1/4 Is this an installed Linux?"
rootfs=$(findmnt -n -o FSTYPE / 2>/dev/null)
if [ -d /run/live ] || [ -d /cdrom/casper ] || grep -qw 'boot=casper\|rd.live.image\|boot=live' /proc/cmdline 2>/dev/null \
   || [ "$rootfs" = overlay ] || [ "$rootfs" = squashfs ] || [ "$rootfs" = tmpfs ]; then
  if [ "${LINUXIFY_TEST_ALLOW_LIVE:-0}" = 1 ]; then
    note "Live session detected, continuing ONLY because LINUXIFY_TEST_ALLOW_LIVE=1 (testing)."
  else
    refuse "this looks like a live USB/installer session. Install Linux first, boot into it, then run this."
  fi
else
  note "Yes (root file system: $rootfs)."
fi

# ---------------------------------------------------------------------------
step "2/4 Checking every restored file against the Windows-side checksums"
[ -f "$HERE/SHA256SUMS" ] || refuse "SHA256SUMS not found next to this script."
expected=$(grep -c '  \./home/' "$HERE/SHA256SUMS" || true)
[ "$expected" -gt 0 ] || refuse "the manifest lists no home-folder files to verify."
tmp=$(mktemp)
home_esc=$(printf '%s' "$HOME" | sed 's/[&#\]/\\&/g')
grep '  \./home/' "$HERE/SHA256SUMS" | sed "s#  \./home/#  ${home_esc}/#" > "$tmp"
results=$(sha256sum -c "$tmp" 2>/dev/null)
rm -f "$tmp"
ok=$(printf '%s\n' "$results" | grep -c ': OK$' || true)
bad=$(printf '%s\n' "$results" | grep -v ': OK$' | grep -c . || true)
note "$ok of $expected files match."
if [ "$ok" -ne "$expected" ]; then
  printf '%s\n' "$results" | grep -v ': OK$' | head -20 | sed 's/^/   /'
  missing=$(printf '%s\n' "$results" | grep -c 'FAILED open or read' || true)
  changed=$((bad - missing))
  if [ "$missing" -gt 0 ]; then
    refuse "$missing restored files are missing from your home folder. Run restore.sh again, or copy them back."
  fi
  if [ "$ACCEPT_CHANGED" = 1 ]; then
    note "$changed files differ (you said they were edited on purpose: --accept-changed)."
  else
    refuse "$changed restored files don't match the originals. If you edited them on purpose, re-run with --accept-changed."
  fi
fi

# ---------------------------------------------------------------------------
step "3/4 Finding Windows"
here_dev=$(df --output=source "$HERE" | tail -1)
win_parts=()
while read -r dev fstype; do
  [ "$fstype" = ntfs ] || continue
  mnt=$(findmnt -n -o TARGET "$dev" | head -1)
  if [ -n "$mnt" ]; then
    [ -d "$mnt/Windows/System32" ] && win_parts+=("$dev")
    continue
  fi
  t=$(mktemp -d)
  if sudo mount -o ro "$dev" "$t" 2>/dev/null; then
    [ -d "$t/Windows/System32" ] && win_parts+=("$dev")
    sudo umount "$t"
  fi
  rmdir "$t"
done < <(lsblk -rpno PATH,FSTYPE | awk 'NF==2')
[ "${#win_parts[@]}" -eq 0 ] && refuse "no Windows installation found."
[ "${#win_parts[@]}" -gt 1 ] && refuse "found ${#win_parts[@]} Windows installations (${win_parts[*]}); Linuxify only handles one."
WIN=${win_parts[0]}
DISK=/dev/$(lsblk -no PKNAME "$WIN" | head -1)
note "Windows is on $WIN (disk $DISK)."

targets=("$WIN")
while read -r dev ptype label; do
  ptype=${ptype,,}
  case "$ptype" in
    "$MSR"|"$WINRE") targets+=("$dev") ;;
  esac
  [ "$label" = LNXINST ] && targets+=("$dev")
done < <(lsblk -rpno PATH,PARTTYPE,LABEL "$DISK" | awk 'NF>=2')   # the disk's own line has no PARTTYPE
mapfile -t targets < <(printf '%s\n' "${targets[@]}" | sort -u)

for t in "${targets[@]}"; do
  ptype=$(lsblk -no PARTTYPE "$t" | tr 'A-Z' 'a-z')
  [ "$ptype" = "$ESP" ] && refuse "internal check: $t is an EFI partition."
  [ -n "$(findmnt -n -S "$t")" ] && refuse "$t is mounted. Unmount it first."
  [ "$t" = "$here_dev" ] && refuse "this migration folder lives on $t, which would be deleted. Copy it elsewhere first."
done
[ "$(findmnt -n -o SOURCE /)" = "$WIN" ] && refuse "internal check: / is on the Windows partition."

note "These partitions on $DISK would be DELETED:"
lsblk -o PATH,SIZE,FSTYPE,LABEL,PARTTYPENAME "${targets[@]}" | sed 's/^/     /'
note "Kept: everything else, including:"
lsblk -rpno PATH,SIZE,FSTYPE,LABEL "$DISK" | tail -n +2 | while read -r p rest; do
  printf '%s\n' "${targets[@]}" | grep -qx "$p" || printf '     %s %s\n' "$p" "$rest"
done

boot_ids=()
if command -v efibootmgr >/dev/null; then
  mapfile -t boot_ids < <(efibootmgr 2>/dev/null | sed -n 's/^Boot\([0-9A-Fa-f]\{4\}\)\*\? *Windows Boot Manager.*/\1/p')
fi
note "Windows boot menu entries to remove: ${boot_ids[*]:-none}"
esp_dev=$(lsblk -rpno PATH,PARTTYPE "$DISK" | awk -v e="$ESP" 'tolower($2)==e{print $1; exit}')
[ -z "$esp_dev" ] && esp_dev=$(findmnt -n -o SOURCE /boot/efi 2>/dev/null)
note "Windows boot files on ${esp_dev:-(no EFI partition found)}: EFI/Microsoft will be renamed, not deleted."

# ---------------------------------------------------------------------------
step "4/4 Remove Windows"
if [ "$REALLY" != 1 ]; then
  printf '\nCheck-only run: nothing was changed. To remove Windows, run:  bash remove-windows.sh --really\n'
  exit 0
fi
phrase="DELETE WINDOWS FROM $DISK"
printf '\nThis permanently deletes Windows and the partitions listed above.\nType exactly  %s  to continue: ' "$phrase"
read -r reply </dev/tty || reply=""
[ "$reply" = "$phrase" ] || refuse "confirmation didn't match."

for t in $(printf '%s\n' "${targets[@]}" | sort -r -V); do
  num=$(cat "/sys/class/block/$(basename "$t")/partition")
  note "Deleting $t (partition $num)"
  # --no-reread: Linux itself usually runs from this disk, so sfdisk would
  # otherwise refuse a "busy" disk. Safe here: none of the targets is mounted
  # (checked above). partx then drops the deleted partition from the kernel.
  if ! sudo sfdisk --no-reread --delete "$DISK" "$num"; then
    printf '\nSTOPPED: deleting %s failed. Partitions listed before it may already be deleted;\ncheck with: lsblk %s\n' "$t" "$DISK"
    exit 1
  fi
  sudo partx -d --nr "$num" "$DISK" 2>/dev/null || true
done
for id in "${boot_ids[@]}"; do sudo efibootmgr -q -b "$id" -B && note "Removed boot entry $id"; done
if [ -n "$esp_dev" ]; then
  m=$(findmnt -n -o TARGET "$esp_dev" | head -1); tm=""
  if [ -z "$m" ]; then tm=$(mktemp -d); sudo mount "$esp_dev" "$tm" && m=$tm; fi
  if [ -n "$m" ] && [ -d "$m/EFI/Microsoft" ]; then
    sudo mv "$m/EFI/Microsoft" "$m/EFI/Microsoft.removed-by-linuxify" && note "Renamed EFI/Microsoft on $esp_dev"
  fi
  [ -n "$tm" ] && { sudo umount "$tm"; rmdir "$tm"; }
fi
if command -v update-grub >/dev/null; then sudo update-grub >/dev/null 2>&1 && note "Boot menu updated."
elif command -v grub2-mkconfig >/dev/null; then sudo grub2-mkconfig -o /boot/grub2/grub.cfg >/dev/null 2>&1 && note "Boot menu updated."
fi
printf '{"removed_at":"%s","disk":"%s","partitions":"%s"}\n' "$(date -Is)" "$DISK" "${targets[*]}" > "$HERE/windows-removed.json"
printf '\nDone. Windows is removed. The freed space on %s is unallocated:\nopen the "Disks" app to make a new partition from it (e.g. for more storage).\n' "$DISK"
