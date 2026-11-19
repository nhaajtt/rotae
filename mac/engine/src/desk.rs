//! Desk work: spreadsheets, Office documents and the small chores of writing software.
//! Everything here reads Office files as what they are (a zip of XML) with the system's own
//! `unzip`/`zip`, so nothing needs Office or LibreOffice installed.
use crate::engines::{note, text::xml_ref};
use crate::naming::{ext, out_dir, out_path, stem, temp_file, unique};
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub fn run(paths: &[PathBuf], verb: &str, arg: &str) -> Option<Result<Vec<PathBuf>>> {
    let src = &paths[0];
    Some(match (verb, arg) {
        ("sheet", "xlsx") => to_xlsx(src),
        ("sheet", "csv") => to_csv(src),
        ("sheet", "excel") => csv_for_excel(src),
        ("sheet", "totals") => totals(src),
        ("sheet", "merge") => merge(paths),
        ("office", "media") => media(src),
        ("office", "clean") => clean(src),
        ("office", "stats") => stats(src),
        ("office", "text") => slides_text(src),
        ("dev", "types") => types(src),
        ("dev", "sortkeys") => sort_keys(src),
        ("dev", "datauri") => data_uri(src),
        ("dev", "scales") => scales(src),
        ("verify", _) => verify(src, arg),
        _ => return crate::ledger::run(paths, verb, arg).or_else(|| crate::papers::run(paths, verb, arg)).or_else(|| crate::devkit::run(paths, verb, arg)),
    })
}

/// Group work sees every dropped file at once. Must agree with `isGroup` in the app.
pub fn is_group(verb: &str, arg: &str) -> bool {
    (verb == "sheet" && arg == "merge") || crate::ledger::is_group(verb, arg) || crate::papers::is_group(verb, arg)
}

// ---------------------------------------------------------------- zip, read-only

/// One entry of a zip, never more than `cap_mb`: a small file can claim to unpack to anything.
pub(crate) fn entry(src: &Path, name: &str, cap_mb: u64) -> Result<Vec<u8>> {
    // unzip reads * ? [ ] as a pattern
    let pat: String = name.chars().flat_map(|c| if matches!(c, '*' | '?' | '[' | ']' | '\\') { vec!['\\', c] } else { vec![c] }).collect();
    let mut child = Command::new("/usr/bin/unzip").arg("-p").arg(src).arg(pat).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let cap = cap_mb << 20;
    let mut buf = Vec::new();
    let read = child.stdout.take().context("no output")?.take(cap + 1).read_to_end(&mut buf);
    if read.is_err() || buf.len() as u64 > cap {
        let _ = child.kill();
        let _ = child.wait();
        bail!("this file is too big for this (over {cap_mb} MB inside)");
    }
    let _ = child.wait();
    Ok(buf)
}

pub(crate) fn names(src: &Path) -> Result<Vec<String>> {
    let o = crate::engines::tools::output_within(Command::new("/usr/bin/unzip").arg("-Z1").arg(src), std::time::Duration::from_secs(60))?;
    if !o.status.success() {
        bail!("can't read this document: it may be damaged or password-protected");
    }
    Ok(String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect())
}

pub(crate) fn xml_text(src: &Path, name: &str) -> Result<String> {
    String::from_utf8(entry(src, name, 256)?).map_err(|_| anyhow!("can't read this document: it may be damaged or password-protected"))
}

pub(crate) fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

pub(crate) fn piece(ev: &Event) -> String {
    match ev {
        Event::Text(t) => t.xml10_content().into_owned(),
        Event::GeneralRef(r) => xml_ref(r),
        _ => String::new(),
    }
}

pub(crate) fn ooxml(src: &Path) -> Result<&'static str> {
    Ok(match ext(src).as_str() {
        "docx" | "docm" => "word",
        "xlsx" | "xlsm" => "xl",
        "pptx" => "ppt",
        _ => bail!("this works on .docx, .xlsx and .pptx files; save older formats as one of those first"),
    })
}

// ---------------------------------------------------------------- tables

pub(crate) type Table = Vec<Vec<String>>;

pub(crate) fn read_text(src: &Path) -> Result<String> {
    let b = crate::engines::files::read_capped(src, 200)?;
    Ok(match b.as_slice() {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).map_err(|_| anyhow!("not a text file"))?,
        // Excel's "Unicode Text" export
        [0xFF, 0xFE, rest @ ..] => String::from_utf16_lossy(&rest.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>()),
        [0xFE, 0xFF, rest @ ..] => String::from_utf16_lossy(&rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect::<Vec<_>>()),
        _ => String::from_utf8(b).map_err(|_| anyhow!("not a text file: save it as UTF-8 first"))?,
    })
}

pub(crate) fn delimiter(s: &str, e: &str) -> char {
    if e == "tsv" {
        return '\t';
    }
    let first = s.lines().next().unwrap_or("");
    let (mut quoted, mut n) = (false, [0usize; 3]);
    for c in first.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => n[0] += 1,
            ';' if !quoted => n[1] += 1,
            '\t' if !quoted => n[2] += 1,
            _ => {}
        }
    }
    if n[1] > n[0] && n[1] >= n[2] {
        ';'
    } else if n[2] > n[0] {
        '\t'
    } else {
        ','
    }
}

pub(crate) fn parse_csv(s: &str, d: char) -> Table {
    let (mut rows, mut row, mut f) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut any) = (false, false);
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if quoted {
            if c == '"' {
                if it.peek() == Some(&'"') {
                    f.push('"');
                    it.next();
                } else {
                    quoted = false;
                }
            } else {
                f.push(c);
            }
        } else if c == '"' && f.is_empty() {
            quoted = true;
            any = true;
        } else if c == d {
            row.push(std::mem::take(&mut f));
            any = true;
        } else if c == '\n' || c == '\r' {
            if c == '\r' && it.peek() == Some(&'\n') {
                it.next();
            }
            if any || !f.is_empty() || !row.is_empty() {
                row.push(std::mem::take(&mut f));
                rows.push(std::mem::take(&mut row));
            }
            any = false;
        } else {
            f.push(c);
        }
    }
    if any || !f.is_empty() || !row.is_empty() {
        row.push(f);
        rows.push(row);
    }
    rows
}

/// Text that a spreadsheet would run as a formula gets a leading apostrophe, as elsewhere.
pub(crate) fn inert(v: &str) -> String {
    let risky = v.starts_with(['=', '+', '@', '\t', '\r']) || (v.starts_with('-') && amount(v).is_none());
    if risky { format!("'{v}") } else { v.to_string() }
}

pub(crate) fn csv_field(v: &str, d: char) -> String {
    if v.contains([d, '"', '\n', '\r']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() }
}

