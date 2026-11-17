import AppKit
import SDCore
import Security
import SwiftUI

/// Interface text. English is the key; vi.lproj comes from the Windows translation table.
private let viBundle: Bundle? = Bundle.main.path(forResource: "vi", ofType: "lproj").flatMap { Bundle(path: $0) }
private let systemIsVietnamese = Locale.preferredLanguages.first?.hasPrefix("vi") == true
/// Set by debug tooling to see another language without changing the saved setting.
var forcedLang: String?
func T(_ s: String) -> String {
    let lang = forcedLang ?? Store.shared.state.lang
    guard lang == "vi" || (lang == "system" && systemIsVietnamese), let b = viBundle else { return s }
    let t = b.localizedString(forKey: s, value: s, table: nil)
    // "Images · 3": the count is not part of the name
    if t == s, let r = s.range(of: " · ") { return T(String(s[..<r.lowerBound])) + s[r.lowerBound...] }
    return t
}
/// A label made of parts, like "Convert › JPG": each part is translated on its own.
func TL(_ s: String) -> String { s.components(separatedBy: " › ").map(T).joined(separator: " › ") }

struct Watch: Codable, Hashable, Identifiable { var id = UUID(); var folder: String; var exts: String; var title: String; var action: Action; /** Only files at least this big. */ var minMB: Double? = nil }

struct AppState: Codable {
    var firstRun = true
    var enabled = true
    var digits = false
    var accent = "violet"
    var theme = "system"
    var lang = "system"
    var prefs = WheelPrefs()
    var recipes: [Recipe] = []
    var commands: [UserCommand] = []
    var watches: [Watch] = []
    var shelf: [String] = []
    /// Nothing that reaches the network is offered or run.
    var offline = false
    /// The centre of the wheel repeats what was last done to that kind of file.
    var repeatLast = false
    var last: [String: String] = [:]
    /// How often one action was run on what another had just made, and what came of it.
    var habits: [String: Int] = [:]
    var suggested: [Recipe] = []
    var dismissed: [String] = []
    var r2Account = "", r2Bucket = "", r2Public = "", yaraRules = ""

    init() {}
    init(from d: Decoder) throws {
        // a field added in a later version must not reset everything else
        let c = try d.container(keyedBy: K.self); var s = AppState()
        func g<T: Decodable>(_ k: K, _ p: WritableKeyPath<AppState, T>) { if let v = try? c.decode(T.self, forKey: k) { s[keyPath: p] = v } }
        g(.firstRun, \.firstRun); g(.enabled, \.enabled); g(.digits, \.digits); g(.accent, \.accent); g(.theme, \.theme); g(.lang, \.lang); g(.prefs, \.prefs)
        g(.recipes, \.recipes); g(.commands, \.commands); g(.watches, \.watches); g(.shelf, \.shelf); g(.r2Account, \.r2Account); g(.r2Bucket, \.r2Bucket); g(.r2Public, \.r2Public); g(.yaraRules, \.yaraRules); g(.offline, \.offline); g(.repeatLast, \.repeatLast); g(.last, \.last); g(.habits, \.habits); g(.suggested, \.suggested); g(.dismissed, \.dismissed)
        self = s
    }
    enum K: String, CodingKey { case firstRun, enabled, digits, accent, theme, lang, prefs, recipes, commands, watches, shelf, r2Account, r2Bucket, r2Public, yaraRules, offline, repeatLast, last, habits, suggested, dismissed }
}

let accents: [String: Color] = ["violet": Color(red: 0.42, green: 0.40, blue: 0.96), "gold": Color(red: 0.80, green: 0.60, blue: 0.20), "graphite": .gray, "noir": .primary, "oxblood": Color(red: 0.55, green: 0.13, blue: 0.18), "navy": Color(red: 0.13, green: 0.25, blue: 0.55), "racing": Color(red: 0.05, green: 0.42, blue: 0.27)]

let supportDir: URL = {
    let fm = FileManager.default, base = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    let u = base.appendingPathComponent("Rotae", isDirectory: true)
    // The app was called ShiftDrop before: carry its settings, recipes and add-ons over once
    let old = base.appendingPathComponent("ShiftDrop", isDirectory: true)
    if !fm.fileExists(atPath: u.path), fm.fileExists(atPath: old.path) { try? fm.moveItem(at: old, to: u) }
    try? fm.createDirectory(at: u, withIntermediateDirectories: true)
    return u
}()

/// Atomic, and readable only by this user: these files name the user's documents.
func writePrivate(_ d: Data, to u: URL) {
    try? d.write(to: u, options: .atomic)
    try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: u.path)
}

final class Store: ObservableObject {
    static let shared = Store()
    @Published private(set) var state = AppState()
    private let file = supportDir.appendingPathComponent("state.json")
    private var refuseSaves = false

