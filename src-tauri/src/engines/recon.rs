//! Recon for Pentest user mode. An engagement is a folder holding `scope.txt`; network tools run
//! only on targets that file allows, at most `MAX_ADDRESSES` addresses per run. Results that tools
//! write as data (nmap XML, httpx/nuclei JSON lines, subfinder lists) become findings in the
//! folder's `findings.json`, from which the engagement report is written.
//!
//! scope.txt, one entry per line (`#` starts a comment):
//!   example.com        only that host
//!   *.example.com      the domain and every subdomain
//!   10.0.0.7           one address
//!   10.0.0.0/24        a network (IPv4 or IPv6)
//!   !admin.example.com never, nor its subdomains, even if a wider entry allows them (works for
//!                      addresses and networks too)

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

pub const SCOPE_FILE: &str = "scope.txt";
pub const FINDINGS_FILE: &str = "findings.json";
/// Addresses one run may touch (a /24): no sweeping of large networks by accident.
pub const MAX_ADDRESSES: u128 = 256;

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Host(String),
    Domain(String),
    Net(IpAddr, u8),
}

#[derive(Debug, Default)]
pub struct Scope {
    allow: Vec<Entry>,
    deny: Vec<Entry>,
}

/// The host of a URL, or the line itself.
fn host_of(s: &str) -> &str {
    let s = s.trim();
    let s = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let s = s.split(['/', '?', '#']).next().unwrap_or(s);
    let s = s.rsplit_once('@').map(|(_, h)| h).unwrap_or(s);
    // [v6]:port, host:port (a bare IPv6 address has several colons and is kept whole)
    if let Some(v6) = s.strip_prefix('[') {
        return v6.split(']').next().unwrap_or(v6);
    }
    if s.matches(':').count() == 1 {
        return s.split(':').next().unwrap_or(s);
    }
    s
}

