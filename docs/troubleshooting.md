# Troubleshooting

Find your symptom, or the message text. Anything starting with *Nothing was changed* really means that.

First, two things that help with almost everything:

```powershell
linuxify scan --open          # the report's blockers list says what is wrong and how to fix it
linuxify install <distro>     # dry run: prints the plan and every reason it would refuse
```

## Starting up

**Windows SmartScreen warns about `linuxify.exe`.** It isn't code-signed. Check the hash against
`SHA256SUMS.txt` from the release (`Get-FileHash .\linuxify.exe`), then use *More info → Run anyway* only if it matches.

**The window is blank, tiny or won't open.** Run `linuxify scan` in a terminal: the commands don't need the window.
The window needs a GPU driver that supports OpenGL or wgpu; remote desktop sessions and VMs without 3D
acceleration have worked in testing, but slowly.

**`could not read the latest scan ... Re-run linuxify scan`.** `reports\latest\report.json` is damaged or from a
different version. Run `linuxify scan` again.

**`LOCALAPPDATA is not set`.** Linuxify keeps its files under `%LOCALAPPDATA%\Linuxify`. Run it from a normal
Windows user session.

**`hardware probe output was not valid JSON`.** The PowerShell hardware check printed something unexpected
(often a profile script that writes to the console). Run as a different user or with a clean profile. The
scan continues and the report shows "Hardware check failed" as a blocker, so `install` stays refused.

## `scan` and `choose`

**`winget` is slow or missing.** Use `linuxify scan --no-winget`; the registry list is used alone. Store apps
such as Fortnite or Minecraft will be missing from the list.

**A program is "unknown".** Unknown means the app map has no entry. It is not a promise that it works.
Add an entry to `data\app_map.json` (format in `data/SCHEMA.md`) and run again, or open a pull request.
A file you edit must pass validation: lowercase patterns, no duplicate ids, no empty patterns, and an
"alternative" entry must list alternatives. The error lists every problem.