    init() {
        guard FileManager.default.fileExists(atPath: file.path) else { return }
        do { state = try JSONDecoder().decode(AppState.self, from: Data(contentsOf: file)) }
        catch {
            // keep the damaged file for recovery; starting clean must not destroy it
            try? FileManager.default.copyItem(at: file, to: file.appendingPathExtension("bad"))
            NSLog("Rotae: state unreadable, using defaults")
        }
    }
    func update(_ f: (inout AppState) -> Void) {
        f(&state)
        if refuseSaves { return }
        if let d = try? JSONEncoder().encode(state) { writePrivate(d, to: file) }
        applyTheme()
    }
    func applyTheme() {
        NSApp.appearance = state.theme == "light" ? NSAppearance(named: .aqua) : state.theme == "dark" ? NSAppearance(named: .darkAqua) : nil
    }
    var accent: Color { accents[state.accent] ?? .accentColor }
    func noteUse(_ kind: FileKind, _ id: String) {
        update { var u = $0.prefs.usage["\(kind.rawValue):\(id)"] ?? Usage(); u.bump(now: Date().timeIntervalSince1970); $0.prefs.usage["\(kind.rawValue):\(id)"] = u }
    }
    var settingsForEngine: [String: String] { ["yara_rules": state.yaraRules, "r2_account": state.r2Account, "r2_bucket": state.r2Bucket, "r2_public": state.r2Public] }
}

/// Keys and the saved signature live in the login keychain, never in the state file.
enum Keychain {
    static let service = "app.rotae.mac"
    /// Where keys were kept while the app was called ShiftDrop
    static let oldService = "app.shiftdrop.mac"
    static func get(_ k: String) -> String {
        let v = read(k, service)
        if !v.isEmpty { return v }
        // moved over the first time it is asked for
        let old = read(k, oldService)
        if !old.isEmpty, set(k, old) {
            SecItemDelete([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: oldService, kSecAttrAccount as String: k] as CFDictionary)
        }
        return old
    }
    private static func read(_ k: String, _ svc: String) -> String {
        var out: CFTypeRef?
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: svc, kSecAttrAccount as String: k, kSecReturnData as String: true]
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let d = out as? Data else { return "" }
        return String(data: d, encoding: .utf8) ?? ""
    }
    @discardableResult static func set(_ k: String, _ v: String) -> Bool {
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: k]
        SecItemDelete(q as CFDictionary)
        if v.isEmpty {
            // also the copy from the ShiftDrop days, or get() would bring it back
            SecItemDelete([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: oldService, kSecAttrAccount as String: k] as CFDictionary)
            return true
        }
        var add = q; add[kSecValueData as String] = Data(v.utf8)
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }
    static let secretNames = ["vt_key", "anthropic_key", "r2_key", "r2_secret"]
    static var all: [String: String] { Dictionary(uniqueKeysWithValues: secretNames.map { ($0, get($0)) }) }
}

/// External programs: bundled ones, then the add-on folder, then what the user has installed.
enum Tools {
    static var searchPath: String {
        let h = NSHomeDirectory()
        // system folders first: nothing the user can write may stand in for curl, tar or xattr
        return ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/opt/homebrew/bin", "/usr/local/bin", "\(h)/.local/bin"].joined(separator: ":")
    }
    static func find(_ name: String) -> String? {
        searchPath.split(separator: ":").map { "\($0)/\(name)" }.first { FileManager.default.isExecutableFile(atPath: $0) }
    }
    static let addons: [(id: String, bin: String, label: String, why: String)] = [
        ("ffmpeg", "ffmpeg", "FFmpeg", "Needed for every video and audio action"),
        ("qpdf", "qpdf", "qpdf", "PDF passwords: protect and unlock"),
        ("whisper", "whisper-cli", "whisper.cpp", "Subtitles from speech, offline"),
        ("upscale", "realesrgan-ncnn-vulkan", "Real-ESRGAN", "Sharp 2× and 4× enlargements"),
        ("capa", "capa", "capa", "What a program can do (Mandiant)"),
        ("floss", "floss", "FLOSS", "Strings hidden or built at run time (Mandiant)"),
        ("yara", "yr", "YARA-X", "Scan programs with your own YARA rules"),
        ("upx", "upx", "UPX", "Unpacks programs compressed with UPX"),
        ("libreoffice", "soffice", "LibreOffice", "Word, Excel and PowerPoint conversions"),
    ]
    static func installed(_ a: (id: String, bin: String, label: String, why: String)) -> Bool {
        if a.id == "libreoffice" { return FileManager.default.fileExists(atPath: "/Applications/LibreOffice.app") }
        return find(a.bin) != nil
    }
    static var installedIDs: Set<String> { Set(addons.filter(installed).map(\.id)).union(OnDevice.ready ? ["ondevice"] : []) }
    static var apps: [AppEntry] { knownApps.filter { NSWorkspace.shared.urlForApplication(withBundleIdentifier: $0.bundle) != nil } }
}
