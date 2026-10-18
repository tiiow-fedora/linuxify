//! Writes report.json and a self-contained report.html (inline CSS, no
//! scripts, no external requests). Every piece of scanned text is escaped.

use crate::mapping::{Group, MappedProgram, Status};
use crate::readiness::{gb, Severity};
use crate::scan::hardware::{BitLocker, Firmware, SecureBoot};
use crate::scan::ScanReport;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

/// Only http(s) links become clickable; anything else is shown as text.
fn link(url: &str, text: &str) -> String {
    if url.starts_with("https://") || url.starts_with("http://") {
        format!("<a href=\"{}\" rel=\"noopener noreferrer\">{}</a>", esc(url), esc(text))
    } else {
        esc(text)
    }
}

fn status_class(s: Status) -> &'static str {
    match s {
        Status::Native => "native",
        Status::Alternative => "alt",
        Status::WineOk => "wine",
        Status::Blocked => "blocked",
        Status::Unknown => "unknown",
    }
}

fn linux_cell(m: &MappedProgram) -> String {
    let mut parts = Vec::new();
    if let Some(e) = &m.entry {
        if let Some(l) = &e.linux {
            if let Some(f) = &l.flatpak {
                parts.push(format!("Flatpak <code>{}</code>", esc(f)));
            }
            if let Some(a) = &l.apt {
                parts.push(format!("apt <code>{}</code>", esc(a)));
            }
            if let Some(d) = &l.dnf {
                parts.push(format!("dnf <code>{}</code>", esc(d)));
            }
            if let Some(s) = &l.snap {
                parts.push(format!("snap <code>{}</code>", esc(s)));
            }
            if let Some(u) = &l.url {
                parts.push(link(u, "download page"));
            }
        }
        if !e.alternatives.is_empty() {
            let alts: Vec<String> = e
                .alternatives
                .iter()
                .map(|a| {
                    let mut s = format!("<b>{}</b>", esc(&a.name));
                    if let Some(n) = &a.note {
                        let _ = write!(s, " – {}", esc(n));
                    }
                    s
                })
                .collect();
            parts.push(format!("Try: {}", alts.join("; ")));
        }
    }
    parts.join("<br>")
}

fn notes_cell(m: &MappedProgram) -> String {
    let mut parts = Vec::new();
    if let Some(e) = &m.entry {
        if let Some(n) = &e.notes {
            parts.push(esc(n));
        }
        if let Some(l) = &e.link {
            parts.push(link(l, "more info"));
        }
    }
    if let Some(p) = &m.protondb {
        parts.push(link(p, "ProtonDB reports"));
    }
    if m.group == Group::Unmapped {
        if m.protondb.is_some() {
            parts.push("Steam game: check its ProtonDB rating.".into());
        } else {
            parts.push("Not in Linuxify's list yet.".into());
        }
    }
    parts.join(" ")
}

fn program_table(rows: &[&MappedProgram]) -> String {
    let mut h = String::from("<table><thead><tr><th>Program</th><th>On Linux</th><th>Notes</th></tr></thead><tbody>");
    for m in rows {
        let p = &m.program;
        let mut sub = Vec::new();
        if let Some(v) = &p.version {
            sub.push(esc(v));
        }
        if let Some(pb) = &p.publisher {
            sub.push(esc(pb));
        }
        let _ = write!(
            h,
            "<tr><td><b>{}</b><div class=sub>{}</div></td><td>{}</td><td>{}</td></tr>",
            esc(&p.name),
            sub.join(" · "),
            linux_cell(m),
            notes_cell(m)
        );
    }
    h.push_str("</tbody></table>");
    h
}

fn kv(rows: &[(&str, String)]) -> String {
    let mut h = String::from("<table class=kv><tbody>");
    for (k, v) in rows {
        let _ = write!(h, "<tr><th>{}</th><td>{}</td></tr>", esc(k), v);
    }
    h.push_str("</tbody></table>");
    h
}

