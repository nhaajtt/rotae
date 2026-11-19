//! Bookkeeping chores: e-invoices, an invoice register read from photos and PDFs, reconciling
//! two tables, tidying and splitting them, mail merge. Built on the table and Office helpers
//! in `desk`; every result is a new file next to what was dropped.
use crate::desk::{amount_as, csv_field, delimiter, entry, inert, marks, money, names, parse_csv, piece, read_text, sheets, workbook, xml_escape, Marks, Table};
use crate::engines::note;
use crate::naming::{ext, kind, out_dir, out_path, temp_file, unique, Kind};
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::Event;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
};

pub fn run(paths: &[PathBuf], verb: &str, arg: &str) -> Option<Result<Vec<PathBuf>>> {
    if verb != "ledger" {
        return None;
    }
    let src = paths.first()?;
    Some(match arg {
        "einvoice" => einvoice(paths),
        "invoice" => invoices(paths),
        "reconcile" => reconcile(paths),
        "tidy" => tidy(src),
        "split" => split(src, None),
        "mailmerge" => mailmerge(paths),
        "statement" => statement(src),
        a => split(src, Some(a.strip_prefix("split=")?)),
    })
}

/// Must agree with `isGroup` in the app.
pub fn is_group(verb: &str, arg: &str) -> bool {
    verb == "ledger" && matches!(arg, "einvoice" | "invoice" | "reconcile" | "mailmerge")
}

// ---------------------------------------------------------------- shared

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A cell's value as a file name: nothing in it can choose a folder or hide the file.
fn safe_name(v: &str) -> String {
    let s: String = v.chars().map(|c| if matches!(c, '/' | ':' | '\\') { '-' } else if c.is_control() { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for c in s.trim_start_matches(['.', ' ']).chars() {
        // well under the 255 bytes a name may have, with room for " (2).docx"
        if out.len() + c.len_utf8() > 120 {
            break;
        }
        out.push(c);
    }
    let out = out.trim();
    if out.is_empty() { "(blank)".into() } else { out.to_string() }
}

fn squeeze(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How headers and placeholders are compared: case and spacing don't matter.
fn norm(v: &str) -> String {
    squeeze(&v.to_lowercase())
}

/// The reader marks risky text with an apostrophe; in a document or an .xlsx text cell it is just noise.
fn uninert(v: &str) -> &str {
    match v.strip_prefix('\'') {
        Some(r) if inert(r) == v => r,
        _ => v,
    }
}

fn plain(v: i128) -> String {
    money(v).replace(',', "")
}

struct Loaded {
    t: Table,
    ext: String,
    delim: char,
    bom: bool,
    crlf: bool,
    more_sheets: usize,
}

fn is_table(e: &str) -> bool {
    matches!(e, "csv" | "tsv" | "txt" | "xlsx" | "xlsm")
}

fn load(src: &Path) -> Result<Loaded> {
    let e = ext(src);
    let mut l = Loaded { t: Vec::new(), ext: e.clone(), delim: ',', bom: false, crlf: false, more_sheets: 0 };
    match e.as_str() {
        "xlsx" | "xlsm" => {
            let all = sheets(src)?;
            l.more_sheets = all.len().saturating_sub(1);
            l.t = all.into_iter().next().map(|s| s.1).unwrap_or_default();
            l.ext = "xlsx".into();
        }
        e if is_table(e) => {
            let s = read_text(src)?;
            let mut head = [0u8; 3];
            l.bom = std::fs::File::open(src).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok() && head == [0xEF, 0xBB, 0xBF];
            l.crlf = s.contains("\r\n");
            l.delim = delimiter(&s, e);
            l.t = parse_csv(&s, l.delim);
        }
        _ => bail!("this works on .csv and .xlsx tables"),
    }
    if l.t.is_empty() {
        bail!("this file has no rows");
    }
    Ok(l)
}

/// Writes a table the way its source was written: same kind, delimiter, line ends, byte-order mark.
fn save(t: &Table, like: &Loaded, out: &Path) -> Result<()> {
    if like.ext == "xlsx" {
        return workbook(t, out);
    }
    let eol = if like.crlf { "\r\n" } else { "\n" };
    let mut s = String::from(if like.bom { "\u{feff}" } else { "" });
    for r in t {
        s.push_str(&r.iter().map(|v| csv_field(v, like.delim)).collect::<Vec<_>>().join(&like.delim.to_string()));
        s.push_str(eol);
    }
    Ok(std::fs::write(out, s)?)
}

fn first_sheet_only(l: &Loaded) -> &'static str {
    if l.more_sheets > 0 { " (first sheet only)" } else { "" }
}

// ---------------------------------------------------------------- reading dates and words

/// Lower case, Vietnamese marks removed: OCR and hurried typing both drop them. One character
/// in gives at most one out, so a place in the folded line can be found again in the original.
fn fold_char(c: char) -> Option<char> {
    let c = c.to_lowercase().next().unwrap_or(c);
    Some(match c {
        '\u{300}'..='\u{36f}' => return None,
        'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ' | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' => 'a',
        'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' => 'e',
        'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' => 'i',
        'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ' | 'ớ' | 'ở' | 'ỡ' | 'ợ' => 'o',
        'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' => 'u',
        'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
        'đ' => 'd',
        '\u{a0}' | '\u{202f}' | '\t' => ' ',
        c => c,
    })
}

fn fold(s: &str) -> String {
    s.chars().filter_map(fold_char).collect()
}

/// The rest of the original line from byte `pos` of its folded form.
fn tail<'a>(orig: &'a str, folded: &str, pos: usize) -> &'a str {
    let kept = folded.get(..pos).map_or(0, |p| p.chars().count());
    let mut seen = 0;
    for (i, c) in orig.char_indices() {
        if fold_char(c).is_some() {
            if seen == kept {
                return &orig[i..];
            }
            seen += 1;
        }
    }
    ""
}

struct Tok<'a> {
    s: &'a str,
    at: usize,
    num: bool,
}

/// Runs of digits and runs of letters, so "ngay03thang11" reads the same as "ngay 03 thang 11".
fn toks(s: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut open: Option<(usize, bool)> = None;
    for (i, c) in s.char_indices().chain(std::iter::once((s.len(), ' '))) {
        let k = if c.is_ascii_digit() { Some(true) } else if c.is_alphabetic() { Some(false) } else { None };
        if let Some((a, num)) = open {
            if k != Some(num) {
                out.push(Tok { s: &s[a..i], at: a, num });
                open = None;
            }
        }
        if open.is_none() {
            open = k.map(|num| (i, num));
        }
    }
    out
}

type Ymd = (i64, i64, i64);

fn valid(y: i64, m: i64, d: i64) -> Option<Ymd> {
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let last = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if leap { 29 } else { 28 },
        _ => return None,
    };
    ((1900..=2100).contains(&y) && (1..=last).contains(&d)).then_some((y, m, d))
}

fn month(w: &str) -> Option<i64> {
    const NAMES: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    if w.len() < 3 {
        return None;
    }
    NAMES.iter().position(|m| m.starts_with(w)).map(|i| i as i64 + 1)
}

fn iso(d: Ymd) -> String {
    format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
}

fn day_number((y, m, d): Ymd) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy
}

/// The first date in a folded line: the date, whether it was spelled "ngay … thang … nam …",
/// and where it ends. 03/11/2026 is day then month, as Vietnam writes it, unless that can't be.
fn date_in(s: &str) -> Option<(Ymd, bool, usize)> {
    let t = toks(s);
    let n = |i: usize| t.get(i).filter(|k| k.num && k.s.len() <= 4).and_then(|k| k.s.parse::<i64>().ok());
    let w = |i: usize| t.get(i).filter(|k| !k.num).map(|k| k.s);
    let len = |i: usize| t.get(i).map_or(0, |k| k.s.len());
    let end = |i: usize| t.get(i).map_or(s.len(), |k| k.at + k.s.len());
    let gap = |i: usize| t.get(i).zip(t.get(i + 1)).and_then(|(a, b)| s.get(a.at + a.s.len()..b.at)).map_or("?", str::trim);
    let digits = |i: usize| t.get(i).is_some_and(|k| k.num);
    for i in 0..t.len() {
        if w(i) == Some("ngay") && w(i + 2) == Some("thang") && w(i + 4) == Some("nam") {
            if let Some(d) = n(i + 1).zip(n(i + 3)).zip(n(i + 5)).and_then(|((d, m), y)| valid(y, m, d)) {
                return Some((d, true, end(i + 5)));
            }
        }
        if let (Some(a), Some(b), Some(c)) = (n(i), n(i + 1), n(i + 2)) {
            let sep = gap(i);
            // 1.200.000 and 024-3825-1234 are not dates: nothing of the same kind may be attached
            let longer = (i > 0 && digits(i - 1) && gap(i - 1) == sep) || (digits(i + 3) && gap(i + 2) == sep);
            if sep == gap(i + 1) && matches!(sep, "/" | "-" | ".") && !longer {
                let found = if len(i) == 4 && len(i + 1) <= 2 && len(i + 2) <= 2 {
                    valid(a, b, c)
                } else if len(i + 2) == 4 && len(i) <= 2 && len(i + 1) <= 2 {
                    if b > 12 && a <= 12 { valid(c, a, b) } else { valid(c, b, a) }
                } else {
                    None
                };
                if let Some(d) = found {
                    return Some((d, false, end(i + 2)));
                }
            }
        }
        let near = |i: usize| gap(i).len() <= 2;
        if let (Some(d), Some(m), Some(y)) = (n(i), w(i + 1).and_then(month), n(i + 2)) {
            if let Some(d) = valid(y, m, d).filter(|_| len(i + 2) == 4 && near(i) && near(i + 1)) {
                return Some((d, false, end(i + 2)));
            }
        }
        if let (Some(m), Some(d), Some(y)) = (w(i).and_then(month), n(i + 1), n(i + 2)) {
            if let Some(d) = valid(y, m, d).filter(|_| len(i + 2) == 4 && near(i) && near(i + 1)) {
                return Some((d, false, end(i + 2)));
            }
        }
    }
    None
}

// ---------------------------------------------------------------- invoices from pictures and PDFs

#[derive(Default, Debug, PartialEq, Clone)]
pub(crate) struct Fields {
    pub seller: String,
    pub tax_code: String,
    pub number: String,
    pub serial: String,
    pub date: String,
    pub before_tax: String,
    pub vat: String,
    pub total: String,
}

const CURRENCY: [&str; 5] = ["d", "vnd", "dong", "usd", "eur"];
const TAX_CODE: [&str; 9] = ["ma so thue", "mst", "tax code", "tax id", "tax no", "vat no", "vat reg", "vat number", "tax number"];
const BUYER: [&str; 12] = ["nguoi mua", "don vi mua", "ben mua", "khach hang", "ten don vi", "buyer", "customer", "bill to", "billed to", "sold to", "ship to", "client"];
const SELLER: [&str; 9] = ["don vi ban hang", "nguoi ban hang", "don vi ban", "nguoi ban", "ben ban", "seller", "vendor", "supplier", "sold by"];

/// Where the earliest of `labels` ends in a folded line; a label is whole words, never part of one.
fn label(f: &str, labels: &[&str]) -> Option<usize> {
    labels
        .iter()
        .filter_map(|l| {
            f.match_indices(l)
                .find(|(i, _)| !f[..*i].chars().next_back().is_some_and(char::is_alphanumeric) && !f[i + l.len()..].chars().next().is_some_and(char::is_alphabetic))
                .map(|(i, _)| (i, i + l.len()))
        })
        .min_by_key(|(i, e)| (*i, std::cmp::Reverse(*e)))
        .map(|x| x.1)
}

