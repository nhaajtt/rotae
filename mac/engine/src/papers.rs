//! Paperwork: Word documents to and from Markdown, a look at what a document carries before it
//! is sent, lighter pictures inside Office files, and a redline of two versions of a text.
//! Like `desk`, it reads Office files as zips of XML, so nothing needs Office installed.
use crate::desk::{entry, names, ooxml, piece, read_text, slide_names, xml_escape, xml_text};
use crate::engines::note;
use crate::naming::{ext, out_dir, out_path, stem, temp_file, unique};
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const BAD: &str = "can't read this document: it may be damaged or password-protected";

pub fn run(paths: &[PathBuf], verb: &str, arg: &str) -> Option<Result<Vec<PathBuf>>> {
    if verb != "paper" {
        return None;
    }
    let src = paths.first()?;
    Some(match arg {
        "md" => to_markdown(src),
        "docx" => to_docx(src),
        "check" => check(src),
        "slim" => slim(src),
        "compare" => compare(paths),
        _ => return None,
    })
}

/// Group work sees every dropped file at once. Must agree with `isGroup` in the app.
pub fn is_group(verb: &str, arg: &str) -> bool {
    verb == "paper" && arg == "compare"
}

// ---------------------------------------------------------------- shared

/// An attribute by its name without the prefix: `w:val`, `r:id` and plain `val` all match.
fn av(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.local_name().as_ref() == name).and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

/// `<w:b/>` switches on, `<w:b w:val="0"/>` switches off.
fn on(e: &BytesStart) -> bool {
    !matches!(av(e, "val").as_deref(), Some("0" | "false" | "off" | "none"))
}

/// `r:id`, where the element has an `id` of its own as well.
fn rid(e: &BytesStart) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref().ends_with(":id")).and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

fn flag(e: &BytesStart, name: &str) -> bool {
    matches!(av(e, name).as_deref(), Some("1" | "true" | "on"))
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn each(xml: &str, mut f: impl FnMut(&BytesStart)) -> Result<()> {
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event().map_err(|_| anyhow!(BAD))? {
            Event::Start(e) | Event::Empty(e) => f(&e),
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// The text of every element with this name.
fn tag_text(xml: &str, tag: &str) -> Result<Vec<String>> {
    let mut r = Reader::from_str(xml);
    let (mut out, mut cur) = (Vec::new(), None::<String>);
    loop {
        match r.read_event().map_err(|_| anyhow!(BAD))? {
            Event::Start(e) if e.local_name().as_ref() == tag => cur = Some(String::new()),
            Event::End(e) if e.local_name().as_ref() == tag => out.extend(cur.take()),
            ev @ (Event::Text(_) | Event::GeneralRef(_)) => {
                if let Some(c) = cur.as_mut() {
                    c.push_str(&piece(&ev));
                }
            }
            Event::Eof => return Ok(out),
            _ => {}
        }
    }
}

struct Rel {
    target: String,
    external: bool,
    kind: String,
}

fn rels_of(xml: &str) -> HashMap<String, Rel> {
    let mut m = HashMap::new();
    let _ = each(xml, |e| {
        if e.local_name().as_ref() == "Relationship" {
            if let (Some(id), Some(target)) = (av(e, "Id"), av(e, "Target")) {
                let kind = av(e, "Type").unwrap_or_default().rsplit('/').next().unwrap_or("").to_string();
                m.insert(id, Rel { target, external: av(e, "TargetMode").is_some_and(|t| t.eq_ignore_ascii_case("external")), kind });
            }
        }
    });
    m
}

/// Where a relationship of the part in `dir` points inside the zip; `None` when it leaves it.
fn resolve(dir: &str, target: &str) -> Option<String> {
    let joined = if let Some(abs) = target.strip_prefix('/') { abs.to_string() } else { format!("{dir}/{target}") };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p if p.contains('\\') => return None,
            p => parts.push(p),
        }
    }
    Some(parts.join("/")).filter(|s| !s.is_empty())
}

/// Links worth keeping as links; anything else (file:, javascript:, a UNC path) stays text.
fn web(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|s| u.starts_with(s)) && !u.contains(['\n', '\r'])
}

fn clip(s: &str, max: usize) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() > max { clean.chars().take(max).collect::<String>() + "…" } else { clean }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------- docx → Markdown

#[derive(Default, Clone)]
struct Style {
    name: String,
    based: String,
    outline: Option<usize>,
    num: Option<(String, usize)>,
    bold: bool,
    italic: bool,
    mono: bool,
}

fn mono(font: &str) -> bool {
    let f = font.to_lowercase();
    ["courier", "consolas", "menlo", "monaco", "mono", "lucida console", "source code", "fira code", "cascadia"].iter().any(|m| f.contains(m))
}

fn styles(xml: &str) -> HashMap<String, Style> {
    let mut r = Reader::from_str(xml);
    let (mut out, mut cur) = (HashMap::new(), None::<(String, Style)>);
    while let Ok(ev) = r.read_event() {
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name();
                if name.as_ref() == "style" {
                    cur = Some((av(&e, "styleId").unwrap_or_default(), Style::default()));
                } else if let Some((_, s)) = cur.as_mut() {
                    match name.as_ref() {
                        "name" => s.name = av(&e, "val").unwrap_or_default().to_lowercase(),
                        "basedOn" => s.based = av(&e, "val").unwrap_or_default(),
                        "outlineLvl" => s.outline = av(&e, "val").and_then(|v| v.parse().ok()),
                        "numId" => s.num = Some((av(&e, "val").unwrap_or_default(), s.num.take().map_or(0, |n| n.1))),
                        "ilvl" => s.num = Some((s.num.take().map(|n| n.0).unwrap_or_default(), av(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(0))),
                        "b" => s.bold = on(&e),
                        "i" => s.italic = on(&e),
                        "rFonts" => s.mono = ["ascii", "hAnsi"].iter().any(|a| av(&e, a).is_some_and(|f| mono(&f))),
                        _ => {}
                    }
                }
            }
            Event::End(e) if e.local_name().as_ref() == "style" => out.extend(cur.take()),
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Which lists are bullets: numbering.xml says so per list and level.
#[derive(Default)]
struct Lists {
    levels: HashMap<(String, usize), bool>,
    link: HashMap<String, String>,
    over: HashMap<(String, usize), bool>,
}

impl Lists {
    fn read(xml: &str) -> Lists {
        let mut l = Lists::default();
        let (mut abs, mut num, mut lvl) = (None::<String>, None::<String>, 0usize);
        let mut r = Reader::from_str(xml);
        while let Ok(ev) = r.read_event() {
            match ev {
                Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                    "abstractNum" => (abs, num) = (av(&e, "abstractNumId"), None),
                    "num" => (num, abs) = (av(&e, "numId"), None),
                    "lvl" | "lvlOverride" => lvl = av(&e, "ilvl").and_then(|v| v.parse().ok()).unwrap_or(0),
                    "abstractNumId" => {
                        if let (Some(n), Some(a)) = (&num, av(&e, "val")) {
                            l.link.insert(n.clone(), a);
                        }
                    }
                    "numFmt" => {
                        let bullet = matches!(av(&e, "val").as_deref(), Some("bullet" | "none"));
                        if let Some(a) = &abs {
                            l.levels.insert((a.clone(), lvl), bullet);
                        } else if let Some(n) = &num {
                            l.over.insert((n.clone(), lvl), bullet);
                        }
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
        }
        l
    }

    fn bullet(&self, num: &str, lvl: usize) -> bool {
        let key = (num.to_string(), lvl);
        self.over.get(&key).copied().or_else(|| self.link.get(num).and_then(|a| self.levels.get(&(a.clone(), lvl)).copied())).unwrap_or(true)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Text,
    Heading(usize),
    Quote,
    Code,
}

/// Word keeps the English name of a built-in style ("heading 1") in every language; only the
/// id is localized, so the name is what decides.
fn by_name(name: &str) -> Option<Kind> {
    if name == "title" {
        return Some(Kind::Heading(1));
    }
    if let Some(n) = name.strip_prefix("heading").and_then(|n| n.trim().parse::<usize>().ok()) {
        return Some(Kind::Heading(n.clamp(1, 6)));
    }
    if name.contains("quot") || name == "block text" {
        return Some(Kind::Quote);
    }
    (name.contains("code") || name.contains("preformatted") || name.contains("verbatim")).then_some(Kind::Code)
}

/// Without styles.xml only the id is left: the ids Word writes in other languages.
fn by_id(id: &str) -> Kind {
    let id = id.to_lowercase();
    for p in ["heading", "berschrift", "titre", "ttulo", "titolo", "kop", "rubrik", "overskrift", "otsikko", "nagwek", "nadpis", "cmsor", "balk", "judul", "u"] {
        if let Some(n) = id.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()) {
            return Kind::Heading(n.clamp(1, 6));
        }
    }
    by_name(&id).unwrap_or(Kind::Text)
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Fmt {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    hidden: bool,
}

#[derive(Clone)]
struct Span {
    text: String,
    fmt: Fmt,
    link: Option<String>,
    /// Markdown already: a picture, a footnote mark.
    raw: bool,
}

#[derive(Default)]
struct Para {
    style: String,
    num: Option<String>,
    lvl: usize,
    rule: bool,
    spans: Vec<Span>,
}

enum Block {
    Text(String),
    Heading(usize, String),
    Quote(String),
    Code(String),
    /// depth, marker, text
    Item(usize, String, String),
    Table(Vec<Vec<String>>),
    Rule,
}

fn esc_md(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for (i, c) in s.char_indices() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '~' | '|' => o.extend(['\\', c]),
            // "&copy;" typed out in a document is those six characters, not ©
            '&' if s.get(i + 1..).and_then(|r| r.split_once(';')).is_some_and(|(name, _)| !name.is_empty() && name.len() < 32 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'#')) => o.push_str("\\&"),
            c => o.push(c),
        }
    }
    o
}

/// A line that would start a heading, quote, list or rule only because of its first characters.
fn guard(line: &str) -> String {
    let t = line.trim_start();
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    let after = t.get(digits..).unwrap_or("");
    if (1..=9).contains(&digits) && after.starts_with(['.', ')']) && after.get(1..).is_some_and(|r| r.is_empty() || r.starts_with(' ')) {
        return format!("{}\\{after}", t.get(..digits).unwrap_or(""));
    }
    let listy = t.starts_with(['-', '+']) && t.get(1..).is_some_and(|r| r.is_empty() || r.starts_with([' ', '-']));
    if t.starts_with(['#', '>', '=']) || listy { format!("\\{t}") } else { t.to_string() }
}

fn link_path(s: &str) -> String {
    let mut o = String::new();
    for c in s.trim().chars() {
        if c.is_control() || matches!(c, ' ' | '(' | ')' | '<' | '>' | '[' | ']' | '"' | '\\' | '`' | '\u{a0}') {
            let mut b = [0u8; 4];
            o.extend(c.encode_utf8(&mut b).bytes().map(|x| format!("%{x:02X}")));
        } else {
            o.push(c);
        }
    }
    o
}

fn edges(s: &str) -> (&str, &str, &str) {
    let core = s.trim();
    let lead = s.len() - s.trim_start().len();
    (s.get(..lead).unwrap_or(""), core, s.get(lead + core.len()..).unwrap_or(""))
}

fn styled(s: &Span) -> String {
    if s.raw {
        return s.text.clone();
    }
    let text = s.text.replace('\t', " ");
    let (lead, core, trail) = edges(&text);
    if core.is_empty() {
        return text;
    }
    if s.fmt.code {
        let core = core.replace('\n', " ");
        let longest = core.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let (fence, pad) = ("`".repeat(longest + 1), if core.starts_with('`') || core.ends_with('`') { " " } else { "" });
        return format!("{lead}{fence}{pad}{core}{pad}{fence}{trail}");
    }
    let open = format!("{}{}{}", if s.fmt.strike { "~~" } else { "" }, if s.fmt.bold { "**" } else { "" }, if s.fmt.italic { "*" } else { "" });
    let close: String = open.chars().rev().collect();
    format!("{lead}{open}{}{close}{trail}", esc_md(core))
}

/// One paragraph's runs as Markdown; a line break inside it is still `\n` here.
fn inline(spans: &[Span], plain: bool) -> String {
    let mut merged: Vec<Span> = Vec::new();
    for s in spans.iter().filter(|s| !s.fmt.hidden && !s.text.is_empty()) {
        let mut s = s.clone();
        s.fmt.bold &= !plain;
        match merged.last_mut() {
            Some(l) if !l.raw && !s.raw && l.fmt == s.fmt && l.link == s.link => l.text.push_str(&s.text),
            _ => merged.push(s),
        }
    }
    let mut out = String::new();
    let mut rest = merged.as_slice();
    while let Some(first) = rest.first() {
        let n = rest.iter().take_while(|s| s.link == first.link).count();
        let (group, tail) = rest.split_at(n);
        let body: String = group.iter().map(styled).collect();
        let (lead, core, trail) = edges(&body);
        match &first.link {
            Some(u) if !core.is_empty() => out += &format!("{lead}[{}]({}){trail}", core.replace('\n', " "), link_path(u)),
            _ => out += &body,
        }
        rest = tail;
    }
    out.trim().to_string()
}

fn lines(text: &str, first: &str, rest: &str) -> String {
    text.split('\n').enumerate().map(|(i, l)| format!("{}{}", if i == 0 { first } else { rest }, guard(l))).collect::<Vec<_>>().join("  \n")
}

fn md_table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let line = |r: &Vec<String>| format!("| {} |", (0..cols).map(|i| r.get(i).map(|c| c.replace('\n', "<br>")).unwrap_or_default()).collect::<Vec<_>>().join(" | "));
    let mut out: Vec<String> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        out.push(line(r));
        if i == 0 {
            out.push(format!("|{}", " --- |".repeat(cols)));
        }
    }
    out.join("\n")
}