pub(crate) fn write_csv(t: &Table, eol: &str) -> String {
    let mut out = String::new();
    for r in t {
        out.push_str(&r.iter().map(|v| csv_field(v, ',')).collect::<Vec<_>>().join(","));
        out.push_str(eol);
    }
    out
}

pub(crate) fn table(src: &Path) -> Result<Table> {
    match ext(src).as_str() {
        "xlsx" | "xlsm" => Ok(sheets(src)?.into_iter().next().map(|s| s.1).unwrap_or_default()),
        e => {
            let s = read_text(src)?;
            Ok(parse_csv(&s, delimiter(&s, e)))
        }
    }
}

// ---------------------------------------------------------------- xlsx, reading

pub(crate) fn col_index(cell: &str) -> usize {
    cell.bytes().take_while(u8::is_ascii_alphabetic).fold(0usize, |n, b| n * 26 + (b.to_ascii_uppercase() - b'A') as usize + 1).saturating_sub(1)
}

pub(crate) fn col_name(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

pub(crate) fn is_date_format(id: u32, custom: &HashMap<u32, String>) -> bool {
    if matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58) {
        return true;
    }
    custom.get(&id).is_some_and(|f| {
        // letters inside "quotes" or [brackets] are text or colours, not date parts
        let (mut q, mut b) = (false, false);
        f.chars().any(|c| {
            match c {
                '"' => q = !q,
                '[' => b = true,
                ']' => b = false,
                _ => {}
            }
            !q && !b && matches!(c.to_ascii_lowercase(), 'd' | 'm' | 'y' | 'h' | 's')
        })
    })
}

/// An Excel day number as a date, a time, or both.
pub(crate) fn serial(v: f64, d1904: bool) -> String {
    let days = v.floor();
    let secs = ((v - days) * 86400.0).round() as i64;
    let (days, secs) = if secs >= 86400 { (days as i64 + 1, 0) } else { (days as i64, secs) };
    let clock = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if days == 0 && !d1904 {
        return clock;
    }
    // days since 1970-01-01; Excel's day 25569 is that date (24107 in the 1904 system)
    let z = days - if d1904 { 24107 } else { 25569 } + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    if secs == 0 { format!("{y:04}-{m:02}-{d:02}") } else { format!("{y:04}-{m:02}-{d:02} {clock}") }
}

/// Excel keeps 15 significant digits; printing more shows noise like 0.30000000000000004.
pub(crate) fn tidy_number(raw: &str) -> String {
    match raw.parse::<f64>() {
        Ok(v) if v.is_finite() => format!("{:.14e}", v).parse::<f64>().map(|r| r.to_string()).unwrap_or_else(|_| raw.to_string()),
        _ => raw.to_string(),
    }
}

