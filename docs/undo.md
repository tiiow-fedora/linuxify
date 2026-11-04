# Undo: backing out at every stage

What you can reverse depends on how far you got. The first rows cost nothing; the last row cannot be undone.

| You ran | To back out |
|---|---|
| `scan`, `choose` | Delete `%LOCALAPPDATA%\Linuxify\` if you want it gone. Nothing else changed |
| `try` | `linuxify try <distro> --remove` deletes Linuxify's VM for that distro. ISOs stay in `%LOCALAPPDATA%\Linuxify\isos\`; delete them by hand |
| `migrate` | Delete the `linuxify-migration` folder on the target. Your originals were never touched |
| `install --execute`, before installing Linux | `linuxify install --undo --execute` ([below](#undoing-install)) |
| `install --execute`, then Linux installed | Same command; it also repairs boot entries ([below](#after-linux-is-installed)) |
| `restore.sh` | It never overwrites, so it only added files. Delete what it copied if you want it gone |
| `remove-windows.sh --really` | **Not undoable.** See [below](#removing-windows-is-the-one-step-with-no-undo) |

## Undoing `install`

`install` shrinks C:, creates a small installer partition `LNXINST` (and sometimes `LNXDATA`), and adds a
one-time boot entry. `--undo` reverses exactly those.

1. See what it would do. Without `--execute` it only shows the plan:

   ```powershell
   linuxify install --undo
   ```

2. Run it for real from an **administrator** terminal:

   ```powershell
   linuxify install --undo --execute
   ```

3. Type the phrase it prints, `UNDO LINUXIFY ON DISK <n>`.

It deletes the boot entry, deletes the installer partition(s), and extends C: back over the space. The
partitions are located by their recorded offset and size, not just by name. When it finishes it renames
`install-state.json` to `install-state.undone.json`.

The window's Install page offers the same Undo, with its own dry run and typed phrase.

Because the boot entry is **one-time**, nothing needs undoing if you simply restart without choosing to
install: after one boot the PC starts as before. The partitions stay until you undo.

### If the install was interrupted

`install-state.json` is written before the first change and records the phase reached. After a crash, power
cut or Ctrl+C, run `linuxify install --undo` (dry run first). If the file is missing, Linuxify says "there is
no Linuxify install setup to undo" and does nothing; use Windows Disk Management to look at the layout.

A half-finished install has been undone in testing (an earlier version failed with "Access is denied" while
updating `grub.cfg`; `--undo` restored the disk, and the next run worked).

### After Linux is installed

Undo removes `LNXINST`/`LNXDATA`, which sit *before* the Linux partitions. Deleting them renumbers the
partitions after them, and UEFI boot entries remember a partition number. Linuxify re-points the firmware
entries for Linux automatically as part of `--undo`. If a Linux you installed afterwards no longer starts
(the PC falls through to Windows), run:

```powershell
linuxify install --fix-boot-entries          # dry run: shows what would be rewritten
linuxify install --fix-boot-entries --execute
```

This matters most for installs made with Linuxify 0.1.0, which did not do it.

The space the distro's installer used stays Linux's. Undo does not shrink Linux or touch its partitions. To
remove Linux itself, use Windows Disk Management (delete the Linux partitions, extend C:), and remove the
Linux boot entry. Be careful to pick the right disk.

## Making a PC refuse changes

To stop a PC from ever being changed, create `%LOCALAPPDATA%\Linuxify\no-install.lock` (see the
[safety model](safety-model.md#the-lock-file)). Deleting that file re-enables `install`.

## Removing Windows is the one step with no undo

`remove-windows.sh --really` permanently deletes the Windows partition, the Microsoft reserved partition, the
Windows recovery partition and `LNXINST`. Plan for it as if deleting the disk's past:

- Run the check-only form first and read the list: `bash remove-windows.sh`.
- Keep a real backup somewhere that isn't this PC's disks. Linuxify's staged copy only proves your files
  restored correctly into Linux.
- Check your software licences, any apps that only exist for Windows, and that every file you care about is in your Linux home.
- The script also refuses on its own if any restored file is missing or has been edited since Windows. If you
  changed files on purpose, `--accept-changed` acknowledges that.

Afterwards Linux boots as before, `EFI/Microsoft` is renamed `EFI/Microsoft.removed-by-linuxify`, and the other
disk(s) are untouched. Getting Windows back means reinstalling it from Microsoft's media with your licence.

## Recovering a PC that won't boot

If the PC doesn't start after a change: open the firmware boot menu (usually Esc, F2, F10, F12 or Del at
power-on) and pick **Windows Boot Manager**. If that works, run `linuxify install --undo` (dry run, then
`--execute`) from Windows. If nothing boots, use Microsoft's recovery or installation media. Only attempt
that if you have a backup.