/// Past what sits between a label and its value: "(Seller)", ":", "#".
fn value(f: &str, mut at: usize) -> usize {
    loop {
        let Some(r) = f.get(at..) else { return f.len() };
        let t = r.trim_start();
        at += r.len() - t.len();
        if t.starts_with('(') {
            match t.find(')') {
                Some(c) => at += c + 1,
                None => return at,
            }
        } else if t.starts_with([':', '.', '#', '=']) {
            at += 1;
        } else {
            return at;
        }
    }
}

fn code(s: &str, extra: &[char]) -> String {
    let c: String = s.chars().take_while(|c| c.is_ascii_alphanumeric() || extra.contains(c)).take(24).collect();
    c.trim_end_matches(extra).to_string()
}

/// Number-looking pieces of a folded line that could be money: not a percentage, not part of a
/// date, a code or an account number.
fn numbers(f: &str) -> Vec<&str> {
    let b = f.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let a = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.' || b[i] == b',') {
            i += 1;
        }
        let mut e = i;
        while e > a && !b[e - 1].is_ascii_digit() {
            e -= 1;
        }
        let (tok, left, right) = (&f[a..e], &f[..a], &f[e..]);
        let lead: String = left.chars().rev().take_while(|c| c.is_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect();
        let trail: String = right.chars().take_while(|c| c.is_alphabetic()).collect();
        let mut before = left.chars().rev();
        let joined_left = match before.next() {
            Some('/') => true,
            Some('-') => before.next().is_some_and(char::is_alphanumeric),
            _ => false,
        };
        let mut after = right.chars();
        let joined_right = match after.next() {
            Some('/') => true,
            Some('-') => after.next().is_some_and(char::is_alphanumeric),
            _ => false,
        };
        let digits = tok.bytes().filter(u8::is_ascii_digit).count();
        let id = digits > 15 || (tok.len() > 1 && tok.starts_with('0') && digits == tok.len());
        let unit = |w: &str| w.is_empty() || CURRENCY.contains(&w);
        if !(right.trim_start().starts_with('%') || joined_left || joined_right || id || !unit(&lead) || !unit(&trail)) {
            out.push(tok);
        }
    }
    out
}

/// A line that is one amount and nothing else, as OCR gives the value column of a totals box.
fn lone_amount(f: &str) -> Option<&str> {
    let n = numbers(f);
    let [tok] = n.as_slice() else { return None };
    let rest: String = f.replacen(tok, "", 1).chars().filter(|c| !c.is_whitespace() && !matches!(c, '₫' | '$' | '€' | ':' | '=' | '.' | ',')).collect();
    (rest.is_empty() || CURRENCY.contains(&rest.as_str())).then_some(*tok)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Sum {
    Before,
    Vat,
    Total(bool),
}

/// Which of the three sums a folded line talks about.
fn classify(f: &str) -> Option<Sum> {
    let has = |ks: &[&str]| ks.iter().any(|k| f.contains(k));
    if label(f, &TAX_CODE).is_some() || has(&["so luong", "quantity", "qty", "so trang", "page"]) {
        return None;
    }
    // "VAT rate: 10" says nothing about how much
    if has(&["thue suat", "vat rate", "tax rate"]) && !has(&["tien thue", "vat amount", "tax amount"]) {
        return None;
    }
    if has(&["tien hang", "truoc thue", "chua thue", "chua vat", "chua bao gom", "chua co thue", "subtotal", "sub total", "sub-total", "before tax", "before vat", "pre-tax", "excl", "ex vat", "ex. vat", "net amount", "net total", "taxable amount", "amount before"]) {
        return Some(Sum::Before);
    }
    if has(&["incl", "da bao gom", "bao gom thue", "sau thue", "after tax", "da co thue"]) {
        return Some(Sum::Total(true));
    }
    if has(&["thue gtgt", "tien thue", "thue vat", "tong thue", "gtgt", "vat", "sales tax", "gst"]) || label(f, &["tax"]).is_some() {
        return Some(Sum::Vat);
    }
    if has(&["tong cong", "tong tien", "tong so tien", "tong thanh toan", "so tien thanh toan", "cong tien", "amount due", "balance due"]) || label(f, &["total"]).is_some() {
        return Some(Sum::Total(has(&["thanh toan", "grand", "due", "payment", "payable", "to pay"])));
    }
    None
}

struct Found {
    v: i128,
    strong: bool,
    /// paired with its label by position alone: kept only if the three sums then agree
    loose: bool,
}

/// What an invoice says about itself. A field that isn't clearly there stays empty.
pub(crate) fn extract(text: &str) -> Fields {
    let lines: Vec<(&str, String)> = text.lines().map(str::trim).filter(|l| !l.is_empty()).take(5000).map(|l| (l, fold(l))).collect();
    let mut out = Fields::default();

    // which mark groups thousands: the document usually shows it, Vietnamese ones use the dot
    let all: Vec<String> = lines.iter().flat_map(|l| numbers(&l.1)).map(str::to_string).collect();
    let vietnamese = lines.iter().any(|l| ["hoa don", "thue", "tong", "vnd", "tien"].iter().any(|k| l.1.contains(k)));
    let how = match marks(&vec![all]) {
        Marks::Unknown if vietnamese => Marks::DotGroups,
        m => m,
    };
    let amounts = |f: &str| -> Vec<i128> { numbers(f).iter().filter_map(|t| amount_as(t, how)).collect() };

    let mut buyer_part = false;
    let (mut before, mut vat, mut total): (Vec<Found>, Vec<Found>, Vec<Found>) = (Vec::new(), Vec::new(), Vec::new());
    let mut dates: Vec<(u8, Ymd)> = Vec::new();
    for (i, (orig, f)) in lines.iter().enumerate() {
        let rest = |pos: usize| tail(orig, f, pos).trim();
        let seller_at = label(f, &SELLER);
        if seller_at.is_some() {
            buyer_part = false;
        } else if BUYER.iter().any(|k| f.contains(k)) {
            buyer_part = true;
        }
        if out.seller.is_empty() {
            if let Some(at) = seller_at {
                let here = rest(value(f, at));
                let next = lines.get(i + 1).map(|l| l.0).filter(|l| !l.contains(':')).unwrap_or("");
                // a long name in capitals wraps onto a line of its own in a PDF
                let shouting = |l: &str| l.chars().any(char::is_alphabetic) && !l.chars().any(|c| c.is_lowercase() || c.is_ascii_digit() || c == ':');
                let wrapped = lines.get(i + 1).filter(|l| shouting(here) && shouting(l.0) && l.0.chars().count() <= 40 && label(&l.1, &TAX_CODE).is_none());
                out.seller = match wrapped {
                    Some(more) => company(&format!("{here} {}", more.0)),
                    None => company(if here.is_empty() { next } else { here }),
                };
            }
        }
        if out.tax_code.is_empty() && !buyer_part {
            if let Some(at) = label(f, &TAX_CODE) {
                out.tax_code = tax_code(rest(value(f, at)));
            }
        }
        if out.number.is_empty() {
            let named = label(f, &["so hoa don", "hoa don so", "so hd", "invoice number", "invoice no", "invoice #", "inv no", "inv. no", "inv #"]);
            // a bare "Số:" or "No." counts only where nothing else can be meant
            let bare = label(f, &["so"]).filter(|e| f[*e..].trim_start().starts_with([':', '('])).or_else(|| label(f, &["no"]).filter(|e| *e == 2 && f[*e..].trim_start().starts_with([':', '.', '(', '#'])));
            if let Some(at) = named.or(bare) {
                let c = code(rest(value(f, at)), &['/', '-']);
                if c.bytes().any(|b| b.is_ascii_digit()) {
                    out.number = c;
                }
            }
        }
        if out.serial.is_empty() {
            if let Some(at) = label(f, &["ky hieu", "serial no", "serial", "symbol"]) {
                let c = code(rest(value(f, at)), &['/']);
                if c.len() >= 2 && c.len() <= 12 && c.bytes().any(|b| b.is_ascii_digit()) {
                    out.serial = c;
                }
            }
        }
        if let Some((d, spelled, _)) = date_in(f) {
            let has = |ks: &[&str]| ks.iter().any(|k| f.contains(k));
            if !has(&["due", "han thanh toan", "han tt", "het han", "expiry", "delivery", "giao hang", "order date", "ngay dat"]) {
                let rank = if has(&["invoice date", "ngay lap", "ngay hoa don", "date of issue", "issue date", "issued", "date of invoice"]) {
                    0
                } else if spelled {
                    1
                } else if label(f, &["date", "ngay", "dated"]).is_some() {
                    2
                } else {
                    3
                };
                dates.push((rank, d));
            }
        }
        if lone_amount(f).is_none() {
            if let Some(kind) = classify(f) {
                let a = amounts(f);
                // one row holding all three: before tax, VAT, total
                if let (Sum::Total(_), [.., x, y, z]) = (kind, a.as_slice()) {
                    if x + y == *z && *y > 0 {
                        before.push(Found { v: *x, strong: false, loose: true });
                        vat.push(Found { v: *y, strong: false, loose: true });
                    }
                }
                if let Some(v) = a.last() {
                    let (list, strong) = match kind {
                        Sum::Before => (&mut before, false),
                        Sum::Vat => (&mut vat, false),
                        Sum::Total(s) => (&mut total, s),
                    };
                    list.push(Found { v: *v, strong, loose: false });
                }
            }
        }
    }

    // OCR often reads a totals box as its labels first and its values after
    let is_value = |i: usize| lines.get(i).is_some_and(|l| lone_amount(&l.1).is_some() || (l.1.ends_with('%') && l.1.len() <= 6));
    let mut i = 0;
    while i < lines.len() {
        if !is_value(i) {
            i += 1;
            continue;
        }
        let start = i;
        while is_value(i) {
            i += 1;
        }
        let count = i - start;
        let mut labels = Vec::new();
        for back in (start.saturating_sub(12)..start).rev() {
            if labels.len() == count || is_value(back) {
                break;
            }
            // a line that already carries its own number has its value
            if lines.get(back).is_some_and(|l| !l.1.bytes().any(|b| b.is_ascii_digit())) {
                labels.push(back);
            } else if count == 1 {
                break;
            }
        }
        if labels.len() != count {
            continue;
        }
        labels.reverse();
        for (k, at) in labels.iter().enumerate() {
            let (Some(l), Some(v)) = (lines.get(*at), lines.get(start + k)) else { continue };
            let Some(v) = lone_amount(&v.1).and_then(|t| amount_as(t, how)) else { continue };
            let (list, strong) = match classify(&l.1) {
                Some(Sum::Before) => (&mut before, false),
                Some(Sum::Vat) => (&mut vat, false),
                Some(Sum::Total(s)) => (&mut total, s),
                None => continue,
            };
            list.push(Found { v, strong, loose: count > 1 });
        }
    }

    // sums that agree with each other beat any single reading
    let mut agreed = None;
    'find: for strong in [true, false] {
        for t in total.iter().filter(|t| t.strong == strong) {
            for b in &before {
                if let Some(v) = vat.iter().find(|v| b.v + v.v == t.v) {
                    agreed = Some((b.v, v.v, t.v));
                    break 'find;
                }
            }
        }
    }
    let (b, v, t) = match agreed {
        Some((b, v, t)) => (Some(b), Some(v), Some(t)),
        None => (
            before.iter().find(|f| !f.loose).map(|f| f.v),
            vat.iter().rfind(|f| !f.loose).map(|f| f.v),
            total.iter().rfind(|f| !f.loose && f.strong).or_else(|| total.iter().rfind(|f| !f.loose)).map(|f| f.v),
        ),
    };
    out.before_tax = b.map(plain).unwrap_or_default();
    out.vat = v.map(plain).unwrap_or_default();
    out.total = t.map(plain).unwrap_or_default();
    out.date = dates.iter().min_by_key(|d| d.0).map(|d| iso(d.1)).unwrap_or_default();

    // no "Seller:" printed: Vietnamese invoices open with the seller's registered name
    if out.seller.is_empty() {
        for (orig, f) in &lines {
            if BUYER.iter().any(|k| f.contains(k)) {
                break;
            }
            let opens = ["cong ty", "tong cong ty", "doanh nghiep", "ho kinh doanh", "chi nhanh", "tap doan", "hop tac xa"].iter().any(|k| f.starts_with(k));
            let closes = [" ltd", " ltd.", " inc", " inc.", " llc", " jsc", " corp", " corp.", " gmbh", " plc", " limited", " corporation", " company"].iter().any(|k| f.ends_with(k));
            if opens || closes {
                out.seller = company(orig);
                break;
            }
        }
    }
    out
}