const CSS: &str = r#"
:root{--bg:#f7f7f8;--card:#fff;--fg:#1c1c1f;--muted:#5d5d66;--line:#e2e2e6;--blocker:#b42318;--blocker-bg:#fef0ee;--warn:#9a5b00;--warn-bg:#fff6e5;--info:#1d5fa8;--info-bg:#eef4fc;--ok:#127a45;--ok-bg:#e9f7ef;--wine:#6941c6;--wine-bg:#f3effc;}
@media (prefers-color-scheme:dark){:root{--bg:#141417;--card:#1d1d21;--fg:#ececf0;--muted:#a0a0aa;--line:#33333a;--blocker:#ff8a80;--blocker-bg:#3a1d1b;--warn:#ffc266;--warn-bg:#352a14;--info:#8ab8ff;--info-bg:#18263a;--ok:#6fdc9c;--ok-bg:#15301f;--wine:#c3a8ff;--wine-bg:#2a2140;}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:1100px;margin:0 auto;padding:24px 16px 64px}h1{font-size:28px;margin:0 0 4px}h2{font-size:20px;margin:36px 0 12px;padding-bottom:6px;border-bottom:1px solid var(--line)}h3{font-size:16px;margin:20px 0 8px}
.muted,.sub{color:var(--muted)}.sub{font-size:13px}
.verdict{margin:20px 0;padding:16px 18px;border-radius:10px;font-size:17px}.verdict.bad{background:var(--blocker-bg);color:var(--blocker)}.verdict.good{background:var(--ok-bg);color:var(--ok)}
.finding{background:var(--card);border:1px solid var(--line);border-left:5px solid;border-radius:8px;padding:12px 16px;margin:10px 0}
.finding.blocker{border-left-color:var(--blocker)}.finding.warning{border-left-color:var(--warn)}.finding.info{border-left-color:var(--info)}
.finding .tag{font-size:12px;font-weight:700;text-transform:uppercase;letter-spacing:.04em}.finding.blocker .tag{color:var(--blocker)}.finding.warning .tag{color:var(--warn)}.finding.info .tag{color:var(--info)}
.finding .fix{margin-top:6px;padding:8px 10px;background:var(--bg);border-radius:6px}
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:10px}.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px}.card .n{font-size:26px;font-weight:700}
.card.blocked .n{color:var(--blocker)}.card.alt .n{color:var(--warn)}.card.wine .n{color:var(--wine)}.card.native .n{color:var(--ok)}.card.unknown .n{color:var(--muted)}
table{width:100%;border-collapse:collapse;background:var(--card);border:1px solid var(--line);border-radius:8px;overflow:hidden;font-size:14px}
th,td{text-align:left;vertical-align:top;padding:8px 10px;border-bottom:1px solid var(--line)}thead th{background:var(--bg);font-size:13px;color:var(--muted)}
table.kv th{width:220px;color:var(--muted);font-weight:600}
.wrap{overflow-x:auto}code{font-size:12.5px;background:var(--bg);padding:1px 4px;border-radius:4px;word-break:break-all}
details{margin:10px 0}summary{cursor:pointer;font-weight:600}a{color:var(--info)}
.pill{display:inline-block;padding:1px 8px;border-radius:999px;font-size:12px;font-weight:600}.pill.blocked{background:var(--blocker-bg);color:var(--blocker)}.pill.alt{background:var(--warn-bg);color:var(--warn)}.pill.wine{background:var(--wine-bg);color:var(--wine)}.pill.native{background:var(--ok-bg);color:var(--ok)}.pill.unknown{background:var(--bg);color:var(--muted)}
footer{margin-top:48px;font-size:13px;color:var(--muted)}
"#;

pub fn render_html(r: &ScanReport) -> String {
    let mut h = String::new();
    let _ = write!(
        h,
        "<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>Linux Readiness Report</title><style>{CSS}</style></head><body><main>"
    );
    let _ = write!(
        h,
        "<h1>Linux Readiness Report</h1><div class=muted>{}{} · Linuxify {}</div>",
        r.computer_name.as_deref().map(|n| format!("{} · ", esc(n))).unwrap_or_default(),
        esc(&r.generated_at),
        esc(&r.linuxify_version)
    );

    // ---- Verdict + findings ----
    let n_block = r.findings.iter().filter(|f| f.severity == Severity::Blocker).count();
    let n_warn = r.findings.iter().filter(|f| f.severity == Severity::Warning).count();
    if n_block > 0 {
        let _ = write!(h, "<div class='verdict bad'><b>{n_block} thing(s) must be fixed before installing Linux.</b> Trying Linux in a virtual machine is still safe right now.</div>");
    } else {
        let _ = write!(h, "<div class='verdict good'><b>No blockers found.</b> {n_warn} thing(s) worth knowing are listed below.</div>");
    }
    h.push_str("<h2>Blockers and things to know</h2>");
    for f in &r.findings {
        let (cls, tag) = match f.severity {
            Severity::Blocker => ("blocker", "Blocker"),
            Severity::Warning => ("warning", "Heads-up"),
            Severity::Info => ("info", "Info"),
        };
        let _ = write!(h, "<div class='finding {cls}'><div class=tag>{tag}</div><b>{}</b><div>{}</div>", esc(&f.title), esc(&f.detail));
        if let Some(fix) = &f.how_to_fix {
            let _ = write!(h, "<div class=fix><b>What to do:</b> {}</div>", esc(fix));
        }
        h.push_str("</div>");
    }

    // ---- Programs ----
    let shown: Vec<&MappedProgram> = r.programs.iter().filter(|p| matches!(p.group, Group::Mapped | Group::Unmapped)).collect();
    h.push_str("<h2>Your programs</h2><div class=cards>");
    for s in [Status::Blocked, Status::Alternative, Status::WineOk, Status::Native, Status::Unknown] {
        let n = shown.iter().filter(|p| p.status == s).count();
        let _ = write!(h, "<div class='card {}'><div class=n>{n}</div>{}</div>", status_class(s), s.label());
    }
    h.push_str("</div>");
    for s in [Status::Blocked, Status::Alternative, Status::WineOk, Status::Native, Status::Unknown] {
        let rows: Vec<&MappedProgram> = shown.iter().copied().filter(|p| p.status == s).collect();
        if rows.is_empty() {
            continue;
        }
        let _ = write!(h, "<h3><span class='pill {}'>{}</span> {} program(s)</h3><div class=wrap>{}</div>", status_class(s), s.label(), rows.len(), program_table(&rows));
    }
    let ignored: Vec<&MappedProgram> = r.programs.iter().filter(|p| p.group == Group::Ignored).collect();
    if !ignored.is_empty() {
        let _ = write!(h, "<details><summary>{} Windows runtimes and drivers (not needed on Linux)</summary><ul>", ignored.len());
        for m in ignored {
            let _ = write!(h, "<li>{} <span class=sub>— {}</span></li>", esc(&m.program.name), esc(m.ignore_reason.as_deref().unwrap_or("")));
        }
        h.push_str("</ul></details>");
    }
    let builtin: Vec<&MappedProgram> = r.programs.iter().filter(|p| p.group == Group::WindowsBuiltin).collect();
    if !builtin.is_empty() {
        let _ = write!(h, "<details><summary>{} built-in Windows / Microsoft Store components</summary><ul>", builtin.len());
        for m in builtin {
            let _ = write!(h, "<li>{}</li>", esc(&m.program.name));
        }
        h.push_str("</ul></details>");
    }

    // ---- Hardware ----
    let hw = &r.hardware;
    h.push_str("<h2>Your hardware</h2>");
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(c) = &hw.computer {
        rows.push(("Computer", esc(&format!("{} {}", c.manufacturer.as_deref().unwrap_or(""), c.model.as_deref().unwrap_or("")))));
    }
    if let Some(o) = &hw.os {
        rows.push(("Windows", esc(&format!("{} (build {}, {})", o.caption.as_deref().unwrap_or("?"), o.build.as_deref().unwrap_or("?"), o.arch.as_deref().unwrap_or("?")))));
    }
    for c in &hw.cpus {
        rows.push(("CPU", esc(&format!("{} — {} cores / {} threads", c.name.as_deref().unwrap_or("?"), c.cores.unwrap_or(0), c.threads.unwrap_or(0)))));
    }
    if let Some(ram) = hw.ram_bytes {
        rows.push(("Memory", esc(&gb(ram))));
    }
    for g in &hw.gpus {
        let v = if g.is_virtual { " (virtual adapter, ignored)".to_string() } else { format!(" — {:?}", g.vendor) };
        rows.push(("Graphics", esc(&format!("{}{v}", g.name))));
    }
    rows.push(("Boot mode", esc(match hw.firmware {
        Firmware::Uefi => "UEFI",
        Firmware::Bios => "Legacy BIOS",
        Firmware::Unknown => "Unknown",
    })));
    rows.push(("Secure Boot", esc(match hw.secure_boot {
        SecureBoot::On => "On",
        SecureBoot::Off => "Off",
        SecureBoot::NotSupported => "Not available (BIOS mode)",
        SecureBoot::Unknown => "Unknown",
    })));
    for b in &hw.bitlocker {
        let s = match b.state {
            BitLocker::Active => "Encrypted",
            BitLocker::Off => "Not encrypted",
            BitLocker::Unknown => "Unknown",
        };
        rows.push(("BitLocker", esc(&format!("{}: {s} ({})", b.mount, b.detail))));
    }
    rows.push(("Disk controller mode", esc(if hw.rst_vmd.detected { "Intel RST/VMD (RAID) — Linux may not see the disk" } else { "Standard (AHCI/NVMe) — fine for Linux" })));
    if let Some(c) = &hw.c_drive {
        let shrink = hw.c_shrinkable.map(|s| format!(", can shrink by {}", gb(s))).unwrap_or_else(|| ", exact shrink size needs admin".into());
        rows.push(("C: drive", esc(&format!("{} free of {}{shrink}", gb(c.free), gb(c.size)))));
    }
    rows.push(("Fast Startup", esc(match hw.fast_startup {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Unknown",
    })));
    if hw.wifi.is_empty() {
        rows.push(("Wi-Fi", "No Wi-Fi adapter found (wired only)".into()));
    }
    for w in &hw.wifi {
        rows.push(("Wi-Fi", esc(&format!("{} ({}) — {}", w.description, w.chipset_vendor, w.linux_note))));
    }
    h.push_str(&kv(&rows));

    h.push_str("<h3>Disks and partitions</h3><div class=wrap><table><thead><tr><th>Disk</th><th>Partition</th><th>Letter</th><th>Size</th><th>Free</th><th>Type</th></tr></thead><tbody>");
    for d in &hw.disks {
        let flags = [(d.is_boot, "Windows disk"), (d.is_system, "boot partition here")]
            .iter()
            .filter(|(b, _)| *b)
            .map(|(_, s)| *s)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(
            h,
            "<tr><td rowspan={}><b>Disk {}</b><div class=sub>{}<br>{} · {} · {}<br>{}{}</div></td>",
            d.partitions.len().max(1),
            d.number,
            esc(d.model.as_deref().unwrap_or("?")),
            esc(&gb(d.size)),
            esc(d.bus_type.as_deref().unwrap_or("?")),
            esc(d.partition_style.as_deref().unwrap_or("?")),
            esc(&flags),
            if d.unallocated > 1_000_000_000 { format!("<br>{} unallocated", esc(&gb(d.unallocated))) } else { String::new() }
        );
        if d.partitions.is_empty() {
            h.push_str("<td colspan=5 class=muted>no partitions</td></tr>");
        }
        for (i, p) in d.partitions.iter().enumerate() {
            if i > 0 {
                h.push_str("<tr>");
            }
            let kind = match p.gpt_type.as_deref().map(str::to_lowercase).as_deref() {
                Some("{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}") => "EFI boot partition".to_string(),
                Some("{e3c9e316-0b5c-4db8-817d-f92df00215ae}") => "Microsoft reserved".to_string(),
                Some("{de94bba4-06d1-4d40-a16a-bfd50179d6ac}") => "Windows recovery".to_string(),
                _ => p.file_system.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into()),
            };
            let _ = write!(
                h,
                "<td>{}{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                p.number,
                p.label.as_deref().filter(|l| !l.is_empty()).map(|l| format!(" <span class=sub>{}</span>", esc(l))).unwrap_or_default(),
                p.drive_letter.map(|c| format!("{c}:")).unwrap_or_default(),
                esc(&gb(p.size)),
                p.free.map(|f| esc(&gb(f))).unwrap_or_default(),
                esc(&kind)
            );
        }
    }
    h.push_str("</tbody></table></div>");

    // ---- Data ----
    let ud = &r.user_data;
    h.push_str("<h2>Your files and settings</h2><h3>Folders</h3><div class=wrap><table><thead><tr><th>Folder</th><th>Size</th><th>Files</th><th>Notes</th></tr></thead><tbody>");
    for f in &ud.folders {
        let mut notes = Vec::new();
        if !f.exists {
            notes.push("not found".to_string());
        }
        if f.stats.cloud_only_files > 0 {
            notes.push(format!("{} online-only files ({})", f.stats.cloud_only_files, gb(f.stats.cloud_only_bytes)));
        }
        if f.stats.unreadable > 0 {
            notes.push(format!("{} items couldn't be read", f.stats.unreadable));
        }
        if f.stats.links_skipped > 0 {
            notes.push(format!("{} shortcuts/links skipped", f.stats.links_skipped));
        }
        let _ = write!(
            h,
            "<tr><td><b>{}</b><div class=sub>{}</div></td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(&f.label),
            esc(&f.path),
            esc(&gb(f.stats.bytes)),
            f.stats.files,
            esc(&notes.join("; "))
        );
    }
    let total: u64 = ud.folders.iter().map(|f| f.stats.bytes).sum();
    let _ = write!(h, "<tr><td><b>Total</b></td><td><b>{}</b></td><td></td><td></td></tr></tbody></table></div>", esc(&gb(total)));

    if !ud.browsers.is_empty() || !ud.mail.is_empty() {
        h.push_str("<h3>Browser and email profiles</h3><div class=wrap><table><thead><tr><th>App</th><th>Profile</th><th>Size</th></tr></thead><tbody>");
        for b in ud.browsers.iter().chain(ud.mail.iter()) {
            let _ = write!(h, "<tr><td>{}</td><td>{}<div class=sub>{}</div></td><td>{}</td></tr>", esc(&b.app), esc(&b.profile_name), esc(&b.path), esc(&gb(b.bytes)));
        }
        h.push_str("</tbody></table></div><p class=muted>Bookmarks, saved passwords and extensions in these profiles can be carried over in the data-migration step. Signing in to browser sync (Firefox Account, Google) is the simplest backup.</p>");
    }

    if !ud.keepass_files.is_empty() || !ud.outlook_pst_files.is_empty() {
        h.push_str("<h3>Important files found</h3><ul>");
        for k in &ud.keepass_files {
            let _ = write!(h, "<li>KeePass database: <code>{}</code> — opens as-is in KeePassXC on Linux.</li>", esc(k));
        }
        for p in &ud.outlook_pst_files {
            let _ = write!(h, "<li>Outlook data file: <code>{}</code> — can be imported into Thunderbird.</li>", esc(p));
        }
        h.push_str("</ul>");
    }

    for lib in &ud.steam {
        let _ = write!(h, "<details><summary>Steam library {} — {} games, {}</summary><ul>", esc(&lib.path), lib.games.len(), esc(&gb(lib.total_bytes)));
        for g in &lib.games {
            let _ = write!(h, "<li>{} <span class=sub>{}</span> · {}</li>", esc(&g.name), esc(&gb(g.size_on_disk)), link(&format!("https://www.protondb.com/app/{}", g.appid), "ProtonDB"));
        }
        h.push_str("</ul><p class=muted>Games are best re-downloaded on Linux: Steam on Linux doesn't reliably run games from Windows (NTFS) drives.</p></details>");
    }

    if !ud.app_data.is_empty() {
        h.push_str("<h3>App data worth knowing about</h3><div class=wrap><table><thead><tr><th>What</th><th>Size</th><th>On Linux</th></tr></thead><tbody>");
        for a in &ud.app_data {
            let _ = write!(h, "<tr><td><b>{}</b><div class=sub>{}</div></td><td>{}</td><td>{}</td></tr>", esc(&a.name), esc(&a.path), esc(&gb(a.bytes)), esc(&a.note));
        }
        h.push_str("</tbody></table></div>");
    }

    let _ = write!(
        h,
        "<footer>Program mappings come from {} and are a guide, not a guarantee: anything Linuxify isn't sure about is marked Unknown rather than guessed. Programs were read from the Windows registry{}. This report was produced read-only; nothing on your PC was changed.</footer></main></body></html>",
        esc(&r.app_map_source),
        if r.winget_used { " and winget" } else { " (winget was not used)" }
    );
    h
}

