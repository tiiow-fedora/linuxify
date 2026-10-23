//! `linuxify choose`: a short plain-language quiz, combined with the scan,
//! ranks the distros in data/distros.json. All logic here is pure.
//!
//! How scoring works: every answer adds weight to one or more *traits*
//! (beginner, gaming, nvidia...). Each distro rates itself 0–5 on every
//! trait. A distro's score is the weighted sum of its ratings, shown as a
//! percentage of the best possible score for those weights.

use crate::scan::hardware::{SecureBoot, Vendor};
use crate::scan::ScanReport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const EMBEDDED_DISTROS: &str = include_str!("../data/distros.json");

/// Every trait a distro must rate itself on, with a label for "weaker on ...".
pub const TRAITS: [(&str, &str); 20] = [
    ("beginner", "beginner-friendliness"),
    ("gaming", "gaming"),
    ("nvidia", "NVIDIA driver setup"),
    ("light", "older or low-memory PCs"),
    ("stable", "stability"),
    ("fresh", "up-to-date software"),
    ("windows_like", "a Windows-like layout"),
    ("modern_look", "a modern look"),
    ("privacy", "privacy"),
    ("low_maintenance", "hands-off updates"),
    ("customizable", "customisation"),
    ("dev", "programming"),
    ("community", "community help"),
    ("creative", "creative/pro apps"),
    ("diy", "learning Linux hands-on"),
    ("software_range", "software available in the package manager"),
    ("out_of_box", "codecs and drivers set up out of the box"),
    ("foss", "free/open-source-only setups"),
    ("enterprise", "matching mainstream work/enterprise Linux"),
    ("rhel_family", "Red Hat-compatible workplaces"),
];

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecureBootSupport {
    Supported,
    NeedsKeyEnrolment,
    NeedsDisabling,
    #[default]
    Unknown,
}

/// The installer a distro's live ISO starts. It decides how Linuxify prepares the
/// installer partition and what it tells the user to pick (see install::installer_hint).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Installer {
    /// Ubiquity (Linux Mint): must unmount the live medium, so it needs `toram`.
    Ubiquity,
    /// Ubuntu's Flutter installer (ubuntu-desktop-bootstrap).
    UbuntuDesktop,
    Calamares,
    Anaconda,
    /// Anaconda from an installer ISO (`inst.stage2=`, Bazzite): installs from the hard drive,
    /// protects only LNXINST itself, so no RAM copy is needed.
    AnacondaClassic,
    /// YaST (openSUSE): skips every disk with a mounted filesystem.
    Yast,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distro {
    pub id: String,
    pub name: String,
    pub homepage: String,
    pub desktop: String,
    /// Package manager family: apt, dnf, zypper, rpm-ostree.
    pub family: String,
    pub flatpak_ready: bool,
    pub secure_boot: SecureBootSupport,
    #[serde(default)]
    pub installer: Installer,
    pub min_ram_gb: u32,
    /// Lowest x86-64 level (2, 3) the distro's binaries run on; 0 = any.
    #[serde(default)]
    pub min_cpu_level: u8,
    /// Why the ISO can't start from Linuxify's installer partition (USB only).
    #[serde(default)]
    pub usb_only: Option<String>,
    pub traits: BTreeMap<String, u8>,
    #[serde(default)]
    pub highlights: BTreeMap<String, String>,
    #[serde(default)]
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistroList {
    pub schema_version: u32,
    pub distros: Vec<Distro>,
}

impl DistroList {
    pub fn parse(json: &str) -> Result<DistroList, String> {
        let list: DistroList = serde_json::from_str(json).map_err(|e| format!("distros.json is not valid: {e}"))?;
        let mut problems = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for d in &list.distros {
            if !ids.insert(d.id.as_str()) {
                problems.push(format!("duplicate id '{}'", d.id));
            }
            for (t, _) in TRAITS {
                match d.traits.get(t) {
                    None => problems.push(format!("'{}' is missing trait '{t}'", d.id)),
                    Some(v) if *v > 5 => problems.push(format!("'{}' trait '{t}' must be 0-5", d.id)),
                    _ => {}
                }
            }
            for k in d.traits.keys().chain(d.highlights.keys()) {
                if !TRAITS.iter().any(|(t, _)| t == k) {
                    problems.push(format!("'{}' uses unknown trait '{k}'", d.id));
                }
            }
            if !d.homepage.starts_with("https://") {
                problems.push(format!("'{}' homepage must be https", d.id));
            }
        }
        if problems.is_empty() {
            Ok(list)
        } else {
            Err(format!("distros.json has problems:\n  - {}", problems.join("\n  - ")))
        }
    }

    pub fn load(data_dir: Option<&std::path::Path>) -> Result<(DistroList, String), String> {
        let mut candidates = Vec::new();
        if let Some(d) = data_dir {
            candidates.push(d.join("distros.json"));
        }
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
            candidates.push(exe_dir.join("data").join("distros.json"));
        }
        for c in candidates {
            if let Ok(t) = std::fs::read_to_string(&c) {
                return Ok((DistroList::parse(&t)?, c.display().to_string()));
            }
        }
        if data_dir.is_some() {
            return Err("no distros.json in the given --data-dir".into());
        }
        Ok((DistroList::parse(EMBEDDED_DISTROS)?, "built-in copy".into()))
    }
}