pub(crate) fn sheets(src: &Path) -> Result<Vec<(String, Table)>> {
    let book = xml_text(src, "xl/workbook.xml").context("not an Excel workbook")?;
    let (mut listed, mut d1904) = (Vec::new(), false);
    let mut r = quick_xml::Reader::from_str(&book);
    loop {
        match r.read_event().context("broken workbook")? {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                "sheet" => listed.push((attr(&e, "name").unwrap_or_default(), attr(&e, "r:id").unwrap_or_default())),
                "workbookPr" => d1904 = matches!(attr(&e, "date1904").as_deref(), Some("1" | "true")),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let mut target = HashMap::new();
    if let Ok(rels) = xml_text(src, "xl/_rels/workbook.xml.rels") {
        let mut r = quick_xml::Reader::from_str(&rels);
        while let Ok(ev) = r.read_event() {
            match ev {
                Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "Relationship" => {
                    if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                        target.insert(id, t);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }
    // shared strings: one per <si>, its <t> pieces joined (phonetic guides left out)
    let mut shared = Vec::new();
    if let Ok(x) = xml_text(src, "xl/sharedStrings.xml") {
        let mut r = quick_xml::Reader::from_str(&x);
        let (mut cur, mut in_t, mut skip) = (String::new(), false, 0u32);
        loop {
            match r.read_event().context("broken workbook")? {
                Event::Start(e) => match e.local_name().as_ref() {
                    "si" => cur.clear(),
                    "rPh" => skip += 1,
                    "t" => in_t = skip == 0,
                    _ => {}
                },
                Event::End(e) => match e.local_name().as_ref() {
                    "si" => shared.push(std::mem::take(&mut cur)),
                    "rPh" => skip = skip.saturating_sub(1),
                    "t" => in_t = false,
                    _ => {}
                },
                Event::Empty(e) if e.local_name().as_ref() == "si" => shared.push(String::new()),
                ev @ (Event::Text(_) | Event::GeneralRef(_)) if in_t => cur.push_str(&piece(&ev)),
                Event::Eof => break,
                _ => {}
            }
        }
    }
    // which cell styles show a number as a date
    let mut dates = Vec::new();
    if let Ok(x) = xml_text(src, "xl/styles.xml") {
        let mut r = quick_xml::Reader::from_str(&x);
        let (mut custom, mut in_xfs) = (HashMap::new(), false);
        while let Ok(ev) = r.read_event() {
            match ev {
                Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (attr(&e, "numFmtId").and_then(|v| v.parse().ok()), attr(&e, "formatCode")) {
                            custom.insert(id, code);
                        }
                    }
                    "cellXfs" => in_xfs = true,
                    "xf" if in_xfs => dates.push(is_date_format(attr(&e, "numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0), &custom)),
                    _ => {}
                },
                Event::End(e) if e.local_name().as_ref() == "cellXfs" => in_xfs = false,
                Event::Eof => break,
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for (i, (name, id)) in listed.iter().enumerate() {
        let path = match target.get(id) {
            Some(t) if t.starts_with('/') => t.trim_start_matches('/').to_string(),
            Some(t) => format!("xl/{t}"),
            None => format!("xl/worksheets/sheet{}.xml", i + 1),
        };
        // chart sheets and the like have no cells
        if !path.contains("worksheets/") {
            continue;
        }
        let x = xml_text(src, &path)?;
        out.push((name.clone(), sheet(&x, &shared, &dates, d1904)?));
    }
    if out.is_empty() {
        bail!("this workbook has no sheets with cells");
    }
    Ok(out)
}

pub(crate) fn sheet(x: &str, shared: &[String], dates: &[bool], d1904: bool) -> Result<Table> {
    let mut r = quick_xml::Reader::from_str(x);
    let mut rows: Table = Vec::new();
    let (mut row, mut col, mut kind, mut style) = (Vec::<String>::new(), 0usize, String::new(), 0usize);
    let (mut val, mut in_v, mut in_t, mut cells) = (String::new(), false, false, 0u64);
    loop {
        match r.read_event().context("broken sheet")? {
            Event::Start(e) => match e.local_name().as_ref() {
                "row" => {
                    // rows left out of the file are empty rows
                    if let Some(n) = attr(&e, "r").and_then(|v| v.parse::<usize>().ok()) {
                        while rows.len() + 1 < n && rows.len() < 1_048_576 {
                            rows.push(Vec::new());
                        }
                    }
                    row.clear();
                }
                "c" => {
                    col = attr(&e, "r").map(|c| col_index(&c)).unwrap_or(row.len());
                    kind = attr(&e, "t").unwrap_or_default();
                    style = attr(&e, "s").and_then(|v| v.parse().ok()).unwrap_or(0);
                    val.clear();
                }
                "v" => in_v = true,
                "t" => in_t = true,
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                "v" => in_v = false,
                "t" => in_t = false,
                "c" => {
                    let text = match kind.as_str() {
                        "s" => inert(val.trim().parse::<usize>().ok().and_then(|i| shared.get(i)).map(String::as_str).unwrap_or("")),
                        "inlineStr" | "str" => inert(&val),
                        "b" => if val.trim() == "1" { "TRUE".into() } else { "FALSE".into() },
                        "e" => val.clone(),
                        _ if val.is_empty() => String::new(),
                        _ if dates.get(style).copied().unwrap_or(false) => val.trim().parse::<f64>().map(|v| serial(v, d1904)).unwrap_or_else(|_| val.clone()),
                        _ => tidy_number(val.trim()),
                    };
                    if col >= 16_384 {
                        bail!("this sheet is too big for this");
                    }
                    while row.len() < col {
                        row.push(String::new());
                    }
                    row.push(text);
                    cells += 1;
                    if cells > 20_000_000 {
                        bail!("this sheet is too big for this");
                    }
                }
                "row" => rows.push(std::mem::take(&mut row)),
                _ => {}
            },
            ev @ (Event::Text(_) | Event::GeneralRef(_)) if in_v || in_t => val.push_str(&piece(&ev)),
            Event::Eof => break,
            _ => {}
        }
    }
    // a sheet is as wide as its widest row, and ends at its last row with anything in it
    while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
        rows.pop();
    }
    let width = rows.iter().map(|r| r.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1)).max().unwrap_or(0);
    for r in &mut rows {
        r.resize(width, String::new());
    }
    Ok(rows)
}

pub(crate) fn to_csv(src: &Path) -> Result<Vec<PathBuf>> {
    let all = sheets(src)?;
    let many = all.iter().filter(|s| !s.1.is_empty()).count() > 1;
    let mut out = Vec::new();
    for (name, t) in all {
        if t.is_empty() {
            continue;
        }
        // a sheet name can hold characters a file name can't
        let safe: String = name.chars().map(|c| if matches!(c, '/' | ':' | '\\' | '\0') || c.is_control() { '-' } else { c }).collect();
        let p = out_path(src, if many { Some(safe.trim()) } else { None }, "csv");
        std::fs::write(&p, write_csv(&t, "\n"))?;
        out.push(p);
    }
    if out.is_empty() {
        bail!("this workbook has no sheets with cells");
    }
    Ok(out)
}

// ---------------------------------------------------------------- xlsx, writing

pub(crate) fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\t' | '\n' | '\r' => o.push(c),
            c if (c as u32) < 0x20 || matches!(c as u32, 0xFFFE | 0xFFFF) => {}
            c => o.push(c),
        }
    }
    o
}

/// A plain number Excel can hold exactly. Codes with leading zeros, long account numbers and
/// anything with separators stay text, so nothing is silently changed.
pub(crate) fn plain_number(s: &str) -> bool {
    let b = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = b.split_once('.').unwrap_or((b, "0"));
    !int.is_empty()
        && !frac.is_empty()
        && int.bytes().all(|c| c.is_ascii_digit())
        && frac.bytes().all(|c| c.is_ascii_digit())
        && (int == "0" || !int.starts_with('0'))
        && int.len() + if b.contains('.') { frac.len() } else { 0 } <= 15
}

/// Whether a cell may be stored as a number without changing what it means. "120.000" is one
/// hundred and twenty thousand in a Vietnamese ledger and 120 in an American one, so unless the
/// table shows which, it stays exactly as typed.
pub(crate) fn safe_number(v: &str, how: Marks) -> bool {
    if !plain_number(v) {
        return false;
    }
    match v.split_once('.') {
        None => true,
        Some(_) if how == Marks::DotGroups => false,
        Some((_, frac)) => frac.len() != 3 || how == Marks::CommaGroups,
    }
}

pub(crate) fn workbook(t: &Table, out: &Path) -> Result<()> {
    let how = marks(t);
    if t.len() > 1_048_576 || t.iter().any(|r| r.len() > 16_384) {
        bail!("this table is too big for an Excel sheet (over 1,048,576 rows or 16,384 columns)");
    }
    let width = t.iter().map(Vec::len).max().unwrap_or(0);
    let header = t.len() > 1 && t[0].iter().any(|c| !c.is_empty()) && !t[0].iter().any(|c| plain_number(c));
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">");
    if header {
        s.push_str("<sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>");
    }
    if width > 0 {
        s.push_str("<cols>");
        for c in 0..width {
            let longest = t.iter().take(2000).filter_map(|r| r.get(c)).map(|v| v.lines().map(|l| l.chars().count()).max().unwrap_or(0)).max().unwrap_or(0);
            s.push_str(&format!("<col min=\"{0}\" max=\"{0}\" width=\"{1}\" customWidth=\"1\"/>", c + 1, (longest + 2).clamp(8, 60)));
        }
        s.push_str("</cols>");
    }
    s.push_str("<sheetData>");
    for (ri, row) in t.iter().enumerate() {
        s.push_str(&format!("<row r=\"{}\">", ri + 1));
        for (ci, v) in row.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            let at = format!("{}{}", col_name(ci), ri + 1);
            let bold = if header && ri == 0 { " s=\"1\"" } else { "" };
            if safe_number(v, how) && !(header && ri == 0) {
                s.push_str(&format!("<c r=\"{at}\"><v>{v}</v></c>"));
            } else {
                s.push_str(&format!("<c r=\"{at}\" t=\"inlineStr\"{bold}><is><t xml:space=\"preserve\">{}</t></is></c>", xml_escape(v)));
            }
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData>");
    if header && width > 0 {
        s.push_str(&format!("<autoFilter ref=\"A1:{}{}\"/>", col_name(width - 1), t.len()));
    }
    s.push_str("</worksheet>");

    let dir = temp_file("xlsx");
    let made = (|| -> Result<()> {
        for d in ["_rels", "xl/_rels", "xl/worksheets"] {
            std::fs::create_dir_all(dir.join(d))?;
        }
        std::fs::write(dir.join("[Content_Types].xml"), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/></Types>")?;
        std::fs::write(dir.join("_rels/.rels"), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")?;
        std::fs::write(dir.join("xl/workbook.xml"), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>")?;
        std::fs::write(dir.join("xl/_rels/workbook.xml.rels"), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>")?;
        std::fs::write(dir.join("xl/styles.xml"), "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"2\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/></cellXfs></styleSheet>")?;
        std::fs::write(dir.join("xl/worksheets/sheet1.xml"), &s)?;
        // zip into a fresh file, then put it in the place already reserved for the result
        let packed = dir.join("out.zip");
        let o = crate::engines::tools::output_within(Command::new("/usr/bin/zip").current_dir(&dir).args(["-q", "-X", "-r"]).arg(&packed).args(["[Content_Types].xml", "_rels", "xl"]), std::time::Duration::from_secs(300))?;
        if !o.status.success() {
            bail!("couldn't write the workbook: {}", String::from_utf8_lossy(&o.stderr).trim());
        }
        std::fs::copy(&packed, out)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    made
}

pub(crate) fn to_xlsx(src: &Path) -> Result<Vec<PathBuf>> {
    let t = table(src)?;
    if t.is_empty() {
        bail!("this file has no rows");
    }
    let out = out_path(src, None, "xlsx");
    workbook(&t, &out)?;
    Ok(vec![out])
}

/// Excel reads a CSV as the system's old code page unless it starts with a byte-order mark,
/// which turns Vietnamese (and most other languages) into garbage. This is the form it opens right.
pub(crate) fn csv_for_excel(src: &Path) -> Result<Vec<PathBuf>> {
    let s = read_text(src)?;
    let t = parse_csv(&s, delimiter(&s, &ext(src)));
    let out = out_path(src, Some("for Excel"), "csv");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(write_csv(&t, "\r\n").as_bytes());
    std::fs::write(&out, bytes)?;
    Ok(vec![out])
}

// ---------------------------------------------------------------- totals

/// An amount as written in a ledger, in millionths so sums are exact: 1,234.56 · 1.234,56 ·
/// (1 234) · -5 · 12,5% · 1.200.000 ₫. `None` when the cell isn't a number.
pub(crate) fn amount(raw: &str) -> Option<i128> {
    amount_as(raw, Marks::Unknown)
}

/// Which mark groups thousands in a table. "120.000" alone could be 120 or 120,000; another
/// cell like "1.200.000" or "1.234,5" in the same table settles it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Marks {
    Unknown,
    DotGroups,
    CommaGroups,
}

pub(crate) fn marks(t: &Table) -> Marks {
    let (mut dot, mut comma) = (0, 0);
    for v in t.iter().flatten() {
        let s: String = v.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',')).collect();
        if s.len() != v.chars().filter(|c| !c.is_whitespace() && !matches!(c, '(' | ')' | '-' | '+' | '₫' | 'đ' | '$' | '€' | '%')).count() {
            continue; // has letters or other signs: not an amount
        }
        let (d, c) = (s.matches('.').count(), s.matches(',').count());
        let groups = |sep: char| s.split(sep).skip(1).all(|g| g.len() == 3);
        if (d > 1 && c == 0 && groups('.')) || (d >= 1 && c == 1 && s.rfind(',') > s.rfind('.')) {
            dot += 1;
        } else if (c > 1 && d == 0 && groups(',')) || (c >= 1 && d == 1 && s.rfind('.') > s.rfind(',')) {
            comma += 1;
        }
    }
    if dot > comma { Marks::DotGroups } else if comma > dot { Marks::CommaGroups } else { Marks::Unknown }
}

pub(crate) fn amount_as(raw: &str, marks: Marks) -> Option<i128> {
    let mut s: String = raw.trim().chars().filter(|c| !c.is_whitespace() && !matches!(c, '₫' | '$' | '€' | '£' | '¥' | '%' | '\u{a0}' | '\u{202f}')).collect();
    for unit in ["VND", "VNĐ", "USD", "EUR", "đ", "Đ"] {
        if let Some(r) = s.strip_suffix(unit).or_else(|| s.strip_prefix(unit)) {
            s = r.to_string();
        }
    }
    let mut neg = false;
    if s.starts_with('(') && s.ends_with(')') && s.len() > 2 {
        neg = true;
        s = s[1..s.len() - 1].to_string();
    }
    if let Some(r) = s.strip_prefix('-').or_else(|| s.strip_suffix('-')) {
        neg = !neg;
        s = r.to_string();
    } else if let Some(r) = s.strip_prefix('+') {
        s = r.to_string();
    }
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b',') || !s.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let (dots, commas) = (s.matches('.').count(), s.matches(',').count());
    // groups of exactly three digits after the first mean the mark separates thousands
    let grouped = |sep: char| {
        let mut p = s.split(sep);
        let first = p.next().unwrap_or("");
        !first.is_empty() && first.len() <= 3 && first != "0" && p.all(|g| g.len() == 3)
    };
    // several of one mark can only be thousands, and then every group must be three digits
    if (dots > 1 && commas == 0 && !grouped('.')) || (commas > 1 && dots == 0 && !grouped(',')) {
        return None;
    }
    let decimal = match (dots, commas) {
        (0, 0) => None,
        // the table has shown which mark it groups thousands with
        (1, 0) if marks == Marks::DotGroups && grouped('.') => None,
        (0, 1) if marks == Marks::CommaGroups && grouped(',') => None,
        (0, 1) if marks == Marks::DotGroups => Some(','),
        (1, 0) => Some('.'),
        (_, 0) => None,
        // "1,234" is a thousand and something; "12,5" is twelve and a half
        (0, 1) => if grouped(',') { None } else { Some(',') },
        (0, _) => None,
        _ => Some(if s.rfind('.') > s.rfind(',') { '.' } else { ',' }),
    };
    let (int, frac) = match decimal {
        Some(d) => {
            let (a, b) = s.rsplit_once(d)?;
            (a.replace(['.', ','], ""), b.to_string())
        }
        None => (s.replace(['.', ','], ""), String::new()),
    };
    if frac.contains(['.', ',']) || int.len() > 24 || frac.len() > 12 {
        return None;
    }
    let mut f = frac;
    f.truncate(6);
    while f.len() < 6 {
        f.push('0');
    }
    let v = if int.is_empty() { 0 } else { int.parse::<i128>().ok()? } * 1_000_000 + f.parse::<i128>().ok()?;
    Some(if neg { -v } else { v })
}

pub(crate) fn money(v: i128) -> String {
    let (neg, v) = (v < 0, v.unsigned_abs());
    let (int, frac) = (v / 1_000_000, v % 1_000_000);
    let digits = int.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let f = format!("{frac:06}");
    let f = f.trim_end_matches('0');
    format!("{}{grouped}{}{f}", if neg { "-" } else { "" }, if f.is_empty() { "" } else { "." })
}

pub(crate) fn totals(src: &Path) -> Result<Vec<PathBuf>> {
    let t = table(src)?;
    if t.is_empty() {
        bail!("this file has no rows");
    }
    let width = t.iter().map(Vec::len).max().unwrap_or(0);
    let header = t.len() > 1 && t[0].iter().all(|c| amount(c).is_none());
    let body = &t[if header { 1 } else { 0 }..];
    let how = marks(&t);
    let mut report = format!("{}\n{} rows\n\n", src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), body.len());
    let mut first = None;
    for c in 0..width {
        let cells: Vec<&str> = body.iter().filter_map(|r| r.get(c)).map(|v| v.trim()).filter(|v| !v.is_empty()).collect();
        // customer numbers, tax codes, account numbers: digits, but nothing to add up
        if cells.iter().any(|v| (v.len() > 1 && v.starts_with('0') && v.bytes().all(|b| b.is_ascii_digit())) || (v.len() > 15 && v.bytes().all(|b| b.is_ascii_digit()))) {
            continue;
        }
        let nums: Vec<i128> = cells.iter().filter_map(|v| amount_as(v, how)).collect();
        // a column counts as numbers when nearly all of what's in it reads as one
        if nums.is_empty() || nums.len() * 10 < cells.len() * 8 {
            continue;
        }
        let name = if header { t[0].get(c).cloned().unwrap_or_default() } else { String::new() };
        let name = if name.trim().is_empty() { format!("Column {}", col_name(c)) } else { name.trim().to_string() };
        let sum: i128 = nums.iter().sum();
        let (min, max) = (nums.iter().min().copied().unwrap_or(0), nums.iter().max().copied().unwrap_or(0));
        report.push_str(&format!("{name}\n  Sum      {}\n  Count    {}\n  Average  {}\n  Smallest {}\n  Largest  {}\n", money(sum), nums.len(), money(sum / nums.len() as i128), money(min), money(max)));
        if nums.len() < cells.len() {
            report.push_str(&format!("  Skipped  {} cells that aren't numbers\n", cells.len() - nums.len()));
        }
        report.push('\n');
        first.get_or_insert((name, sum));
    }
    let Some((name, sum)) = first else { bail!("no columns of numbers found in this file") };
    let out = out_path(src, Some("Totals"), "txt");
    std::fs::write(&out, &report)?;
    crate::clipboard::set(&money(sum))?;
    note(&format!("{name}: {} (copied)", money(sum)));
    Ok(vec![out])
}

/// Stacks tables on top of each other. Same headers line up by name; a column only some files
/// have is kept, empty where it was missing.
pub(crate) fn merge(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    if paths.len() < 2 {
        bail!("choose two or more files to merge");
    }
    let mut head: Vec<String> = Vec::new();
    let mut rows: Table = Vec::new();
    for p in paths {
        let t = table(p)?;
        let Some((h, body)) = t.split_first() else { continue };
        let map: Vec<usize> = h
            .iter()
            .map(|name| {
                head.iter().position(|x| x.trim().eq_ignore_ascii_case(name.trim())).unwrap_or_else(|| {
                    head.push(name.clone());
                    head.len() - 1
                })
            })
            .collect();
        for r in body {
            let mut line = vec![String::new(); head.len()];
            for (i, v) in r.iter().enumerate() {
                if let Some(&to) = map.get(i) {
                    line[to] = v.clone();
                }
            }
            rows.push(line);
        }
    }
    if head.is_empty() {
        bail!("these files have no rows");
    }
    for r in &mut rows {
        r.resize(head.len(), String::new());
    }
    let n = rows.len();
    rows.insert(0, head);
    let dir = paths[0].parent().context("no folder")?;
    let out = unique(dir, "Merged", "csv");
    std::fs::write(&out, write_csv(&rows, "\n"))?;
    note(&format!("{n} rows from {} files", paths.len()));
    Ok(vec![out])
}

// ---------------------------------------------------------------- office documents

pub(crate) fn media(src: &Path) -> Result<Vec<PathBuf>> {
    let root = ooxml(src)?;
    let prefix = format!("{root}/media/");
    let found: Vec<String> = names(src)?.into_iter().filter(|n| n.starts_with(&prefix) && !n.ends_with('/')).collect();
    if found.is_empty() {
        bail!("no pictures found inside this document");
    }
    let dir = out_dir(src, "media")?;
    for n in &found {
        // only ever the last part of the name: nothing inside the document chooses a folder
        let base = Path::new(n).file_name().map(|b| b.to_string_lossy().into_owned()).unwrap_or_default();
        if base.is_empty() || base == "." || base == ".." {
            continue;
        }
        let p = Path::new(&base);
        let out = unique(&dir, &stem(p), &ext(p));
        std::fs::write(out, entry(src, n, 512)?)?;
    }
    note(&format!("{} files", found.len()));
    Ok(vec![dir])
}

/// Empties `<tag>…</tag>` for each tag, leaving the element so the file stays valid.
pub(crate) fn blank(xml: &str, tags: &[&str]) -> String {
    let mut s = xml.to_string();
    for t in tags {
        let (open, close) = (format!("<{t}"), format!("</{t}>"));
        let mut from = 0;
        while let Some(a) = s[from..].find(&open).map(|i| i + from) {
            let after = a + open.len();
            // "<dc:creator" must not match "<dc:creatorX"
            if !s[after..].starts_with(['>', ' ', '/']) {
                from = after;
                continue;
            }
            let Some(gt) = s[after..].find('>').map(|i| i + after) else { break };
            if s[..gt].ends_with('/') {
                from = gt;
                continue;
            }
            let Some(end) = s[gt..].find(&close).map(|i| i + gt) else { break };
            s.replace_range(gt + 1..end, "");
            from = gt + 1 + close.len();
        }
    }
    s
}

/// Empties every `name="…"` attribute value.
pub(crate) fn blank_attr(xml: &str, name: &str) -> String {
    let key = format!(" {name}=\"");
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(i) = rest.find(&key) {
        out.push_str(&rest[..i + key.len()]);
        rest = &rest[i + key.len()..];
        match rest.find('"') {
            Some(q) => rest = &rest[q..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

pub(crate) fn clean(src: &Path) -> Result<Vec<PathBuf>> {
    let root = ooxml(src)?;
    let all = names(src)?;
    let dir = temp_file("clean");
    let out = out_path(src, Some("Clean"), &ext(src));
    let done = (|| -> Result<usize> {
        let mut changed: Vec<String> = Vec::new();
        let mut put = |name: &str, body: String| -> Result<()> {
            let p = dir.join(name);
            std::fs::create_dir_all(p.parent().context("no folder")?)?;
            std::fs::write(p, body)?;
            changed.push(name.to_string());
            Ok(())
        };
        if all.iter().any(|n| n == "docProps/core.xml") {
            put("docProps/core.xml", blank(&xml_text(src, "docProps/core.xml")?, &["dc:creator", "cp:lastModifiedBy", "cp:lastPrinted", "dc:description", "cp:keywords", "cp:category", "cp:contentStatus"]))?;
        }
        if all.iter().any(|n| n == "docProps/app.xml") {
            put("docProps/app.xml", blank(&xml_text(src, "docProps/app.xml")?, &["Company", "Manager", "HyperlinkBase", "TotalTime"]))?;
        }
        // who wrote each comment and tracked change
        let authored: &[&str] = match root {
            "word" => &["word/document.xml", "word/comments.xml", "word/footnotes.xml", "word/endnotes.xml"],
            "ppt" => &["ppt/commentAuthors.xml"],
            _ => &["xl/comments1.xml", "xl/comments2.xml", "xl/comments3.xml"],
        };
        for name in authored.iter().filter(|n| all.iter().any(|a| a == *n)) {
            let x = xml_text(src, name)?;
            let y = match root {
                "word" => blank_attr(&blank_attr(&x, "w:author"), "w:initials"),
                "ppt" => blank_attr(&blank_attr(&x, "name"), "initials"),
                _ => blank(&x, &["author"]),
            };
            if y != x {
                put(name, y)?;
            }
        }
        std::fs::copy(src, &out)?;
        if !changed.is_empty() {
            // zip replaces just these parts inside the copy; everything else is untouched
            let o = crate::engines::tools::output_within(Command::new("/usr/bin/zip").current_dir(&dir).args(["-q", "-X"]).arg(&out).args(&changed), std::time::Duration::from_secs(300))?;
            if !o.status.success() {
                bail!("couldn't write the document: {}", String::from_utf8_lossy(&o.stderr).trim());
            }
        }
        Ok(changed.len())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    done?;
    note("Author, company and reviewer names removed");
    Ok(vec![out])
}

pub(crate) fn count_words(xml: &str, tag: &str) -> (usize, usize) {
    let mut r = quick_xml::Reader::from_str(xml);
    let (mut words, mut chars, mut in_t, mut cur) = (0, 0, false, String::new());
    while let Ok(ev) = r.read_event() {
        match ev {
            Event::Start(e) if e.local_name().as_ref() == tag => in_t = true,
            Event::End(e) if e.local_name().as_ref() == tag => in_t = false,
            // a paragraph ends a word even with no space typed
            Event::End(e) if e.local_name().as_ref() == "p" => {
                words += cur.split_whitespace().count();
                chars += cur.chars().filter(|c| !c.is_whitespace()).count();
                cur.clear();
            }
            ev @ (Event::Text(_) | Event::GeneralRef(_)) if in_t => cur.push_str(&piece(&ev)),
            Event::Eof => break,
            _ => {}
        }
    }
    (words + cur.split_whitespace().count(), chars + cur.chars().filter(|c| !c.is_whitespace()).count())
}

pub(crate) fn slide_names(all: &[String]) -> Vec<String> {
    let mut slides: Vec<(u32, String)> = all
        .iter()
        .filter_map(|n| n.strip_prefix("ppt/slides/slide").and_then(|r| r.strip_suffix(".xml")).and_then(|d| d.parse().ok()).map(|i| (i, n.clone())))
        .collect();
    slides.sort();
    slides.into_iter().map(|s| s.1).collect()
}

pub(crate) fn stats(src: &Path) -> Result<Vec<PathBuf>> {
    let root = ooxml(src)?;
    let all = names(src)?;
    let app = xml_text(src, "docProps/app.xml").unwrap_or_default();
    let prop = |tag: &str| app.split_once(&format!("<{tag}>")).and_then(|(_, r)| r.split_once(&format!("</{tag}>"))).map(|(v, _)| v.trim().to_string()).filter(|v| !v.is_empty() && v != "0");
    let line = match root {
        "word" => {
            let (w, c) = count_words(&xml_text(src, "word/document.xml")?, "t");
            let pages = prop("Pages").map(|p| format!(", {p} pages when last saved")).unwrap_or_default();
            format!("{w} words, {c} characters{pages}")
        }
        "ppt" => {
            let slides = slide_names(&all);
            let mut w = 0;
            for s in &slides {
                w += count_words(&xml_text(src, s)?, "t").0;
            }
            format!("{} slides, {w} words", slides.len())
        }
        _ => {
            let s = sheets(src)?;
            let cells: usize = s.iter().map(|t| t.1.iter().map(|r| r.iter().filter(|c| !c.is_empty()).count()).sum::<usize>()).sum();
            format!("{} sheets, {} rows, {cells} filled cells", s.len(), s.iter().map(|t| t.1.len()).sum::<usize>())
        }
    };
    crate::clipboard::set(&line)?;
    note(&line);
    Ok(vec![])
}

pub(crate) fn slides_text(src: &Path) -> Result<Vec<PathBuf>> {
    if ooxml(src)? != "ppt" {
        bail!("this works on .pptx files");
    }
    let mut body = String::new();
    for (i, name) in slide_names(&names(src)?).iter().enumerate() {
        let x = xml_text(src, name)?;
        let mut r = quick_xml::Reader::from_str(&x);
        let (mut in_t, mut line, mut lines) = (false, String::new(), Vec::new());
        while let Ok(ev) = r.read_event() {
            match ev {
                Event::Start(e) if e.local_name().as_ref() == "t" => in_t = true,
                Event::End(e) if e.local_name().as_ref() == "t" => in_t = false,
                Event::End(e) if e.local_name().as_ref() == "p" => {
                    if !line.trim().is_empty() {
                        lines.push(std::mem::take(&mut line));
                    }
                    line.clear();
                }
                ev @ (Event::Text(_) | Event::GeneralRef(_)) if in_t => line.push_str(&piece(&ev)),
                Event::Eof => break,
                _ => {}
            }
        }
        body.push_str(&format!("— Slide {} —\n{}\n\n", i + 1, lines.join("\n")));
    }
    if body.is_empty() {
        bail!("no slides found in this presentation");
    }
    let out = out_path(src, None, "txt");
    std::fs::write(&out, body)?;
    Ok(vec![out])
}

// ---------------------------------------------------------------- for developers

pub(crate) fn json(src: &Path) -> Result<serde_json::Value> {
    serde_json::from_str(&read_text(src)?).map_err(|e| anyhow!("not valid JSON: {e}"))
}

pub(crate) fn sorted(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            let ordered: BTreeMap<String, Value> = m.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            Value::Object(ordered.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        other => other,
    }
}

pub(crate) fn sort_keys(src: &Path) -> Result<Vec<PathBuf>> {
    let out = out_path(src, Some("Sorted"), "json");
    std::fs::write(&out, serde_json::to_string_pretty(&sorted(json(src)?))? + "\n")?;
    Ok(vec![out])
}

/// TypeScript types that describe a JSON file: one interface per object shape.
pub(crate) struct Types {
    out: Vec<(String, String)>,
}

impl Types {
    fn name(&self, hint: &str) -> String {
        let mut n: String = hint.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).map(|p| { let mut c = p.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default() }).collect();
        if n.is_empty() || n.starts_with(|c: char| c.is_ascii_digit()) {
            n = format!("Item{n}");
        }
        let (base, mut i) = (n.clone(), 2);
        while self.out.iter().any(|o| o.0 == n) {
            n = format!("{base}{i}");
            i += 1;
        }
        n
    }
    fn object(&mut self, hint: &str, shapes: &[&serde_json::Map<String, serde_json::Value>]) -> String {
        let name = self.name(hint);
        self.out.push((name.clone(), String::new())); // reserve the name before going deeper
        let mut keys: Vec<&String> = Vec::new();
        for s in shapes {
            for k in s.keys() {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        let mut body = format!("export interface {name} {{\n");
        for k in keys {
            let vals: Vec<&serde_json::Value> = shapes.iter().filter_map(|s| s.get(k)).collect();
            let optional = vals.len() < shapes.len();
            let ident = k.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') && !k.starts_with(|c: char| c.is_ascii_digit()) && !k.is_empty();
            let key = if ident { k.clone() } else { serde_json::to_string(k).unwrap_or_default() };
            let ty = self.of(k, &vals);
            body.push_str(&format!("  {key}{}: {ty};\n", if optional { "?" } else { "" }));
        }
        body.push_str("}\n");
        if let Some(slot) = self.out.iter_mut().find(|o| o.0 == name) {
            slot.1 = body;
        }
        name
    }
    fn of(&mut self, hint: &str, vals: &[&serde_json::Value]) -> String {
        use serde_json::Value;
        let mut kinds: Vec<String> = Vec::new();
        let mut add = |t: String| {
            if !kinds.contains(&t) {
                kinds.push(t);
            }
        };
        let objects: Vec<&serde_json::Map<String, serde_json::Value>> = vals.iter().filter_map(|v| v.as_object()).collect();
        let items: Vec<&serde_json::Value> = vals.iter().filter_map(|v| v.as_array()).flatten().collect();
        let has_array = vals.iter().any(|v| v.is_array());
        for v in vals {
            match v {
                Value::Null => add("null".into()),
                Value::Bool(_) => add("boolean".into()),
                Value::Number(_) => add("number".into()),
                Value::String(_) => add("string".into()),
                _ => {}
            }
        }
        if !objects.is_empty() {
            add(self.object(hint, &objects));
        }
        if has_array {
            let inner = if items.is_empty() { "unknown".to_string() } else { self.of(hint.strip_suffix('s').unwrap_or(hint), &items) };
            add(if inner.contains(' ') { format!("({inner})[]") } else { format!("{inner}[]") });
        }
        if kinds.is_empty() { "unknown".into() } else { kinds.join(" | ") }
    }
}

pub(crate) fn types(src: &Path) -> Result<Vec<PathBuf>> {
    let v = json(src)?;
    let mut t = Types { out: Vec::new() };
    let root = t.name(&stem(src));
    let top = t.of(&root, &[&v]);
    let mut text = String::new();
    if top != root {
        text.push_str(&format!("export type {} = {top};\n\n", t.name(&format!("{root} file"))));
    }
    text.push_str(&t.out.iter().map(|o| o.1.as_str()).collect::<Vec<_>>().join("\n"));
    let out = out_path(src, Some("types"), "ts");
    std::fs::write(&out, text)?;
    Ok(vec![out])
}

pub(crate) fn data_uri(src: &Path) -> Result<Vec<PathBuf>> {
    use base64::Engine;
    let bytes = crate::engines::files::read_capped(src, 4)?;
    let mime = infer::get(&bytes).map(|k| k.mime_type().to_string()).unwrap_or_else(|| {
        match ext(src).as_str() {
            "svg" => "image/svg+xml",
            "css" => "text/css",
            "json" => "application/json",
            "html" | "htm" => "text/html",
            "js" | "mjs" => "text/javascript",
            _ => "text/plain",
        }
        .to_string()
    });
    let uri = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
    note(&format!("Data URI copied ({} characters)", uri.len()));
    crate::clipboard::set(&uri)?;
    Ok(vec![])
}

/// Treats the picture as the @3x original and writes @2x and @1x beside it.
pub(crate) fn scales(src: &Path) -> Result<Vec<PathBuf>> {
    let (w, h) = image::image_dimensions(src).map_err(|_| anyhow!("can't read this image"))?;
    let long = w.max(h);
    if long < 3 {
        bail!("this picture is too small to scale down");
    }
    let dir = src.parent().context("no folder")?;
    let e = if matches!(ext(src).as_str(), "jpg" | "jpeg") { "jpg" } else { "png" };
    let base = stem(src);
    let base = base.strip_suffix("@3x").unwrap_or(&base);
    let mut out = Vec::new();
    for (tag, side) in [("@2x", (long as f64 * 2.0 / 3.0).round() as u32), ("@1x", (long as f64 / 3.0).round() as u32)] {
        let p = unique(dir, &format!("{base}{tag}"), e);
        crate::native::fit(side.max(1), src, &p)?;
        out.push(p);
    }
    Ok(out)
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Checks the file against a checksum the user copied from a download page.
pub(crate) fn verify(src: &Path, pasted: &str) -> Result<Vec<PathBuf>> {
    use md5::Digest;
    // the first run of hex digits of a length a checksum has
    let want = pasted
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|p| matches!(p.len(), 32 | 40 | 64 | 128))
        .map(str::to_lowercase)
        .context("no checksum on the clipboard: copy the MD5, SHA-1, SHA-256 or SHA-512 first")?;
    let mut f = std::fs::File::open(src)?;
    let (mut a, mut b, mut c, mut d) = (md5::Md5::new(), sha1::Sha1::new(), sha2::Sha256::new(), sha2::Sha512::new());
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        match want.len() {
            32 => a.update(&buf[..n]),
            40 => b.update(&buf[..n]),
            64 => c.update(&buf[..n]),
            _ => d.update(&buf[..n]),
        }
    }
    let (name, got) = match want.len() {
        32 => ("MD5", hex(&a.finalize())),
        40 => ("SHA-1", hex(&b.finalize())),
        64 => ("SHA-256", hex(&c.finalize())),
        _ => ("SHA-512", hex(&d.finalize())),
    };
    if got != want {
        bail!("checksum does not match: this file's {name} is {got}");
    }
    note(&format!("{name} matches: the file is exactly what was published"));
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_as_people_write_them() {
        let m = |s: &str| amount(s).map(money);
        assert_eq!(m("1,234.56").as_deref(), Some("1,234.56"));
        assert_eq!(m("1.234,56").as_deref(), Some("1,234.56"));
        assert_eq!(m("1.200.000 ₫").as_deref(), Some("1,200,000"));
        assert_eq!(m("1,200,000đ").as_deref(), Some("1,200,000"));
        assert_eq!(m("(1 234)").as_deref(), Some("-1,234"));
        assert_eq!(m("-5").as_deref(), Some("-5"));
        assert_eq!(m("12,5").as_deref(), Some("12.5"));
        assert_eq!(m("12.5").as_deref(), Some("12.5"));
        assert_eq!(m("1,234").as_deref(), Some("1,234"));
        assert_eq!(m("0.1").as_deref(), Some("0.1"));
        assert_eq!(m("$ 99.90").as_deref(), Some("99.9"));
        assert_eq!(m("abc"), None);
        assert_eq!(m("2024-01-05"), None);
        assert_eq!(m(""), None);
        // one clear cell decides how the doubtful ones in the table are read
        let vn: Table = vec![vec!["1.200.000".into(), "120.000".into()]];
        assert_eq!(marks(&vn), Marks::DotGroups);
        assert_eq!(amount_as("120.000", Marks::DotGroups).map(money).as_deref(), Some("120,000"));
        assert_eq!(amount_as("(50.000)", Marks::DotGroups).map(money).as_deref(), Some("-50,000"));
        assert_eq!(amount_as("12,5", Marks::DotGroups).map(money).as_deref(), Some("12.5"));
        assert_eq!(amount_as("120.000", Marks::Unknown).map(money).as_deref(), Some("120"));
        let us: Table = vec![vec!["1,200,000.50".into(), "Total".into()]];
        assert_eq!(marks(&us), Marks::CommaGroups);
        assert_eq!(marks(&vec![vec!["12.5".into(), "abc".into()]]), Marks::Unknown);
        // exact, where floating point would drift
        assert_eq!(money(amount("0.1").unwrap() + amount("0.2").unwrap()), "0.3");
    }

    #[test]
    fn csv_quotes_and_delimiters() {
        let t = parse_csv("a;b\n\"x;1\";\"say \"\"hi\"\"\"\r\n\n3;\n", delimiter("a;b", "csv"));
        assert_eq!(t, vec![vec!["a", "b"], vec!["x;1", "say \"hi\""], vec!["3", ""]]);
        assert_eq!(write_csv(&t, "\n"), "a,b\nx;1,\"say \"\"hi\"\"\"\n3,\n");
        assert_eq!(delimiter("a\tb\tc", "txt"), '\t');
    }

    #[test]
    fn excel_details() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(702), "AAA");
        assert_eq!(col_index("AA10"), 26);
        assert_eq!(serial(45292.0, false), "2024-01-01");
        assert_eq!(serial(45292.5, false), "2024-01-01 12:00:00");
        assert_eq!(serial(0.25, false), "06:00:00");
        assert_eq!(tidy_number("0.30000000000000004"), "0.3");
        assert_eq!(tidy_number("1234.5600000000002"), "1234.56");
        assert!(plain_number("12.50") && plain_number("-3") && plain_number("0.5"));
        // codes and long account numbers must stay exactly as typed
        assert!(!plain_number("007") && !plain_number("1234567890123456") && !plain_number("1,234") && !plain_number("1e5"));
        assert!(safe_number("12.5", Marks::Unknown) && safe_number("120", Marks::DotGroups) && safe_number("120.000", Marks::CommaGroups));
        assert!(!safe_number("120.000", Marks::Unknown) && !safe_number("120.000", Marks::DotGroups) && !safe_number("1.5", Marks::DotGroups));
        assert!(is_date_format(14, &HashMap::new()) && !is_date_format(2, &HashMap::new()));
        assert!(is_date_format(164, &HashMap::from([(164, "dd/mm/yyyy".to_string())])));
        assert!(!is_date_format(165, &HashMap::from([(165, "#,##0 \"đ\";[Red]-#,##0".to_string())])));
        assert_eq!(inert("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(inert("-5"), "-5");
        assert_eq!(inert("-cmd"), "'-cmd");
    }

    #[test]
    fn properties_are_emptied_not_removed() {
        let x = "<cp:coreProperties><dc:creator>An</dc:creator><dc:title>T</dc:title><cp:lastModifiedBy>B</cp:lastModifiedBy><dc:creator/></cp:coreProperties>";
        assert_eq!(blank(x, &["dc:creator", "cp:lastModifiedBy"]), "<cp:coreProperties><dc:creator></dc:creator><dc:title>T</dc:title><cp:lastModifiedBy></cp:lastModifiedBy><dc:creator/></cp:coreProperties>");
        assert_eq!(blank_attr("<w:ins w:id=\"1\" w:author=\"An Nguyen\" w:date=\"x\"/><w:del w:author=\"B\"/>", "w:author"), "<w:ins w:id=\"1\" w:author=\"\" w:date=\"x\"/><w:del w:author=\"\"/>");
    }

    #[test]
    fn typescript_from_json() {
        let v: serde_json::Value = serde_json::from_str(r#"{"id":1,"tags":["a"],"owner":{"name":"x","age":null},"items":[{"sku":"a","qty":1},{"sku":"b"}]}"#).unwrap();
        let mut t = Types { out: Vec::new() };
        let root = t.name("order");
        assert_eq!(t.of(&root, &[&v]), "Order");
        let all = t.out.iter().map(|o| o.1.as_str()).collect::<Vec<_>>().join("\n");
        assert!(all.contains("export interface Order {\n  id: number;\n  tags: string[];\n  owner: Owner;\n  items: Item[];\n}"), "{all}");
        assert!(all.contains("export interface Item {\n  sku: string;\n  qty?: number;\n}"), "{all}");
        assert!(all.contains("age: null;"), "{all}");
    }
}
