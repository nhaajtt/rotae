import Foundation

/// Actions travel to the engine as JSON, exactly the shape of its serde `Action` enum.
public enum JSON: Codable, Hashable {
    case s(String), n(Double), b(Bool), a([JSON]), o([String: JSON])

    public init(from d: Decoder) throws {
        let c = try d.singleValueContainer()
        if let v = try? c.decode(Bool.self) { self = .b(v) }
        else if let v = try? c.decode(Double.self) { self = .n(v) }
        else if let v = try? c.decode(String.self) { self = .s(v) }
        else if let v = try? c.decode([JSON].self) { self = .a(v) }
        else { self = .o(try c.decode([String: JSON].self)) }
    }
    public func encode(to e: Encoder) throws {
        var c = e.singleValueContainer()
        switch self {
        case .s(let v): try c.encode(v)
        case .n(let v): try c.encode(v)
        case .b(let v): try c.encode(v)
        case .a(let v): try c.encode(v)
        case .o(let v): try c.encode(v)
        }
    }
    public var string: String? { if case .s(let v) = self { return v }; return nil }
}

public typealias Action = [String: JSON]
public func act(_ op: String, _ rest: [String: JSON] = [:]) -> Action { rest.merging(["op": .s(op)]) { a, _ in a } }

public enum FileKind: String, Codable, CaseIterable, Identifiable {
    case image, video, audio, pdf, text, office, archive, exe, pcap, other, mixed
    public var id: String { rawValue }
    public var title: String {
        switch self {
        case .image: return "Images"; case .video: return "Video"; case .audio: return "Audio"; case .pdf: return "PDF"; case .text: return "Text"
        case .office: return "Office"; case .archive: return "Archives"; case .exe: return "Programs"; case .pcap: return "Captures"; case .other: return "Other"; case .mixed: return "Mixed"
        }
    }
}

