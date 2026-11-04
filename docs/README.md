# Linuxify documentation

Linuxify helps you move from Windows to Linux in five steps: **scan, choose, try, migrate, install**.
Steps 1–4 never change your disks. Step 5 can, and is wrapped in several layers of protection.

| Read this | When |
|---|---|
| [install.md](install.md) | Getting Linuxify itself, and a start-to-finish walkthrough of a migration |
| [safety-model.md](safety-model.md) | **Before you run `install`.** What Linuxify will and won't do to your disk, and every gate in the way |
| [undo.md](undo.md) | Backing out: at each stage, including after Linux is installed and after Windows is removed |
| [faq.md](faq.md) | Short answers to the usual questions |
| [troubleshooting.md](troubleshooting.md) | A message you don't understand, or something that didn't work |
| [testing.md](testing.md) | Contributors: unit, property and fuzz tests, and how to run them |
| [web-quiz.md](web-quiz.md) | Plugging a website questionnaire into `choose --web` |

Other files in the repository: `PROGRESS.md` (what was verified, and what wasn't), `DECISIONS.md`
(why things are the way they are), `data/SCHEMA.md` (the app-mapping file format).

> **Honest status.** The install step has been tested only in throwaway VirtualBox VMs
> (Windows 10 22H2, UEFI), once per offered distro, and never on real hardware. Treat it as
> experimental. [safety-model.md](safety-model.md#what-has-and-has-not-been-tested) lists exactly
> what that covers.