fn assemble(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut prev: Option<&Block> = None;
    let mut code: Vec<&str> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if let Block::Code(c) = b {
            // one paragraph per line in the document, one fenced block here
            code.push(c);
            if matches!(blocks.get(i + 1), Some(Block::Code(_))) {
                continue;
            }
        }
        let s = match b {
            Block::Text(t) => lines(t, "", ""),
            Block::Heading(n, t) => {
                let t = guard(t);
                format!("{} {}", "#".repeat(*n), t.strip_suffix('#').map(|h| format!("{h}\\#")).unwrap_or(t))
            }
            Block::Quote(t) => lines(t, "> ", "> "),
            Block::Item(depth, marker, t) => {
                let first = format!("{}{marker} ", " ".repeat(depth * 4));
                lines(t, &first, &" ".repeat(first.len()))
            }
            Block::Table(rows) => md_table(rows),
            Block::Rule => "---".into(),
            Block::Code(_) => {
                let body = std::mem::take(&mut code).join("\n");
                let body = body.trim_end_matches('\n');
                let fence = "`".repeat(body.split(|c| c != '`').map(str::len).max().unwrap_or(0).max(2) + 1);
                format!("{fence}\n{body}\n{fence}")
            }
        };
        out += match (prev, b) {
            (None, _) => "",
            (Some(Block::Item(..)), Block::Item(..)) => "\n",
            (Some(Block::Quote(_)), Block::Quote(_)) => "\n>\n",
            _ => "\n\n",
        };
        out += &s;
        prev = Some(b);
    }
    out
}

/// The picture types a document may hand over; nothing else inside the zip is copied out.
const PICTURES: &[&str] = &["png", "jpg", "jpeg", "jfif", "gif", "bmp", "tif", "tiff", "webp", "svg", "emf", "wmf", "heic"];

#[derive(Default)]
struct Doc<'a> {
    /// `None` in tests: pictures are then named but not written.
    src: Option<&'a Path>,
    all: &'a [String],
    styles: HashMap<String, Style>,
    lists: Lists,
    counters: HashMap<String, Vec<u64>>,
    dir: Option<PathBuf>,
    saved: HashMap<String, String>,
    /// Footnotes in the order the text first mentions them.
    cited: Vec<String>,
}

impl Doc<'_> {
    fn kind(&self, id: &str) -> Kind {
        let mut id = id.to_string();
        for _ in 0..8 {
            let Some(s) = self.styles.get(&id) else { return by_id(&id) };
            if let Some(k) = by_name(&s.name) {
                return k;
            }
            if let Some(n) = s.outline.filter(|n| *n < 9) {
                return Kind::Heading((n + 1).min(6));
            }
            if s.based.is_empty() {
                break;
            }
            id = s.based.clone();
        }
        Kind::Text
    }

    fn style_num(&self, id: &str) -> Option<(String, usize)> {
        let mut id = id.to_string();
        for _ in 0..8 {
            let s = self.styles.get(&id)?;
            if s.num.is_some() {
                return s.num.clone();
            }
            id = s.based.clone();
        }
        None
    }

    fn cite(&mut self, key: String) -> String {
        let n = self.cited.iter().position(|k| *k == key).unwrap_or_else(|| {
            self.cited.push(key);
            self.cited.len() - 1
        });
        format!("[^{}]", n + 1)
    }

    /// Copies one picture out next to the result and returns the link to it. The name is the
    /// last part of the entry's own name: nothing inside the document chooses a folder.
    fn picture(&mut self, name: &str) -> Option<String> {
        if let Some(done) = self.saved.get(name) {
            return Some(done.clone());
        }
        let inside = Path::new(name);
        let base = inside.file_name()?.to_string_lossy().into_owned();
        if !self.all.iter().any(|n| n == name) || !PICTURES.contains(&ext(inside).as_str()) || base.starts_with('.') || self.saved.len() >= 2000 {
            return None;
        }
        let link = match self.src {
            Some(src) => {
                let bytes = entry(src, name, 256).ok().filter(|b| !b.is_empty())?;
                if self.dir.is_none() {
                    self.dir = Some(out_dir(src, "media").ok()?);
                }
                let dir = self.dir.as_ref()?;
                let out = unique(dir, &stem(inside), &ext(inside));
                std::fs::write(&out, bytes).ok()?;
                format!("{}/{}", file_name(dir), file_name(&out))
            }
            None => format!("media/{base}"),
        };
        self.saved.insert(name.to_string(), link.clone());
        Some(link)
    }

    fn image(&mut self, embed: Option<String>, linked: Option<String>, rels: &HashMap<String, Rel>, alt: &str) -> String {
        let alt = esc_md(&clip(alt, 200));
        let stored = embed.and_then(|id| rels.get(&id)).filter(|r| !r.external).and_then(|r| resolve("word", &r.target)).and_then(|n| self.picture(&n));
        // a picture the document only points at: kept as a pointer when it is on the web
        let remote = || linked.and_then(|id| rels.get(&id)).filter(|r| r.external && web(&r.target) && !r.target.starts_with("mailto")).map(|r| r.target.clone());
        match stored.or_else(remote) {
            Some(to) => format!("![{alt}]({})", link_path(&to)),
            None => alt,
        }
    }

    fn block(&mut self, p: Para, plain: bool, after: Option<usize>) -> Option<Block> {
        let kind = self.kind(&p.style);
        if kind == Kind::Code {
            return Some(Block::Code(p.spans.iter().filter(|s| !s.fmt.hidden).map(|s| s.text.as_str()).collect()));
        }
        let text = inline(&p.spans, plain || matches!(kind, Kind::Heading(_)));
        if text.is_empty() {
            return p.rule.then_some(Block::Rule);
        }
        let num = match p.num {
            Some(n) => Some((n, p.lvl)),
            None => self.style_num(&p.style),
        };
        Some(match (kind, num.filter(|n| n.0 != "0" && !n.0.is_empty())) {
            (Kind::Heading(n), _) => Block::Heading(n, text.replace('\n', " ")),
            (_, Some((num, lvl))) => {
                let lvl = lvl.min(8);
                let marker = if self.lists.bullet(&num, lvl) {
                    "-".to_string()
                } else {
                    let c = self.counters.entry(num).or_default();
                    c.resize(lvl + 1, 0);
                    let n = c.last_mut().map(|n| {
                        *n += 1;
                        *n
                    });
                    format!("{}.", n.unwrap_or(1))
                };
                // a list that starts three levels in would read as an indented code block
                Block::Item(lvl.min(after.map_or(0, |d| d + 1)), marker, text)
            }
            (Kind::Quote, _) => Block::Quote(text),
            _ => Block::Text(text),
        })
    }

    /// One XML part as blocks. `wrap` names the element that holds each footnote; the document
    /// itself has none and comes back as a single group.
    fn walk(&mut self, xml: &str, rels: &HashMap<String, Rel>, wrap: &str) -> Result<Vec<(String, Vec<Block>)>> {
        let bad = |_| anyhow!(BAD);
        let mut r = Reader::from_str(xml);
        let mut groups: Vec<(String, Vec<Block>)> = if wrap.is_empty() { vec![(String::new(), Vec::new())] } else { Vec::new() };
        let mut paras: Vec<Para> = Vec::new();
        let (mut in_run, mut in_rpr, mut in_ppr, mut in_bdr, mut in_t, mut in_instr) = (false, false, false, false, false, false);
        let mut fmt = Fmt::default();
        let (mut link, mut field, mut instr) = (None::<String>, None::<String>, None::<String>);
        let mut alt = String::new();
        let (mut depth, mut rows, mut cell, mut span) = (0usize, Vec::<Vec<String>>::new(), None::<Vec<String>>, 1usize);
        let push = |paras: &mut Vec<Para>, text: &str, fmt: Fmt, link: Option<&String>, raw: bool| {
            if let Some(p) = paras.last_mut() {
                p.spans.push(Span { text: text.to_string(), fmt, link: link.cloned(), raw });
            }
        };
        loop {
            let ev = r.read_event().map_err(bad)?;
            match &ev {
                Event::Start(e) | Event::Empty(e) => {
                    let open = matches!(ev, Event::Start(_));
                    let name = e.local_name();
                    match name.as_ref() {
                        // deleted text, the older half of a move, the legacy copy of a text box,
                        // and settings whose children share names with what is read below
                        "del" | "moveFrom" | "Fallback" | "pPrChange" | "rPrChange" | "sectPr" | "tblPr" | "tblGrid" | "trPr" if open => {
                            r.read_to_end(e.name()).map_err(bad)?;
                        }
                        "p" if open => paras.push(Para::default()),
                        "pPr" if open => in_ppr = true,
                        "pBdr" if open => in_bdr = true,
                        "r" if open => (in_run, fmt) = (true, Fmt::default()),
                        "rPr" if open && in_run => in_rpr = true,
                        "t" if open => in_t = true,
                        "instrText" if open => in_instr = true,
                        "bottom" if in_bdr => {
                            if let Some(p) = paras.last_mut() {
                                p.rule = true;
                            }
                        }
                        "pStyle" | "numId" | "ilvl" if in_ppr => {
                            if let (Some(p), Some(v)) = (paras.last_mut(), av(e, "val")) {
                                match name.as_ref() {
                                    "pStyle" => p.style = v,
                                    "numId" => p.num = Some(v),
                                    _ => p.lvl = v.parse().unwrap_or(0),
                                }
                            }
                        }
                        "b" if in_rpr => fmt.bold = on(e),
                        "i" if in_rpr => fmt.italic = on(e),
                        "strike" | "dstrike" if in_rpr => fmt.strike = on(e),
                        "vanish" if in_rpr => fmt.hidden = on(e),
                        "rFonts" if in_rpr => fmt.code |= ["ascii", "hAnsi"].iter().any(|a| av(e, a).is_some_and(|f| mono(&f))),
                        "rStyle" if in_rpr => {
                            if let Some(s) = av(e, "val").and_then(|v| self.styles.get(&v)) {
                                fmt.bold |= s.bold;
                                fmt.italic |= s.italic;
                                fmt.code |= s.mono || s.name.contains("code") || s.name.contains("verbatim");
                            }
                        }
                        "tab" | "ptab" if in_run && !in_rpr => push(&mut paras, "\t", fmt, None, false),
                        "br" | "cr" if in_run && !in_rpr && !matches!(av(e, "type").as_deref(), Some("page" | "column")) => push(&mut paras, "\n", fmt, None, false),
                        "noBreakHyphen" if in_run => push(&mut paras, "-", fmt, link.as_ref().or(field.as_ref()), false),
                        "hyperlink" if open => link = av(e, "id").and_then(|id| rels.get(&id)).filter(|x| x.external && web(&x.target)).map(|x| x.target.clone()),
                        "fldSimple" if open => field = av(e, "instr").and_then(|i| field_link(&i)),
                        "fldChar" => match av(e, "fldCharType").as_deref() {
                            Some("begin") => instr = Some(String::new()),
                            Some("separate") => field = instr.take().and_then(|i| field_link(&i)),
                            _ => (field, instr) = (None, None),
                        },
                        "footnoteReference" | "endnoteReference" if in_run => {
                            if let Some(id) = av(e, "id") {
                                let mark = self.cite(format!("{}{id}", name.as_ref().get(..1).unwrap_or("f")));
                                push(&mut paras, &mark, fmt, None, true);
                            }
                        }
                        "docPr" => alt = av(e, "descr").unwrap_or_default(),
                        "blip" | "imagedata" => {
                            let (embed, linked) = if name.as_ref() == "blip" { (av(e, "embed"), av(e, "link")) } else { (av(e, "id"), None) };
                            if embed.is_some() || linked.is_some() {
                                let md = self.image(embed, linked, rels, &std::mem::take(&mut alt));
                                push(&mut paras, &md, fmt, link.as_ref().or(field.as_ref()), true);
                            }
                        }
                        "tbl" if open => {
                            depth += 1;
                            if depth == 1 {
                                rows.clear();
                            }
                        }
                        "tr" if open && depth == 1 => rows.push(Vec::new()),
                        "tc" if open && depth == 1 => (cell, span) = (Some(Vec::new()), 1),
                        "gridSpan" if depth == 1 => span = av(e, "val").and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 63),
                        n if open && !wrap.is_empty() && n == wrap => groups.push((av(e, "id").unwrap_or_default(), Vec::new())),
                        _ => {}
                    }
                }
                Event::Text(_) | Event::GeneralRef(_) if in_t && in_run => push(&mut paras, &piece(&ev), fmt, link.as_ref().or(field.as_ref()), false),
                Event::Text(_) | Event::GeneralRef(_) if in_instr => {
                    if let Some(i) = instr.as_mut() {
                        i.push_str(&piece(&ev));
                    }
                }
                Event::End(e) => match e.local_name().as_ref() {
                    "t" => in_t = false,
                    "instrText" => in_instr = false,
                    "r" => (in_run, in_rpr) = (false, false),
                    "rPr" => in_rpr = false,
                    "pPr" => in_ppr = false,
                    "pBdr" => in_bdr = false,
                    "hyperlink" => link = None,
                    "fldSimple" => field = None,
                    "p" => {
                        let Some(p) = paras.pop() else { continue };
                        if let Some(c) = cell.as_mut() {
                            // the first row becomes the header, which is bold by itself
                            let line = match self.block(p, rows.len() == 1, None) {
                                Some(Block::Text(t) | Block::Heading(_, t) | Block::Quote(t)) => t,
                                Some(Block::Item(_, m, t)) => format!("{m} {t}"),
                                Some(Block::Code(t)) if !t.trim().is_empty() => format!("`{}`", t.trim().replace('`', "'").replace('\n', "`\n`")),
                                _ => String::new(),
                            };
                            if !line.is_empty() {
                                c.push(line.split('\n').map(guard).collect::<Vec<_>>().join("\n"));
                            }
                        } else if let Some((_, blocks)) = groups.last_mut() {
                            let after = match blocks.last() {
                                Some(Block::Item(d, ..)) => Some(*d),
                                _ => None,
                            };
                            blocks.extend(self.block(p, false, after));
                        }
                    }
                    "tc" if depth == 1 => {
                        if let (Some(c), Some(row)) = (cell.take(), rows.last_mut()) {
                            row.push(c.join("\n"));
                            row.extend((1..span).map(|_| String::new()));
                        }
                    }
                    "tbl" => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 && rows.iter().any(|r| !r.is_empty()) {
                            if let Some((_, blocks)) = groups.last_mut() {
                                blocks.push(Block::Table(std::mem::take(&mut rows)));
                            }
                        }
                    }
                    _ => {}
                },
                Event::Eof => return Ok(groups),
                _ => {}
            }
        }
    }
}

