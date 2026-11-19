// Rotae from a terminal or a script: the same actions and the same engine as the wheel.
//   rotae list <file>...                      what can be done to these files
//   rotae <action> [--set key=value] <file>...   do it; results are printed one per line
import Foundation
import SDCore

func die(_ m: String, _ code: Int32 = 2) -> Never { FileHandle.standardError.write(Data((m + "\n").utf8)); exit(code) }

var args = Array(CommandLine.arguments.dropFirst())
guard let verb = args.first, verb != "-h", verb != "--help" else {
    print("usage: rotae list <file>...\n       rotae <action> [--set key=value]... <file>...\n\nActions are named as `rotae list` prints them, e.g. convert/png, compress/strong, totals.")
    exit(0)
}
args.removeFirst()
var sets: [String: String] = [:], files: [String] = []
var i = 0
while i < args.count {
    if args[i] == "--set", i + 1 < args.count, let eq = args[i + 1].firstIndex(of: "=") {
        sets[String(args[i + 1][..<eq])] = String(args[i + 1][args[i + 1].index(after: eq)...]); i += 2
    } else { files.append(URL(fileURLWithPath: args[i]).standardizedFileURL.path); i += 1 }
}
guard !files.isEmpty else { die("no files given") }
for f in files where !FileManager.default.fileExists(atPath: f) { die("no such file: \(f)") }

// what is next to this program is what the app ships; nothing is looked up on PATH
let here = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath().deletingLastPathComponent()
let engine = here.appendingPathComponent("sd-engine")
let tools: Set<String> = Set(["ffmpeg", "qpdf", "whisper-cli", "upx"].filter { n in ["/opt/homebrew/bin", "/usr/local/bin"].contains { FileManager.default.isExecutableFile(atPath: "\($0)/\(n)") } }.map { $0 == "whisper-cli" ? "whisper" : $0 }).union(["ondevice"])

/// Every action with the path that names it: "convert/png", "excel/workbook .xlsx".
func named(_ items: [MenuItem], _ prefix: String = "") -> [(String, MenuItem)] {
    items.flatMap { it -> [(String, MenuItem)] in
        let n = prefix + it.label.lowercased()
        return it.sub.isEmpty ? [(n, it)] : named(it.sub, n + "/")
    }
}
let menu = buildMenu(files, apps: [], tools: tools)
var seen = Set<String>()
let all = named(menu.items + menu.extra).filter { $0.1.action != nil && seen.insert($0.0).inserted }

if verb == "list" {
    for (n, it) in all { print(it.ask.isEmpty ? n : "\(n)  (--set \(it.ask.map { "\($0.key)=…" }.joined(separator: " --set ")))") }
    exit(0)
}
let want = verb.lowercased()
let hits = all.filter { $0.0 == want }.isEmpty ? all.filter { $0.0.hasSuffix("/" + want) || $0.1.id == want } : all.filter { $0.0 == want }
guard let (name, item) = hits.first else { die("no action called \(verb) for these files; try: rotae list <file>") }
if hits.count > 1 { die("\(verb) could be: \(hits.map(\.0).joined(separator: ", "))") }
guard var action = item.action else { die("\(name) only works in the app") }
for f in item.ask { guard let v = sets[f.key] else { die("\(name) needs --set \(f.key)=…") }; action[f.key] = f.kind == "number" ? .n(Double(v) ?? 0) : .s(v) }
if usesNetwork(action) { die("\(name) uses an online service: run it from the app, where its key is kept") }

var failed = false
for unit in isGroup(action) ? [files] : files.map({ [$0] }) {
    let p = Process(), inp = Pipe(), out = Pipe()
    let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("rotae-\(UUID().uuidString)", isDirectory: true)
    try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: tmp) }
    p.executableURL = engine
    p.environment = ["PATH": "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin", "HOME": NSHomeDirectory(), "TMPDIR": tmp.path + "/", "LANG": "en_US.UTF-8"]
    p.standardInput = inp; p.standardOutput = out; p.standardError = FileHandle.nullDevice
    do { try p.run() } catch { die("the engine is missing: \(engine.path)", 3) }
    var req: [String: Any] = ["v": 1, "paths": unit, "binDir": here.deletingLastPathComponent().appendingPathComponent("Frameworks").path, "unattended": false, "secrets": [String: String](), "settings": [String: String]()]
    req["action"] = try? JSONSerialization.jsonObject(with: JSONEncoder().encode(action))
    if var d = try? JSONSerialization.data(withJSONObject: req) { d.append(10); try? inp.fileHandleForWriting.write(contentsOf: d) }
    let data = out.fileHandleForReading.readDataToEndOfFile()
    try? inp.fileHandleForWriting.close()
    p.waitUntilExit()
    var ended = false
    for line in data.split(separator: 10) {
        guard let e = EngineEvent.parse(Data(line)) else { continue }
        switch e.event {
        case "note": if let t = e.text { FileHandle.standardError.write(Data((t + "\n").utf8)) }
        case "clipboard": if let t = e.text, (e.outputs ?? []).isEmpty { print(t) }
        case "done": ended = true; (e.notes ?? []).forEach { FileHandle.standardError.write(Data(($0 + "\n").utf8)) }; (e.outputs ?? []).forEach { print($0) }
        case "error": ended = true; failed = true; FileHandle.standardError.write(Data("\(unit.map { ($0 as NSString).lastPathComponent }.joined(separator: ", ")): \(e.detail ?? e.code ?? "failed")\n".utf8))
        default: break
        }
    }
    if !ended { failed = true; FileHandle.standardError.write(Data("the engine stopped without an answer\n".utf8)) }
}
exit(failed ? 1 : 0)