// ---------------------------------------------------------------------------
// The quiz
// ---------------------------------------------------------------------------

pub struct QuizOption {
    /// Stable key used in answer files, e.g. "never".
    pub key: &'static str,
    pub label: &'static str,
    pub weights: &'static [(&'static str, f64)],
}

pub struct Question {
    pub id: &'static str,
    pub text: &'static str,
    pub options: &'static [QuizOption],
}

macro_rules! opt {
    ($key:literal, $label:literal, [$(($t:literal, $w:expr)),* $(,)?]) => {
        QuizOption { key: $key, label: $label, weights: &[$(($t, $w)),*] }
    };
}

pub const QUESTIONS: &[Question] = &[
    Question {
        id: "experience",
        text: "Have you used Linux before?",
        options: &[
            opt!("never", "Never – I've only used Windows (or a Mac)", [("beginner", 3.0), ("community", 2.0)]),
            opt!("a_little", "A little – I've tried it or used a Raspberry Pi / Steam Deck", [("beginner", 1.5), ("community", 1.0)]),
            opt!("comfortable", "Yes – I'm comfortable with a terminal", [("customizable", 1.0)]),
        ],
    },
    Question {
        id: "gaming",
        text: "How much do you play PC games?",
        options: &[
            opt!("none", "Rarely or never", []),
            opt!("casual", "Sometimes", [("gaming", 1.5)]),
            opt!("lots", "A lot – gaming is a big reason I use this PC", [("gaming", 4.0), ("nvidia", 1.0)]),
        ],
    },
    Question {
        id: "creative",
        text: "Do you do creative work (video editing, photos, 3D, music)?",
        options: &[
            opt!("none", "No", []),
            opt!("hobby", "As a hobby", [("creative", 1.0), ("fresh", 0.5)]),
            opt!("pro", "Seriously – it's work or a big part of my time", [("creative", 3.0), ("fresh", 1.0), ("nvidia", 1.5), ("stable", 0.5)]),
        ],
    },
    Question {
        id: "hardware_age",
        text: "How old is this computer?",
        options: &[
            // A newer PC can comfortably run a full-featured desktop.
            opt!("new", "Less than 5 years old", [("modern_look", 0.5)]),
            opt!("middle", "5 to 10 years old", [("light", 1.5)]),
            opt!("old", "Over 10 years old, or it already feels slow", [("light", 4.0)]),
        ],
    },
    Question {
        id: "stability",
        text: "Which matters more to you?",
        options: &[
            opt!("stable", "Rock-solid: I want it to stay the same and never surprise me", [("stable", 3.0)]),
            opt!("balanced", "A balance of both", [("stable", 1.0), ("fresh", 1.0)]),
            opt!("fresh", "The newest versions of apps and drivers", [("fresh", 3.0)]),
        ],
    },
    Question {
        id: "look",
        text: "What should the desktop look like?",
        options: &[
            opt!("windows", "Like Windows – taskbar and start menu at the bottom", [("windows_like", 3.0)]),
            opt!("modern", "Modern and minimal – more like a Mac or a phone", [("modern_look", 3.0)]),
            opt!("either", "I don't mind", []),
        ],
    },
    Question {
        id: "privacy",
        text: "How important is privacy (no tracking, no company in control)?",
        options: &[
            opt!("very", "Very important", [("privacy", 3.0)]),
            opt!("somewhat", "Somewhat", [("privacy", 1.0)]),
            opt!("dont_mind", "I don't mind", []),
        ],
    },
    Question {
        id: "updates",
        text: "How do you feel about updates?",
        options: &[
            opt!("hands_off", "Just handle them for me – I never want an update to break things", [("low_maintenance", 3.0)]),
            opt!("normal", "I'll click 'update' now and then", [("low_maintenance", 1.0)]),
            opt!("control", "I like to control what gets installed and when", [("customizable", 1.5)]),
        ],
    },
    Question {
        id: "tinkering",
        text: "Do you enjoy tweaking and customising your computer?",
        options: &[
            opt!("just_work", "No – I want it to just work", [("beginner", 1.0), ("low_maintenance", 1.0)]),
            opt!("some", "A bit", [("customizable", 1.0)]),
            opt!("love", "Yes, I love making it my own", [("customizable", 3.0)]),
        ],
    },
    Question {
        id: "dev",
        text: "Do you write code or do programming?",
        options: &[
            opt!("no", "No", []),
            opt!("some", "Sometimes", [("dev", 1.5)]),
            opt!("main", "Yes, a lot – it's a main use of this PC", [("dev", 3.0), ("fresh", 1.0)]),
        ],
    },
    Question {
        id: "learning",
        text: "Do you want to learn how Linux works under the hood?",
        options: &[
            opt!("no", "Not really – I just want to use my computer", []),
            opt!("curious", "I'm curious – a bit at a time", [("diy", 1.0)]),
            opt!("yes", "Yes – I'd like to build my system piece by piece", [("diy", 3.0), ("customizable", 1.0)]),
        ],
    },
    Question {
        id: "software",
        text: "How important is finding almost any app in the built-in software store?",
        options: &[
            opt!("very", "Very – I use lots of niche tools", [("software_range", 3.0)]),
            opt!("somewhat", "Somewhat", [("software_range", 1.0)]),
            opt!("not", "Not very – I use a handful of common apps", []),
        ],
    },
    Question {
        id: "extras",
        text: "Should media codecs and drivers be set up for you straight away?",
        options: &[
            opt!("yes", "Yes – videos and graphics should just work", [("out_of_box", 3.0)]),
            opt!("dont_mind", "I don't mind doing a few setup steps", [("out_of_box", 1.0)]),
            opt!("foss_only", "No – I'd rather run only free/open-source software", [("foss", 3.0)]),
        ],
    },
    Question {
        id: "work",
        text: "Does your job or school use a specific Linux you should match?",
        options: &[
            opt!("no", "No", []),
            opt!("nice", "It would be nice to match what's used at work", [("enterprise", 1.5)]),
            opt!("ubuntu", "Yes – Ubuntu or Debian-based", [("enterprise", 4.0)]),
            opt!("redhat", "Yes – Red Hat-compatible (RHEL, Rocky, Alma, Fedora)", [("enterprise", 2.0), ("rhel_family", 3.0)]),
        ],
    },
];