/// The address in a `HYPERLINK "https://…"` field.
fn field_link(instr: &str) -> Option<String> {
    let rest = instr.trim().strip_prefix("HYPERLINK")?;
    let url = rest.split('"').nth(1)?;
    web(url).then(|| url.to_string())
}

fn markdown_of(doc: &mut Doc, xml: &str, rels: &HashMap<String, Rel>, notes: &[(&str, String, HashMap<String, Rel>)]) -> Result<String> {
    let body = doc.walk(xml, rels, "")?;
    let mut md = body.first().map(|g| assemble(&g.1)).unwrap_or_default();
    let mut found: HashMap<String, Vec<Block>> = HashMap::new();
    for (wrap, part, part_rels) in notes {
        for (id, blocks) in doc.walk(part, part_rels, wrap)? {
            found.insert(format!("{}{id}", wrap.get(..1).unwrap_or("f")), blocks);
        }
    }
    // a footnote can cite nothing new, so the list is final here
    for (i, key) in doc.cited.clone().iter().enumerate() {
        let text = found.get(key).map(|b| assemble(b)).unwrap_or_default();
        md += &format!("\n\n[^{}]: {}", i + 1, text.replace('\n', "\n    "));
    }
    Ok(md.trim().to_string() + "\n")
}

fn to_markdown(src: &Path) -> Result<Vec<PathBuf>> {
    if !matches!(ext(src).as_str(), "docx" | "docm") {
        bail!("this works on .docx files; save older formats as .docx first");
    }
    let all = names(src)?;
    let has = |n: &str| all.iter().any(|a| a == n);
    if !has("word/document.xml") {
        bail!(BAD);
    }
    let part = |n: &str| if has(n) { xml_text(src, n).unwrap_or_default() } else { String::new() };
    let mut doc = Doc { src: Some(src), all: &all, styles: styles(&part("word/styles.xml")), lists: Lists::read(&part("word/numbering.xml")), ..Default::default() };
    let notes: Vec<(&str, String, HashMap<String, Rel>)> =
        ["footnote", "endnote"].iter().map(|w| (*w, part(&format!("word/{w}s.xml")), rels_of(&part(&format!("word/_rels/{w}s.xml.rels"))))).collect();
    let md = markdown_of(&mut doc, &xml_text(src, "word/document.xml")?, &rels_of(&part("word/_rels/document.xml.rels")), &notes)?;
    if md.trim().is_empty() {
        bail!("no text found in this document");
    }
    let out = out_path(src, None, "md");
    std::fs::write(&out, md)?;
    let mut made = vec![out];
    if let Some(dir) = doc.dir {
        note(&format!("{} saved in “{}”", plural(doc.saved.len(), "picture", "pictures"), file_name(&dir)));
        made.push(dir);
    }
    Ok(made)
}

// ---------------------------------------------------------------- Markdown → docx

const W_NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
/// A4 with 2.54 cm margins leaves this much across, in twentieths of a point.
const TEXT_WIDTH: usize = 9026;
const RULE: &str = "<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"auto\"/></w:pBdr></w:pPr></w:p>";

fn text_runs(rpr: &str, s: &str) -> String {
    let mut x = format!("<w:r>{rpr}");
    let mut part = String::new();
    let put = |x: &mut String, part: &mut String| {
        if !part.is_empty() {
            *x += &format!("<w:t xml:space=\"preserve\">{}</w:t>", xml_escape(part));
            part.clear();
        }
    };
    for c in s.chars() {
        match c {
            '\n' | '\t' => {
                put(&mut x, &mut part);
                x += if c == '\n' { "<w:br/>" } else { "<w:tab/>" };
            }
            '\r' => {}
            c => part.push(c),
        }
    }
    put(&mut x, &mut part);
    x + "</w:r>"
}

/// Rows of a table being read: is it the header, and each cell's runs.
type TableRows = Vec<(bool, Vec<String>)>;

/// The body of a Word document being written, one Markdown event at a time.
#[derive(Default)]
struct Writer {
    body: String,
    links: Vec<String>,
    /// One per list in the text: numbered or not, first number, level.
    nums: Vec<(bool, u64, usize)>,
    images: usize,
    para: Option<String>,
    heading: Option<usize>,
    quote: usize,
    lists: Vec<usize>,
    /// The list item whose number has not been given to a paragraph yet.
    pending: Option<(usize, usize)>,
    link: Option<usize>,
    code: Option<String>,
    bold: u32,
    italic: u32,
    strike: u32,
    head: bool,
    last_table: bool,
}

impl Writer {
    fn text(&mut self, s: &str, code: bool) {
        let mut rpr = String::new();
        if code {
            rpr += "<w:rStyle w:val=\"VerbatimChar\"/>";
        } else if self.link.is_some() {
            rpr += "<w:rStyle w:val=\"Hyperlink\"/>";
        }
        for (set, tag) in [(self.bold > 0 || self.head, "<w:b/><w:bCs/>"), (self.italic > 0, "<w:i/><w:iCs/>"), (self.strike > 0, "<w:strike/>")] {
            if set {
                rpr += tag;
            }
        }
        if !rpr.is_empty() {
            rpr = format!("<w:rPr>{rpr}</w:rPr>");
        }
        let runs = text_runs(&rpr, s);
        let runs = match self.link {
            Some(n) => format!("<w:hyperlink r:id=\"rId{}\" w:history=\"1\">{runs}</w:hyperlink>", n + FIXED_RELS),
            None => runs,
        };
        self.para.get_or_insert_with(String::new).push_str(&runs);
    }

    fn block(&mut self, xml: &str, table: bool) {
        // Word joins two tables that touch, and wants a paragraph after the last one
        if table && self.last_table {
            self.body += "<w:p/>";
        }
        self.body += xml;
        self.last_table = table;
    }

    fn indent(&self) -> usize {
        self.lists.len() * 720 + self.quote.saturating_sub(1) * 720
    }

    fn flush(&mut self) {
        let Some(runs) = self.para.take() else { return };
        let mut ppr = String::new();
        let numbered = if self.heading.is_none() { self.pending.take() } else { None };
        if let Some(n) = self.heading {
            ppr += &format!("<w:pStyle w:val=\"Heading{}\"/>", n.clamp(1, 6));
        } else if self.quote > 0 {
            ppr += "<w:pStyle w:val=\"Quote\"/>";
        } else if !self.lists.is_empty() {
            ppr += "<w:pStyle w:val=\"ListParagraph\"/>";
        }
        match numbered {
            Some((num, lvl)) => ppr += &format!("<w:numPr><w:ilvl w:val=\"{lvl}\"/><w:numId w:val=\"{num}\"/></w:numPr>"),
            // more paragraphs of the same item line up under its text
            None if self.heading.is_none() && self.indent() > 0 => ppr += &format!("<w:ind w:left=\"{}\"/>", self.indent()),
            None => {}
        }
        if !ppr.is_empty() {
            ppr = format!("<w:pPr>{ppr}</w:pPr>");
        }
        self.block(&format!("<w:p>{ppr}{runs}</w:p>"), false);
    }

    /// Before a block that is not a paragraph: an item that starts with one still gets its bullet.
    fn lead(&mut self) {
        if self.pending.is_some() {
            self.para.get_or_insert_with(String::new);
        }
        self.flush();
    }