**`app_map.json is not valid` / `distros.json has problems`.** You passed `--data-dir` (or put a `data\` folder
next to the exe) with a bad file. Fix the lines named, or delete that file to fall back to the built-in copy.

**`choose --answers file.json`: `missing answer for ...` / `must be one of ...`.** Every question needs an answer using the
exact option keys. `linuxify quiz-spec` prints them. Files saved from PowerShell 5 with a byte-order mark are accepted.

**All my top picks have lower scores than I expected.** The ratings are subjective. Change one answer and
look at how the list moves, then `try` your top two.

**A distro I wanted is missing from the results.** It was left out because the PC has less RAM than the
distro needs. The reason is listed next to it.

## `try` (virtual machine)

**`no VM program found`.** Install VirtualBox (the only backend tested), or pass `--vm hyperv` / `--vm qemu`.
`linuxify try --list` shows what was found. Hyper-V needs administrator rights to create a VM.

**The download stopped or the PC went to sleep.** Run the same command again: downloads resume from the last
checkpoint, and a crash can cost at most the last 64 MB.

**`... returned HTTP 404 (the release may have moved; data/isos.json needs updating)` or `no ISO matching ...`.** The
distro released a new version and the pinned folder in `data\isos.json` is gone. Edit that file (or wait for an
update). Debian, Arch, Tumbleweed and Bazzite follow "latest" automatically; Fedora, Mint, the Ubuntu family,
NixOS and AlmaLinux are pinned.

**The checksum doesn't match.** Delete the partial file under `%LOCALAPPDATA%\Linuxify\isos\` and run again. If it
keeps failing, **do not use that ISO**, and try another network. A bad hash can mean a corrupt download or a
tampered one.

**The VM boots to a black screen or won't start under UEFI.** Check virtualisation is on in firmware, and that
Hyper-V isn't holding it when you use VirtualBox. Give the VM more RAM; the default is sized from your PC.

**`try --remove` says the VM is locked.** VirtualBox keeps a VM locked for a few seconds after power-off.
Linuxify retries for up to 20 s; if it still fails, close the VirtualBox window and retry. It only ever deletes
the VM it created, named `linuxify-<distro>`.

## `migrate`

**`the target is inside ... which is being copied`**. Pick a target outside your Documents/Desktop/etc.

**`refusing to write into a system folder`.** Choose a different target, such as a second drive or USB disk.

**Some files were skipped or flagged.** `flagged.txt` in the staging folder lists each with the reason (system
or shortcut files, junk, locked files, and so on). Locked files (often a running browser's database) are retried when
you run the same command again; close the app first.

**It was interrupted.** Run the same command: it resumes. A file is reused only if the source's size and time
match and the copy is still there with the right size; a partial file is replaced.

**`manifest.json is damaged` / `no manifest.json`.** Re-run `migrate` against that target. `--verify` re-checks
without copying.

**`--verify` reports mismatched files.** The staged copy changed after being made. Run `migrate` again; it
re-copies those files.

## `install`

Refusals print every reason together. Common ones:

**`Linuxify must run from an administrator terminal`.** Open *Terminal (Admin)* / *PowerShell (Admin)* and repeat with `--execute`.

**`readiness blocker: BitLocker is on for C:`** (or *Could not tell whether C: is encrypted*). Decrypt the drive and
wait for decryption to finish; suspending is not enough. See the [FAQ](faq.md#what-about-bitlocker).

**`readiness blocker: Disk controller is in Intel RST/VMD (RAID) mode`.** Most Linux installers can't see the disk in
this mode. Changing it in firmware can stop Windows from booting unless prepared correctly. Look for your vendor's
guidance (switch to AHCI via Safe Mode) before touching it, and back up first.

**`Not enough room to shrink C:`.** You need about 50 GB that Windows can shrink. Free space, empty the Recycle
Bin, turn off hibernation and System Restore temporarily, and re-scan as administrator for the true figure.
Immovable files can limit shrinking even with plenty of free space.

**`... installer can't start while Secure Boot is on`.** Turn Secure Boot off in firmware, or pick a distro that
supports it. Linuxify refuses early so that C: is never shrunk for an installer that can't start.

**`this PC is running on battery`.** Plug in the charger.

**`the installer ISO hasn't been downloaded and verified`.** Run `linuxify try <distro> --download-only`.

**`no verified copy of your files` / `your staged copy ... failed verification` / `is on the same disk as Windows`.**
Run `linuxify migrate --target <other drive>` (a different physical disk), and `--verify` it.

**`a previous Linuxify install setup is still in place`.** Run `linuxify install --undo` (dry run first, then `--execute`)
or finish that install.

**The typed phrase is rejected.** It must match exactly, including capitals and spacing, and it contains the *current*
disk number and shrink size. If anything changed since your dry run, the phrase changes: read the new plan, then type the new phrase.

**`could not update grub.cfg: Access is denied`** (older versions). Fixed; update Linuxify, run `--undo`, and try again.

**The window says the install is blocked and shows a red list.** Same checks as the terminal. Each line says what
to do, and some have a button (for example, to download and verify the ISO).

**`no-install.lock`.** A file at `%LOCALAPPDATA%\Linuxify\no-install.lock` blocks all disk changes. If you created it,
delete it to proceed. If the PC's owner did, ask them.

**The distro is refused.** NixOS can't start from Linuxify's installer partition (by design); AlmaLinux 10 is refused on
CPUs older than x86-64-v3; distros without an `isos.json` entry cannot be installed automatically.

## After restarting into the installer

**The PC boots Windows, not the installer.** The boot entry is one-time and may have been consumed. Open the firmware
boot menu at power-on and choose the Linuxify entry or `LNXINST`. If Secure Boot is on, the distro must support it.

**The installer wants to erase the whole disk, or offers to shrink Windows again.** Cancel and read the plan's
installer hint, which names the right option for each installer. Ubuntu-family: *Install … alongside Windows Boot Manager*.
Calamares (Debian, Kubuntu, Lubuntu): *Replace a partition*, then click the grey "Free Space" block. Fedora (Anaconda):
*Share disk with other operating systems*, with "Reclaim additional space" unticked. openSUSE: pick GRUB2 for EFI as the boot
loader. Omarchy: *Free space install*. Pick the Windows disk (the plan prints its size) and not any other drive.

**Fedora doesn't show a boot menu / I can't reach Windows.** Fedora hides it. Press Esc during start-up, type
`normal` at a `grub>` prompt if one appears, and pick *Windows Boot Manager*.

**Omarchy doesn't list Windows.** Run `sudo limine-scan` once Omarchy is running and choose *Windows Boot Manager*.

**The installer says it cannot unmount `/cdrom`.** This was fixed by loading the live system into RAM (`toram`), which needs
enough RAM; the plan refuses distros the PC doesn't have enough for. Reboot and retry on a PC with more memory.

## Undo and boot problems

See [undo.md](undo.md). In short: `linuxify install --undo`, and `linuxify install --fix-boot-entries` if an installed Linux
stopped starting after an undo. If nothing boots, use the firmware boot menu to pick *Windows Boot Manager*.

## On Linux: `restore.sh`

**`SHA256SUMS` check fails.** Some file is missing or changed since Windows. Re-run `linuxify migrate --target ... --verify`
on Windows, or recopy the folder (a USB stick may have failed). `restore.sh` stops rather than restoring damaged data.

**Some packages "failed".** It lists them at the end and carries on. A Flatpak that needs more space than a live session's RAM
overlay can fail there; run it again on the installed system. On an rpm-ostree (immutable) system native packages are skipped on purpose. Only the apt
path has been verified end to end.

**Files landed next to my translated folders** (for example `Documents` beside `Dokumente`). Linuxify uses English
folder names. Move them by hand; `restore.sh` never overwrites.

**My Wi-Fi doesn't work on first boot.** Broadcom and some Realtek USB adapters need extra drivers. Have a cable or phone
tethering ready. The scan's Wi-Fi section names your chipset and says what to expect.

**An app that needs a kernel module (such as VirtualBox) installed but won't load.** With Secure Boot on, the module has to be
signed and enrolled through the distro's MOK flow. That is the distro's step, not Linuxify's.

## On Linux: `remove-windows.sh`

It refuses a lot, deliberately, and each refusal says "Nothing was changed". Typical ones: running as root (run as yourself), on
a live USB, a restored file missing or edited (restore it, or `--accept-changed` if you edited on purpose), more than
one Windows found, or a Windows partition is mounted. After a successful `--really`, restart once.

## Still stuck

Open an issue at <https://github.com/tiiow-fedora/linuxify/issues> with: the command, the exact message, your Windows version,
and the dry-run output of `linuxify install <distro>`.