/// The distro recommendation page (`linuxify choose`).
pub fn render_choice_html(rec: &crate::chooser::Recommendation, generated_at: &str, scan_used: bool) -> String {
    let mut h = String::new();
    let _ = write!(
        h,
        "<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>Your Linux Match</title><style>{CSS}\n.rank{{font-size:13px;font-weight:700;color:var(--muted);text-transform:uppercase;letter-spacing:.04em}}.score{{float:right;font-size:22px;font-weight:700;color:var(--ok)}}ul{{margin:6px 0 0;padding-left:20px}}</style></head><body><main>"
    );
    let _ = write!(h, "<h1>Your Linux match</h1><div class=muted>{} · {}</div>", esc(generated_at), if scan_used { "based on your answers and your PC scan" } else { "based on your answers only (no scan found)" });
    if !rec.scan_notes.is_empty() {
        h.push_str("<h2>What your PC told us</h2><ul>");
        for n in &rec.scan_notes {
            let _ = write!(h, "<li>{}</li>", esc(n));
        }
        h.push_str("</ul>");
    }
    h.push_str("<h2>Top picks</h2>");
    for (i, r) in rec.ranking.iter().take(3).enumerate() {
        let _ = write!(
            h,
            "<div class='finding info'><div class=score>{:.0}%</div><div class=rank>#{}</div><b style='font-size:18px'>{}</b> <span class=sub>{} desktop · {}</span>",
            r.score,
            i + 1,
            esc(&r.name),
            esc(&r.desktop),
            link(&r.homepage, "website")
        );
        if !r.reasons.is_empty() {
            h.push_str("<div><b>Why it fits you</b><ul>");
            for x in &r.reasons {
                let _ = write!(h, "<li>{}</li>", esc(x));
            }
            h.push_str("</ul></div>");
        }
        if !r.cautions.is_empty() {
            h.push_str("<div class=fix><b>Good to know</b><ul>");
            for x in &r.cautions {
                let _ = write!(h, "<li>{}</li>", esc(x));
            }
            h.push_str("</ul></div>");
        }
        h.push_str("</div>");
    }
    h.push_str("<h2>Full ranking</h2><table><thead><tr><th>Distro</th><th>Desktop</th><th>Match</th></tr></thead><tbody>");
    for r in &rec.ranking {
        let _ = write!(h, "<tr><td>{}</td><td>{}</td><td>{:.0}%</td></tr>", esc(&r.name), esc(&r.desktop), r.score);
    }
    for e in &rec.excluded {
        let _ = write!(h, "<tr><td>{}</td><td colspan=2 class=muted>Not suitable: {}</td></tr>", esc(&e.name), esc(&e.why));
    }
    h.push_str("</tbody></table>");
    h.push_str("<footer>The match score compares what you said matters with how each distro is rated in Linuxify's data/distros.json. Treat it as a starting point: the next step (<code>linuxify try</code>) lets you test-drive a distro safely before installing anything. Want a second opinion? The quiz at <a href=\"https://distrochooser.de/\" rel=\"noopener noreferrer\">distrochooser.de</a> is independent of Linuxify.</footer></main></body></html>");
    h
}

/// Writes report.json + report.html into a timestamped folder and a `latest`
/// copy. Returns the timestamped folder.
pub fn write(r: &ScanReport, reports_root: &Path) -> Result<PathBuf, String> {
    let stamp = chrono::Local::now().format("scan-%Y%m%d-%H%M%S").to_string();
    let dir = reports_root.join(stamp);
    let latest = reports_root.join("latest");
    let json = serde_json::to_string_pretty(r).map_err(|e| e.to_string())?;
    let html = render_html(r);
    for d in [&dir, &latest] {
        std::fs::create_dir_all(d).map_err(|e| format!("could not create {}: {e}", d.display()))?;
        std::fs::write(d.join("report.json"), &json).map_err(|e| e.to_string())?;
        std::fs::write(d.join("report.html"), &html).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(esc("<script>alert('x')&\"</script>"), "&lt;script&gt;alert(&#39;x&#39;)&amp;&quot;&lt;/script&gt;");
    }

    #[test]
    fn only_http_links_are_clickable() {
        assert!(link("https://example.org", "x").starts_with("<a href"));
        assert_eq!(link("javascript:alert(1)", "x"), "x");
    }
}