    fn table(&mut self, aligns: &[pulldown_cmark::Alignment], rows: &[(bool, Vec<String>)]) {
        use pulldown_cmark::Alignment;
        let cols = rows.iter().map(|r| r.1.len()).max().unwrap_or(0).clamp(1, 63);
        let w = TEXT_WIDTH / cols;
        let mut x = String::from("<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"1\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>");
        x += &format!("<w:gridCol w:w=\"{w}\"/>").repeat(cols);
        x += "</w:tblGrid>";
        for (head, cells) in rows {
            x += if *head { "<w:tr><w:trPr><w:tblHeader/></w:trPr>" } else { "<w:tr>" };
            for c in 0..cols {
                let jc = match aligns.get(c) {
                    Some(Alignment::Center) => "<w:pPr><w:jc w:val=\"center\"/></w:pPr>",
                    Some(Alignment::Right) => "<w:pPr><w:jc w:val=\"right\"/></w:pPr>",
                    _ => "",
                };
                x += &format!("<w:tc><w:tcPr><w:tcW w:w=\"{w}\" w:type=\"dxa\"/></w:tcPr><w:p>{jc}{}</w:p></w:tc>", cells.get(c).map(String::as_str).unwrap_or(""));
            }
            x += "</w:tr>";
        }
        self.block(&(x + "</w:tbl>"), true);
    }
}

fn is_br(html: &str) -> bool {
    let t = html.trim().to_ascii_lowercase();
    t.strip_prefix("<br").is_some_and(|r| matches!(r.trim(), ">" | "/>"))
}

/// Markdown as the body of a Word document. `hard` keeps every line of a plain text file on
/// its own line; Markdown joins them into paragraphs.
fn build(md: &str, hard: bool) -> Writer {
    use pulldown_cmark::{Event as E, Options, Parser, Tag, TagEnd};
    let mut w = Writer::default();
    let mut table: Option<(Vec<pulldown_cmark::Alignment>, TableRows)> = None;
    let (mut row, mut html_lines) = (Vec::<String>::new(), 0usize);
    for ev in Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS) {
        match ev {
            E::Start(tag) => match tag {
                Tag::Paragraph => w.flush(),
                Tag::HtmlBlock => {
                    w.flush();
                    html_lines = 0;
                }
                Tag::Heading { level, .. } => {
                    w.flush();
                    w.heading = Some(level as usize);
                }
                Tag::BlockQuote(_) => {
                    w.flush();
                    w.quote += 1;
                }
                Tag::CodeBlock(_) => {
                    w.lead();
                    w.code = Some(String::new());
                }
                Tag::List(start) => {
                    w.flush();
                    w.nums.push((start.is_some(), start.unwrap_or(1), w.lists.len().min(8)));
                    w.lists.push(w.nums.len());
                }
                Tag::Item => {
                    w.flush();
                    w.pending = w.lists.last().map(|n| (*n, (w.lists.len() - 1).min(8)));
                }
                Tag::Table(aligns) => {
                    w.lead();
                    table = Some((aligns, Vec::new()));
                }
                Tag::TableHead => {
                    w.head = true;
                    row.clear();
                }
                Tag::TableRow => row.clear(),
                Tag::TableCell => w.para = Some(String::new()),
                Tag::Emphasis => w.italic += 1,
                Tag::Strong => w.bold += 1,
                Tag::Strikethrough => w.strike += 1,
                Tag::Link { dest_url, .. } => {
                    w.link = web(&dest_url).then(|| {
                        w.links.push(dest_url.trim().to_string());
                        w.links.len()
                    })
                }
                Tag::Image { .. } => w.images += 1,
                _ => {}
            },
            E::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::HtmlBlock => w.flush(),
                TagEnd::Heading(_) => {
                    w.flush();
                    w.heading = None;
                }
                TagEnd::BlockQuote(_) => {
                    w.flush();
                    w.quote = w.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    let code = w.code.take().unwrap_or_default();
                    let ind = if w.indent() > 0 { format!("<w:ind w:left=\"{}\"/>", w.indent()) } else { String::new() };
                    w.block(&format!("<w:p><w:pPr><w:pStyle w:val=\"SourceCode\"/>{ind}</w:pPr>{}</w:p>", text_runs("", code.trim_end_matches(['\n', '\r']))), false);
                }
                TagEnd::List(_) => {
                    w.flush();
                    w.lists.pop();
                }
                TagEnd::Item => {
                    w.lead();
                    w.pending = None;
                }
                TagEnd::TableCell => row.push(w.para.take().unwrap_or_default()),
                TagEnd::TableHead | TagEnd::TableRow => {
                    if let Some(t) = table.as_mut() {
                        t.1.push((w.head, std::mem::take(&mut row)));
                    }
                    w.head = false;
                }
                TagEnd::Table => {
                    if let Some((aligns, rows)) = table.take() {
                        w.table(&aligns, &rows);
                    }
                }
                TagEnd::Emphasis => w.italic = w.italic.saturating_sub(1),
                TagEnd::Strong => w.bold = w.bold.saturating_sub(1),
                TagEnd::Strikethrough => w.strike = w.strike.saturating_sub(1),
                TagEnd::Link => w.link = None,
                _ => {}
            },
            E::Text(t) => match w.code.as_mut() {
                Some(c) => c.push_str(&t),
                None => w.text(&t, false),
            },
            E::Code(t) => w.text(&t, true),
            // HTML in Markdown is shown as the text it is; only a line break is honoured
            E::InlineHtml(t) => w.text(if is_br(&t) { "\n" } else { &t }, false),
            E::Html(t) => {
                if html_lines > 0 {
                    w.text("\n", false);
                }
                html_lines += 1;
                w.text(t.trim_end_matches(['\n', '\r']), false);
            }
            E::SoftBreak => w.text(if hard { "\n" } else { " " }, false),
            E::HardBreak => w.text("\n", false),
            E::Rule => {
                    w.lead();
                    w.block(RULE, false);
                }
            E::TaskListMarker(done) => w.text(if done { "☑ " } else { "☐ " }, false),
            _ => {}
        }
    }
    w.flush();
    if w.last_table {
        w.body += "<w:p/>";
    }
    w
}

/// Relationships every document made here has before its links: styles, numbering, settings, theme.
const FIXED_RELS: usize = 4;

fn numbering_xml(nums: &[(bool, u64, usize)]) -> String {
    let mut x = format!("{XML}<w:numbering {W_NS}>");
    for (id, bullets) in [(0, true), (1, false)] {
        x += &format!("<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>");
        for l in 0..9usize {
            let (fmt, text) = if bullets { ("bullet", ["•", "◦", "▪"][l % 3].to_string()) } else { ("decimal", format!("%{}.", l + 1)) };
            x += &format!("<w:lvl w:ilvl=\"{l}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr></w:lvl>", 720 * (l + 1));
        }
        x += "</w:abstractNum>";
    }
    // a list of its own for every list in the text, so each starts counting where it says
    for (i, (ordered, start, lvl)) in nums.iter().enumerate() {
        x += &format!("<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/>", i + 1, if *ordered { 1 } else { 0 });
        if *ordered {
            x += &format!("<w:lvlOverride w:ilvl=\"{lvl}\"><w:startOverride w:val=\"{}\"/></w:lvlOverride>", (*start).min(32_000));
        }
        x += "</w:num>";
    }
    x + "</w:numbering>"
}

/// Built-in style ids with fonts and colours taken from the theme, so choosing another theme or
/// style set in Word restyles the whole document.
fn styles_xml() -> String {
    let mut x = format!("{XML}<w:styles {W_NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme=\"minorHAnsi\" w:eastAsiaTheme=\"minorEastAsia\" w:hAnsiTheme=\"minorHAnsi\" w:cstheme=\"minorBidi\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"259\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>");
    x += "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>";
    for (i, size) in [40, 32, 28, 24, 22, 22].iter().enumerate() {
        let italic = if i == 3 || i == 5 { "<w:i/><w:iCs/>" } else { "" };
        x += &format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{n}\"><w:name w:val=\"heading {n}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"{before}\" w:after=\"80\"/><w:outlineLvl w:val=\"{i}\"/></w:pPr><w:rPr><w:rFonts w:asciiTheme=\"majorHAnsi\" w:eastAsiaTheme=\"majorEastAsia\" w:hAnsiTheme=\"majorHAnsi\" w:cstheme=\"majorBidi\"/>{italic}<w:color w:val=\"2F5496\" w:themeColor=\"accent1\" w:themeShade=\"BF\"/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>",
            n = i + 1,
            before = if i == 0 { 360 } else { 160 }
        );
    }
    x += "<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"29\"/><w:qFormat/><w:pPr><w:spacing w:before=\"160\"/><w:ind w:left=\"720\" w:right=\"720\"/></w:pPr><w:rPr><w:i/><w:iCs/><w:color w:val=\"404040\" w:themeColor=\"text1\" w:themeTint=\"BF\"/></w:rPr></w:style>";
    x += "<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"34\"/><w:qFormat/><w:pPr><w:ind w:left=\"720\"/><w:contextualSpacing/></w:pPr></w:style>";
    x += "<w:style w:type=\"paragraph\" w:customStyle=\"1\" w:styleId=\"SourceCode\"><w:name w:val=\"Source Code\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F2F2F2\"/><w:spacing w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Courier New\" w:hAnsi=\"Courier New\" w:cs=\"Courier New\"/><w:noProof/><w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:style>";
    x += "<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\"><w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/><w:unhideWhenUsed/></w:style>";
    x += "<w:style w:type=\"character\" w:customStyle=\"1\" w:styleId=\"VerbatimChar\"><w:name w:val=\"Verbatim Char\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:qFormat/><w:rPr><w:rFonts w:ascii=\"Courier New\" w:hAnsi=\"Courier New\" w:cs=\"Courier New\"/><w:noProof/><w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:style>";
    x += "<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:rPr><w:color w:val=\"0563C1\" w:themeColor=\"hyperlink\"/><w:u w:val=\"single\"/></w:rPr></w:style>";
    x += "<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:uiPriority w:val=\"99\"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>";
    x += "<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:basedOn w:val=\"TableNormal\"/><w:uiPriority w:val=\"39\"/><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:tblPr><w:tblBorders>";
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        x += &format!("<w:{side} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>");
    }
    x + "</w:tblBorders></w:tblPr></w:style><w:style w:type=\"numbering\" w:default=\"1\" w:styleId=\"NoList\"><w:name w:val=\"No List\"/><w:uiPriority w:val=\"99\"/><w:semiHidden/><w:unhideWhenUsed/></w:style></w:styles>"
}