fn company(v: &str) -> String {
    // columns of a PDF line are set apart by wide gaps; the name is the first of them
    let v = v.split("  ").next().unwrap_or("").trim().trim_end_matches([',', ';', ':']).trim();
    let f = fold(v);
    if v.starts_with('(') || !v.chars().any(char::is_alphabetic) || ["ky,", "ghi ro", "signature", "signed", "sign,"].iter().any(|k| f.contains(k)) {
        return String::new();
    }
    v.chars().take(120).collect()
}

/// A Vietnamese tax code: ten digits, or ten and a three-digit branch. Printed in boxes, the
/// digits come apart; anything else is left out rather than guessed.
fn tax_code(v: &str) -> String {
    let run: String = v.chars().take_while(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '.')).collect();
    let digits: String = run.chars().filter(char::is_ascii_digit).collect();
    match digits.len() {
        10 => digits,
        13 => format!("{}-{}", &digits[..10], &digits[10..]),
        _ => String::new(),
    }
}

fn pdf_text(src: &Path) -> Result<String> {
    let doc = crate::engines::pdf::load(src)?;
    let mut text = String::new();
    for page in doc.pages().iter().take(500) {
        text.push_str(&page.text().map(|t| t.all()).unwrap_or_default());
        text.push('\n');
        if text.len() > 16 << 20 {
            break;
        }
    }
    if text.chars().filter(|c| !c.is_whitespace()).take(8).count() < 8 {
        bail!("no text found in this PDF: it is a scan, run \"OCR to PDF\" on it first");
    }
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn invoice_text(src: &Path) -> Result<String> {
    if std::fs::metadata(src)?.len() > 200 << 20 {
        bail!("this file is too big for this (over 200 MB)");
    }
    match kind(src) {
        Kind::Pdf => pdf_text(src),
        Kind::Image => crate::native::ocr(src),
        _ => bail!("this works on photos, scans and PDFs of invoices"),
    }
}

const REGISTER: [&str; 9] = ["Date", "Invoice no.", "Serial", "Seller", "Tax code", "Before tax", "VAT", "Total", "File"];
const REGISTER_NAME: &str = "Invoices";

/// The register in this folder, if it is one of ours and nothing but our rows would be lost by
/// writing it again.
fn register(path: &Path) -> Option<Table> {
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut all = sheets(path).ok()?;
    if all.len() != 1 {
        return None;
    }
    let t = all.pop()?.1;
    let ours = t.first().is_some_and(|h| h.iter().map(|c| c.trim()).eq(REGISTER));
    ours.then(|| t.into_iter().map(|r| r.iter().map(|c| uninert(c).to_string()).collect()).collect())
}

fn invoices(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let (mut rows, mut unread, mut no_total, mut first_error): (Table, usize, usize, Option<anyhow::Error>) = (Vec::new(), 0, 0, None);
    for p in paths {
        let f = match invoice_text(p) {
            Ok(text) => extract(&text),
            Err(e) => {
                unread += 1;
                first_error.get_or_insert(e);
                Fields::default()
            }
        };
        if f.total.is_empty() {
            no_total += 1;
        }
        rows.push(vec![f.date, f.number, f.serial, f.seller, f.tax_code, f.before_tax, f.vat, f.total, file_name(p)]);
    }
    if unread == paths.len() {
        return Err(first_error.unwrap_or_else(|| anyhow!("no files to read")));
    }
    let dir = paths.first().and_then(|p| p.parent()).context("no folder")?;
    let existing = dir.join(format!("{REGISTER_NAME}.xlsx"));
    let tsv: String = rows.iter().map(|r| r.iter().map(|c| inert(&c.replace(['\t', '\n', '\r'], " "))).collect::<Vec<_>>().join("\t") + "\n").collect();
    let out = match register(&existing) {
        Some(mut t) => {
            if dir.join(format!("~${REGISTER_NAME}.xlsx")).exists() || dir.join(format!(".~lock.{REGISTER_NAME}.xlsx#")).exists() {
                bail!("{REGISTER_NAME}.xlsx is open in another app: close it first, then drop the invoices again");
            }
            t.extend(rows);
            // written whole beside it, then swapped in: the register is never half-written
            let fresh = unique(dir, &format!(".{REGISTER_NAME}"), "tmp");
            workbook(&t, &fresh)?;
            std::fs::rename(&fresh, &existing)?;
            existing
        }
        None => {
            let out = unique(dir, REGISTER_NAME, "xlsx");
            let mut t = vec![REGISTER.iter().map(|s| s.to_string()).collect::<Vec<_>>()];
            t.extend(rows);
            workbook(&t, &out)?;
            out
        }
    };
    crate::clipboard::set(&tsv)?;
    let n = paths.len();
    let mut line = format!("{n} invoice{} added", if n == 1 { "" } else { "s" });
    if no_total - unread > 0 {
        line.push_str(&format!(", {} had no total", no_total - unread));
    }
    if unread > 0 {
        line.push_str(&format!(", {unread} could not be read"));
    }
    note(&format!("{line} (rows copied)"));
    Ok(vec![out])
}

// ---------------------------------------------------------------- Vietnamese e-invoice XML

#[derive(Default)]
struct Node {
    name: String,
    text: String,
    kids: Vec<Node>,
}

impl Node {
    fn kid(&self, name: &str) -> Option<&Node> {
        self.kids.iter().find(|k| k.name == name)
    }
    fn at(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |n, p| n.kid(p))
    }
    fn val(&self, path: &[&str]) -> String {
        self.at(path).map(|n| squeeze(&n.text)).unwrap_or_default()
    }
    // depth is capped when the tree is built, so this can't run away
    fn find(&self, name: &str) -> Option<&Node> {
        if self.name == name { Some(self) } else { self.kids.iter().find_map(|k| k.find(name)) }
    }
}

fn tree(xml: &str) -> Result<Node> {
    let mut r = quick_xml::Reader::from_str(xml);
    let mut stack = vec![Node::default()];
    let mut count = 0usize;
    let damaged = || anyhow!("this XML file is damaged or empty and can't be read");
    loop {
        match r.read_event().map_err(|_| damaged())? {
            Event::Start(e) => {
                count += 1;
                if stack.len() > 64 || count > 500_000 {
                    bail!("this XML is too big or too deeply nested to be an invoice");
                }
                stack.push(Node { name: e.local_name().as_ref().to_string(), ..Node::default() });
            }
            Event::Empty(e) => {
                count += 1;
                if let Some(top) = stack.last_mut() {
                    top.kids.push(Node { name: e.local_name().as_ref().to_string(), ..Node::default() });
                }
            }
            Event::End(_) => {
                let done = stack.pop().ok_or_else(damaged)?;
                stack.last_mut().ok_or_else(damaged)?.kids.push(done);
            }
            Event::CData(c) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(c.as_ref());
                }
            }
            ev @ (Event::Text(_) | Event::GeneralRef(_)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&piece(&ev));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    match (stack.pop(), stack.is_empty()) {
        (Some(root), true) if !root.kids.is_empty() => Ok(root),
        _ => Err(damaged()),
    }
}

fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, r) = b.split_first()?;
    let (&l, r) = r.split_first()?;
    let (len, r) = if l < 0x80 {
        (l as usize, r)
    } else {
        let n = (l & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        (r.get(..n)?.iter().fold(0usize, |a, &x| a << 8 | x as usize), r.get(n..)?)
    };
    Some((tag, r.get(..len)?, r.get(len..)?))
}

/// Who a certificate was issued to: the common name, and the tax code Vietnamese issuers put
/// in the user id. Read straight from the certificate; nothing about it is checked.
fn cert_subject(der: &[u8]) -> Option<String> {
    let (_, cert, _) = tlv(der)?;
    let (_, mut tbs, _) = tlv(cert)?;
    if tbs.first() == Some(&0xA0) {
        tbs = tlv(tbs)?.2;
    }
    // serial number, algorithm, issuer, validity
    for _ in 0..4 {
        tbs = tlv(tbs)?.2;
    }
    let (_, mut subject, _) = tlv(tbs)?;
    let (mut cn, mut org, mut uid) = (String::new(), String::new(), String::new());
    while let Some((_, mut set, rest)) = tlv(subject) {
        subject = rest;
        while let Some((_, pair, more)) = tlv(set) {
            set = more;
            let Some((0x06, oid, v)) = tlv(pair) else { continue };
            let Some((tag, val, _)) = tlv(v) else { continue };
            let text = if tag == 0x1E { String::from_utf16_lossy(&val.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect::<Vec<_>>()) } else { String::from_utf8_lossy(val).into_owned() };
            match oid {
                [0x55, 4, 3] => cn = text,
                [0x55, 4, 10] => org = text,
                [9, 0x92, 0x26, 0x89, 0x93, 0xF2, 0x2C, 0x64, 1, 1] => uid = text,
                _ => {}
            }
        }
    }
    let name = if cn.is_empty() { org } else { cn };
    let who = [name, uid].into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(", ");
    (!who.is_empty()).then_some(who)
}

/// The common name out of "CN=…, O=…, C=VN"; the whole line when it has none.
fn common_name(dn: &str) -> String {
    let Some(at) = dn.find("CN=") else { return dn.chars().take(120).collect() };
    let rest = &dn[at + 3..];
    let mut end = rest.len();
    for (i, _) in rest.match_indices(',') {
        let next = rest[i + 1..].trim_start();
        let key = next.split('=').next().unwrap_or("");
        if next.contains('=') && !key.is_empty() && key.len() <= 24 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
            end = i;
            break;
        }
    }
    rest[..end].trim().trim_matches('"').chars().take(120).collect()
}

#[derive(Default, Debug)]
struct EInvoice {
    number: String,
    form: String,
    series: String,
    date: String,
    seller: String,
    seller_tax: String,
    seller_address: String,
    buyer: String,
    buyer_tax: String,
    currency: String,
    words: String,
    before: Option<i128>,
    vat: Option<i128>,
    total: Option<i128>,
    lines: Table,
    /// how far the lines are from the stated before-tax sum, and before tax + VAT from the total
    off_lines: Option<i128>,
    off_total: Option<i128>,
    signed: bool,
    signers: Vec<String>,
}

impl EInvoice {
    fn serial(&self) -> String {
        format!("{}{}", self.form, self.series)
    }
    fn adds_up(&self) -> Option<bool> {
        match (self.off_lines, self.off_total) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) == 0 && b.unwrap_or(0) == 0),
        }
    }
    fn check(&self) -> String {
        let mut parts = Vec::new();
        match self.off_lines {
            Some(0) => parts.push("lines add up to the stated total before tax".to_string()),
            Some(d) => parts.push(format!("lines are off by {} from the stated total before tax", money(d))),
            None => {}
        }
        match self.off_total {
            Some(0) => parts.push("before tax + VAT matches the total".to_string()),
            Some(d) => parts.push(format!("before tax + VAT is off by {} from the total", money(d))),
            None => {}
        }
        if parts.is_empty() { "totals could not be checked".into() } else { parts.join(", ") }
    }
    fn signature(&self) -> String {
        match (self.signed, self.signers.is_empty()) {
            (false, _) => "no digital signature in the file".into(),
            (true, true) => "digital signature present, signer not named, signature NOT verified".into(),
            (true, false) => format!("digital signature present ({}), signature NOT verified", self.signers.join("; ")),
        }
    }
}