/// Everyone benefits from being able to find help.
const BASE_WEIGHTS: &[(&str, f64)] = &[("community", 1.0)];

/// Answers by question id → option key.
pub type Answers = BTreeMap<String, String>;

pub fn validate_answers(a: &Answers) -> Result<(), String> {
    for q in QUESTIONS {
        let Some(k) = a.get(q.id) else {
            return Err(format!("missing answer for '{}'", q.id));
        };
        if !q.options.iter().any(|o| o.key == k) {
            let keys: Vec<_> = q.options.iter().map(|o| o.key).collect();
            return Err(format!("'{}' must be one of {}, not '{k}'", q.id, keys.join(", ")));
        }
    }
    if let Some(extra) = a.keys().find(|k| !QUESTIONS.iter().any(|q| q.id == k.as_str())) {
        return Err(format!("unknown question '{extra}'"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// What the scan tells us
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ScanHints {
    pub nvidia_gpu: bool,
    pub ram_gb: Option<f64>,
    pub cpu_level: Option<u8>,
    pub secure_boot_on: bool,
    pub steam_games: usize,
    /// Programs the scan marked as "won't work on Linux".
    pub blocked_programs: Vec<String>,
}

pub fn hints_from_scan(r: &ScanReport) -> ScanHints {
    use crate::mapping::{Group, Status};
    ScanHints {
        nvidia_gpu: r.hardware.gpus.iter().any(|g| !g.is_virtual && g.vendor == Vendor::Nvidia),
        ram_gb: r.hardware.ram_bytes.map(|b| b as f64 / (1u64 << 30) as f64),
        cpu_level: r.hardware.cpu_level,
        secure_boot_on: r.hardware.secure_boot == SecureBoot::On,
        steam_games: r.user_data.steam.iter().map(|l| l.games.len()).sum(),
        blocked_programs: r
            .programs
            .iter()
            .filter(|p| p.group == Group::Mapped && p.status == Status::Blocked)
            .map(|p| p.program.name.clone())
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ranked {
    pub id: String,
    pub name: String,
    pub desktop: String,
    pub homepage: String,
    /// 0–100: how well this distro fits the weighted wishes.
    pub score: f64,
    pub reasons: Vec<String>,
    pub cautions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Excluded {
    pub id: String,
    pub name: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub weights: BTreeMap<String, f64>,
    /// Plain-English notes on how the scan changed things.
    pub scan_notes: Vec<String>,
    pub ranking: Vec<Ranked>,
    pub excluded: Vec<Excluded>,
}

/// Turns answers + scan hints into trait weights, with notes explaining
/// any scan-driven change.
pub fn weights_for(answers: &Answers, hints: Option<&ScanHints>) -> (BTreeMap<String, f64>, Vec<String>) {
    let mut w: BTreeMap<String, f64> = TRAITS.iter().map(|(t, _)| (t.to_string(), 0.0)).collect();
    let mut add = |t: &str, v: f64| *w.get_mut(t).expect("known trait") += v;
    for (t, v) in BASE_WEIGHTS {
        add(t, *v);
    }
    for q in QUESTIONS {
        if let Some(o) = answers.get(q.id).and_then(|k| q.options.iter().find(|o| o.key == k)) {
            for (t, v) in o.weights {
                add(t, *v);
            }
        }
    }
    let mut notes = Vec::new();
    if let Some(h) = hints {
        if h.nvidia_gpu {
            add("nvidia", 3.0);
            notes.push("Your PC has an NVIDIA graphics card, so distros that make NVIDIA's driver easy were favoured.".into());
        }
        match h.ram_gb {
            Some(r) if r < 4.0 => {
                add("light", 4.0);
                notes.push(format!("Your PC has {r:.1} GB of RAM, so lightweight distros were strongly favoured."));
            }
            Some(r) if r < 8.0 => {
                add("light", 1.5);
                notes.push(format!("Your PC has {r:.1} GB of RAM, so lighter distros got a small boost."));
            }
            _ => {}
        }
        if h.steam_games >= 5 && answers.get("gaming").is_some_and(|g| g != "none") {
            add("gaming", 1.0);
            notes.push(format!("You have {} Steam games installed, so gaming got a little extra weight.", h.steam_games));
        }
    }
    (w, notes)
}

fn trait_label(t: &str) -> &'static str {
    TRAITS.iter().find(|(k, _)| *k == t).map(|(_, l)| *l).unwrap_or("?")
}

/// Why a distro can't start on this CPU, if it can't. Unknown level = no verdict.
pub fn cpu_too_old(need: u8, have: Option<u8>) -> Option<String> {
    let have = have?;
    (have < need).then(|| {
        format!("needs a newer processor (x86-64-v{need}: AVX2, roughly Intel 2013 / AMD 2015 or later); this one is x86-64-v{have}, so it would stop at boot")
    })
}

pub fn rank(list: &DistroList, answers: &Answers, hints: Option<&ScanHints>) -> Recommendation {
    let (weights, mut scan_notes) = weights_for(answers, hints);
    let total_w: f64 = weights.values().sum();
    let mut ranking = Vec::new();
    let mut excluded = Vec::new();

    for d in &list.distros {
        if let Some(ram) = hints.and_then(|h| h.ram_gb) {
            // A little slack: Windows reports usable RAM slightly under the stick size.
            if ram + 0.5 < d.min_ram_gb as f64 {
                excluded.push(Excluded {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    why: format!("needs at least {} GB of RAM; this PC has {ram:.1} GB", d.min_ram_gb),
                });
                continue;
            }
        }
        if let Some(why) = cpu_too_old(d.min_cpu_level, hints.and_then(|h| h.cpu_level)) {
            excluded.push(Excluded { id: d.id.clone(), name: d.name.clone(), why });
            continue;
        }

        let contrib: Vec<(&str, f64)> = weights
            .iter()
            .map(|(t, w)| (t.as_str(), w * *d.traits.get(t).unwrap_or(&0) as f64))
            .collect();
        let raw: f64 = contrib.iter().map(|(_, c)| c).sum();
        let mut score = if total_w > 0.0 { raw / (5.0 * total_w) * 100.0 } else { 0.0 };

        let mut cautions = d.caveats.clone();
        if hints.is_some_and(|h| h.secure_boot_on) {
            match d.secure_boot {
                SecureBootSupport::NeedsDisabling => {
                    score -= 5.0;
                    cautions.insert(0, "Secure Boot is on in your PC; you'd probably need to turn it off to install this one.".into());
                }
                SecureBootSupport::NeedsKeyEnrolment => cautions.insert(0, "Secure Boot is on in your PC; the installer will ask you to enrol a key once (it walks you through it).".into()),
                SecureBootSupport::Unknown => cautions.insert(0, "Secure Boot is on in your PC and Linuxify doesn't know whether this distro boots with it on; test it in the trial step first.".into()),
                SecureBootSupport::Supported => {}
            }
        }
        // Strong wishes this distro is weak on.
        for (t, w) in &weights {
            if *w >= 2.0 && *d.traits.get(t).unwrap_or(&0) <= 2 {
                cautions.push(format!("Weaker on {}.", trait_label(t)));
            }
        }

        let mut by_contrib = contrib.clone();
        by_contrib.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(b.0)));
        let reasons: Vec<String> = by_contrib
            .iter()
            .filter(|(_, c)| *c > 0.0)
            .filter_map(|(t, _)| d.highlights.get(*t).cloned())
            .take(3)
            .collect();

        ranking.push(Ranked {
            id: d.id.clone(),
            name: d.name.clone(),
            desktop: d.desktop.clone(),
            homepage: d.homepage.clone(),
            score: (score * 10.0).round() / 10.0,
            reasons,
            cautions,
        });
    }

    // Highest score first; ties broken by id so results are repeatable.
    ranking.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(a.id.cmp(&b.id)));

    if let Some(h) = hints {
        if !h.blocked_programs.is_empty() {
            scan_notes.push(format!(
                "Whichever distro you pick, these won't run on Linux: {}. Keeping Windows as a dual boot covers them.",
                h.blocked_programs.join(", ")
            ));
        }
    }
    Recommendation { weights, scan_notes, ranking, excluded }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> DistroList {
        DistroList::parse(EMBEDDED_DISTROS).expect("data/distros.json must validate")
    }

    /// The answer profiles live in tests/profiles/*.json so the CLI can use them too.
    fn profile(name: &str) -> Answers {
        let text = match name {
            "newcomer" => include_str!("../tests/profiles/newcomer.json"),
            "gamer" => include_str!("../tests/profiles/gamer.json"),
            "old_laptop" => include_str!("../tests/profiles/old_laptop.json"),
            "developer" => include_str!("../tests/profiles/developer.json"),
            "privacy_veteran" => include_str!("../tests/profiles/privacy_veteran.json"),
            "creator" => include_str!("../tests/profiles/creator.json"),
            "arch_tinkerer" => include_str!("../tests/profiles/arch_tinkerer.json"),
            "enterprise_worker" => include_str!("../tests/profiles/enterprise_worker.json"),
            "gaming_tinkerer" => include_str!("../tests/profiles/gaming_tinkerer.json"),
            "redhat_worker" => include_str!("../tests/profiles/redhat_worker.json"),
            _ => panic!("no profile {name}"),
        };
        let a: Answers = serde_json::from_str(text).unwrap();
        validate_answers(&a).unwrap_or_else(|e| panic!("{name}: {e}"));
        a
    }

    fn top(a: &Answers, h: Option<&ScanHints>) -> Vec<String> {
        rank(&list(), a, h).ranking.into_iter().take(3).map(|r| r.id).collect()
    }

    #[test]
    fn old_cpu_excludes_only_v3_distros() {
        let h = ScanHints { cpu_level: Some(2), ..Default::default() };
        let r = rank(&list(), &Answers::default(), Some(&h));
        let alma = r.excluded.iter().find(|e| e.id == "almalinux").expect("alma excluded");
        assert!(alma.why.contains("x86-64-v3") && alma.why.contains("x86-64-v2"));
        let mut ids: Vec<_> = r.excluded.iter().map(|e| e.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["almalinux", "pikaos"]);
        assert!(cpu_too_old(3, Some(3)).is_none() && cpu_too_old(3, None).is_none() && cpu_too_old(0, Some(1)).is_none());
    }

    fn hints(nvidia: bool, ram: f64) -> ScanHints {
        ScanHints { nvidia_gpu: nvidia, ram_gb: Some(ram), ..Default::default() }
    }

    fn newcomer() -> Answers { profile("newcomer") }
    fn gamer() -> Answers { profile("gamer") }
    fn creator() -> Answers { profile("creator") }
    fn old_laptop() -> Answers { profile("old_laptop") }
    fn developer() -> Answers { profile("developer") }
    fn privacy_veteran() -> Answers { profile("privacy_veteran") }

    /// Prints every profile's top 5 (run with --nocapture to review).
    #[test]
    fn print_profile_rankings() {
        for name in ["newcomer", "gamer", "old_laptop", "developer", "privacy_veteran", "creator", "arch_tinkerer", "enterprise_worker", "gaming_tinkerer", "redhat_worker"] {
            for (label, h) in [("amd 32GB", hints(false, 32.0)), ("nvidia 32GB", hints(true, 32.0)), ("3GB", hints(false, 3.0))] {
                let r = rank(&list(), &profile(name), Some(&h));
                let t: Vec<String> = r.ranking.iter().take(5).map(|x| format!("{} {:.0}", x.id, x.score)).collect();
                println!("{name:<18} {label:<12} {}", t.join(" | "));
            }
        }
    }

    #[test]
    fn distros_file_is_valid() {
        assert!(list().distros.len() >= 7);
    }

    #[test]
    fn validation_catches_bad_data() {
        let bad = r#"{"schema_version":1,"distros":[{"id":"x","name":"X","homepage":"http://x","desktop":"d","family":"apt","flatpak_ready":true,"secure_boot":"supported","min_ram_gb":1,"traits":{"beginner":9,"bogus":1}}]}"#;
        let e = DistroList::parse(bad).unwrap_err();
        assert!(e.contains("must be 0-5"));
        assert!(e.contains("missing trait 'gaming'"));
        assert!(e.contains("unknown trait 'bogus'"));
        assert!(e.contains("https"));
    }

    #[test]
    fn answer_validation() {
        let mut a = newcomer();
        a.insert("gaming".into(), "sometimes".into());
        assert!(validate_answers(&a).unwrap_err().contains("must be one of"));
        a.remove("gaming");
        assert!(validate_answers(&a).unwrap_err().contains("missing"));
    }

    fn trait_of(id: &str, t: &str) -> u8 {
        list().distros.iter().find(|d| d.id == id).unwrap().traits[t]
    }

    #[test]
    fn newcomer_gets_mint_first() {
        let t = top(&newcomer(), Some(&hints(false, 16.0)));
        assert_eq!(t[0], "linux-mint", "{t:?}");
        assert!(t.iter().all(|id| trait_of(id, "beginner") >= 4), "{t:?}");
    }

    #[test]
    fn gamer_with_nvidia_gets_bazzite_then_pop() {
        let t = top(&gamer(), Some(&hints(true, 32.0)));
        assert_eq!(t[..2], ["bazzite", "pop-os"], "{t:?}");
    }

    #[test]
    fn old_low_ram_pc_gets_lightweight_and_excludes_heavy() {
        let r = rank(&list(), &old_laptop(), Some(&hints(false, 3.0)));
        let ids: Vec<_> = r.ranking.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids[0], "linux-mint-xfce", "{ids:?}");
        assert!(ids[..5].iter().all(|id| trait_of(id, "light") >= 4 || *id == "linux-mint"), "{ids:?}");
        for heavy in ["bazzite", "fedora-workstation", "garuda", "elementary-os"] {
            assert!(r.excluded.iter().any(|e| e.id == heavy), "{heavy} should be excluded");
        }
    }

    #[test]
    fn developer_gets_fast_moving_distros() {
        let t = top(&developer(), Some(&hints(false, 32.0)));
        assert!(t.iter().all(|id| trait_of(id, "fresh") >= 4 && trait_of(id, "dev") >= 3), "{t:?}");
    }

    #[test]
    fn privacy_veteran_gets_debian() {
        let t = top(&privacy_veteran(), Some(&hints(false, 16.0)));
        assert_eq!(t[0], "debian", "{t:?}");
        assert!(t.iter().all(|id| trait_of(id, "privacy") == 5), "{t:?}");
    }

    #[test]
    fn creator_with_nvidia_gets_pop() {
        let t = top(&creator(), Some(&hints(true, 32.0)));
        assert_eq!(t[0], "pop-os", "{t:?}");
    }

    #[test]
    fn arch_tinkerer_gets_arch() {
        let t = top(&profile("arch_tinkerer"), Some(&hints(false, 32.0)));
        assert_eq!(t[0], "arch", "{t:?}");
        assert!(t.iter().all(|id| trait_of(id, "diy") >= 3), "{t:?}");
    }

    #[test]
    fn enterprise_worker_gets_ubuntu_and_no_hobby_distros() {
        let t = top(&profile("enterprise_worker"), Some(&hints(false, 16.0)));
        assert_eq!(t[0], "ubuntu", "{t:?}");
        assert!(t.iter().all(|id| trait_of(id, "enterprise") >= 3), "{t:?}");
    }

    #[test]
    fn redhat_worker_gets_rhel_family() {
        let t = top(&profile("redhat_worker"), Some(&hints(false, 16.0)));
        assert_eq!(t[0], "almalinux", "{t:?}");
        assert!(t.iter().all(|id| trait_of(id, "rhel_family") >= 2), "{t:?}");
    }

    #[test]
    fn gaming_tinkerer_gets_cachyos() {
        let t = top(&profile("gaming_tinkerer"), Some(&hints(true, 32.0)));
        assert_eq!(t[0], "cachyos", "{t:?}");
        assert!(t.iter().all(|id| trait_of(id, "gaming") >= 4), "{t:?}");
    }

    #[test]
    fn every_distro_can_win_something_or_is_niche() {
        // Guard against a distro whose ratings make it unreachable: it should
        // reach the top 5 for at least one profile + hardware combination.
        let names = ["newcomer", "gamer", "old_laptop", "developer", "privacy_veteran", "creator", "arch_tinkerer", "enterprise_worker", "gaming_tinkerer", "redhat_worker"];
        let mut seen = std::collections::HashSet::new();
        for n in names {
            for h in [hints(false, 32.0), hints(true, 32.0), hints(false, 3.0), hints(false, 1.5)] {
                for r in rank(&list(), &profile(n), Some(&h)).ranking.into_iter().take(5) {
                    seen.insert(r.id);
                }
            }
        }
        let missing: Vec<_> = list().distros.iter().map(|d| d.id.clone()).filter(|id| !seen.contains(id)).collect();
        println!("never in a top 5: {missing:?}");
        assert!(missing.len() <= 8, "too many unreachable distros: {missing:?}");
    }

    #[test]
    fn changing_one_answer_changes_the_result() {
        // Same newcomer, but now a heavy gamer with an NVIDIA card.
        let mut a = newcomer();
        a.insert("gaming".into(), "lots".into());
        let t = top(&a, Some(&hints(true, 16.0)));
        assert_ne!(t[0], "linux-mint", "{t:?}");
        // And the NVIDIA card alone moves Pop!_OS up for the creator.
        let without = rank(&list(), &creator(), Some(&hints(false, 32.0)));
        let with = rank(&list(), &creator(), Some(&hints(true, 32.0)));
        let pos = |r: &Recommendation| r.ranking.iter().position(|x| x.id == "pop-os").unwrap();
        assert!(pos(&with) <= pos(&without));
    }

    #[test]
    fn secure_boot_on_penalises_and_warns() {
        let mut h = hints(true, 32.0);
        let off = rank(&list(), &gamer(), Some(&h));
        h.secure_boot_on = true;
        let on = rank(&list(), &gamer(), Some(&h));
        let get = |r: &Recommendation, id: &str| r.ranking.iter().find(|x| x.id == id).unwrap().clone();
        assert!(get(&on, "pop-os").score < get(&off, "pop-os").score);
        assert!(get(&on, "pop-os").cautions[0].contains("turn it off"));
        assert!(get(&on, "bazzite").cautions[0].contains("enrol a key"));
    }

    #[test]
    fn works_without_a_scan_and_gives_reasons() {
        let r = rank(&list(), &newcomer(), None);
        assert_eq!(r.ranking.len(), list().distros.len());
        assert!(r.excluded.is_empty());
        assert!(!r.ranking[0].reasons.is_empty());
        assert!(r.ranking.iter().all(|x| (0.0..=100.0).contains(&x.score)));
    }

    #[test]
    fn steam_library_only_counts_if_user_games() {
        let h = ScanHints { steam_games: 30, ..Default::default() };
        let (w_none, _) = weights_for(&newcomer(), Some(&h));
        let (w_gamer, notes) = weights_for(&gamer(), Some(&h));
        assert_eq!(w_none["gaming"], 0.0);
        assert_eq!(w_gamer["gaming"], 5.0);
        assert!(notes.iter().any(|n| n.contains("30 Steam games")));
    }

}