/// The smallest theme the format allows: the colours and fonts the styles above refer to.
fn theme_xml() -> String {
    let mut x = format!("{XML}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office Theme\"><a:themeElements><a:clrScheme name=\"Office\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>");
    for (name, rgb) in [("dk2", "44546A"), ("lt2", "E7E6E6"), ("accent1", "4472C4"), ("accent2", "ED7D31"), ("accent3", "A5A5A5"), ("accent4", "FFC000"), ("accent5", "5B9BD5"), ("accent6", "70AD47"), ("hlink", "0563C1"), ("folHlink", "954F72")] {
        x += &format!("<a:{name}><a:srgbClr val=\"{rgb}\"/></a:{name}>");
    }
    x += "</a:clrScheme><a:fontScheme name=\"Office\"><a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"Office\">";
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    x += &format!("<a:fillStyleLst>{}</a:fillStyleLst><a:lnStyleLst>", fill.repeat(3));
    for w in [6350, 12700, 19050] {
        x += &format!("<a:ln w=\"{w}\">{fill}</a:ln>");
    }
    x + &format!("</a:lnStyleLst><a:effectStyleLst>{}</a:effectStyleLst><a:bgFillStyleLst>{}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>", "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3), fill.repeat(3))
}

fn to_docx(src: &Path) -> Result<Vec<PathBuf>> {
    if !matches!(ext(src).as_str(), "md" | "markdown" | "txt") {
        bail!("this works on Markdown and plain text files (.md, .markdown, .txt)");
    }
    if std::fs::metadata(src)?.len() > 20 << 20 {
        bail!("this file is too big for this (over 20 MB)");
    }
    let w = build(&read_text(src)?, ext(src) == "txt");
    if w.body.is_empty() {
        bail!("no text found in this file");
    }
    let dir = temp_file("docx");
    let out = out_path(src, None, "docx");
    let made = (|| -> Result<()> {
        for d in ["_rels", "word/_rels", "word/theme"] {
            std::fs::create_dir_all(dir.join(d))?;
        }
        let part = |kind: &str| format!("application/vnd.openxmlformats-officedocument.{kind}+xml");
        let mut types = format!("{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>");
        for (name, kind) in [("word/document.xml", "wordprocessingml.document.main"), ("word/styles.xml", "wordprocessingml.styles"), ("word/numbering.xml", "wordprocessingml.numbering"), ("word/settings.xml", "wordprocessingml.settings"), ("word/theme/theme1.xml", "theme")] {
            types += &format!("<Override PartName=\"/{name}\" ContentType=\"{}\"/>", part(kind));
        }
        std::fs::write(dir.join("[Content_Types].xml"), types + "</Types>")?;
        let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        std::fs::write(dir.join("_rels/.rels"), format!("{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{rel}/officeDocument\" Target=\"word/document.xml\"/></Relationships>"))?;
        let mut rels = format!("{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
        for (i, (kind, target)) in [("styles", "styles.xml"), ("numbering", "numbering.xml"), ("settings", "settings.xml"), ("theme", "theme/theme1.xml")].iter().enumerate() {
            rels += &format!("<Relationship Id=\"rId{}\" Type=\"{rel}/{kind}\" Target=\"{target}\"/>", i + 1);
        }
        for (i, url) in w.links.iter().enumerate() {
            rels += &format!("<Relationship Id=\"rId{}\" Type=\"{rel}/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>", i + 1 + FIXED_RELS, xml_escape(url));
        }
        std::fs::write(dir.join("word/_rels/document.xml.rels"), rels + "</Relationships>")?;
        std::fs::write(dir.join("word/document.xml"), format!("{XML}<w:document {W_NS}><w:body>{}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>", w.body))?;
        std::fs::write(dir.join("word/styles.xml"), styles_xml())?;
        std::fs::write(dir.join("word/numbering.xml"), numbering_xml(&w.nums))?;
        // without this Word opens the file in compatibility mode
        std::fs::write(dir.join("word/settings.xml"), format!("{XML}<w:settings {W_NS}><w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>"))?;
        std::fs::write(dir.join("word/theme/theme1.xml"), theme_xml())?;
        let packed = dir.join("out.zip");
        let o = crate::engines::tools::output_within(Command::new("/usr/bin/zip").current_dir(&dir).args(["-q", "-X", "-r"]).arg(&packed).args(["[Content_Types].xml", "_rels", "word"]), Duration::from_secs(300))?;
        if !o.status.success() {
            bail!("couldn't write the document: {}", String::from_utf8_lossy(&o.stderr).trim());
        }
        std::fs::copy(&packed, &out)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    made?;
    if w.images > 0 {
        note(&format!("{} left out: Markdown only points to them", plural(w.images, "picture", "pictures")));
    }
    Ok(vec![out])
}

// ---------------------------------------------------------------- before you send it

/// A web address as its host; a path or anything else as it is written.
fn place(target: &str) -> String {
    let t = target.trim();
    let lower = t.to_ascii_lowercase();
    if let Some(i) = ["http://", "https://"].iter().find(|s| lower.starts_with(**s)).map(|s| s.len()) {
        let host = t.get(i..).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
        return clip(host.rsplit('@').next().unwrap_or(host), 80);
    }
    clip(t, 80)
}

fn listing(items: &BTreeSet<String>) -> String {
    let shown: Vec<&str> = items.iter().take(6).map(String::as_str).collect();
    let more = items.len() - shown.len();
    format!("{}{}", shown.join(", "), if more > 0 { format!(" and {more} more") } else { String::new() })
}

fn by(authors: &BTreeSet<String>) -> String {
    if authors.is_empty() { String::new() } else { format!(" (by {})", listing(authors)) }
}

#[derive(Default)]
struct Revisions {
    ins: usize,
    del: usize,
    moved: usize,
    restyled: usize,
    hidden: usize,
    sample: String,
    authors: BTreeSet<String>,
}

/// Tracked changes and hidden text in one part of a Word document.
fn revisions(xml: &str, f: &mut Revisions) -> Result<()> {
    let mut r = Reader::from_str(xml);
    let (mut in_run, mut in_rpr, mut in_t, mut vanish, mut counted) = (false, false, false, false, false);
    loop {
        match r.read_event().map_err(|_| anyhow!(BAD))? {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name();
                let author = || av(&e, "author").map(|a| clip(&a, 40)).filter(|a| !a.is_empty());
                match name.as_ref() {
                    "ins" | "del" | "moveFrom" if av(&e, "author").is_some() => {
                        match name.as_ref() {
                            "ins" => f.ins += 1,
                            "del" => f.del += 1,
                            _ => f.moved += 1,
                        }
                        f.authors.extend(author());
                    }
                    n if n.ends_with("PrChange") => {
                        f.restyled += 1;
                        f.authors.extend(author());
                    }
                    "r" => (in_run, vanish, counted) = (true, false, false),
                    "rPr" if in_run => in_rpr = true,
                    "vanish" if in_rpr => vanish = on(&e),
                    "t" => in_t = true,
                    _ => {}
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                "r" => (in_run, in_rpr) = (false, false),
                "rPr" => in_rpr = false,
                "t" => in_t = false,
                _ => {}
            },
            ev @ (Event::Text(_) | Event::GeneralRef(_)) if in_t && in_run && vanish => {
                let t = piece(&ev);
                if !t.trim().is_empty() {
                    if !counted {
                        f.hidden += 1;
                    }
                    counted = true;
                    if f.sample.chars().count() < 40 {
                        f.sample.push_str(&t);
                    }
                }
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn check_word(src: &Path, all: &[String], out: &mut Vec<String>) -> Result<()> {
    let mut f = Revisions::default();
    for n in all.iter().filter(|n| {
        let part = n.strip_prefix("word/").unwrap_or("/");
        !part.contains('/') && part.ends_with(".xml") && ["document", "footnotes", "endnotes", "header", "footer"].iter().any(|p| part.starts_with(p))
    }) {
        revisions(&xml_text(src, n)?, &mut f)?;
    }
    if f.ins + f.del + f.moved + f.restyled > 0 {
        let mut kinds = Vec::new();
        for (n, one, many) in [(f.ins, "insertion", "insertions"), (f.del, "deletion", "deletions"), (f.moved, "move", "moves"), (f.restyled, "formatting change", "formatting changes")] {
            if n > 0 {
                kinds.push(plural(n, one, many));
            }
        }
        out.push(format!("Tracked changes: {}{}. Anyone can read what was deleted.", kinds.join(", "), by(&f.authors)));
    }
    if all.iter().any(|n| n == "word/comments.xml") {
        let (mut n, mut who) = (0, BTreeSet::new());
        each(&xml_text(src, "word/comments.xml")?, |e| {
            if e.local_name().as_ref() == "comment" {
                n += 1;
                who.extend(av(e, "author").map(|a| clip(&a, 40)).filter(|a| !a.is_empty()));
            }
        })?;
        if n > 0 {
            out.push(format!("Comments: {n}{}", by(&who)));
        }
    }
    if f.hidden > 0 {
        out.push(format!("Hidden text: {} (“{}”)", plural(f.hidden, "piece", "pieces"), clip(f.sample.trim(), 40)));
    }
    if all.iter().any(|n| n == "word/settings.xml") {
        each(&xml_text(src, "word/settings.xml")?, |e| match e.local_name().as_ref() {
            "documentProtection" if flag(e, "enforcement") => out.push(format!("Editing is restricted ({})", clip(&av(e, "edit").unwrap_or_else(|| "protected".into()), 20))),
            "writeProtection" => out.push("Asks to be opened read-only".into()),
            "trackRevisions" if on(e) => out.push("Track Changes is switched on: whatever the reader edits will be recorded too".into()),
            _ => {}
        })?;
    }
    Ok(())
}

fn check_excel(src: &Path, all: &[String], out: &mut Vec<String>) -> Result<()> {
    let book = xml_text(src, "xl/workbook.xml")?;
    let (mut hidden, mut very) = (Vec::new(), Vec::new());
    let mut locked = false;
    each(&book, |e| match e.local_name().as_ref() {
        "sheet" => {
            let name = clip(&av(e, "name").unwrap_or_default(), 40);
            match av(e, "state").as_deref() {
                Some("hidden") => hidden.push(name),
                Some("veryHidden") => very.push(name),
                _ => {}
            }
        }
        "workbookProtection" => locked = flag(e, "lockStructure") || flag(e, "lockWindows"),
        _ => {}
    })?;
    if !hidden.is_empty() {
        out.push(format!("Hidden {}: {}", if hidden.len() == 1 { "sheet" } else { "sheets" }, hidden.join(", ")));
    }
    if !very.is_empty() {
        out.push(format!("Very hidden {} (not listed under Unhide): {}", if very.len() == 1 { "sheet" } else { "sheets" }, very.join(", ")));
    }
    let (mut rows, mut cols, mut protected) = (0usize, 0usize, 0usize);
    for n in all.iter().filter(|n| n.strip_prefix("xl/worksheets/").is_some_and(|s| !s.contains('/') && s.ends_with(".xml"))).take(500) {
        each(&xml_text(src, n)?, |e| match e.local_name().as_ref() {
            "row" if flag(e, "hidden") => rows += 1,
            "col" if flag(e, "hidden") => {
                let at = |a: &str| av(e, a).and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).clamp(1, 16_384);
                cols += at("max").saturating_sub(at("min")) + 1;
            }
            "sheetProtection" => protected += 1,
            _ => {}
        })?;
    }
    if rows + cols > 0 {
        out.push(format!("Hidden cells: {} and {} (filtered-out rows count too)", plural(rows, "row", "rows"), plural(cols, "column", "columns")));
    }
    // a name whose formula mentions [another workbook] carries that file's path or number
    let outside: BTreeSet<String> = {
        let mut r = Reader::from_str(&book);
        let (mut found, mut cur) = (BTreeSet::new(), None::<(String, String)>);
        loop {
            match r.read_event().map_err(|_| anyhow!(BAD))? {
                Event::Start(e) if e.local_name().as_ref() == "definedName" => cur = Some((av(&e, "name").unwrap_or_default(), String::new())),
                Event::End(e) if e.local_name().as_ref() == "definedName" => found.extend(cur.take().filter(|c| c.1.contains('[')).map(|c| clip(&c.0, 40))),
                ev @ (Event::Text(_) | Event::GeneralRef(_)) => {
                    if let Some(c) = cur.as_mut() {
                        c.1.push_str(&piece(&ev));
                    }
                }
                Event::Eof => break found,
                _ => {}
            }
        }
    };
    if !outside.is_empty() {
        out.push(format!("Names that point at other workbooks: {}", listing(&outside)));
    }
    let links = all.iter().filter(|n| n.strip_prefix("xl/externalLinks/").is_some_and(|s| !s.contains('/') && s.ends_with(".xml"))).count();
    if links > 0 {
        out.push(format!("Formulas read from {} (their last values are stored in this file)", plural(links, "other workbook", "other workbooks")));
    }
    let (mut notes, mut threads, mut who) = (0usize, 0usize, BTreeSet::new());
    for n in all.iter().filter(|n| n.ends_with(".xml")) {
        if n.strip_prefix("xl/comments").is_some_and(|s| !s.contains('/')) {
            let x = xml_text(src, n)?;
            each(&x, |e| notes += usize::from(e.local_name().as_ref() == "comment"))?;
            who.extend(tag_text(&x, "author")?.iter().map(|a| clip(a, 40)).filter(|a| !a.is_empty() && !a.starts_with("tc={")));
        } else if n.starts_with("xl/threadedComments/") {
            each(&xml_text(src, n)?, |e| threads += usize::from(e.local_name().as_ref() == "threadedComment"))?;
        } else if n.starts_with("xl/persons/") {
            each(&xml_text(src, n)?, |e| who.extend(av(e, "displayName").map(|a| clip(&a, 40)).filter(|a| !a.is_empty())))?;
        }
    }
    // every threaded comment is also kept as an old-style note for older Excel
    if notes.max(threads) > 0 {
        out.push(format!("Comments and notes on cells: {}{}", notes.max(threads), by(&who)));
    }
    if locked || protected > 0 {
        out.push(match (locked, protected) {
            (true, 0) => "The workbook's structure is protected".to_string(),
            (l, n) => format!("{} protected{}", plural(n, "sheet is", "sheets are"), if l { ", and so is the workbook's structure" } else { "" }),
        });
    }
    Ok(())
}

fn check_slides(src: &Path, all: &[String], out: &mut Vec<String>) -> Result<()> {
    // slides in the order they are shown, not the order of their file names
    let rels = rels_of(&xml_text(src, "ppt/_rels/presentation.xml.rels").unwrap_or_default());
    let (mut order, mut verifier) = (Vec::<String>::new(), false);
    each(&xml_text(src, "ppt/presentation.xml")?, |e| match e.local_name().as_ref() {
        "sldId" => order.extend(rid(e).and_then(|id| rels.get(&id)).and_then(|r| resolve("ppt", &r.target))),
        "modifyVerifier" => verifier = true,
        _ => {}
    })?;
    order.retain(|n| all.contains(n));
    if order.is_empty() {
        order = slide_names(all);
    }
    let mut hidden = Vec::new();
    for (i, n) in order.iter().enumerate().take(5000) {
        let mut first = true;
        each(&xml_text(src, n)?, |e| {
            if std::mem::take(&mut first) && matches!(av(e, "show").as_deref(), Some("0" | "false")) {
                hidden.push((i + 1).to_string());
            }
        })?;
    }
    if !hidden.is_empty() {
        out.push(format!("Hidden {}: {}", if hidden.len() == 1 { "slide" } else { "slides" }, hidden.join(", ")));
    }
    let mut notes = 0;
    for n in all.iter().filter(|n| n.strip_prefix("ppt/notesSlides/").is_some_and(|s| !s.contains('/') && s.ends_with(".xml"))) {
        let x = xml_text(src, n)?;
        let mut r = Reader::from_str(&x);
        let (mut in_t, mut any) = (false, false);
        loop {
            match r.read_event().map_err(|_| anyhow!(BAD))? {
                // the slide number on a notes page is a field, not something anyone typed
                Event::Start(e) if e.local_name().as_ref() == "fld" => {
                    r.read_to_end(e.name()).map_err(|_| anyhow!(BAD))?;
                }
                Event::Start(e) if e.local_name().as_ref() == "t" => in_t = true,
                Event::End(e) if e.local_name().as_ref() == "t" => in_t = false,
                ev @ (Event::Text(_) | Event::GeneralRef(_)) if in_t => any |= !piece(&ev).trim().is_empty(),
                Event::Eof => break,
                _ => {}
            }
        }
        notes += usize::from(any);
    }
    if notes > 0 {
        out.push(format!("Speaker notes on {}", plural(notes, "slide", "slides")));
    }
    let (mut comments, mut who) = (0usize, BTreeSet::new());
    for n in all.iter().filter(|n| n.ends_with(".xml")) {
        if n.starts_with("ppt/comments/") {
            each(&xml_text(src, n)?, |e| comments += usize::from(e.local_name().as_ref() == "cm"))?;
        } else if n == "ppt/commentAuthors.xml" || n == "ppt/authors.xml" {
            each(&xml_text(src, n)?, |e| {
                if matches!(e.local_name().as_ref(), "cmAuthor" | "author") {
                    who.extend(av(e, "name").map(|a| clip(&a, 40)).filter(|a| !a.is_empty()));
                }
            })?;
        }
    }
    if comments > 0 {
        out.push(format!("Comments: {comments}{}", by(&who)));
    }
    if verifier {
        out.push("Marked read-only with a password".into());
    }
    Ok(())
}

/// Everything a reader could find in the file that its pages do not show.
fn findings(src: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let root = ooxml(src)?;
    let all = names(src)?;
    let has = |n: &str| all.iter().any(|a| a == n);
    if !has(&format!("{root}/{}.xml", ["document", "workbook", "presentation"][["word", "xl", "ppt"].iter().position(|r| *r == root).unwrap_or(0)])) {
        bail!(BAD);
    }
    let (mut out, mut also) = (Vec::new(), Vec::new());
    let mut people = Vec::new();
    for (part, fields) in [("docProps/core.xml", [("creator", "author"), ("lastModifiedBy", "last saved by")]), ("docProps/app.xml", [("Company", "company"), ("Manager", "manager")])] {
        if has(part) {
            let x = xml_text(src, part)?;
            for (tag, label) in fields {
                if let Some(v) = tag_text(&x, tag)?.into_iter().map(|v| clip(v.trim(), 60)).find(|v| !v.is_empty()) {
                    people.push(format!("{label} {v}"));
                }
            }
        }
    }
    if !people.is_empty() {
        out.push(format!("Names: {}", people.join(", ")));
    }
    match root {
        "word" => check_word(src, &all, &mut out)?,
        "xl" => check_excel(src, &all, &mut out)?,
        _ => check_slides(src, &all, &mut out)?,
    }
    if all.iter().any(|n| n.ends_with("vbaProject.bin")) {
        out.push("Macros (VBA code that can run when the file is opened)".into());
    }
    if all.iter().any(|n| n.starts_with("xl/macrosheets/")) {
        out.push("Excel 4 macro sheets".into());
    }
    let inside = |dir: &str| all.iter().filter(|n| n.strip_prefix(&format!("{root}/{dir}/")).is_some_and(|s| !s.is_empty() && !s.contains('/'))).count();
    if inside("embeddings") > 0 {
        out.push(format!("Embedded files: {} (whole documents or objects stored inside this one)", inside("embeddings")));
    }
    if inside("activeX") > 0 {
        out.push("ActiveX controls".into());
    }
    let custom = all.iter().filter(|n| n.strip_prefix("customXml/item").is_some_and(|s| s.strip_suffix(".xml").is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit())))).count();
    if custom > 0 {
        out.push(format!("Custom XML: {} (data other software or a template added)", plural(custom, "part", "parts")));
    }
    if has("docProps/custom.xml") {
        let mut props = BTreeSet::new();
        each(&xml_text(src, "docProps/custom.xml")?, |e| {
            if e.local_name().as_ref() == "property" {
                props.extend(av(e, "name").map(|n| clip(&n, 40)));
            }
        })?;
        if !props.is_empty() {
            out.push(format!("Custom properties: {}", listing(&props)));
        }
    }
    // what the file reaches for outside itself
    let (mut pictures, mut templates, mut files, mut local, mut sites) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for n in all.iter().filter(|n| n.ends_with(".rels")).take(5000) {
        for r in rels_of(&xml_text(src, n)?).into_values().filter(|r| r.external) {
            match r.kind.as_str() {
                "hyperlink" if web(&r.target) => sites.insert(place(&r.target)),
                "hyperlink" => local.insert(place(&r.target)),
                "image" => pictures.insert(place(&r.target)),
                "attachedTemplate" => templates.insert(place(&r.target)),
                _ => files.insert(place(&r.target)),
            };
        }
    }
    for (set, label) in [(&pictures, "Pictures that are linked, not stored in the file"), (&templates, "Attached template"), (&files, "Linked files and data sources"), (&local, "Links to files and folders")] {
        if !set.is_empty() {
            out.push(format!("{label}: {}", listing(set)));
        }
    }
    if !sites.is_empty() {
        also.push(format!("Web links go to: {}", listing(&sites)));
    }
    Ok((out, also))
}

fn check(src: &Path) -> Result<Vec<PathBuf>> {
    let (found, also) = findings(src)?;
    let verdict = if found.is_empty() { "nothing hidden in this file".to_string() } else { format!("{} to look at", plural(found.len(), "thing", "things")) };
    let mut report = format!("{}: {verdict}\n", file_name(src));
    for line in found.iter().chain(&also) {
        report += &format!("\n• {line}");
    }
    crate::clipboard::set(&(report + "\n"))?;
    note(&if found.is_empty() { verdict } else { format!("{verdict} (list copied)") });
    Ok(vec![])
}

// ---------------------------------------------------------------- smaller pictures

const SIDE: u32 = 2000;

/// A zip entry name that is also a safe relative path to write under a folder of ours, and
/// that `zip` will not read as a pattern.
fn plain_entry(name: &str) -> bool {
    !name.starts_with(['/', '-']) && !name.contains(['\\', '*', '?', '[', ']', '\0']) && name.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

/// The same picture in fewer bytes, in the same format; `None` when it should be left alone.
fn lighter(old: &[u8], jpeg: bool) -> Option<Vec<u8>> {
    use image::{codecs, ImageDecoder, ImageEncoder, ImageFormat};
    let format = if jpeg { ImageFormat::Jpeg } else { ImageFormat::Png };
    if image::guess_format(old).ok()? != format {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(old), format);
    let mut limits = image::Limits::default();
    (limits.max_image_width, limits.max_image_height, limits.max_alloc) = (Some(30_000), Some(30_000), Some(1 << 30));
    reader.limits(limits);
    let mut dec = reader.into_decoder().ok()?;
    // a turned photo or a CMYK one would come back looking different
    if dec.orientation().ok()? != image::metadata::Orientation::NoTransforms || matches!(dec.original_color_type(), image::ExtendedColorType::Cmyk8 | image::ExtendedColorType::Cmyk16) {
        return None;
    }
    let icc = dec.icc_profile().ok().flatten();
    // a JPEG cut short still decodes, with grey where the rest was: leave such a file as it is
    if jpeg && !old.iter().rev().find(|b| **b != 0).is_some_and(|b| *b == 0xD9) {
        return None;
    }
    let (w, h) = dec.dimensions();
    let big = w.max(h) > SIDE;
    let mut opts = oxipng::Options::from_preset(2);
    opts.strip = oxipng::StripChunks::Safe;
    opts.timeout = Some(Duration::from_secs(20));
    if !jpeg && !big {
        // on the bytes as they are: lossless, and nothing about the picture can change
        return oxipng::optimize_from_memory(old, &opts).ok();
    }
    if !jpeg && old.windows(4).any(|c| c == b"acTL") {
        return None; // animated
    }
    let img = image::DynamicImage::from_decoder(dec).ok()?;
    let img = if big { img.resize(SIDE, SIDE, image::imageops::FilterType::Lanczos3) } else { img };
    let mut buf = Vec::new();
    if jpeg {
        let mut enc = codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 82);
        if let Some(p) = icc {
            let _ = enc.set_icc_profile(p);
        }
        match img.color() {
            image::ColorType::L8 | image::ColorType::L16 | image::ColorType::La8 | image::ColorType::La16 => enc.encode_image(&img.to_luma8()).ok()?,
            _ => enc.encode_image(&img.to_rgb8()).ok()?,
        }
        return Some(buf);
    }
    let mut enc = codecs::png::PngEncoder::new(&mut buf);
    if let Some(p) = icc {
        let _ = enc.set_icc_profile(p);
    }
    img.write_with_encoder(enc).ok()?;
    Some(oxipng::optimize_from_memory(&buf, &opts).unwrap_or(buf))
}

fn megabytes(n: u64) -> String {
    if n < 1_000_000 { format!("{} KB", (n as f64 / 1000.0).round().max(1.0)) } else { format!("{:.1} MB", n as f64 / 1e6) }
}

fn slim(src: &Path) -> Result<Vec<PathBuf>> {
    let root = ooxml(src)?;
    let prefix = format!("{root}/media/");
    let all = names(src)?;
    let before = std::fs::metadata(src)?.len();
    let dir = temp_file("slim");
    let out = out_path(src, Some("Smaller"), &ext(src));
    let done = (|| -> Result<usize> {
        let mut changed: Vec<&String> = Vec::new();
        for n in all.iter().filter(|n| n.starts_with(&prefix) && plain_entry(n)) {
            let jpeg = match ext(Path::new(n)).as_str() {
                "jpg" | "jpeg" => true,
                "png" => false,
                _ => continue,
            };
            // a picture that is broken, huge or odd stays exactly as it is
            let Ok(old) = entry(src, n, 256) else { continue };
            let Some(new) = std::panic::catch_unwind(|| lighter(&old, jpeg)).ok().flatten().filter(|new| new.len() < old.len()) else { continue };
            let to = dir.join(n);
            std::fs::create_dir_all(to.parent().context("no folder")?)?;
            std::fs::write(to, new)?;
            changed.push(n);
        }
        if changed.is_empty() {
            bail!("this file is already small");
        }
        std::fs::copy(src, &out)?;
        // zip replaces just these parts inside the copy; everything else is untouched
        for group in changed.chunks(500) {
            let o = crate::engines::tools::output_within(Command::new("/usr/bin/zip").current_dir(&dir).args(["-q", "-X"]).arg(&out).args(group), Duration::from_secs(600))?;
            if !o.status.success() {
                bail!("couldn't write the document: {}", String::from_utf8_lossy(&o.stderr).trim());
            }
        }
        Ok(changed.len())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    // out_path() already made the file; never leave it behind when there is nothing to give
    let n = done.inspect_err(|_| {
        let _ = std::fs::remove_file(&out);
    })?;
    let after = std::fs::metadata(&out)?.len();
    if after * 100 > before * 95 {
        let _ = std::fs::remove_file(&out);
        bail!("this file is already small");
    }
    note(&format!("{} → {}, {}", megabytes(before), megabytes(after), plural(n, "picture", "pictures")));
    Ok(vec![out])
}

// ---------------------------------------------------------------- redline

/// Paragraphs of a Word document as plain text, with tracked changes accepted.
fn docx_paras(xml: &str) -> Result<Vec<String>> {
    let bad = |_| anyhow!(BAD);
    let mut r = Reader::from_str(xml);
    let (mut out, mut open, mut in_t) = (Vec::new(), Vec::<String>::new(), false);
    loop {
        match r.read_event().map_err(bad)? {
            Event::Start(e) => match e.local_name().as_ref() {
                "del" | "moveFrom" | "Fallback" => {
                    r.read_to_end(e.name()).map_err(bad)?;
                }
                "p" => open.push(String::new()),
                "t" => in_t = true,
                _ => {}
            },
            Event::Empty(e) if matches!(e.local_name().as_ref(), "tab" | "br" | "cr") => {
                if let Some(p) = open.last_mut() {
                    p.push(' ');
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                "t" => in_t = false,
                "p" => out.extend(open.pop()),
                _ => {}
            },
            ev @ (Event::Text(_) | Event::GeneralRef(_)) if in_t => {
                if let Some(p) = open.last_mut() {
                    p.push_str(&piece(&ev));
                }
            }
            Event::Eof => return Ok(out),
            _ => {}
        }
    }
}

fn odt_paras(xml: &str) -> Result<Vec<String>> {
    let bad = |_| anyhow!(BAD);
    let mut r = Reader::from_str(xml);
    let (mut out, mut open) = (Vec::new(), Vec::<String>::new());
    loop {
        match r.read_event().map_err(bad)? {
            Event::Start(e) | Event::Empty(e) if matches!(e.local_name().as_ref(), "s" | "tab" | "line-break") => {
                if let Some(p) = open.last_mut() {
                    p.push(' ');
                }
            }
            Event::Start(e) => match e.local_name().as_ref() {
                // what was deleted while changes were tracked, and comments in the margin
                "tracked-changes" | "annotation" => {
                    r.read_to_end(e.name()).map_err(bad)?;
                }
                "p" | "h" => open.push(String::new()),
                _ => {}
            },
            Event::End(e) if matches!(e.local_name().as_ref(), "p" | "h") => out.extend(open.pop()),
            ev @ (Event::Text(_) | Event::GeneralRef(_)) => {
                if let Some(p) = open.last_mut() {
                    p.push_str(&piece(&ev));
                }
            }
            Event::Eof => return Ok(out),
            _ => {}
        }
    }
}

/// A web page as the paragraphs a reader sees: no tags, scripts or styles.
fn html_paras(html: &str) -> Vec<String> {
    const BLOCKS: &[&str] = &["p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "table", "ul", "ol", "dl", "dt", "dd", "blockquote", "section", "article", "header", "footer", "main", "aside", "nav", "pre", "hr", "body", "figure", "figcaption", "form", "option"];
    let mut text = String::new();
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        let (before, tag) = rest.split_at(lt);
        text.push_str(before);
        if let Some(body) = tag.strip_prefix("<!--") {
            rest = body.split_once("-->").map_or("", |x| x.1);
            continue;
        }
        let Some((inside, after)) = tag.get(1..).and_then(|t| t.split_once('>')) else {
            rest = "";
            break;
        };
        let name: String = inside.trim_start_matches('/').chars().take_while(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
        rest = after;
        if matches!(name.as_str(), "script" | "style" | "template" | "noscript" | "title") && !inside.starts_with('/') {
            let close = format!("</{name}");
            rest = after.to_ascii_lowercase().find(&close).and_then(|i| after.get(i..)).unwrap_or("");
        } else if BLOCKS.contains(&name.as_str()) {
            text.push('\n');
        } else if matches!(name.as_str(), "td" | "th") {
            text.push(' ');
        }
    }
    text.push_str(rest);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(amp) = rest.find('&') {
        let (before, tail) = rest.split_at(amp);
        out.push_str(before);
        let end = tail.find(';').filter(|i| *i <= 10);
        let name = end.and_then(|i| tail.get(1..i)).unwrap_or("");
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            n => n.strip_prefix('#').and_then(|d| match d.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok(),
                None => d.parse().ok(),
            })
            .and_then(char::from_u32),
        };
        match (decoded, end) {
            (Some(c), Some(i)) => {
                out.push(c);
                rest = tail.get(i + 1..).unwrap_or("");
            }
            _ => {
                out.push('&');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out.lines().map(str::to_string).collect()
}

/// Markdown keeps a paragraph on several lines; a list item or heading is its own paragraph.
fn md_paras(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut fresh = true;
    for l in text.lines() {
        let t = l.trim();
        let digits = t.bytes().take_while(u8::is_ascii_digit).count();
        let starts = t.starts_with(['#', '>', '|', '`']) || ["- ", "* ", "+ "].iter().any(|m| t.starts_with(m)) || (digits > 0 && t.get(digits..).is_some_and(|r| r.starts_with(". ") || r.starts_with(") ")));
        match out.last_mut() {
            _ if t.is_empty() => fresh = true,
            Some(last) if !fresh && !starts => {
                last.push(' ');
                last.push_str(t);
            }
            _ => {
                // "## Title" and a Word heading "Title" are the same words
                out.push(t.trim_start_matches('#').trim_start().to_string());
                fresh = t.starts_with('#');
            }
        }
    }
    out
}

/// One printed line of a PDF page, in points from the page's bottom-left corner.
struct Line {
    left: f32,
    right: f32,
    bottom: f32,
    height: f32,
    text: String,
}

/// Lines of a PDF page joined back into paragraphs. A PDF keeps no paragraphs, so this reads
/// the page the way a person does: a line that stops well before the right margin ends one,
/// and so does a gap wider than the page's usual line spacing.
fn reflow(lines: &[Line], page_width: f32) -> Vec<String> {
    let margin = lines.iter().map(|l| l.left).fold(f32::MAX, f32::min);
    let full = margin + (page_width - 2.0 * margin).max(0.0) * 0.8;
    let pitch = lines.windows(2).filter_map(|w| Some(w.first()?.bottom - w.get(1)?.bottom)).filter(|p| *p > 1.0).fold(f32::MAX, f32::min);
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<&Line> = None;
    for l in lines {
        let joins = prev.is_some_and(|p| {
            let step = p.bottom - l.bottom;
            step > 0.0 && step <= pitch * 1.35 && p.right >= full
        });
        match out.last_mut() {
            Some(last) if joins => {
                last.push(' ');
                last.push_str(&l.text);
            }
            _ => out.push(l.text.clone()),
        }
        prev = Some(l);
    }
    out
}

fn pdf_paras(src: &Path) -> Result<Vec<String>> {
    let doc = crate::engines::pdf::load(src)?;
    let mut out = Vec::new();
    for page in doc.pages().iter() {
        let Ok(text) = page.text() else { continue };
        let mut lines: Vec<Line> = Vec::new();
        for seg in text.segments().iter() {
            let (b, t) = (seg.bounds(), seg.text());
            let (left, right, bottom, height) = (b.left().value, b.right().value, b.bottom().value, b.height().value.abs());
            match lines.last_mut() {
                // pieces of one line: a change of font splits a line into several
                Some(l) if (l.bottom - bottom).abs() < l.height.max(height) * 0.5 && left >= l.left => {
                    l.right = l.right.max(right);
                    l.text.push(' ');
                    l.text.push_str(&t);
                }
                _ => lines.push(Line { left, right, bottom, height, text: t }),
            }
        }
        out.extend(reflow(&lines, page.width().value));
    }
    Ok(out)
}

fn paragraphs(src: &Path) -> Result<Vec<String>> {
    let name = file_name(src);
    if std::fs::metadata(src)?.len() > 200 << 20 {
        bail!("{name} is too big for this (over 200 MB)");
    }
    let zipped = |part: &str| -> Result<String> {
        if !names(src)?.iter().any(|n| n == part) {
            bail!(BAD);
        }
        xml_text(src, part)
    };
    let raw = match ext(src).as_str() {
        "docx" | "docm" => docx_paras(&zipped("word/document.xml")?)?,
        "odt" => odt_paras(&zipped("content.xml")?)?,
        "pdf" => pdf_paras(src)?,
        "md" | "markdown" => md_paras(&read_text(src)?),
        "txt" => read_text(src)?.lines().map(str::to_string).collect(),
        "html" | "htm" => html_paras(&read_text(src)?),
        "rtf" => {
            let o = crate::engines::tools::output_within(Command::new("/usr/bin/textutil").args(["-convert", "txt", "-stdout", "-encoding", "UTF-8", "--"]).arg(src), Duration::from_secs(60))?;
            if !o.status.success() {
                bail!(BAD);
            }
            String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect()
        }
        _ => bail!("“{name}” is not a document this can compare: use Word (.docx), PDF, Markdown, text, RTF, ODT or HTML files"),
    };
    let out: Vec<String> = raw.iter().map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect();
    if out.is_empty() {
        bail!("no text found in “{name}”: if it is a scan, make it searchable first");
    }
    if out.len() > 200_000 || out.iter().map(String::len).sum::<usize>() > 20 << 20 {
        bail!("“{name}” is too long to compare");
    }
    Ok(out)
}

fn esc_html(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

enum Row<'a> {
    Same(&'a str),
    Add(&'a str),
    Del(&'a str),
    Change(&'a str, &'a str),
}

fn words<'a>(a: &'a str, b: &'a str) -> similar::TextDiff<'a, 'a, 'a, str> {
    similar::TextDiff::configure().timeout(Duration::from_millis(400)).diff_words(a, b)
}

/// The two texts lined up paragraph by paragraph. Inside a stretch that was rewritten, a
/// paragraph is "changed" when most of its words are still there, otherwise removed and added.
fn align<'a>(old: &'a [String], new: &'a [String]) -> Vec<Row<'a>> {
    use similar::DiffOp;
    let ops = similar::capture_diff_slices_deadline(similar::Algorithm::Myers, old, new, Some(Instant::now() + Duration::from_secs(20)));
    let pick = |all: &'a [String], at: usize, len: usize| all.get(at..at + len).unwrap_or(&[]).iter().map(String::as_str);
    let mut rows = Vec::new();
    for op in ops {
        match op {
            DiffOp::Equal { old_index, len, .. } => rows.extend(pick(old, old_index, len).map(Row::Same)),
            DiffOp::Delete { old_index, old_len, .. } => rows.extend(pick(old, old_index, old_len).map(Row::Del)),
            DiffOp::Insert { new_index, new_len, .. } => rows.extend(pick(new, new_index, new_len).map(Row::Add)),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                let fresh: Vec<&str> = pick(new, new_index, new_len).collect();
                let mut next = 0;
                for was in pick(old, old_index, old_len) {
                    let near = fresh.iter().enumerate().skip(next).take(4).find(|(_, now)| words(was, now).ratio() >= 0.5).map(|(i, _)| i);
                    match near {
                        Some(i) => {
                            rows.extend(fresh.iter().take(i).skip(next).map(|s| Row::Add(s)));
                            rows.extend(fresh.get(i).map(|now| Row::Change(was, now)));
                            next = i + 1;
                        }
                        None => rows.push(Row::Del(was)),
                    }
                }
                rows.extend(fresh.iter().skip(next).map(|s| Row::Add(s)));
            }
        }
    }
    rows
}

fn word_diff(a: &str, b: &str) -> String {
    use similar::ChangeTag;
    let (mut out, mut run, mut tag) = (String::new(), String::new(), ChangeTag::Equal);
    let close = |out: &mut String, run: &mut String, tag: ChangeTag| {
        if !run.is_empty() {
            let text = esc_html(run);
            *out += &match tag {
                ChangeTag::Equal => text,
                ChangeTag::Delete => format!("<del>{text}</del>"),
                ChangeTag::Insert => format!("<ins>{text}</ins>"),
            };
            run.clear();
        }
    };
    for c in words(a, b).iter_all_changes() {
        if c.tag() != tag {
            close(&mut out, &mut run, tag);
            tag = c.tag();
        }
        run.push_str(c.value());
    }
    close(&mut out, &mut run, tag);
    out
}

const REDLINE_CSS: &str = ":root{color-scheme:light dark;--fg:#1d1d1f;--dim:#6e6e73;--bg:#fff;--line:#d2d2d7;--add:#0a6b2d;--addbg:#e3f6e8;--del:#b3261e;--delbg:#fde7e5}\
@media(prefers-color-scheme:dark){:root{--fg:#f5f5f7;--dim:#a1a1a6;--bg:#1c1c1e;--line:#3a3a3c;--add:#6fdc8c;--addbg:#12351f;--del:#ff8a80;--delbg:#3d1a17}}\
body{font:16px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;color:var(--fg);background:var(--bg);max-width:820px;margin:40px auto;padding:0 24px;overflow-wrap:anywhere}\
h1{font-size:22px;line-height:1.3;margin:0 0 8px}h1 span{color:var(--dim);font-weight:400}header{border-bottom:1px solid var(--line);margin-bottom:24px;padding-bottom:16px}\
header p{margin:4px 0;color:var(--dim);font-size:14px}.a{color:var(--add)}.d{color:var(--del)}\
ins{color:var(--add);background:var(--addbg);text-decoration:underline;text-decoration-thickness:1.5px;text-underline-offset:2px;border-radius:2px}\
del{color:var(--del);background:var(--delbg);text-decoration:line-through;border-radius:2px}\
main p{margin:0 0 14px;padding-left:12px;border-left:3px solid transparent}p.add{border-color:var(--add)}p.del{border-color:var(--del)}p.chg{border-color:var(--dim)}\
.gap{color:var(--dim);font-size:13px;text-align:center;margin:18px 0;padding:6px;border:1px dashed var(--line);border-radius:6px}\
@media print{:root{--fg:#000;--dim:#555;--bg:#fff;--line:#bbb;--add:#0a6b2d;--addbg:transparent;--del:#b3261e;--delbg:transparent}body{margin:0;max-width:none;font-size:11pt}main p{break-inside:avoid}ins{font-weight:600}}";

fn stamp(p: &Path) -> (std::time::SystemTime, String) {
    let t = std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as libc::time_t;
    // the reader's own clock, as Finder shows it
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    (t, format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min))
}

/// The page itself, and how many paragraphs were added, removed and changed.
fn redline(old: (&str, &str, &[String]), new: (&str, &str, &[String])) -> (String, [usize; 3]) {
    let rows = align(old.2, new.2);
    let (mut body, mut counts) = (String::new(), [0usize; 3]);
    let mut i = 0;
    while let Some(row) = rows.get(i) {
        match row {
            Row::Same(_) => {
                let n = rows.iter().skip(i).take_while(|r| matches!(r, Row::Same(_))).count();
                // two paragraphs of context on each side of a change; none at the very ends
                let (head, tail) = (if i == 0 { 0 } else { 2 }, if i + n == rows.len() { 0 } else { 2 });
                let hidden = if n > head + tail + 1 { n - head - tail } else { 0 };
                for (k, r) in rows.iter().skip(i).take(n).enumerate() {
                    if hidden > 0 && k == head {
                        body += &format!("<div class=\"gap\">… {hidden} unchanged paragraphs …</div>\n");
                    }
                    if let (Row::Same(t), false) = (r, hidden > 0 && (head..head + hidden).contains(&k)) {
                        body += &format!("<p>{}</p>\n", esc_html(t));
                    }
                }
                i += n;
                continue;
            }
            Row::Add(t) => {
                counts[0] += 1;
                body += &format!("<p class=\"add\"><ins>{}</ins></p>\n", esc_html(t));
            }
            Row::Del(t) => {
                counts[1] += 1;
                body += &format!("<p class=\"del\"><del>{}</del></p>\n", esc_html(t));
            }
            Row::Change(a, b) => {
                counts[2] += 1;
                body += &format!("<p class=\"chg\">{}</p>\n", word_diff(a, b));
            }
        }
        i += 1;
    }
    let [added, removed, changed] = counts;
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{o} → {n}</title><style>{REDLINE_CSS}</style></head><body><header><h1>{o} <span>→</span> {n}</h1><p>Older: {o}, saved {ot} · Newer: {n}, saved {nt}</p><p><span class=\"a\">{added} added</span> · <span class=\"d\">{removed} removed</span> · {changed} changed (paragraphs)</p></header>\n<main>\n{body}</main></body></html>\n",
        o = esc_html(old.0),
        n = esc_html(new.0),
        ot = esc_html(old.1),
        nt = esc_html(new.1),
    );
    (html, counts)
}

fn compare(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let [a, b] = paths else { bail!("choose exactly two documents to compare") };
    let ((ta, sa), (tb, sb)) = (stamp(a), stamp(b));
    // the older one is the "before"; dropped order decides when the times are the same
    let (old, new, so, sn) = if tb < ta { (b, a, sb, sa) } else { (a, b, sa, sb) };
    let (po, pn) = (paragraphs(old)?, paragraphs(new)?);
    if po == pn {
        bail!("these two documents have the same text");
    }
    let (html, [added, removed, changed]) = redline((&file_name(old), &so, &po), (&file_name(new), &sn, &pn));
    let out = out_path(new, Some(&format!("vs {}", clip(&stem(old), 60))), "html");
    std::fs::write(&out, html)?;
    note(&format!("{added} added, {removed} removed, {changed} changed"));
    Ok(vec![out])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "xmlns:w=\"w\" xmlns:r=\"r\"";

    fn md(body: &str) -> String {
        let styles = styles(&format!("<w:styles {NS}><w:style w:styleId=\"berschrift2\"><w:name w:val=\"heading 2\"/></w:style><w:style w:styleId=\"Mine\"><w:basedOn w:val=\"berschrift2\"/></w:style><w:style w:styleId=\"Lst\"><w:pPr><w:numPr><w:numId w:val=\"7\"/></w:numPr></w:pPr></w:style></w:styles>"));
        let lists = Lists::read(&format!("<w:numbering {NS}><w:abstractNum w:abstractNumId=\"3\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"decimal\"/></w:lvl></w:abstractNum><w:num w:numId=\"7\"><w:abstractNumId w:val=\"3\"/></w:num><w:num w:numId=\"8\"><w:abstractNumId w:val=\"3\"/><w:lvlOverride w:ilvl=\"0\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/></w:lvl></w:lvlOverride></w:num></w:numbering>"));
        let mut doc = Doc { styles, lists, ..Default::default() };
        let rels = rels_of("<Relationships><Relationship Id=\"l\" Type=\"x/hyperlink\" Target=\"https://a.vn/x y\" TargetMode=\"External\"/><Relationship Id=\"j\" Type=\"x/hyperlink\" Target=\"javascript:1\" TargetMode=\"External\"/></Relationships>");
        markdown_of(&mut doc, &format!("<w:document {NS}><w:body>{body}</w:body></w:document>"), &rels, &[]).unwrap()
    }

    fn p(style: &str, runs: &str) -> String {
        format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>{runs}</w:p>")
    }

    fn r(props: &str, text: &str) -> String {
        format!("<w:r><w:rPr>{props}</w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
    }

    #[test]
    fn headings_by_name_base_and_foreign_id() {
        assert_eq!(md(&p("berschrift2", &r("<w:b/>", "Mục lục"))), "## Mục lục\n");
        assert_eq!(md(&p("Mine", &r("", "A #"))), "## A \\#\n");
        assert_eq!(by_id("Titre3"), Kind::Heading(3));
        assert_eq!(by_id("Heading9"), Kind::Heading(6));
        assert_eq!(by_id("Normal"), Kind::Text);
        assert_eq!(by_name("intense quote"), Some(Kind::Quote));
    }

    #[test]
    fn runs_join_and_escape() {
        let x = md(&p("", &(r("<w:b/>", "đậm ") + &r("<w:b/>", "nữa") + &r("", " a*b_c [x] &amp;amp; R&amp;D ") + &r("<w:i/><w:strike/>", "nghiêng") + &r("<w:vanish/>", "ẩn") + &r("<w:b w:val=\"0\"/>", " thường"))));
        assert_eq!(x, "**đậm nữa** a\\*b\\_c \\[x\\] \\&amp; R&D ~~*nghiêng*~~ thường\n");
        assert_eq!(md(&p("", &r("<w:rFonts w:ascii=\"Courier New\"/>", "a`b"))), "``a`b``\n");
        assert_eq!(guard("12) x"), "12\\) x");
        assert_eq!(guard("2026. Năm"), "2026\\. Năm");
        assert_eq!(guard("- x"), "\\- x");
        assert_eq!(guard("-5 độ"), "-5 độ");
        assert_eq!(guard("3.5 kg"), "3.5 kg");
    }

    #[test]
    fn lists_links_tables_and_changes() {
        let li = |num: &str, lvl: u8, t: &str| format!("<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{lvl}\"/><w:numId w:val=\"{num}\"/></w:numPr></w:pPr>{}</w:p>", r("", t));
        assert_eq!(md(&(li("7", 0, "a") + &li("7", 0, "b") + &li("8", 0, "c") + &li("8", 3, "d") + &p("Lst", &r("", "e")) + &li("0", 0, "f"))), "1. a\n2. b\n- c\n    - d\n3. e\n\nf\n");
        let link = format!("<w:p><w:hyperlink r:id=\"l\">{}{}</w:hyperlink><w:hyperlink r:id=\"j\">{}</w:hyperlink><w:ins w:id=\"1\">{}</w:ins><w:del w:id=\"2\"><w:r><w:t>gone</w:t></w:r></w:del></w:p>", r("", "một "), r("<w:b/>", "hai"), r("", " js"), r("", " mới"));
        assert_eq!(md(&link), "[một **hai**](https://a.vn/x%20y) js mới\n");
        let field = format!("<w:p><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText> HYPERLINK \"mailto:a@b.vn\" </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>{}<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>{}</w:p>", r("", "thư"), r("", " sau"));
        assert_eq!(md(&field), "[thư](mailto:a@b.vn) sau\n");
        let cell = |inner: &str| format!("<w:tc><w:tcPr><w:gridSpan w:val=\"2\"/></w:tcPr>{inner}</w:tc>");
        let table = format!("<w:tbl><w:tr>{}</w:tr><w:tr>{}<w:tc><w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl>", cell(&p("", &r("<w:b/>", "H"))), cell(&(p("", &r("", "a|b")) + &p("", &r("", "# c")))), p("", &r("", "trong")));
        assert_eq!(md(&table), "| H |  |  |\n| --- | --- | --- |\n| a\\|b<br>\\# c |  | trong |\n");
        assert!(Doc::default().walk("<w:p><w:r><w:t>x</w:r></w:p>", &HashMap::new(), "").is_err());
    }

    #[test]
    fn paths_inside_a_zip() {
        assert_eq!(resolve("word", "media/a.png").as_deref(), Some("word/media/a.png"));
        assert_eq!(resolve("word", "../customXml/item1.xml").as_deref(), Some("customXml/item1.xml"));
        assert_eq!(resolve("word", "/word/media/a.png").as_deref(), Some("word/media/a.png"));
        assert_eq!(resolve("word", "../../evil.png"), None);
        assert_eq!(resolve("word", "media\\..\\a.png"), None);
        assert!(plain_entry("ppt/media/image1.png") && !plain_entry("ppt/media/../../x.png") && !plain_entry("/etc/x.png") && !plain_entry("ppt/media/a[1].png") && !plain_entry("-x"));
        assert!(web("HTTPS://a.vn") && web("mailto:a@b") && !web("file:///etc/passwd") && !web("javascript:alert(1)") && !web("\\\\host\\share"));
        assert_eq!(place("https://user:pw@intranet.cty.vn:8080/a/b?c"), "intranet.cty.vn:8080");
        assert_eq!(link_path("Tệp (1) media/a b.png"), "Tệp%20%281%29%20media/a%20b.png");
    }

    #[test]
    fn markdown_into_word() {
        let w = build("# Tiêu *đề*\n\n- a\n  1. b\n\n  tiếp\n- [x] c\n\n[l](https://a.vn/?x=1&y=2) [m](javascript:1) ![hình](a.png) <b>&</b><br>x\n", false);
        assert!(w.body.starts_with("<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t xml:space=\"preserve\">Tiêu </w:t></w:r><w:r><w:rPr><w:i/><w:iCs/></w:rPr>"), "{}", w.body);
        assert_eq!(w.nums, vec![(false, 1, 0), (true, 1, 1)]);
        assert!(w.body.contains("<w:numPr><w:ilvl w:val=\"1\"/><w:numId w:val=\"2\"/></w:numPr>") && w.body.contains("<w:ind w:left=\"720\"/></w:pPr><w:r><w:t xml:space=\"preserve\">tiếp"), "{}", w.body);
        assert!(w.body.contains("☑ ") && w.body.contains("&lt;b&gt;") && w.body.contains("<w:br/>"));
        assert_eq!((w.links.as_slice(), w.images), (["https://a.vn/?x=1&y=2".to_string()].as_slice(), 1));
        assert_eq!(w.body.matches("<w:hyperlink r:id=\"rId5\"").count(), 1);
        let t = build("|a|b|\n|-|-:|\n|1|2|\n\n|c|\n|-|\n", false);
        assert_eq!(t.body.matches("<w:tbl>").count(), 2);
        assert!(t.body.contains("</w:tbl><w:p/><w:tbl>") && t.body.ends_with("</w:tbl><w:p/>") && t.body.contains("<w:jc w:val=\"right\"/>"));
        assert!(build("a\nb", true).body.contains("<w:br/>") && !build("a\nb", false).body.contains("<w:br/>"));
        assert!(numbering_xml(&[(true, 4, 1)]).contains("<w:lvlOverride w:ilvl=\"1\"><w:startOverride w:val=\"4\"/>"));
    }

    #[test]
    fn text_out_of_other_formats() {
        assert_eq!(html_paras("<title>t</title><p>A &amp; b&#7879; &lt;i&gt;</p><script>var x='<p>no</p>'</script><!-- <p>c</p> --><div>d<br>e &nope; f"), ["", "A & bệ <i>", "", "d", "e &nope; f"]);
        assert_eq!(md_paras("# H\na\nb\n\n- x\n- y\n  z\n1. n"), ["H", "a b", "- x", "- y z", "1. n"]);
        assert_eq!(docx_paras("<w:p><w:r><w:t>a</w:t><w:tab/><w:t>b</w:t></w:r><w:del><w:r><w:t>x</w:t></w:r></w:del></w:p><w:p/>").unwrap(), ["a b"]);
        assert_eq!(odt_paras("<o><text:h>T</text:h><text:tracked-changes><text:p>old</text:p></text:tracked-changes><text:p>a<text:s/>b<text:span>c</text:span></text:p></o>").unwrap(), ["T", "a bc"]);
        let line = |right: f32, bottom: f32, t: &str| Line { left: 72.0, right, bottom, height: 11.0, text: t.into() };
        let page = [line(520.0, 700.0, "một"), line(300.0, 686.0, "hai."), line(523.0, 672.0, "ba"), line(521.0, 644.0, "bốn"), line(200.0, 630.0, "năm"), line(522.0, 760.0, "cột")];
        assert_eq!(reflow(&page, 595.0), ["một hai.", "ba", "bốn năm", "cột"]);
    }

    #[test]
    fn redline_page() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let same: Vec<&str> = (0..9).map(|_| "giữ").collect();
        let (mut a, mut b) = (v(&["Giá là 10 triệu <b>", "bỏ đoạn này đi nhé"]), v(&["thêm mới hoàn toàn", "Giá là 12 triệu <b>"]));
        a.extend(v(&same));
        b.extend(v(&same));
        b.push("cuối".into());
        let (html, counts) = redline(("a<.md", "t", &a), ("b.md", "t", &b));
        assert_eq!(counts, [2, 1, 1]);
        assert!(html.contains("<p class=\"chg\">Giá là <del>10</del><ins>12</ins> triệu &lt;b&gt;</p>"), "{html}");
        assert!(html.contains("… 5 unchanged paragraphs …") && html.matches("<p>giữ</p>").count() == 4 && html.contains("<h1>a&lt;.md <span>"));
        assert_eq!(esc_html("<a href='x'>&\u{1}"), "&lt;a href=&#39;x&#39;&gt;&amp; ");
    }

    #[test]
    fn pictures_get_lighter_or_stay() {
        let mut png = Vec::new();
        image::RgbImage::from_fn(2400, 300, |x, y| image::Rgb([(x / 10) as u8, (y / 2) as u8, 90])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let small = lighter(&png, false).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().width(), SIDE);
        let mut jpg = Vec::new();
        image::RgbImage::from_fn(600, 400, |x, y| image::Rgb([(x / 3) as u8, (y / 2) as u8, 90])).write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
        assert_eq!(image::guess_format(&lighter(&jpg, true).unwrap()).unwrap(), image::ImageFormat::Jpeg);
        assert!(lighter(&jpg[..jpg.len() / 2], true).is_none() && lighter(&jpg, false).is_none() && lighter(b"\x89PNG\r\n\x1a\nrubbish", false).is_none() && lighter(&[], true).is_none());
        assert_eq!((megabytes(12_400_000), megabytes(30_400)), ("12.4 MB".to_string(), "30 KB".to_string()));
    }
}