fn number(v: &str) -> Option<i128> {
    amount_as(v, Marks::Unknown)
}

fn tidy_amount(v: &str) -> String {
    number(v).map(plain).unwrap_or_else(|| v.to_string())
}

fn einvoice_of(xml: &str) -> Result<EInvoice> {
    let root = tree(xml)?;
    let data = root.find("HDon").and_then(|h| h.kid("DLHDon")).or_else(|| root.find("DLHDon")).filter(|d| d.kid("TTChung").is_some() && d.kid("NDHDon").is_some());
    let Some(data) = data else { bail!("this isn't a Vietnamese e-invoice: the XML has no HDon/DLHDon section") };
    let body = data.kid("NDHDon").context("no invoice body")?;
    let mut inv = EInvoice {
        number: data.val(&["TTChung", "SHDon"]),
        form: data.val(&["TTChung", "KHMSHDon"]),
        series: data.val(&["TTChung", "KHHDon"]),
        date: data.val(&["TTChung", "NLap"]),
        currency: data.val(&["TTChung", "DVTTe"]),
        seller: body.val(&["NBan", "Ten"]),
        seller_tax: body.val(&["NBan", "MST"]),
        seller_address: body.val(&["NBan", "DChi"]),
        buyer: body.val(&["NMua", "Ten"]),
        buyer_tax: body.val(&["NMua", "MST"]),
        words: body.val(&["TToan", "TgTTTBChu"]),
        before: number(&body.val(&["TToan", "TgTCThue"])),
        vat: number(&body.val(&["TToan", "TgTThue"])),
        total: number(&body.val(&["TToan", "TgTTTBSo"])),
        ..EInvoice::default()
    };
    if inv.buyer.is_empty() {
        inv.buyer = body.val(&["NMua", "HVTNMHang"]);
    }
    if let Some((d, _, _)) = date_in(&fold(&inv.date)) {
        inv.date = iso(d);
    }
    let (mut sum, mut any) = (0i128, false);
    for l in body.kid("DSHHDVu").map(|d| d.kids.as_slice()).unwrap_or_default().iter().filter(|k| k.name == "HHDVu").take(50_000) {
        let v = |n: &str| l.val(&[n]);
        if let Some(a) = number(&v("ThTien")) {
            // TChat: 3 is a trade discount and comes off, 4 is a note that carries no amount
            match v("TChat").as_str() {
                "3" => sum -= a,
                "4" => {}
                _ => sum += a,
            }
            any = true;
        }
        inv.lines.push(vec![v("STT"), v("THHDVu"), v("DVTinh"), tidy_amount(&v("SLuong")), tidy_amount(&v("DGia")), tidy_amount(&v("ThTien")), v("TSuat")]);
    }
    if any {
        inv.off_lines = inv.before.map(|b| sum - b);
    }
    let fees: i128 = body.at(&["TToan", "DSLPhi"]).map(|d| d.kids.iter().filter_map(|k| number(&k.val(&["TPhi"]))).sum()).unwrap_or(0);
    if let (Some(b), Some(v), Some(t)) = (inv.before, inv.vat, inv.total) {
        inv.off_total = Some(b + v + fees - t);
    }
    if let Some(sigs) = root.find("DSCKS") {
        inv.signed = sigs.find("SignatureValue").is_some();
        for who in &sigs.kids {
            if who.find("SignatureValue").is_none() {
                continue;
            }
            let role = match who.name.as_str() {
                "NBan" => "seller",
                "NMua" => "buyer",
                "CQT" => "tax authority",
                _ => "other",
            };
            let named = who.find("X509SubjectName").map(|n| common_name(&squeeze(&n.text))).filter(|s| !s.is_empty());
            let from_cert = || {
                use base64::Engine;
                let b64: String = who.find("X509Certificate")?.text.chars().filter(|c| !c.is_whitespace()).take(64_000).collect();
                cert_subject(&base64::engine::general_purpose::STANDARD.decode(b64).ok()?)
            };
            if let Some(name) = named.or_else(from_cert) {
                inv.signers.push(format!("{role}: {name}"));
            }
        }
    }
    Ok(inv)
}

fn read_einvoice(src: &Path) -> Result<EInvoice> {
    if ext(src) != "xml" {
        bail!("this works on .xml e-invoice files");
    }
    einvoice_of(&read_text(src)?)
}