fn valid_name(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && !h.starts_with(['-', '.'])
        && h.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

fn parse_net(s: &str) -> Option<(IpAddr, u8)> {
    let (ip, bits) = s.split_once('/')?;
    let ip: IpAddr = ip.parse().ok()?;
    let bits: u8 = bits.parse().ok()?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    (bits <= max).then_some((ip, bits))
}

fn parse_entry(line: &str) -> Option<Entry> {
    let s = line.trim().to_ascii_lowercase();
    if let Some((ip, bits)) = parse_net(&s) {
        return Some(Entry::Net(ip, bits));
    }
    if let Some(d) = s.strip_prefix("*.") {
        let d = host_of(d);
        return valid_name(d).then(|| Entry::Domain(d.to_string()));
    }
    let h = host_of(&s);
    if let Ok(ip) = h.parse::<IpAddr>() {
        return Some(Entry::Net(ip, if ip.is_ipv4() { 32 } else { 128 }));
    }
    valid_name(h).then(|| Entry::Host(h.trim_end_matches('.').to_string()))
}

fn bits_of(ip: IpAddr) -> (u128, u8) {
    match ip {
        IpAddr::V4(v) => (u32::from(v) as u128, 32),
        IpAddr::V6(v) => (u128::from(v), 128),
    }
}

/// Network `a/abits` lies wholly inside `b/bbits` (same family).
fn net_within(a: IpAddr, abits: u8, b: IpAddr, bbits: u8) -> bool {
    let ((x, w), (y, w2)) = (bits_of(a), bits_of(b));
    if w != w2 || abits < bbits {
        return false;
    }
    let shift = w - bbits;
    shift >= 128 || (x >> shift) == (y >> shift)
}

fn net_overlaps(a: IpAddr, abits: u8, b: IpAddr, bbits: u8) -> bool {
    net_within(a, abits, b, bbits) || net_within(b, bbits, a, abits)
}

impl Scope {
    pub fn parse(text: &str) -> Scope {
        let mut s = Scope::default();
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (deny, body) = match line.strip_prefix('!') {
                Some(b) => (true, b),
                None => (false, line),
            };
            if let Some(e) = parse_entry(body) {
                if deny {
                    s.deny.push(e)
                } else {
                    s.allow.push(e)
                }
            }
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.allow.is_empty()
    }

    fn name_matches(e: &Entry, h: &str) -> bool {
        match e {
            Entry::Host(x) => x == h,
            Entry::Domain(d) => h == d || h.ends_with(&format!(".{d}")),
            Entry::Net(..) => false,
        }
    }

    /// Whether a target (host, address or network) may be touched.
    pub fn allows(&self, t: &Target) -> bool {
        match t {
            // an excluded name takes its subdomains with it: being too careful beats scanning them
            Target::Name(h) => {
                let denied = self.deny.iter().any(|e| match e {
                    Entry::Host(x) | Entry::Domain(x) => h == x || h.ends_with(&format!(".{x}")),
                    Entry::Net(..) => false,
                });
                !denied && self.allow.iter().any(|e| Self::name_matches(e, h))
            }
            Target::Net(ip, bits) => {
                let denied = self.deny.iter().any(|e| matches!(e, Entry::Net(d, db) if net_overlaps(*ip, *bits, *d, *db)));
                !denied && self.allow.iter().any(|e| matches!(e, Entry::Net(a, ab) if net_within(*ip, *bits, *a, *ab)))
            }
        }
    }

    /// Targets implied by the scope itself (when scope.txt is what was dropped): hosts, the apex of
    /// each `*.domain`, and addresses/networks.
    fn as_targets(&self) -> Vec<Target> {
        self.allow
            .iter()
            .map(|e| match e {
                Entry::Host(h) | Entry::Domain(h) => Target::Name(h.clone()),
                Entry::Net(ip, b) => Target::Net(*ip, *b),
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Name(String),
    Net(IpAddr, u8),
}

impl Target {
    fn addresses(&self) -> u128 {
        match self {
            Target::Name(_) => 1,
            Target::Net(ip, b) => {
                let w = if ip.is_ipv4() { 32 } else { 128 };
                let free = w - b;
                if free >= 127 {
                    u128::MAX
                } else {
                    1u128 << free
                }
            }
        }
    }

    pub fn text(&self) -> String {
        match self {
            Target::Name(h) => h.clone(),
            Target::Net(ip, b) if (ip.is_ipv4() && *b == 32) || (ip.is_ipv6() && *b == 128) => ip.to_string(),
            Target::Net(ip, b) => format!("{ip}/{b}"),
        }
    }
}

fn parse_target(line: &str) -> Option<Target> {
    match parse_entry(line)? {
        Entry::Host(h) | Entry::Domain(h) => Some(Target::Name(h)),
        Entry::Net(ip, b) => Some(Target::Net(ip, b)),
    }
}

/// The targets of a run: the in-scope lines of the dropped file (or the scope's own entries when
/// scope.txt itself was dropped), plus the lines that were refused and why.
pub struct Plan {
    pub scope: Scope,
    pub folder: PathBuf,
    pub targets: Vec<Target>,
    pub refused: Vec<String>,
}

pub fn plan(src: &Path) -> Result<Plan> {
    let folder = src.parent().context("no folder")?.to_path_buf();
    let scope_path = folder.join(SCOPE_FILE);
    let text = std::fs::read_to_string(&scope_path)
        .map_err(|_| anyhow::anyhow!("recon needs a scope.txt in the same folder listing what you are authorized to test (one host, *.domain or network per line)"))?;
    let scope = Scope::parse(&text);
    if scope.is_empty() {
        bail!("scope.txt lists nothing to test: add the hosts, *.domains or networks you are authorized for");
    }
    let dropped_scope = src.file_name().is_some_and(|n| n == SCOPE_FILE);
    let mut targets = Vec::new();
    let mut refused = Vec::new();
    if dropped_scope {
        // a denied entry (e.g. !10.0.0.9 inside an allowed /24) is never scanned as a whole network
        targets = scope.as_targets().into_iter().filter(|t| scope.allows(t)).collect();
    } else {
        let body = std::fs::read_to_string(src).context("not a text file")?;
        for raw in body.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            match parse_target(line) {
                Some(t) if scope.allows(&t) => targets.push(t),
                Some(_) => refused.push(format!("{line} (outside scope.txt)")),
                None => refused.push(format!("{line} (not a host, address or network)")),
            }
        }
    }
    targets.dedup();
    let mut seen = std::collections::HashSet::new();
    targets.retain(|t| seen.insert(t.text()));
    if targets.is_empty() {
        bail!("no target is inside scope.txt{}", if refused.is_empty() { String::new() } else { format!(": refused {}", refused.join(", ")) });
    }
    let total: u128 = targets.iter().map(Target::addresses).fold(0u128, |a, b| a.saturating_add(b));
    if total > MAX_ADDRESSES {
        bail!(
            "these targets cover {} addresses; one run may touch at most {MAX_ADDRESSES} (a /24). Split them into smaller target files",
            if total == u128::MAX { "too many".into() } else { total.to_string() }
        );
    }
    Ok(Plan { scope, folder, targets, refused })
}

// --- Findings ------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Finding {
    /// port | web | vuln | subdomain | dns
    pub kind: String,
    /// info | low | medium | high | critical
    pub severity: String,
    pub target: String,
    pub detail: String,
    pub source: String,
    pub time: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Run {
    pub tool: String,
    pub time: u64,
    pub targets: usize,
    pub report: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Findings {
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn finding(kind: &str, severity: &str, target: String, detail: String, source: &str) -> Finding {
    Finding { kind: kind.into(), severity: severity.into(), target, detail, source: source.into(), time: now() }
}

/// nmap -oX: one finding per open port.
pub fn parse_nmap(xml: &str) -> Vec<Finding> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let (mut host, mut port, mut proto, mut open, mut service) = (String::new(), String::new(), String::new(), false, String::new());
    let attr = |e: &quick_xml::events::BytesStart, k: &str| {
        e.attributes()
            .flatten()
            .find(|a| a.key.local_name().as_ref() == k)
            .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
            .unwrap_or_default()
    };
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                "host" => host.clear(),
                "address" if host.is_empty() || attr(&e, "addrtype") != "mac" => {
                    if attr(&e, "addrtype") != "mac" {
                        host = attr(&e, "addr");
                    }
                }
                "hostname" if attr(&e, "type") == "user" => {
                    let n = attr(&e, "name");
                    if !n.is_empty() {
                        host = format!("{n} ({host})");
                    }
                }
                "port" => {
                    port = attr(&e, "portid");
                    proto = attr(&e, "protocol");
                    open = false;
                    service.clear();
                }
                "state" => open = attr(&e, "state") == "open",
                "service" => {
                    service =
                        [attr(&e, "name"), attr(&e, "product"), attr(&e, "version"), attr(&e, "extrainfo")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.local_name().as_ref() == "port" => {
                if open && !host.is_empty() {
                    out.push(finding("port", "info", format!("{host}:{port}/{proto}"), if service.is_empty() { "open".into() } else { service.clone() }, "nmap"));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// httpx -json: one finding per live web server.
pub fn parse_httpx(jsonl: &str) -> Vec<Finding> {
    jsonl
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            let url = v["url"].as_str()?.to_string();
            let status = v["status_code"].as_u64().or_else(|| v["status-code"].as_u64()).map(|s| s.to_string()).unwrap_or_default();
            let title = v["title"].as_str().unwrap_or("").trim().to_string();
            let tech = v["tech"].as_array().map(|a| a.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let detail = [status, title, tech].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            Some(finding("web", "info", url, detail, "httpx"))
        })
        .collect()
}

/// nuclei -jsonl: one finding per match, with the template's severity.
pub fn parse_nuclei(jsonl: &str) -> Vec<Finding> {
    jsonl
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            let at = v["matched-at"].as_str().or_else(|| v["host"].as_str())?.to_string();
            let info = &v["info"];
            let sev = info["severity"].as_str().unwrap_or("info").to_ascii_lowercase();
            let sev = if ["info", "low", "medium", "high", "critical"].contains(&sev.as_str()) { sev } else { "info".into() };
            let name = info["name"].as_str().unwrap_or("").to_string();
            let id = v["template-id"].as_str().unwrap_or("").to_string();
            Some(finding("vuln", &sev, at, format!("{name} [{id}]"), "nuclei"))
        })
        .collect()
}

/// subfinder/dnsx line lists: one finding per name, marked when it falls outside the scope.
pub fn parse_lines(text: &str, kind: &str, source: &str, scope: &Scope) -> Vec<Finding> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let name = l.split_whitespace().next().unwrap_or(l).to_ascii_lowercase();
            let inside = parse_target(&name).is_some_and(|t| scope.allows(&t));
            finding(kind, "info", name, if inside { l.to_string() } else { format!("{l} (outside scope: not tested further)") }, source)
        })
        .collect()
}

/// Reads what a tool wrote into its output folder, by parser name.
pub fn parse_output(parser: &str, out: &Path, scope: &Scope) -> Vec<Finding> {
    let read = |f: &str| std::fs::read_to_string(out.join(f)).unwrap_or_default();
    match parser {
        "nmap" => parse_nmap(&read("nmap.xml")),
        "httpx" => parse_httpx(&read("httpx.jsonl")),
        "nuclei" => parse_nuclei(&read("nuclei.jsonl")),
        "subdomains" => parse_lines(&read("subdomains.txt"), "subdomain", "subfinder", scope),
        "dns" => parse_lines(&read("dnsx.txt"), "dns", "dnsx", scope),
        _ => Vec::new(),
    }
}

/// Adds a run and its findings to the folder's findings.json (duplicates are kept once).
pub fn record(folder: &Path, run: Run, new: Vec<Finding>) -> Result<usize> {
    let path = folder.join(FINDINGS_FILE);
    let mut all: Findings = match std::fs::read_to_string(&path) {
        Err(_) => Findings::default(),
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
            // earlier findings that can't be read are kept, never overwritten by this run's
            let keep = folder.join("findings.broken.json");
            let _ = std::fs::rename(&path, &keep);
            super::note("findings.json couldn't be read; it was kept as findings.broken.json and a new one started");
            Findings::default()
        }),
    };
    let before = all.findings.len();
    for f in new {
        if !all.findings.iter().any(|x| x.kind == f.kind && x.target == f.target && x.detail == f.detail) {
            all.findings.push(f);
        }
    }
    all.runs.push(run);
    let added = all.findings.len() - before;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&all)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(added)
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn date(t: u64) -> String {
    // days since 1970 → y-m-d (civil-from-days)
    let z = (t / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", (t % 86400) / 3600, (t % 3600) / 60)
}

/// The engagement report (Markdown) from scope.txt and findings.json in the folder.
pub fn report(folder: &Path) -> Result<String> {
    let scope = std::fs::read_to_string(folder.join(SCOPE_FILE)).context("no scope.txt in this folder")?;
    let all: Findings = std::fs::read_to_string(folder.join(FINDINGS_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .context("no findings yet: run a recon tool on a target file in this folder first")?;
    let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Engagement".into());
    let order = ["critical", "high", "medium", "low", "info"];
    let mut md = format!("# {name}: engagement report\n\nGenerated {}.\n\n## Scope\n\n```\n{}\n```\n\n## Summary\n\n", date(now()), scope.trim());
    md.push_str("| Severity | Findings |\n|---|---|\n");
    for s in order {
        let n = all.findings.iter().filter(|f| f.severity == s).count();
        if n > 0 {
            md.push_str(&format!("| {s} | {n} |\n"));
        }
    }
    let sections = [("vuln", "Vulnerabilities"), ("port", "Open ports"), ("web", "Web servers"), ("subdomain", "Subdomains"), ("dns", "DNS records")];
    for (kind, title) in sections {
        let mut list: Vec<&Finding> = all.findings.iter().filter(|f| f.kind == kind).collect();
        if list.is_empty() {
            continue;
        }
        list.sort_by_key(|f| (order.iter().position(|s| *s == f.severity).unwrap_or(9), f.target.clone()));
        md.push_str(&format!("\n## {title} ({})\n\n| Severity | Target | Detail | Source |\n|---|---|---|---|\n", list.len()));
        for f in list {
            md.push_str(&format!("| {} | {} | {} | {} |\n", f.severity, md_cell(&f.target), md_cell(&f.detail), f.source));
        }
    }
    md.push_str("\n## Activity\n\n| When | Tool | Targets | Raw output |\n|---|---|---|---|\n");
    for r in &all.runs {
        md.push_str(&format!("| {} | {} | {} | {} |\n", date(r.time), md_cell(&r.tool), r.targets, md_cell(&r.report)));
    }
    Ok(md)
}

pub const SCOPE_TEMPLATE: &str = "# What you are authorized to test, one entry per line.\n#   example.com        only that host\n#   *.example.com      the domain and all subdomains\n#   10.0.0.0/24        a network\n#   !vpn.example.com   never test this one\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_rules() {
        let s = Scope::parse("*.example.com\nshop.test # comment\n10.0.0.0/24\n!admin.example.com\n!10.0.0.9\nhttps://app.other.org:8443/x\n2001:db8::/120\n");
        let t = |x: &str| parse_target(x).unwrap();
        assert!(s.allows(&t("example.com")) && s.allows(&t("a.b.example.com")) && s.allows(&t("https://www.example.com/login")));
        assert!(!s.allows(&t("admin.example.com")) && !s.allows(&t("x.admin.example.com")));
        assert!(!s.allows(&t("example.com.evil.net")) && !s.allows(&t("notexample.com")));
        assert!(s.allows(&t("shop.test")) && !s.allows(&t("www.shop.test")));
        assert!(s.allows(&t("10.0.0.5")) && !s.allows(&t("10.0.0.9")) && !s.allows(&t("10.0.1.1")));
        assert!(s.allows(&t("10.0.0.128/25")) && !s.allows(&t("10.0.0.0/25")) && !s.allows(&t("10.0.0.0/23")) && !s.allows(&t("10.0.0.8/29")));
        assert!(s.allows(&t("app.other.org")) && s.allows(&t("2001:db8::7")) && !s.allows(&t("2001:db9::1")));
        assert!(parse_target("-oX /etc/x").is_none() && parse_target("a;b").is_none());
    }

    #[test]
    fn plans_and_limits() {
        let d = std::env::temp_dir().join("rotae-tests").join("recon");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let targets = d.join("targets.txt");
        std::fs::write(&targets, "a.example.com\nevil.net\n").unwrap();
        assert!(plan(&targets).is_err(), "no scope.txt must refuse");
        std::fs::write(d.join(SCOPE_FILE), "*.example.com\n10.0.0.0/16\n").unwrap();
        let p = plan(&targets).unwrap();
        assert_eq!(p.targets, vec![Target::Name("a.example.com".into())]);
        assert_eq!(p.refused.len(), 1);
        std::fs::write(&targets, "10.0.0.0/23\n").unwrap();
        assert!(plan(&targets).is_err(), "more than 256 addresses must refuse");
        let from_scope = plan(&d.join(SCOPE_FILE));
        assert!(from_scope.is_err(), "the /16 in the scope is too wide to scan in one run");
        std::fs::write(d.join(SCOPE_FILE), "*.example.com\n10.0.0.0/28\n").unwrap();
        let p = plan(&d.join(SCOPE_FILE)).unwrap();
        assert_eq!(p.targets.len(), 2);
    }

    #[test]
    fn parsers_and_report() {
        let xml = r#"<nmaprun><host><address addr="10.0.0.5" addrtype="ipv4"/><hostnames><hostname name="a.example.com" type="user"/></hostnames><ports><port protocol="tcp" portid="22"><state state="open"/><service name="ssh" product="OpenSSH" version="9.6"/></port><port protocol="tcp" portid="23"><state state="closed"/></port></ports></host></nmaprun>"#;
        let f = parse_nmap(xml);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].target, "a.example.com (10.0.0.5):22/tcp");
        assert!(f[0].detail.contains("OpenSSH 9.6"));
        let n = parse_nuclei(r#"{"template-id":"git-config","info":{"name":"Git Config","severity":"medium"},"matched-at":"https://a.example.com/.git/config"}"#);
        assert_eq!((n[0].severity.as_str(), n[0].kind.as_str()), ("medium", "vuln"));
        let h = parse_httpx(r#"{"url":"https://a.example.com","status_code":200,"title":"Home","tech":["nginx"]}"#);
        assert!(h[0].detail.contains("200") && h[0].detail.contains("nginx"));
        let d = std::env::temp_dir().join("rotae-tests").join("recon-report");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(SCOPE_FILE), "*.example.com\n").unwrap();
        let run = Run { tool: "nmap".into(), time: 0, targets: 1, report: "x.txt".into() };
        assert_eq!(record(&d, run.clone(), [f, n, h].concat()).unwrap(), 3);
        assert_eq!(record(&d, run, parse_nmap(xml)).unwrap(), 0, "duplicates are kept once");
        let md = report(&d).unwrap();
        assert!(md.contains("## Vulnerabilities (1)") && md.contains("Git Config") && md.contains("| medium | 1 |"), "{md}");
        assert_eq!(date(0), "1970-01-01 00:00 UTC");
    }
}
