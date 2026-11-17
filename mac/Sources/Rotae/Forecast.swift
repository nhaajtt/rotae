import AppKit
import PDFKit
import SDCore

/// What an action would make of the file, worked out while the pointer rests on its cell:
/// "4.2 MB → 610 KB". The work is done on a copy in a private folder that is removed again,
/// so looking never leaves anything next to the original.
final class Forecast {
    static let shared = Forecast()
    private var cache: [String: String] = [:]
    private var running: Process?
    private var pending: DispatchWorkItem?
    private var turn = 0

    /// Only quick, repeatable work on one modest file is worth previewing.
    static func worth(_ a: Action, path: String) -> Bool {
        guard !ProcessInfo.processInfo.isLowPowerModeEnabled, let s = fileStamp(path), s.0 > 0 else { return false }
        let op = a["op"]?.string ?? "", k = kindOf(path)
        if k == .image, s.0 <= 40 << 20 { return ["compress", "resize", "convert"].contains(op) && a["to"]?.string != "pdf" || (op == "native" && a["verb"]?.string == "fit") }
        if k == .pdf, s.0 <= 25 << 20 { return op == "compress" }
        return false
    }
    func cancel() {
        turn += 1
        pending?.cancel(); pending = nil
        if let p = running, p.isRunning { kill(p.processIdentifier, SIGTERM) }
        running = nil
    }
    func ask(_ a: Action, path: String, _ done: @escaping (String?) -> Void) {
        cancel()
        guard Self.worth(a, path: path), let stamp = fileStamp(path), let json = try? JSONEncoder().encode(a) else { done(nil); return }
        let key = "\(path)|\(stamp.0)|\(stamp.1)|\(String(decoding: json, as: UTF8.self))"
        if let c = cache[key] { done(c); return }
        let mine = turn
        // a pointer passing over a cell is not a question; one that rests there is
        let w = DispatchWorkItem { [weak self] in self?.run(a, path, stamp.0, key, mine, done) }
        pending = w
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.28, execute: w)
    }
    private func run(_ a: Action, _ path: String, _ size: Int64, _ key: String, _ mine: Int, _ done: @escaping (String?) -> Void) {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("rotae-look-\(UUID().uuidString)", isDirectory: true)
        let copy = dir.appendingPathComponent((path as NSString).lastPathComponent)
        do {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try fm.copyItem(atPath: path, toPath: copy.path) // a clone on APFS: no bytes move
        } catch { try? fm.removeItem(at: dir); done(nil); return }
        let p = Process(), inp = Pipe(), out = Pipe()
        p.executableURL = Jobs.shared.engine()
        p.environment = ["PATH": Tools.searchPath, "HOME": NSHomeDirectory(), "TMPDIR": dir.path + "/", "LANG": "en_US.UTF-8"]
        p.standardInput = inp; p.standardOutput = out; p.standardError = FileHandle.nullDevice
        p.qualityOfService = .utility
        guard (try? p.run()) != nil else { try? fm.removeItem(at: dir); done(nil); return }
        running = p
        var req: [String: Any] = ["v": 1, "paths": [copy.path], "binDir": Jobs.shared.binDir, "unattended": true, "secrets": [String: String](), "settings": [String: String]()]
        req["action"] = try? JSONSerialization.jsonObject(with: JSONEncoder().encode(a))
        if var d = try? JSONSerialization.data(withJSONObject: req) { d.append(10); try? inp.fileHandleForWriting.write(contentsOf: d) }
        DispatchQueue.global(qos: .utility).async {
            let data = out.fileHandleForReading.readDataToEndOfFile()
            try? inp.fileHandleForWriting.close()
            p.waitUntilExit()
            var made: Int64?
            for l in data.split(separator: 10) {
                if let e = EngineEvent.parse(Data(l)), e.event == "done", let outs = e.outputs, !outs.isEmpty {
                    made = outs.reduce(Int64(0)) { $0 + (fileStamp($1)?.0 ?? 0) }
                }
            }
            try? fm.removeItem(at: dir)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if running === p { running = nil }
                guard let made, made > 0 else { if mine == turn { done(nil) }; return }
                let f = ByteCountFormatter(); f.countStyle = .file
                let text = "\(f.string(fromByteCount: size)) → \(f.string(fromByteCount: made))"
                cache[key] = text
                if cache.count > 200 { cache.removeAll() }
                if mine == turn { done(text) }
            }
        }
    }

    /// A PDF that is only pictures of pages: the useful first step is to read it.
    static func isScan(_ path: String) -> Bool {
        guard kindOf(path) == .pdf, let s = fileStamp(path), s.0 < 200 << 20, let d = PDFDocument(url: URL(fileURLWithPath: path)), !d.isLocked, d.pageCount > 0 else { return false }
        for i in 0..<min(3, d.pageCount) { if (d.page(at: i)?.string ?? "").filter({ !$0.isWhitespace }).count > 12 { return false } }
        return true
    }
}