fn einvoice(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let shown = |v: Option<i128>| v.map(plain).unwrap_or_default();
    let yes_no = |v: Option<bool>| match v {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Not checked",
    };
    if let [src] = paths {
        let inv = read_einvoice(src)?;
        let field = |k: &str, v: String| vec![k.to_string(), v];
        let mut t: Table = vec![
            field("Field", "Value".into()),
            field("Invoice number", inv.number.clone()),
            field("Serial", inv.serial()),
            field("Date", inv.date.clone()),
            field("Seller", inv.seller.clone()),
            field("Seller tax code", inv.seller_tax.clone()),
            field("Seller address", inv.seller_address.clone()),
            field("Buyer", inv.buyer.clone()),
            field("Buyer tax code", inv.buyer_tax.clone()),
            field("Currency", inv.currency.clone()),
            field("Before tax", shown(inv.before)),
            field("VAT", shown(inv.vat)),
            field("Total", shown(inv.total)),
            field("Total in words", inv.words.clone()),
            field("Adds up", format!("{}: {}", yes_no(inv.adds_up()), inv.check())),
            field("Digital signature", inv.signature()),
            Vec::new(),
            ["No.", "Item", "Unit", "Quantity", "Unit price", "Amount", "VAT rate"].iter().map(|s| s.to_string()).collect(),
        ];
        t.extend(inv.lines.iter().cloned());
        let out = out_path(src, None, "xlsx");
        workbook(&t, &out)?;
        note(&format!("Invoice {}: {}; {}", inv.number, inv.check(), inv.signature()));
        return Ok(vec![out]);
    }
    let mut t: Table = vec![["Invoice no.", "Serial", "Date", "Seller", "Seller tax code", "Buyer", "Before tax", "VAT", "Total", "Adds up", "File"].iter().map(|s| s.to_string()).collect()];
    let (mut skipped, mut off, mut signed, mut first_error) = (Vec::new(), 0, 0, None);
    for p in paths {
        match read_einvoice(p) {
            Ok(inv) => {
                off += usize::from(inv.adds_up() == Some(false));
                signed += usize::from(inv.signed);
                t.push(vec![inv.number.clone(), inv.serial(), inv.date.clone(), inv.seller.clone(), inv.seller_tax.clone(), inv.buyer.clone(), shown(inv.before), shown(inv.vat), shown(inv.total), yes_no(inv.adds_up()).into(), file_name(p)]);
            }
            Err(e) => {
                skipped.push(file_name(p));
                first_error.get_or_insert(e);
            }
        }
    }
    let n = t.len() - 1;
    if n == 0 {
        return Err(first_error.unwrap_or_else(|| anyhow!("no files to read")));
    }
    let dir = paths.first().and_then(|p| p.parent()).context("no folder")?;
    let out = unique(dir, "E-invoice register", "xlsx");
    workbook(&t, &out)?;
    let mut line = format!("{n} invoices, {} with totals that don't add up, {signed} carry a signature (NOT verified)", if off == 0 { "none".to_string() } else { off.to_string() });
    if !skipped.is_empty() {
        line.push_str(&format!("; {} skipped, not e-invoices: {}", skipped.len(), skipped.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
    }
    note(&line);
    Ok(vec![out])
}

// ---------------------------------------------------------------- reconcile

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role {
    Debit,
    Credit,
    Amount,
    Date,
}

/// What a column holds, going by its header.
fn role(header: &str) -> Option<Role> {
    let h = squeeze(&fold(header));
    let has = |ks: &[&str]| ks.iter().any(|k| h.contains(k));
    let is = |ks: &[&str]| ks.contains(&h.as_str());
    if has(&["so du", "balance", "luy ke"]) {
        return None;
    }
    let debit = has(&["ghi no", "debit", "withdraw", "tien ra", "money out", "paid out", "phat sinh no"]) || is(&["no", "chi", "rut"]);
    let credit = has(&["ghi co", "credit", "deposit", "tien vao", "money in", "paid in", "phat sinh co"]) || is(&["co", "thu", "nop"]);
    match (debit, credit) {
        (true, true) => Some(Role::Amount),
        (true, false) => Some(Role::Debit),
        (false, true) => Some(Role::Credit),
        _ if has(&["so tien", "amount", "thanh tien", "gia tri", "tong tien", "phat sinh"]) || is(&["tien", "value", "total", "tong cong"]) => Some(Role::Amount),
        _ if h.split(|c: char| !c.is_alphanumeric()).any(|w| w == "ngay" || w == "date") => Some(Role::Date),
        _ => None,
    }
}

struct Side {
    name: String,
    t: Table,
    head: Option<usize>,
    /// amount columns and the sign each counts with
    amounts: Vec<(usize, i128)>,
    date: Option<usize>,
    how: Marks,
}

impl Side {
    fn first_row(&self) -> usize {
        self.head.map_or(0, |h| h + 1)
    }
    fn amount(&self, row: &[String]) -> Option<i128> {
        let got: Vec<i128> = self.amounts.iter().filter_map(|(c, sign)| row.get(*c).and_then(|v| amount_as(v, self.how)).map(|v| v * sign)).collect();
        let v: i128 = got.iter().sum();
        (!got.is_empty() && v != 0).then_some(v)
    }
    fn day(&self, row: &[String]) -> Option<i64> {
        self.date.and_then(|c| row.get(c)).and_then(|v| date_in(&fold(v))).map(|d| day_number(d.0))
    }
}

fn side(src: &Path) -> Result<Side> {
    let name = file_name(src);
    let t = load(src).with_context(|| name.clone())?.t;
    let how = marks(&t);
    let mut s = Side { name, t, head: None, amounts: Vec::new(), date: None, how };
    // statements open with the account's details; the header is the first row that names an amount
    for (i, row) in s.t.iter().enumerate().take(30) {
        let roles: Vec<Option<Role>> = row.iter().map(|c| role(c)).collect();
        let col = |want: Role| roles.iter().position(|r| *r == Some(want));
        let amounts = match (col(Role::Debit), col(Role::Credit), col(Role::Amount)) {
            (Some(d), Some(c), _) => vec![(c, 1), (d, -1)],
            (_, _, Some(a)) => vec![(a, 1)],
            (Some(d), None, None) => vec![(d, 1)],
            (None, Some(c), None) => vec![(c, 1)],
            _ => continue,
        };
        (s.head, s.amounts, s.date) = (Some(i), amounts, col(Role::Date));
        break;
    }
    if s.head.is_none() && s.t.len() > 1 && s.t.first().is_some_and(|r| r.iter().all(|c| amount_as(c, how).is_none() && date_in(&fold(c)).is_none())) {
        s.head = Some(0);
    }
    let body = s.t.get(s.first_row()..).unwrap_or_default();
    let width = body.iter().map(Vec::len).max().unwrap_or(0);
    let mostly = |hits: usize, of: usize| of > 0 && hits * 10 >= of * 8;
    for c in 0..width {
        if s.date.is_some() && !s.amounts.is_empty() {
            break;
        }
        let cells: Vec<&str> = body.iter().filter_map(|r| r.get(c)).map(|v| v.trim()).filter(|v| !v.is_empty()).collect();
        if s.date.is_none() && mostly(cells.iter().filter(|v| date_in(&fold(v)).is_some()).count(), cells.len()) {
            s.date = Some(c);
            continue;
        }
        // codes and account numbers are digits but not money; nor is a column that just counts rows
        let ids = cells.iter().any(|v| (v.len() > 1 && v.starts_with('0') && v.bytes().all(|b| b.is_ascii_digit())) || (v.len() > 15 && v.bytes().all(|b| b.is_ascii_digit())));
        let counting = mostly(cells.iter().enumerate().filter(|(i, v)| v.parse::<usize>().ok() == Some(i + 1)).count(), cells.len());
        if s.amounts.is_empty() && !ids && !counting && mostly(cells.iter().filter(|v| amount_as(v, how).is_some()).count(), cells.len()) {
            s.amounts = vec![(c, 1)];
        }
    }
    if s.amounts.is_empty() {
        bail!("no amount column found in {}", s.name);
    }
    Ok(s)
}

/// Pairs entries of equal size one to one, nearest dates first. Two dated entries more than
/// three days apart are not the same payment.
fn pair(a: &[(i128, Option<i64>)], b: &[(i128, Option<i64>)]) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let (mut ma, mut mb) = (vec![None; a.len()], vec![None; b.len()]);
    let mut groups: HashMap<u128, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (i, x) in a.iter().enumerate() {
        groups.entry(x.0.unsigned_abs()).or_default().0.push(i);
    }
    for (j, y) in b.iter().enumerate() {
        if let Some(g) = groups.get_mut(&y.0.unsigned_abs()) {
            g.1.push(j);
        }
    }
    let apart = |i: usize, j: usize| match (a.get(i).and_then(|x| x.1), b.get(j).and_then(|y| y.1)) {
        (Some(x), Some(y)) => Some((x - y).abs()).filter(|d| *d <= 3),
        // an undated entry can be any day, but a dated pair is the better match
        _ => Some(4),
    };
    for (ia, ib) in groups.into_values() {
        if ia.len() * ib.len() <= 250_000 {
            let mut all: Vec<(i64, usize, usize)> = ia.iter().flat_map(|&i| ib.iter().filter_map(move |&j| apart(i, j).map(|d| (d, i, j)))).collect();
            all.sort_unstable();
            for (_, i, j) in all {
                if ma[i].is_none() && mb[j].is_none() {
                    (ma[i], mb[j]) = (Some(j), Some(i));
                }
            }
            continue;
        }
        // thousands of entries of one size: walk both in date order instead of weighing every pair
        let key = |list: &[(i128, Option<i64>)], i: usize| (list.get(i).and_then(|x| x.1).unwrap_or(i64::MAX), i);
        let (mut ia, mut ib) = (ia, ib);
        ia.sort_unstable_by_key(|&i| key(a, i));
        ib.sort_unstable_by_key(|&j| key(b, j));
        let (mut x, mut y) = (0, 0);
        while let (Some(&i), Some(&j)) = (ia.get(x), ib.get(y)) {
            if apart(i, j).is_some() {
                (ma[i], mb[j]) = (Some(j), Some(i));
                x += 1;
                y += 1;
            } else if key(a, i) < key(b, j) {
                x += 1;
            } else {
                y += 1;
            }
        }
    }
    (ma, mb)
}

fn reconcile(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let [pa, pb] = paths else { bail!("choose exactly two tables to reconcile, such as a bank statement and a ledger") };
    let (a, b) = (side(pa)?, side(pb)?);
    let (name_a, name_b) = if a.name == b.name { (format!("A: {}", a.name), format!("B: {}", b.name)) } else { (a.name.clone(), b.name.clone()) };
    let entries = |s: &Side| -> Vec<(usize, i128, Option<i64>)> { s.t.iter().enumerate().skip(s.first_row()).filter_map(|(i, r)| s.amount(r).map(|v| (i, v, s.day(r)))).collect() };
    let (ea, eb) = (entries(&a), entries(&b));
    let slim = |e: &[(usize, i128, Option<i64>)]| e.iter().map(|x| (x.1, x.2)).collect::<Vec<_>>();
    let (ma, mb) = pair(&slim(&ea), &slim(&eb));

    let mut out: Table = Vec::new();
    let mut totals = [(0usize, 0i128); 2];
    let mut matched = 0;
    for (k, (s, mine, theirs, links, name)) in [(&a, &ea, &eb, &ma, &name_a), (&b, &eb, &ea, &mb, &name_b)].into_iter().enumerate() {
        if k == 1 {
            out.push(Vec::new());
        }
        let mut head = vec!["Status".to_string(), "File".into(), "Row".into(), "Partner row".into()];
        head.extend(s.head.and_then(|h| s.t.get(h)).cloned().unwrap_or_default());
        out.push(head);
        let at: HashMap<usize, usize> = mine.iter().enumerate().map(|(n, e)| (e.0, n)).collect();
        for (i, row) in s.t.iter().enumerate().skip(s.first_row()) {
            if row.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            let (status, partner) = match at.get(&i).and_then(|n| mine.get(*n).zip(links.get(*n))) {
                None => ("No amount".to_string(), String::new()),
                Some((_, Some(j))) => {
                    matched += usize::from(k == 0);
                    ("Matched".to_string(), theirs.get(*j).map(|e| (e.0 + 1).to_string()).unwrap_or_default())
                }
                Some((e, None)) => {
                    totals[k].0 += 1;
                    totals[k].1 += e.1.abs();
                    (format!("Only in {name}"), String::new())
                }
            };
            let mut line = vec![status, s.name.clone(), (i + 1).to_string(), partner];
            line.extend(row.iter().map(|c| uninert(c).to_string()));
            out.push(line);
        }
    }
    let path = out_path(pa, Some("Reconciled"), "xlsx");
    workbook(&out, &path)?;
    note(&format!("{matched} matched, {} only in {name_a} ({}), {} only in {name_b} ({})", totals[0].0, money(totals[0].1), totals[1].0, money(totals[1].1)));
    Ok(vec![path])
}

// ---------------------------------------------------------------- tidy, split

/// The table cleaned up, and how many cells, empty rows, empty columns and repeated rows went.
fn tidied(t: Table) -> (Table, [usize; 4]) {
    let mut cells = 0;
    let mut rows: Table = t
        .into_iter()
        .map(|r| {
            r.into_iter()
                .map(|c| {
                    // a line break inside a cell is meant; spaces around and doubled within are not
                    let clean = c.lines().map(squeeze).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
                    cells += usize::from(clean != c);
                    clean
                })
                .collect()
        })
        .collect();
    let all = rows.len();
    rows.retain(|r: &Vec<String>| r.iter().any(|c| !c.is_empty()));
    let empty_rows = all - rows.len();
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let keep: Vec<bool> = (0..width).map(|c| rows.iter().any(|r| r.get(c).is_some_and(|v| !v.is_empty()))).collect();
    let empty_cols = keep.iter().filter(|k| !**k).count();
    if empty_cols > 0 {
        for r in &mut rows {
            let mut i = 0;
            r.retain(|_| {
                i += 1;
                keep.get(i - 1).copied().unwrap_or(true)
            });
        }
    }
    let all = rows.len();
    let mut seen = HashSet::new();
    rows.retain(|r| {
        let used = r.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1);
        seen.insert(r.get(..used).unwrap_or_default().to_vec())
    });
    let repeats = all - rows.len();
    (rows, [cells, empty_rows, empty_cols, repeats])
}

fn tidy(src: &Path) -> Result<Vec<PathBuf>> {
    let l = load(src)?;
    let (t, [cells, rows, cols, repeats]) = tidied(l.t.clone());
    if cells + rows + cols + repeats == 0 {
        bail!("nothing to tidy: this table is already clean");
    }
    if t.is_empty() {
        bail!("this file has no rows");
    }
    let out = out_path(src, Some("Tidy"), &l.ext);
    save(&t, &l, &out)?;
    let count = |n: usize, one: &str, many: &str| (n > 0).then(|| format!("{n} {}", if n == 1 { one } else { many }));
    let parts: Vec<String> = [count(cells, "cell trimmed", "cells trimmed"), count(rows, "empty row removed", "empty rows removed"), count(cols, "empty column removed", "empty columns removed"), count(repeats, "duplicate row removed", "duplicate rows removed")].into_iter().flatten().collect();
    note(&format!("{}{}", parts.join(", "), first_sheet_only(&l)));
    Ok(vec![out])
}

fn split(src: &Path, by: Option<&str>) -> Result<Vec<PathBuf>> {
    let l = load(src)?;
    let Some((head, body)) = l.t.split_first() else { bail!("this file has no rows") };
    let col = match by.map(str::trim).filter(|h| !h.is_empty()) {
        None => 0,
        Some(h) => head.iter().position(|c| norm(c) == norm(h)).with_context(|| format!("no column named \"{h}\" in this table"))?,
    };
    let mut order: Vec<&str> = Vec::new();
    let mut groups: HashMap<&str, Vec<&Vec<String>>> = HashMap::new();
    for r in body.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())) {
        let v = r.get(col).map_or("", |v| v.trim());
        groups.entry(v).or_insert_with(|| {
            order.push(v);
            Vec::new()
        }).push(r);
    }
    if order.is_empty() {
        bail!("this table has no rows under its header to split");
    }
    if order.len() > 500 {
        bail!("too many different values in that column ({}): splitting stops at 500 files", order.len());
    }
    let title = head.get(col).map(|h| squeeze(h)).filter(|h| !h.is_empty());
    let dir = out_dir(src, &title.map_or("split".to_string(), |h| format!("by {}", safe_name(&h).chars().take(40).collect::<String>())))?;
    let mut rows = 0;
    for v in &order {
        let mut t = vec![head.clone()];
        t.extend(groups.get(v).into_iter().flatten().map(|r| (*r).clone()));
        rows += t.len() - 1;
        save(&t, &l, &unique(&dir, &safe_name(uninert(v)), &l.ext))?;
    }
    note(&format!("{} files from {rows} rows{}", order.len(), first_sheet_only(&l)));
    Ok(vec![dir])
}

// ---------------------------------------------------------------- mail merge

#[derive(Debug, PartialEq)]
enum Seg {
    Text(String),
    Slot { key: String, raw: String },
}