let EXT: [(FileKind, Set<String>)] = [
    (.image, ["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "ico", "avif", "heic", "heif", "jxl", "jxr", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf"]),
    (.video, ["mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "flv", "mts", "m2ts", "3gp"]),
    (.audio, ["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "wma", "aiff", "aif"]),
    (.pdf, ["pdf"]),
    (.text, ["txt", "md", "markdown", "csv", "tsv", "json", "log", "xml", "html", "htm", "yaml", "yml", "ini", "cfg", "conf", "toml", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "rs", "c", "h", "cpp", "hpp", "cc", "cs", "java", "kt", "go", "rb", "php", "sh", "bash", "ps1", "bat", "cmd", "sql", "css", "scss", "lua", "swift", "dart", "vue", "svelte"]),
    (.archive, ["zip", "tar", "gz", "tgz", "7z", "rar", "bz2", "xz", "zst"]),
    (.exe, ["exe", "dll", "sys", "ocx", "scr", "cpl", "drv", "efi", "so", "elf"]),
    (.pcap, ["pcap", "pcapng", "cap"]),
    (.office, ["doc", "docx", "docm", "odt", "rtf", "xls", "xlsx", "xlsm", "ods", "ppt", "pptx", "odp"]),
]

public func extOf(_ p: String) -> String { (p as NSString).pathExtension.lowercased() }
public func kindOf(_ p: String) -> FileKind {
    var dir: ObjCBool = false
    if FileManager.default.fileExists(atPath: p, isDirectory: &dir), dir.boolValue { return .other }
    let e = extOf(p)
    return EXT.first { $0.1.contains(e) }?.0 ?? .other
}
public func menuKind(_ paths: [String]) -> FileKind {
    let k = Set(paths.map(kindOf))
    return k.count == 1 ? k.first! : .mixed
}

/// Must agree with the engine's `is_group`: these see the whole selection as one unit.
public func isGroup(_ a: Action) -> Bool {
    switch a["op"]?.string ?? "" {
    case "zip", "hash", "copy_path", "copy_text", "merge", "diff", "shred", "slideshow", "contact_sheet", "copy_path_as": return true
    case "hashes": return a["algo"]?.string != "all"
    // a recipe that begins by joining files takes them all at once
    case "chain": if case .a(let steps)? = a["steps"], case .o(let first)? = steps.first { return isGroup(first) }; return false
    case "native":
        let v = a["verb"]?.string ?? "", g = a["arg"]?.string ?? ""
        return (v == "sheet" && g == "merge") || (v == "ledger" && ["einvoice", "invoice", "reconcile", "mailmerge"].contains(g)) || (v == "paper" && g == "compare")
    default: return false
    }
}

/// Whether this reaches beyond the Mac: what "offline" turns off.
public func usesNetwork(_ a: Action) -> Bool {
    if ["virus_total", "ask_claude"].contains(a["op"]?.string ?? "") { return true }
    if case .a(let steps)? = a["steps"] { return steps.contains { if case .o(let s) = $0 { return usesNetwork(s) }; return false } }
    return false
}
public func usesNetwork(_ i: MenuItem) -> Bool { i.share != nil || (i.action.map { usesNetwork($0) } ?? false) }

public enum ResourceClass: Hashable { case cpu, media, network, exclusive(String) }
public func resourceClass(_ a: Action, paths: [String]) -> ResourceClass {
    let op = a["op"]?.string ?? ""
    if op == "custom" || op == "chain" { return .exclusive(((paths.first ?? "/") as NSString).deletingLastPathComponent) }
    if ["virus_total", "ask_claude"].contains(op) { return .network }
    return paths.contains { [.video, .audio].contains(kindOf($0)) } ? .media : .cpu
}

public func unattendedAllowed(_ a: Action) -> Bool {
    let ok: Set<String> = ["convert", "compress", "resize", "rotate", "strip_metadata", "extract_audio", "mute", "crop", "add_bg", "trim", "format_json", "zip", "hash", "hashes", "identify", "target_size", "speed", "flip", "thumbnail", "audio_fix", "ocr", "qr_read", "icon_set", "palette", "watermark", "vectorize", "remove_bg", "blur_faces", "upscale", "page_numbers", "pdf_text", "extract_images", "native"]
    let op = a["op"]?.string ?? ""
    if op == "chain", case .a(let steps)? = a["steps"] {
        return steps.allSatisfy { if case .o(let s) = $0 { return unattendedAllowed(s) }; return false }
    }
    return ok.contains(op)
}

/// One helper event, as written by the engine.
public struct EngineEvent: Decodable {
    public let event: String
    public var pid: Int32?
    public var path: String?
    public var f: Double?
    public var text: String?
    public var outputs: [String]?
    public var notes: [String]?
    public var links: [String]?
    public var code: String?
    public var detail: String?
    public var lan: LanShare?
    public struct LanShare: Decodable { public let url: String; public let qr: String; public let minutes: Int }
    public static func parse(_ line: Data) -> EngineEvent? { try? JSONDecoder().decode(EngineEvent.self, from: line) }
}

// MARK: Wheel geometry

public enum WheelHit: Equatable { case none, center, wedge(Int), sub(Int, Int), leaf(Int, Int, Int) }

/// Three rings: purpose, action, variant. Everything is reachable without leaving the drag.
public struct WheelGeometry {
    public static let core = 54.0, r1 = 150.0, r2In = 156.0, r2 = 222.0, r3In = 228.0, r3 = 294.0
    public static func mid(_ i: Int, of n: Int) -> Double { Double(i) * 360 / Double(n) }
    public static func cellWidth(_ count: Int) -> Double { min(36, 340 / Double(max(count, 1))) }
    public static func leafWidth(_ count: Int) -> Double { min(27, 340 / Double(max(count, 1))) }
    public static func subMid(_ j: Int, count: Int, wedge: Int, of n: Int) -> Double {
        mid(wedge, of: n) + (Double(j) - Double(count - 1) / 2) * cellWidth(count)
    }
    public static func leafMid(_ k: Int, count: Int, center: Double) -> Double { center + (Double(k) - Double(count - 1) / 2) * leafWidth(count) }
    static func diff(_ a: Double, _ b: Double) -> Double { abs((a - b + 540).truncatingRemainder(dividingBy: 360) - 180) }

    /// `dx` right, `dy` up from the centre. `open`/`openCell` say which outer rings are showing.
    public static func hit(dx: Double, dy: Double, count n: Int, open: Int?, subCount: Int, openCell: Int? = nil, leafCount: Int = 0) -> WheelHit {
        let r = (dx * dx + dy * dy).squareRoot()
        if r <= core { return .center }
        if n == 0 { return .none }
        var ang = atan2(dx, dy) * 180 / .pi
        if ang < 0 { ang += 360 }
        let wedge = Int((ang + 180 / Double(n)) / (360 / Double(n))) % n
        if r <= r1 { return .wedge(wedge) }
        if r > r2, r <= r3, let w = open, let c = openCell, leafCount > 0 {
            let center = subMid(c, count: subCount, wedge: w, of: n), width = leafWidth(leafCount)
            for k in 0..<leafCount where diff(ang, leafMid(k, count: leafCount, center: center)) <= width / 2 { return .leaf(w, c, k) }
        }
        if r <= r2, let w = open, subCount > 0 {
            let width = cellWidth(subCount)
            for j in 0..<subCount where diff(ang, subMid(j, count: subCount, wedge: w, of: n)) <= width / 2 { return .sub(w, j) }
        }
        return r <= r2In ? .wedge(wedge) : .none
    }
}

/// How much an action has been used, fading with a 30-day half-life.
public struct Usage: Codable, Equatable {
    public var score = 0.0
    public var at = 0.0
    public init() {}
    public func value(now: Double) -> Double { score * pow(0.5, max(0, now - at) / (30 * 86400)) }
    public mutating func bump(now: Double) { score = value(now: now) + 1; at = now }
}

public func applyRename(name: String, n: Int, pattern: String, find: String, replace: String, regex: Bool, caseMode: String, date: Date) -> String {
    let ext = (name as NSString).pathExtension
    var stem = (name as NSString).deletingPathExtension
    if !find.isEmpty {
        if regex, let r = try? NSRegularExpression(pattern: find) {
            stem = r.stringByReplacingMatches(in: stem, range: NSRange(stem.startIndex..., in: stem), withTemplate: replace)
        } else { stem = stem.replacingOccurrences(of: find, with: replace) }
    }
    let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd"; f.timeZone = .current
    var out = (pattern.isEmpty ? "{name}" : pattern).replacingOccurrences(of: "{name}", with: stem).replacingOccurrences(of: "{n}", with: String(n)).replacingOccurrences(of: "{date}", with: f.string(from: date))
    switch caseMode {
    case "lower": out = out.lowercased()
    case "upper": out = out.uppercased()
    case "title": out = out.capitalized
    case "slug":
        out = out.folding(options: .diacriticInsensitive, locale: .current).lowercased().components(separatedBy: CharacterSet.alphanumerics.inverted).filter { !$0.isEmpty }.joined(separator: "-")
    default: break
    }
    return ext.isEmpty ? out : out + "." + ext
}