/// `{{ … }}` in a run of text: (start, end, what is between).
fn slots(s: &str) -> Vec<(usize, usize, &str)> {
    let (mut out, mut from) = (Vec::new(), 0);
    while let Some(a) = s.get(from..).and_then(|r| r.find("{{")).map(|i| i + from) {
        let Some(b) = s.get(a + 2..).and_then(|r| r.find("}}")).map(|i| i + a + 2) else { break };
        let name = &s[a + 2..b];
        // "{{{Name}}" or a stray "{{" before a real one: start again from the inner braces
        if name.contains('{') || name.trim().is_empty() || name.len() > 200 || name.contains('<') {
            from = a + 1;
            continue;
        }
        out.push((a, b + 2, name));
        from = b + 2;
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// One part of a .docx cut into fixed text and placeholders. Word breaks what you type into
/// several runs whenever spelling, language or formatting changes underneath, so a placeholder
/// can lie across many `<w:t>`; its pieces are gathered into the first of them.
fn template(xml: &str) -> Vec<Seg> {
    // every <w:t>…</w:t>: (start of tag, start of text, end of text)
    let (mut spans, mut from) = (Vec::new(), 0);
    while let Some(a) = xml.get(from..).and_then(|r| r.find("<w:t")).map(|i| i + from) {
        let after = a + 4;
        if !xml[after..].starts_with(['>', ' ', '\t', '\r', '\n']) {
            from = after;
            continue;
        }
        let Some(gt) = xml[after..].find('>').map(|i| i + after) else { break };
        if xml[..gt].ends_with('/') {
            from = gt;
            continue;
        }
        let Some(end) = xml[gt..].find("</w:t>").map(|i| i + gt) else { break };
        spans.push((a, gt + 1, end));
        from = end + 6;
    }
    let mut texts: Vec<String> = spans.iter().map(|s| xml[s.1..s.2].to_string()).collect();
    // a placeholder never crosses a paragraph
    let ends: Vec<usize> = xml.match_indices("</w:p>").map(|m| m.0).collect();
    let (mut i, mut p) = (0, 0);
    while i < spans.len() {
        while ends.get(p).is_some_and(|e| *e < spans[i].0) {
            p += 1;
        }
        let stop = ends.get(p).copied().unwrap_or(xml.len());
        let mut j = i;
        while j < spans.len() && spans[j].0 < stop {
            j += 1;
        }
        let whole: String = texts[i..j].concat();
        let mut starts = Vec::with_capacity(j - i);
        let mut at = 0;
        for t in &texts[i..j] {
            starts.push(at);
            at += t.len();
        }
        let owner = |pos: usize| starts.iter().rposition(|s| *s <= pos).unwrap_or(0);
        let (mut cut, mut add) = (vec![0usize; j - i], vec![String::new(); j - i]);
        for (a, b, _) in slots(&whole) {
            let (first, last) = (owner(a), owner(b - 1));
            if first == last {
                continue;
            }
            let first_end = starts.get(first + 1).copied().unwrap_or(whole.len());
            add[first] = whole[first_end..b].to_string();
            for k in first + 1..last {
                cut[k] = texts[i + k].len();
            }
            cut[last] = b - starts[last];
        }
        for k in 0..j - i {
            if cut[k] > 0 || !add[k].is_empty() {
                texts[i + k] = format!("{}{}", &texts[i + k][cut[k]..], add[k]);
            }
        }
        i = j.max(i + 1);
    }
    let mut out = Vec::new();
    let mut text = String::new();
    let mut done = 0;
    for (s, body) in spans.iter().zip(&texts) {
        text.push_str(&xml[done..s.0]);
        let found = slots(body);
        // a value may begin or end with a space, which Word drops unless told to keep it
        text.push_str(if found.is_empty() { &xml[s.0..s.1] } else { "<w:t xml:space=\"preserve\">" });
        let mut at = 0;
        for (a, b, name) in found {
            text.push_str(&body[at..a]);
            out.push(Seg::Text(std::mem::take(&mut text)));
            out.push(Seg::Slot { key: norm(&unescape(name)), raw: body[a..b].to_string() });
            at = b;
        }
        text.push_str(&body[at..]);
        done = s.2;
    }
    text.push_str(&xml[done..]);
    out.push(Seg::Text(text));
    out
}

fn fill(segs: &[Seg], values: &HashMap<String, &str>) -> String {
    let mut out = String::new();
    for s in segs {
        match s {
            Seg::Text(t) => out.push_str(t),
            Seg::Slot { key, raw } => match values.get(key) {
                Some(v) => out.push_str(&xml_escape(&v.replace("\r\n", "\n").replace('\r', "\n")).replace('\n', "</w:t><w:br/><w:t xml:space=\"preserve\">")),
                None => out.push_str(raw),
            },
        }
    }
    out
}

fn mailmerge(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    const NEEDS: &str = "choose one .docx template and one .csv or .xlsx list of names";
    let [a, b] = paths else { bail!(NEEDS) };
    let (doc, list) = match (ext(a).as_str(), ext(b).as_str()) {
        ("docx", e) if is_table(e) => (a, b),
        (e, "docx") if is_table(e) => (b, a),
        _ => bail!(NEEDS),
    };
    let data = load(list)?;
    let Some((head, body)) = data.t.split_first() else { bail!("this file has no rows") };
    let rows: Vec<&Vec<String>> = body.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect();
    if rows.is_empty() {
        bail!("the list has no rows under its header");
    }
    if rows.len() > 2000 {
        bail!("this list has {} rows: mail merge stops at 2000 documents", rows.len());
    }
    // only these parts are rewritten, and only under names that can't leave the work folder
    let plain_name = |n: &str| n.strip_prefix("word/").is_some_and(|f| (f == "document.xml" || ((f.starts_with("header") || f.starts_with("footer")) && f.ends_with(".xml"))) && f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'));
    let parts: Vec<String> = names(doc)?.into_iter().filter(|n| plain_name(n)).take(64).collect();
    if !parts.iter().any(|n| n == "word/document.xml") {
        bail!("can't read this document: it may be damaged or password-protected");
    }
    let mut cut = Vec::new();
    for n in &parts {
        let xml = String::from_utf8(entry(doc, n, 64)?).map_err(|_| anyhow!("can't read this document: it may be damaged or password-protected"))?;
        cut.push((n, template(&xml)));
    }
    let columns: HashMap<String, usize> = head.iter().enumerate().rev().map(|(i, h)| (norm(h), i)).filter(|(h, _)| !h.is_empty()).collect();
    let (mut known, mut unknown) = (0, Vec::new());
    for s in cut.iter().flat_map(|c| &c.1) {
        if let Seg::Slot { key, raw } = s {
            if columns.contains_key(key) {
                known += 1;
            } else if !unknown.contains(raw) {
                unknown.push(raw.clone());
            }
        }
    }
    if known == 0 && unknown.is_empty() {
        bail!("no placeholders found in this template: write them like {{{{Name}}}}, matching the list's column headers");
    }
    if known == 0 {
        bail!("none of the placeholders match a column of the list: {}", unknown.iter().take(5).cloned().collect::<Vec<_>>().join(", "));
    }
    cut.retain(|c| c.1.iter().any(|s| matches!(s, Seg::Slot { key, .. } if columns.contains_key(key))));

    let dir = out_dir(doc, "merged")?;
    let work = temp_file("merge");
    let made = (|| -> Result<()> {
        std::fs::create_dir_all(work.join("word"))?;
        for r in &rows {
            let values: HashMap<String, &str> = columns.iter().map(|(h, i)| (h.clone(), r.get(*i).map_or("", |v| uninert(v)))).collect();
            for (n, segs) in &cut {
                std::fs::write(work.join(n), fill(segs, &values))?;
            }
            let out = unique(&dir, &safe_name(uninert(r.first().map_or("", |v| v.as_str()))), "docx");
            std::fs::copy(doc, &out)?;
            // zip swaps just these parts inside the copy; every other entry stays as it was, in place
            let o = crate::engines::tools::output_within(Command::new("/usr/bin/zip").current_dir(&work).args(["-q", "-X"]).arg(&out).args(cut.iter().map(|c| c.0)), std::time::Duration::from_secs(300))?;
            if !o.status.success() {
                bail!("couldn't write the document: {}", String::from_utf8_lossy(&o.stderr).trim());
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    made?;
    let mut line = format!("{} documents{}", rows.len(), first_sheet_only(&data));
    if !unknown.is_empty() {
        line.push_str(&format!("; left as written, no such column: {}", unknown.iter().take(5).cloned().collect::<Vec<_>>().join(", ")));
    }
    note(&line);
    Ok(vec![dir])
}

// ---------------------------------------------------------------- bank statement PDF

/// Lines that open with a date, cut into date, details and the amounts that close the line.
fn statement_rows(text: &str) -> Table {
    let dated: Vec<(&str, String, usize)> = text.lines().filter_map(|l| {
        let f = fold(l.trim());
        let (d, _, end) = date_in(&f)?;
        // the date has to be what the line starts with
        (end <= 12 || f.starts_with("ngay")).then(|| (tail(l.trim(), &f, end), iso(d), end))
    }).take(200_000).collect();
    let how = marks(&dated.iter().map(|r| r.0.split_whitespace().map(str::to_string).collect()).collect());
    // 120.000 is a hundred and twenty thousand or a hundred and twenty: only rewritten when the page shows which
    let shown = |v: &str| match amount_as(v, how) {
        Some(a) if how != Marks::Unknown && v.bytes().any(|b| b.is_ascii_digit()) => plain(a),
        _ => v.to_string(),
    };
    let mut rows: Table = Vec::new();
    for (rest, date, _) in dated {
        let mut row = vec![date];
        if rest.trim().contains("  ") {
            row.extend(rest.split("  ").map(str::trim).filter(|c| !c.is_empty()).map(shown));
        } else {
            let words: Vec<&str> = rest.split_whitespace().collect();
            let tail = words.iter().rev().take_while(|w| w.bytes().any(|b| b.is_ascii_digit()) && amount_as(w, how).is_some()).count();
            let (text, sums) = words.split_at(words.len() - tail);
            row.push(text.join(" "));
            row.extend(sums.iter().map(|w| shown(w)));
        }
        rows.push(row);
    }
    rows
}

fn statement(src: &Path) -> Result<Vec<PathBuf>> {
    if kind(src) != Kind::Pdf {
        bail!("this works on .pdf bank statements");
    }
    let rows = statement_rows(&pdf_text(src)?);
    if rows.is_empty() {
        bail!("no dated lines found in this PDF: it doesn't read like a bank statement");
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(2);
    let mut t: Table = vec![(0..width).map(|i| match i {
        0 => "Date".to_string(),
        1 => "Details".to_string(),
        i => format!("Column {}", i + 1),
    }).collect()];
    let n = rows.len();
    t.extend(rows);
    let out = out_path(src, None, "xlsx");
    workbook(&t, &out)?;
    note(&format!("{n} rows read from the statement; check them against the PDF"));
    Ok(vec![out])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: [&str; 8]) -> Fields {
        let [seller, tax_code, number, serial, date, before_tax, vat, total] = v.map(str::to_string);
        Fields { seller, tax_code, number, serial, date, before_tax, vat, total }
    }

    #[test]
    fn folding_and_dates() {
        assert_eq!(fold("Tổng cộng tiền thanh toán"), "tong cong tien thanh toan");
        assert_eq!(fold("ĐƠN VỊ BÁN HÀNG"), "don vi ban hang");
        assert_eq!(fold("Người mua: Nguyễn Thị Hương"), "nguoi mua: nguyen thi huong");
        // marks typed as separate combining characters
        assert_eq!(fold("Ha\u{300} No\u{323}\u{302}i"), "ha noi");
        assert_eq!(tail("Số: AB/12", &fold("Số: AB/12"), 4), "AB/12");
        let d = |s: &str| date_in(&fold(s)).map(|d| iso(d.0));
        assert_eq!(d("Ngày 03 tháng 11 năm 2026").as_deref(), Some("2026-11-03"));
        assert_eq!(d("Ngay03thang11nam2026").as_deref(), Some("2026-11-03"));
        assert_eq!(d("ngày 3 tháng 2 năm 2026").as_deref(), Some("2026-02-03"));
        assert_eq!(d("03/11/2026").as_deref(), Some("2026-11-03"));
        assert_eq!(d("Date: 3-11-2026 10:30").as_deref(), Some("2026-11-03"));
        assert_eq!(d("03.11.2026").as_deref(), Some("2026-11-03"));
        assert_eq!(d("2026-11-03T08:00:00").as_deref(), Some("2026-11-03"));
        assert_eq!(d("11/25/2026").as_deref(), Some("2026-11-25"));
        assert_eq!(d("3 November 2026").as_deref(), Some("2026-11-03"));
        assert_eq!(d("Nov 3, 2026").as_deref(), Some("2026-11-03"));
        assert_eq!(d("03-Nov-2026").as_deref(), Some("2026-11-03"));
        assert_eq!(d("31/02/2026"), None);
        assert_eq!(d("1.200.000.000"), None);
        assert_eq!(d("024-3825-1234"), None);
        assert_eq!(d("03/11/26"), None);
        assert_eq!(d("12.500.2026 đ"), None);
        assert_eq!(day_number((2026, 3, 1)) - day_number((2026, 2, 27)), 2);
        assert_eq!(day_number((1970, 1, 1)), 719468);
    }

    #[test]
    fn money_on_a_line() {
        assert_eq!(numbers("tong cong: 12.500.000 d"), vec!["12.500.000"]);
        assert_eq!(numbers("vnd 12,500,000"), vec!["12,500,000"]);
        assert_eq!(numbers("thue suat 10% tien thue 1.250.000d"), vec!["1.250.000"]);
        assert_eq!(numbers("ngay 03/11/2026 so 0000123 ky hieu 1c26taa"), Vec::<&str>::new());
        assert_eq!(numbers("inv-2026-001 a4 x 5"), vec!["5"]);
        assert_eq!(lone_amount("13.750.000 d"), Some("13.750.000"));
        assert_eq!(lone_amount("vnd 13,750,000"), Some("13,750,000"));
        assert_eq!(lone_amount("tong 13.750.000"), None);
        assert_eq!(classify("cong tien hang (total amount):"), Some(Sum::Before));
        assert_eq!(classify("tien thue gtgt (vat amount):"), Some(Sum::Vat));
        assert_eq!(classify("thue suat gtgt: 10%"), None);
        assert_eq!(classify("tong cong tien thanh toan"), Some(Sum::Total(true)));
        assert_eq!(classify("tong cong"), Some(Sum::Total(false)));
        assert_eq!(classify("total (incl. vat)"), Some(Sum::Total(true)));
        assert_eq!(classify("total excl. vat"), Some(Sum::Before));
        assert_eq!(classify("ma so thue (tax code): 0101234567"), None);
        assert_eq!(classify("hinh thuc thanh toan: ck"), None);
        assert_eq!(classify("tong so luong: 5"), None);
    }

    #[test]
    fn vietnamese_invoice() {
        let text = "HÓA ĐƠN GIÁ TRỊ GIA TĂNG\nKý hiệu (Serial): 1C26TAA\nSố (No.): 0000123\nNgày 03 tháng 11 năm 2026\n\
            Đơn vị bán hàng (Seller): CÔNG TY TNHH THƯƠNG MẠI MINH ANH\nMã số thuế (Tax code): 0101234567\nĐịa chỉ: 12 Lý Thường Kiệt, Hà Nội\nĐiện thoại: 024 3825 1234\n\
            Họ tên người mua hàng (Buyer): Nguyễn Văn An\nTên đơn vị: CÔNG TY CỔ PHẦN SAO MAI\nMã số thuế (Tax code): 0309876543\nHình thức thanh toán: CK\n\
            STT Tên hàng hóa Đơn vị Số lượng Đơn giá Thành tiền\n1 Giấy in A4 Ram 100 85.000 8.500.000\n2 Mực máy in Hộp 4 1.000.000 4.000.000\n\
            Cộng tiền hàng (Total amount): 12.500.000\nThuế suất GTGT (VAT rate): 10% Tiền thuế GTGT (VAT amount): 1.250.000\n\
            Tổng cộng tiền thanh toán (Total payment): 13.750.000\nSố tiền viết bằng chữ: Mười ba triệu bảy trăm năm mươi nghìn đồng\n\
            Người mua hàng (Buyer) Người bán hàng (Seller)\n(Ký, ghi rõ họ tên) (Ký, đóng dấu)\nKý ngày: 04/11/2026";
        assert_eq!(extract(text), f(["CÔNG TY TNHH THƯƠNG MẠI MINH ANH", "0101234567", "0000123", "1C26TAA", "2026-11-03", "12500000", "1250000", "13750000"]));
    }

    #[test]
    fn ocr_noise() {
        // no diacritics, labels and values as OCR reads a two-column totals box, currency glued on
        let text = "CONG TY CO PHAN SAO MAI\nMST: 0 3 0 9 8 7 6 5 4 3 - 0 0 1\nHOA DON BAN HANG\nSo: 00045\nKy hieu: 2C26MSM\nNgay 3 thang 2 nam 2026\n\
            Khach hang: Cong ty TNHH Binh Minh\nMST: 0101234567\nCong tien hang:\nThue suat GTGT: 8%\nTien thue GTGT:\nTong cong:\n1.000.000đ\n80.000 đ\n1.080.000 VND";
        assert_eq!(extract(text), f(["CONG TY CO PHAN SAO MAI", "0309876543-001", "00045", "2C26MSM", "2026-02-03", "1000000", "80000", "1080000"]));
        // the same box, but the sums don't agree: positions alone are not trusted
        let bad = "Cong tien hang:\nTien thue GTGT:\nTong cong:\n1.000.000\n80.000\n9.999.999";
        assert_eq!(extract(bad), Fields::default());
        // one label, its value on the next line
        assert_eq!(extract("Tong cong\n12.500.000 đ").total, "12500000");
        assert_eq!(extract("Tổng cộng: 12.500.000 đ").total, "12500000");
        assert_eq!(extract("Total: VND 12,500,000").total, "12500000");
        assert_eq!(extract("Tiền thuế: 120.000").vat, "120000");
        // a summary row with all three
        let row = extract("Tong cong: 12.500.000 1.250.000 13.750.000");
        assert_eq!((row.before_tax.as_str(), row.vat.as_str(), row.total.as_str()), ("12500000", "1250000", "13750000"));
    }

    #[test]
    fn english_invoice() {
        let text = "Acme Trading Co., Ltd\n12 Harbour Road\nVAT No: 0312345678\nINVOICE\nInvoice No.: INV-2026-0042\nInvoice date: 11/25/2026\nDue date: 12/25/2026\n\
            Bill To: Sao Mai JSC\nTax code: 0309876543\nDescription Qty Unit price Amount\nPaper A4 100 3.50 350.00\nToner 4 220.00 880.00\n\
            Subtotal: 1,230.00\nVAT (10%): 123.00\nTotal due: USD 1,353.00\nAccount No: 001100220033";
        assert_eq!(extract(text), f(["Acme Trading Co., Ltd", "0312345678", "INV-2026-0042", "", "2026-11-25", "1230", "123", "1353"]));
        let other = "Seller: Northwind Ltd      Invoice #7781\nDate: 3 November 2026\nNet amount 2,000.00\nSales tax 160.00\nGrand Total 2,160.00";
        assert_eq!(extract(other), f(["Northwind Ltd", "", "7781", "", "2026-11-03", "2000", "160", "2160"]));
    }

    #[test]
    fn nothing_is_guessed() {
        assert_eq!(extract(""), Fields::default());
        assert_eq!(extract("Xin chào\nSố điện thoại: 0912345678\nSố tài khoản: 123456789\nThông tin: 0912 345 678\n12.500.000"), Fields::default());
        // a tax code of the wrong length, a two-digit year, a rate without an amount
        let e = extract("Seller: An Phat\nMST: 01012345\nNgày 03/11/26\nThuế suất: 10\nHạn thanh toán: 30 ngày");
        assert_eq!(e, f(["An Phat", "", "", "", "", "", "", ""]));
        // a name in capitals that wraps takes its second line; anything else on the next line is left alone
        assert_eq!(extract("Đơn vị bán hàng (Seller): CÔNG TY TNHH THƯƠNG MẠI \nMINH ANH\nMã số thuế: 0101234567").seller, "CÔNG TY TNHH THƯƠNG MẠI MINH ANH");
        assert_eq!(extract("Seller: ACME TRADING\nMST: 0101234567").seller, "ACME TRADING");
        assert_eq!(extract("Seller: ACME TRADING\n12 HARBOUR ROAD").seller, "ACME TRADING");
        assert_eq!(extract("Seller: Acme Trading\nHANOI").seller, "Acme Trading");
        // the buyer's tax code is not the seller's
        assert_eq!(extract("Khách hàng: Sao Mai\nMã số thuế: 0309876543").tax_code, "");
        assert_eq!(tax_code("0101234567 Địa chỉ"), "0101234567");
        assert_eq!(tax_code("0101234567-001"), "0101234567-001");
        assert_eq!(tax_code("010123456"), "");
    }

    const CERT: &str = "MIICOzCCAaQCCQDXOgqkS+UYHjANBgkqhkiG9w0BAQsFADBiMQswCQYDVQQGEwJWTjESMBAGA1UEBwwJSMOgIE7hu5lpMR4wHAYKCZImiZPyLGQBAQwOTVNUOjAxMDEyMzQ1NjcxHzAdBgNVBAMMFkPDlE5HIFRZIFROSEggTUlOSCBBTkgwHhcNMjYxMTEzMTI0ODIxWhcNMzYxMTEwMTI0ODIxWjBiMQswCQYDVQQGEwJWTjESMBAGA1UEBwwJSMOgIE7hu5lpMR4wHAYKCZImiZPyLGQBAQwOTVNUOjAxMDEyMzQ1NjcxHzAdBgNVBAMMFkPDlE5HIFRZIFROSEggTUlOSCBBTkgwgZ8wDQYJKoZIhvcNAQEBBQADgY0AMIGJAoGBAMSHHeiRCTeqz0eOVDi9Os8fXr6844wAMafF8p9Ia+tOi4E24PSSNgk1HakBF0dmvx2qGhaG+vYTr0ACa+Ng2ZlMInPTZ9m/r3X8m3eWsT15PH0NqbVpDtEwGcp9Gc/D8ccYqhWutVCnt6T/tKDt0AmMz5AJppqTx47qFZ5/ypWJAgMBAAEwDQYJKoZIhvcNAQELBQADgYEAEnlT4o4q94Z7PneHZ2TEG7q88+RVVLSzCKBvNcoew8/Rb/mUMl1r8/FdRJw2p4gGTfZylxvbLX+GfRNqoepj9t09T/oBLzrrfp+r8z2fmbZZ5r2oy6O/zlRLM5aMnyIWyIfHrGGHqkhpX3yyT36Nd4xEkaqKdrD3sJPpxdnZiXo=";

    fn sample(lines: &str, totals: &str, signature: &str) -> String {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><HDon><DLHDon Id=\"d1\"><TTChung><PBan>2.0.1</PBan><THDon>Hóa đơn giá trị gia tăng</THDon><KHMSHDon>1</KHMSHDon><KHHDon>C26TAA</KHHDon><SHDon>123</SHDon><NLap>2026-11-03</NLap><DVTTe>VND</DVTTe></TTChung>\
            <NDHDon><NBan><Ten>CÔNG TY TNHH MINH ANH &amp; CON</Ten><MST>0101234567</MST><DChi>12 Lý Thường Kiệt, Hà Nội</DChi></NBan><NMua><Ten>CÔNG TY CỔ PHẦN SAO MAI</Ten><MST>0309876543</MST></NMua>\
            <DSHHDVu>{lines}</DSHHDVu><TToan>{totals}</TToan></NDHDon></DLHDon>{signature}</HDon>")
    }

    const LINES: &str = "<HHDVu><TChat>1</TChat><STT>1</STT><THHDVu>Giấy in A4</THHDVu><DVTinh>Ram</DVTinh><SLuong>100</SLuong><DGia>85000</DGia><ThTien>8500000</ThTien><TSuat>10%</TSuat></HHDVu>\
        <HHDVu><TChat>1</TChat><STT>2</STT><THHDVu>Mực máy in</THHDVu><DVTinh>Hộp</DVTinh><SLuong>4</SLuong><DGia>1050000.00</DGia><ThTien>4200000.00</ThTien><TSuat>10%</TSuat></HHDVu>\
        <HHDVu><TChat>3</TChat><STT>3</STT><THHDVu>Chiết khấu</THHDVu><ThTien>200000</ThTien><TSuat>10%</TSuat></HHDVu>\
        <HHDVu><TChat>4</TChat><THHDVu>Giao hàng tận nơi</THHDVu></HHDVu>";
    const TOTALS: &str = "<TgTCThue>12500000</TgTCThue><TgTThue>1250000</TgTThue><TgTTTBSo>13750000</TgTTTBSo><TgTTTBChu>Mười ba triệu bảy trăm năm mươi nghìn đồng</TgTTTBChu>";

    #[test]
    fn e_invoice() {
        let inv = einvoice_of(&sample(LINES, TOTALS, "")).unwrap();
        assert_eq!((inv.number.as_str(), inv.serial().as_str(), inv.date.as_str()), ("123", "1C26TAA", "2026-11-03"));
        assert_eq!((inv.seller.as_str(), inv.seller_tax.as_str(), inv.buyer_tax.as_str()), ("CÔNG TY TNHH MINH ANH & CON", "0101234567", "0309876543"));
        assert_eq!((inv.before, inv.vat, inv.total), (Some(12_500_000_000_000), Some(1_250_000_000_000), Some(13_750_000_000_000)));
        assert_eq!(inv.lines.len(), 4);
        assert_eq!(inv.lines[1][4..6], ["1050000".to_string(), "4200000".to_string()]);
        assert_eq!((inv.off_lines, inv.off_total, inv.adds_up()), (Some(0), Some(0), Some(true)));
        assert!(!inv.signed && inv.signature().starts_with("no digital signature"));

        let off = einvoice_of(&sample(LINES, &TOTALS.replace("13750000", "13751000"), "")).unwrap();
        assert_eq!((off.off_lines, off.off_total, off.adds_up()), (Some(0), Some(-1_000_000_000), Some(false)));
        assert!(off.check().contains("off by -1,000 from the total"), "{}", off.check());
        let short = einvoice_of(&sample(&LINES.replace("8500000", "8000000"), TOTALS, "")).unwrap();
        assert_eq!(short.off_lines, Some(-500_000_000_000));
        let bare = einvoice_of(&sample("", "", "")).unwrap();
        assert_eq!((bare.adds_up(), bare.check().as_str()), (None, "totals could not be checked"));

        // wrapped in a message envelope, signed by the seller, subject named
        let named = format!("<DSCKS><NBan><Signature><SignatureValue>AAAA</SignatureValue><KeyInfo><X509Data><X509SubjectName>C=VN, L=Hà Nội, CN=CÔNG TY TNHH MINH ANH, CHI NHÁNH 2, UID=MST:0101234567</X509SubjectName><X509Certificate>{CERT}</X509Certificate></X509Data></KeyInfo></Signature></NBan><CQT/></DSCKS>");
        let inv = einvoice_of(&format!("<TDiep><DLieu>{}</DLieu></TDiep>", sample(LINES, TOTALS, &named).replacen("<?xml version=\"1.0\" encoding=\"UTF-8\"?>", "", 1))).unwrap();
        assert!(inv.signed);
        assert_eq!(inv.signers, ["seller: CÔNG TY TNHH MINH ANH, CHI NHÁNH 2"]);
        assert!(inv.signature().ends_with("signature NOT verified"));
        // subject only inside the certificate
        let cert = format!("<DSCKS><NBan><Signature><SignatureValue>AAAA</SignatureValue><KeyInfo><X509Data><X509Certificate>\n{CERT}\n</X509Certificate></X509Data></KeyInfo></Signature></NBan></DSCKS>");
        assert_eq!(einvoice_of(&sample(LINES, TOTALS, &cert)).unwrap().signers, ["seller: CÔNG TY TNHH MINH ANH, MST:0101234567"]);
        // an unsigned draft keeps the empty block
        assert!(!einvoice_of(&sample(LINES, TOTALS, "<DSCKS><NBan/><NMua/></DSCKS>")).unwrap().signed);

        for bad in ["", "   ", "<a><b></a>", "<HDon><DLHDon>", "not xml at all", "<html><body>Hi</body></html>", "<HDon><DLHDon><TTChung/></DLHDon></HDon>"] {
            assert!(einvoice_of(bad).is_err(), "{bad}");
        }
        assert!(einvoice_of("<html><body>Hi</body></html>").unwrap_err().to_string().contains("isn't a Vietnamese e-invoice"));
        assert!(einvoice_of(&format!("{}x{}", "<a>".repeat(100), "</a>".repeat(100))).is_err());
    }

    #[test]
    fn certificates_never_panic() {
        use base64::Engine;
        let der = base64::engine::general_purpose::STANDARD.decode(CERT).unwrap();
        assert_eq!(cert_subject(&der).as_deref(), Some("CÔNG TY TNHH MINH ANH, MST:0101234567"));
        for cut in 0..der.len() {
            let _ = cert_subject(&der[..cut]);
        }
        assert_eq!(cert_subject(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]), None);
        assert_eq!(common_name("CN=\"A, B\", O=X"), "A, B");
        assert_eq!(common_name("O=No Name"), "O=No Name");
    }

    #[test]
    fn pairing() {
        let d = |n: i64| Some(day_number((2026, 10, n)));
        // equal amounts go to the nearest date; opposite signs still pair; four days apart is too far
        let a = [(500, d(1)), (500, d(5)), (-700, d(2)), (900, d(1)), (40, None)];
        let b = [(500, d(5)), (500, d(9)), (700, d(3)), (900, d(5)), (40, d(20)), (40, d(21))];
        let (ma, mb) = pair(&a, &b);
        assert_eq!(ma, [None, Some(0), Some(2), None, Some(4)]);
        assert_eq!(mb, [Some(1), None, Some(2), None, Some(4), None]);
        // a dated partner is preferred to an undated one
        let (ma, _) = pair(&[(10, d(2))], &[(10, None), (10, d(3))]);
        assert_eq!(ma, [Some(1)]);
        // very many of one amount: still one to one
        let many: Vec<(i128, Option<i64>)> = (0..600).map(|i| (7, d(1 + i % 28))).collect();
        let (ma, mb) = pair(&many, &many);
        assert!(ma.iter().all(Option::is_some) && mb.iter().all(Option::is_some));
        let mut seen: Vec<usize> = ma.iter().flatten().copied().collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 600);
    }

    #[test]
    fn column_roles() {
        assert_eq!(role("Số tiền"), Some(Role::Amount));
        assert_eq!(role("Ghi nợ"), Some(Role::Debit));
        assert_eq!(role("Có"), Some(Role::Credit));
        assert_eq!(role("Debit/Credit"), Some(Role::Amount));
        assert_eq!(role("Ngày giao dịch"), Some(Role::Date));
        assert_eq!(role("Transaction Date"), Some(Role::Date));
        assert_eq!(role("Số dư"), None);
        assert_eq!(role("Balance"), None);
        assert_eq!(role("Diễn giải"), None);
    }

    #[test]
    fn tidying() {
        let row = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let t = vec![row(&[" Tên ", "", "Số  tiền"]), row(&["An", "", "1"]), row(&["", " ", ""]), row(&["An", "", "1", ""]), row(&["Bình\u{a0} Minh", "", "2"])];
        let (out, counts) = tidied(t);
        assert_eq!(out, vec![row(&["Tên", "Số tiền"]), row(&["An", "1"]), row(&["Bình Minh", "2"])]);
        assert_eq!(counts, [4, 1, 2, 1]);
        let clean = vec![row(&["a", "b"]), row(&["1", "line one\nline two"])];
        assert_eq!(tidied(clean.clone()), (clean, [0, 0, 0, 0]));
        assert_eq!(safe_name("../../etc/passwd"), "-..-etc-passwd");
        assert_eq!(safe_name(" .hidden: a/b\\c\n"), "hidden- a-b-c");
        assert_eq!(safe_name("  "), "(blank)");
        assert_eq!(safe_name("..."), "(blank)");
        assert!(safe_name(&"Nguyễn ".repeat(60)).len() <= 120);
        assert_eq!(uninert("'=SUM(A1)"), "=SUM(A1)");
        assert_eq!(uninert("'quoted"), "'quoted");
    }

    #[test]
    fn placeholders_across_runs() {
        let values: HashMap<String, &str> = [("họ tên".to_string(), "Nguyễn <An> & Co"), ("số tiền".to_string(), "1.200.000"), ("ghi chú".to_string(), " dòng 1\ndòng 2")].into();
        // as Word writes it: the braces, the name and the closing braces in three runs
        let xml = "<w:p><w:r><w:t>Kính gửi {{</w:t></w:r><w:proofErr w:type=\"spellStart\"/><w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\"> Họ  Tên </w:t></w:r><w:r><w:t>}}, số {</w:t></w:r><w:r><w:t>{SỐ TIỀN}} đ</w:t></w:r></w:p><w:p><w:r><w:t>{{Ghi chú}} {{Không có}} {{</w:t></w:r></w:p><w:p><w:r><w:tab/><w:t>}} hết</w:t></w:r></w:p>";
        let segs = template(xml);
        let out = fill(&segs, &values);
        assert_eq!(out, "<w:p><w:r><w:t xml:space=\"preserve\">Kính gửi Nguyễn &lt;An&gt; &amp; Co</w:t></w:r><w:proofErr w:type=\"spellStart\"/><w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\"></w:t></w:r><w:r><w:t xml:space=\"preserve\">, số 1.200.000</w:t></w:r><w:r><w:t> đ</w:t></w:r></w:p><w:p><w:r><w:t xml:space=\"preserve\"> dòng 1</w:t><w:br/><w:t xml:space=\"preserve\">dòng 2 {{Không có}} {{</w:t></w:r></w:p><w:p><w:r><w:tab/><w:t>}} hết</w:t></w:r></w:p>");
        // the result is still well-formed
        let mut r = quick_xml::Reader::from_str(&out);
        loop {
            match r.read_event().unwrap() {
                Event::Eof => break,
                _ => continue,
            }
        }
        assert_eq!(segs.iter().filter(|s| matches!(s, Seg::Slot { .. })).count(), 4);
        // nothing to fill, nothing changed
        let same = "<w:p><w:r><w:t>{ not } {{}} one</w:t></w:r><w:tbl/><w:t/></w:p>";
        assert_eq!(template(same), vec![Seg::Text(same.to_string())]);
        assert_eq!(slots("{{{A}} {{B&amp;C}}").iter().map(|s| s.2).collect::<Vec<_>>(), ["A", "B&amp;C"]);
        for broken in ["<w:t", "<w:t>{{a", "<w:t>{{a}}", "</w:t><w:t>", "<w:t>}}{{</w:t></w:p><w:t>x}}</w:t>"] {
            let _ = fill(&template(broken), &values);
        }
    }

    #[test]
    fn statement_lines() {
        let text = "NGÂN HÀNG TMCP\nSao kê tài khoản 0011002233\nNgày Diễn giải Ghi nợ Ghi có Số dư\n01/10/2026 Chuyển khoản cho Minh Anh 1.200.000 8.800.000\ntiếp theo dòng trên\n02/10/2026  Nộp tiền mặt    2.500.000    11.300.000\nTrang 1/1";
        let rows = statement_rows(text);
        assert_eq!(rows, vec![vec!["2026-10-01", "Chuyển khoản cho Minh Anh", "1200000", "8800000"], vec!["2026-10-02", "Nộp tiền mặt", "2500000", "11300000"]]);
        assert!(statement_rows("no dates here\n12.000").is_empty());
    }
}
