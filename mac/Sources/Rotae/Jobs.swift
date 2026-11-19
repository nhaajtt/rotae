import AppKit
import Foundation
import SDCore

enum JobState: String, Codable { case queued, running, done, partial, failed, cancelled }

final class Job: ObservableObject, Identifiable {
    let id = UUID()
    let title: String
    let paths: [String]
    let action: Action
    var unitsTotal = 1, unitsDone = 0
    @Published var state = JobState.queued
    @Published var progress = 0.0
    @Published var outputs: [String] = []
    @Published var notes: [String] = []
    @Published var errors: [(code: String, detail: String, file: String)] = []
    var units: [Unit] = []
    var isActive: Bool { state == .queued || state == .running }
    init(title: String, paths: [String], action: Action) { self.title = title; self.paths = paths; self.action = action }
}

final class Unit {
    unowned let job: Job
    let index: Int
    let paths: [String]
    let resource: ResourceClass
    var process: Process?
    var stdin: Pipe?
    var claims: [String] = []
    var buffer = Data()
    var finished = false
    var cancelled = false
    var eof = false
    var exited = false
    var f = 0.0
    var tmp: URL?
    init(job: Job, index: Int, paths: [String], resource: ResourceClass) { self.job = job; self.index = index; self.paths = paths; self.resource = resource }
}

struct Record: Codable, Identifiable {
    struct Output: Codable { var path: String; var size: Int64; var mtime: Double }
    var id = UUID()
    var title: String
    var date: Date
    var state: JobState
    var paths: [String]
    var outputs: [Output]
    var notes: [String]
    var error: String?
    var code: String?
    /// Dropped when it holds a password or a signature.
    var retry: Action?
    var undone = false
}

func fileStamp(_ p: String) -> (Int64, Double)? {
    guard let a = try? FileManager.default.attributesOfItem(atPath: p) else { return nil }
    return ((a[.size] as? NSNumber)?.int64Value ?? 0, (a[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0)
}

/// Keys only go to the work that uses them.
func needsSecrets(_ a: Action) -> Bool {
    if ["virus_total", "ask_claude"].contains(a["op"]?.string ?? "") { return true }
    if case .a(let s)? = a["steps"] { return s.contains { if case .o(let x) = $0 { return needsSecrets(x) }; return false } }
    return false
}

func actionIsSensitive(_ a: Action) -> Bool {
    if a["password"] != nil || a["png"] != nil { return true }
    if case .a(let s)? = a["steps"] { return s.contains { if case .o(let x) = $0 { return actionIsSensitive(x) }; return false } }
    return false
}

final class Jobs: ObservableObject {
    static let shared = Jobs()
    @Published var jobs: [Job] = []
    @Published var history: [Record] = []
    var onChange: (() -> Void)?
    var onFinished: ((Job) -> Void)?
    private var pending: [Unit] = []
    private var running: [Unit] = []
    private let file = supportDir.appendingPathComponent("history.json")
    private var cpuLimit: Int { let n = max(2, ProcessInfo.processInfo.activeProcessorCount / 2); return ProcessInfo.processInfo.thermalState.rawValue >= 2 ? 1 : n }

    init() {
        if let d = try? Data(contentsOf: file) {
            let dec = JSONDecoder(); dec.dateDecodingStrategy = .secondsSince1970
            history = (try? dec.decode([Record].self, from: d)) ?? []
        }
    }
    func saveHistory() {
        let e = JSONEncoder(); e.dateEncodingStrategy = .secondsSince1970
        if let d = try? e.encode(Array(history.prefix(200))) { writePrivate(d, to: file) }
    }
    var binDir: String { Bundle.main.privateFrameworksPath ?? "" }
    func engine() -> URL { Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/sd-engine") }

    @discardableResult
    func queue(_ paths: [String], _ action: Action, title: String, unattended: Bool = false) -> Job? {
        let op = action["op"]?.string ?? ""
        // folders (and .app bundles) only make sense for these
        let any = ["zip", "copy_path", "copy_path_as", "verify_signature"].contains(op) || (op == "native" && action["verb"]?.string == "kit" && ["project", "secrets", "pack"].contains(action["arg"]?.string ?? ""))
        let usable = paths.filter { p in
            var d: ObjCBool = false
            return FileManager.default.fileExists(atPath: p, isDirectory: &d) && (any || !d.boolValue)
        }
        guard !usable.isEmpty else { return nil }
        if Store.shared.state.offline, usesNetwork(action) { Toasts.shared.say(T("Offline mode is on: this needs the network"), ok: false); return nil }
        let job = Job(title: title, paths: usable, action: action)
        let lists = isGroup(action) ? [usable] : usable.map { [$0] }
        job.units = lists.enumerated().map { Unit(job: job, index: $0.offset, paths: $0.element, resource: resourceClass(action, paths: $0.element)) }
        job.unitsTotal = job.units.count
        self.unattended[ObjectIdentifier(job)] = unattended
        jobs.insert(job, at: 0)
        pending += job.units
        Toasts.shared.track(job)
        pump()
        return job
    }
    private var unattended: [ObjectIdentifier: Bool] = [:]

    private func limit(_ r: ResourceClass) -> Int {
        switch r { case .cpu: return cpuLimit; case .media: return 2; case .network: return 3; case .exclusive: return 1 }
    }
    private var pumping = false
    private func pump() {
        if pumping { return } // a unit that fails to start ends inside this loop
        pumping = true; defer { pumping = false }
        var i = 0
        while i < pending.count {
            let u = pending[i]
            if running.filter({ $0.resource == u.resource }).count < limit(u.resource) { pending.remove(at: i); start(u) } else { i += 1 }
        }
        onChange?()
    }

    private func start(_ u: Unit) {
        u.job.state = .running
        let p = Process(), inp = Pipe(), out = Pipe()
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("rotae-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        u.tmp = tmp
        p.executableURL = engine()
        p.environment = ["PATH": Tools.searchPath, "HOME": NSHomeDirectory(), "TMPDIR": tmp.path + "/", "LANG": "en_US.UTF-8"]
        p.standardInput = inp; p.standardOutput = out; p.standardError = FileHandle.nullDevice
        u.process = p; u.stdin = inp
        running.append(u)
        // over only when the process has exited AND every last event has been read
        out.fileHandleForReading.readabilityHandler = { [weak self] h in
            let d = h.availableData
            if d.isEmpty { h.readabilityHandler = nil }
            DispatchQueue.main.async {
                guard let self else { return }
                if d.isEmpty { u.eof = true; if u.exited { self.ended(u) } } else { self.read(u, d) }
            }
        }
        p.terminationHandler = { [weak self] pr in
            // stragglers in its group, if the helper itself was killed
            kill(-pr.processIdentifier, SIGKILL)
            DispatchQueue.main.async {
                guard let self else { return }
                u.exited = true
                if u.eof { self.ended(u) }
                // a straggler holding the pipe must not keep the job open
                else { DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.ended(u) } }
            }
        }
        do { try p.run() } catch {
            running.removeAll { $0 === u }
            fail(u, code: "crashed", detail: error.localizedDescription); return
        }
        let req: [String: Any] = ["v": 1, "paths": u.paths, "binDir": binDir, "unattended": unattended[ObjectIdentifier(u.job)] ?? false, "secrets": needsSecrets(u.job.action) ? Keychain.all : [:], "settings": Store.shared.settingsForEngine]
        var obj = req
        if let a = try? JSONSerialization.jsonObject(with: JSONEncoder().encode(u.job.action)) { obj["action"] = a }
        if var d = try? JSONSerialization.data(withJSONObject: obj) { d.append(10); try? inp.fileHandleForWriting.write(contentsOf: d) }
    }

    private func read(_ u: Unit, _ d: Data) {
        u.buffer.append(d)
        while let n = u.buffer.firstIndex(of: 10) {
            let line = u.buffer[u.buffer.startIndex..<n]
            u.buffer = Data(u.buffer[u.buffer.index(after: n)...])
            guard let e = EngineEvent.parse(Data(line)) else { continue }
            handle(u, e)
        }
    }

    private func handle(_ u: Unit, _ e: EngineEvent) {
        let j = u.job
        switch e.event {
        case "claim": if let p = e.path { u.claims.append(p) }
        case "progress":
            u.f = e.f ?? 0; updateProgress(j)
        case "clipboard":
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(e.text ?? "", forType: .string)
        case "done":
            u.finished = true; j.outputs += e.outputs ?? []; j.notes += e.notes ?? []
            u.claims = []
        case "error":
            u.finished = true
            removeClaims(u)
            j.errors.append((e.code ?? "unknown", e.detail ?? "", (u.paths.first as NSString?)?.lastPathComponent ?? ""))
        case "cancelled":
            u.finished = true; u.cancelled = true; removeClaims(u)
        default: break
        }
    }
    private func updateProgress(_ j: Job) {
        let inFlight = j.units.map { $0.finished ? 1.0 : $0.f }.reduce(0, +)
        j.progress = inFlight / Double(max(j.unitsTotal, 1))
        onChange?()
    }
    private func removeClaims(_ u: Unit) {
        for p in u.claims { try? FileManager.default.removeItem(atPath: p) }
        u.claims = []
    }
    private func fail(_ u: Unit, code: String, detail: String) {
        u.finished = true; removeClaims(u)
        u.job.errors.append((code, detail, (u.paths.first as NSString?)?.lastPathComponent ?? ""))
        unitEnded(u)
    }

    private func ended(_ u: Unit) {
        guard running.contains(where: { $0 === u }) else { return }
        running.removeAll { $0 === u }
        try? u.stdin?.fileHandleForWriting.close()
        if !u.finished { fail(u, code: "crashed", detail: "The work stopped unexpectedly. Nothing was changed."); return }
        unitEnded(u)
    }
    private func unitEnded(_ u: Unit) {
        if let t = u.tmp { try? FileManager.default.removeItem(at: t) }
        let j = u.job
        j.unitsDone += 1
        updateProgress(j)
        if j.unitsDone >= j.unitsTotal { finish(j) }
        pump()
    }

    private func finish(_ j: Job) {
        unattended[ObjectIdentifier(j)] = nil
        let cancelled = j.units.contains { $0.cancelled }
        j.state = cancelled ? .cancelled : j.errors.isEmpty ? .done : j.outputs.isEmpty && j.notes.isEmpty ? .failed : .partial
        j.progress = 1
        let e = j.errors.first
        if j.state == .done, let prev = history.first(where: { $0.state == .done && !$0.undone && !$0.outputs.isEmpty && $0.date.timeIntervalSinceNow > -900 && Set(j.paths).isSubset(of: Set($0.outputs.map(\.path))) }) {
            Habits.note(first: prev, second: j.action, title: j.title, input: j.paths[0])
        }
        history.insert(Record(title: j.title, date: Date(), state: j.state, paths: j.paths, outputs: j.outputs.compactMap { p in fileStamp(p).map { .init(path: p, size: $0.0, mtime: $0.1) } }, notes: j.notes, error: e?.detail, code: e?.code, retry: actionIsSensitive(j.action) ? nil : j.action), at: 0)
        saveHistory()
        jobs.removeAll { $0 === j }
        watchMade.formUnion(j.outputs)
        onFinished?(j)
        onChange?()
    }
    var watchMade = Set<String>()

    func cancel(_ j: Job) {
        // collected first: finishing a unit starts the next and changes this list
        let waiting = pending.filter { $0.job === j }
        pending.removeAll { $0.job === j }
        for u in waiting { u.finished = true; u.cancelled = true; unitEnded(u) }
        for u in running where u.job === j { stop(u) }
    }
    private func stop(_ u: Unit) {
        guard let p = u.process, p.isRunning else { return }
        let pid = p.processIdentifier
        kill(pid, SIGTERM)
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) { [weak self] in
            guard p.isRunning else { return }
            kill(-pid, SIGKILL)
            u.finished = true; u.cancelled = true
            self?.removeClaims(u)
        }
    }
    func stopEverything() {
        pending.removeAll()
        let list = running
        for u in list { if let p = u.process, p.isRunning { kill(p.processIdentifier, SIGTERM) } }
        let end = Date().addingTimeInterval(1.5)
        while Date() < end, list.contains(where: { $0.process?.isRunning == true }) { usleep(30_000) }
        for u in list { if let p = u.process, p.isRunning { kill(-p.processIdentifier, SIGKILL) }; removeClaims(u) }
    }

    /// Only moves results that are still exactly what the job wrote.
    func undo(_ r: Record) -> String? {
        var changed = 0
        for o in r.outputs {
            guard let s = fileStamp(o.path) else { continue }
            var d: ObjCBool = false
            FileManager.default.fileExists(atPath: o.path, isDirectory: &d)
            if !d.boolValue && (s.0 != o.size || abs(s.1 - o.mtime) > 1) { changed += 1; continue }
            try? FileManager.default.trashItem(at: URL(fileURLWithPath: o.path), resultingItemURL: nil)
        }
        if let i = history.firstIndex(where: { $0.id == r.id }) { history[i].undone = true; saveHistory() }
        return changed > 0 ? "\(changed) \(T("changed since, left in place"))" : nil
    }
    /// The newest thing made and not yet taken back goes to the Trash, from anywhere, by one key.
    func undoLast() {
        guard let r = history.first(where: { !$0.undone && !$0.outputs.isEmpty }) else { Toasts.shared.say(T("Nothing to undo"), ok: false); return }
        let left = undo(r)
        Toasts.shared.say(left ?? "\(T("Undone")): \(T(r.title))", ok: left == nil)
    }
    func retry(_ r: Record) { if let a = r.retry { queue(r.paths, a, title: r.title) } }
}

/// Plain-language reason and next step for each engine error code.
func explain(_ code: String) -> (why: String, how: String, addon: Bool) {
    switch code {
    case "need_ffmpeg": return ("Video and audio actions need FFmpeg, which isn’t installed yet.", "Install it with Homebrew (brew install ffmpeg), then try again.", true)
    case "need_addon": return ("This action needs an add-on that isn’t installed.", "See Settings › Add-ons for what is missing.", true)
    case "need_libreoffice": return ("Converting Word, Excel or PowerPoint needs the free LibreOffice.", "Install LibreOffice, then try again.", true)
    case "ffmpeg_failed": return ("FFmpeg couldn’t process this file.", "It may be damaged or use an unusual format.", false)
    case "wrong_password": return ("The password doesn’t match this file.", "Try again and check Caps Lock and the keyboard language.", false)
    case "password_problem": return ("There’s a problem with the password.", "Use at least 8 characters; a longer phrase is safer.", false)
    case "timed_out": return ("The helper program got stuck on this file, so Rotae stopped it. Nothing was changed.", "The file may be damaged or very large.", false)
    case "need_claude_key": return ("Translate and Summarize use your own Claude API key, which isn’t set.", "Paste your key in Settings.", false)
    case "need_r2": return ("Upload link needs your Cloudflare R2 bucket details.", "Fill them in under Settings.", false)
    case "not_private_network": return ("This Mac isn’t on a private Wi-Fi or home network right now, so the phone link wasn’t opened.", "Join the same Wi-Fi as the phone, or turn off the VPN.", false)
    case "animated_unsupported": return ("This picture is animated; the action would keep only its first frame.", "Convert it to MP4 or WebM video first if you want to keep the animation.", false)
    case "too_big": return ("The file or picture is too big for this action.", "Make it smaller first.", false)
    case "bad_pdf": return ("This PDF couldn’t be opened.", "It may be damaged or password-protected.", false)
    case "bad_image": return ("This picture couldn’t be read.", "It may be damaged, or not really a picture.", false)
    case "bad_archive": return ("This archive couldn’t be extracted.", "It may be damaged or use an unsupported format.", false)
    case "not_text", "bad_data": return ("The file’s content isn’t what this action expects.", "It may be damaged or be another type of file.", false)
    case "shred_links": return ("This is a link or has other names pointing at it, so shredding could destroy something else.", "Shred the original file instead.", false)
    case "no_permission": return ("macOS refused access to this file or folder.", "Allow Rotae in System Settings › Privacy & Security › Files and Folders.", false)
    case "disk_full": return ("The disk is full.", "Free some space, then try again.", false)
    case "crashed": return ("The work stopped unexpectedly. Nothing was changed.", "Try again; if it repeats, the file may be damaged.", false)
    case "no_text": return ("No text was found in this picture.", "A sharper or larger picture may help.", false)
    case "no_faces": return ("No faces were found in this picture.", "Nothing needed blurring, so nothing was changed.", false)
    case "already_small": return ("This file is already as small as it gets.", "Nothing was changed.", false)
    case "one_page": return ("This PDF has only one page.", "There is nothing to split.", false)
    case "already_text": return ("This PDF already has text you can search and copy.", "Nothing needed doing, so nothing was changed.", false)
    case "no_pictures": return ("There are no pictures inside this document.", "Nothing was changed.", false)
    case "no_numbers": return ("No columns of numbers were found.", "Totals needs a table with at least one column of amounts.", false)
    case "checksum_mismatch": return ("The checksum does not match. This is not the file that was published.", "The download may be incomplete or altered. Get it again from the official site.", false)
    case "no_checksum": return ("There is no checksum on the clipboard.", "Copy the MD5, SHA-1, SHA-256 or SHA-512 from the download page, then try again.", false)
    case "wrong_document": return ("This works on .docx, .xlsx and .pptx files.", "Save older formats as one of those first.", false)
    case "bad_document": return ("This document couldn’t be read.", "It may be damaged or password-protected.", false)
    case "nothing_private": return ("Nothing private was found here.", "No e-mail address, phone, ID or account number was recognised, so nothing was changed. Use Redact to cover something by hand.", false)
    case "need_model": return ("The model built into macOS isn’t ready on this Mac.", "Turn on Apple Intelligence in System Settings, then try again. Nothing is sent anywhere either way.", false)
    case "model_language": return ("The model on this Mac doesn’t work in this language yet.", "Try Claude under Get › Claude, which needs your own key.", false)
    case "model_declined": return ("The model on this Mac declined this document.", "It refuses some subjects. Nothing was changed.", false)
    case "no_question": return ("There was no question.", "Type what you want to know about the document.", false)
    case "need_two_docs": return ("Redline needs exactly two documents.", "Select the old and the new version together, then drop them.", false)
    case "same_text": return ("These two documents have the same text.", "There is nothing to show: only formatting, if anything, differs.", false)
    case "too_small": return ("This picture is too small for app icons.", "Use one that is at least 256 pixels on its longer side; 1024 is best.", false)
    case "need_folder": return ("This works on a folder.", "Drop the project folder itself.", false)
    case "wrong_kind": return ("This action doesn’t work on this kind of file.", "The line below says which files it takes.", false)
    case "already_clean": return ("This table is already clean.", "No stray spaces, empty rows or duplicates were found, so nothing was changed.", false)
    case "need_two_tables": return ("Reconcile needs exactly two tables.", "Select both, such as a bank statement and your ledger, then drop them together.", false)
    case "not_einvoice": return ("This XML isn’t a Vietnamese e-invoice.", "It has no invoice section. Use the XML file the seller or the tax portal gave you.", false)
    case "need_template": return ("Mail merge needs one Word template and one list.", "Select a .docx and a .csv or .xlsx together, then drop them.", false)
    case "no_placeholders": return ("The template has no placeholders that match the list.", "Write them like {{Name}}, spelled as the column headers of the list.", false)
    case "file_open": return ("The register is open in another app.", "Close Invoices.xlsx, then drop the invoices again.", false)
    case "not_statement": return ("This PDF doesn’t read like a bank statement.", "No lines beginning with a date were found. A scanned statement needs OCR to PDF first.", false)
    case "no_column": return ("The table has no column with that name.", "Type the header exactly as it appears in the first row.", false)
    case "not_allowed_unattended": return ("This action isn’t allowed to run unattended.", "Run it yourself, or choose another action for this folder.", false)
    default: return ("Something went wrong with this file.", "Details are below.", false)
    }
}

/// Runs a watched folder's action on new, finished files. Files already there are left alone.
final class FolderWatcher {
    var stream: FSEventStreamRef?
    var known: [String: Set<String>] = [:]
    var pendingSize: [String: Int64] = [:]
    var timer: Timer?

    func reload() {
        stop()
        let w = Store.shared.state.watches
        known = Dictionary(uniqueKeysWithValues: w.map { ($0.folder, list($0.folder)) })
        guard !w.isEmpty else { return }
        var ctx = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        let cb: FSEventStreamCallback = { _, info, _, _, _, _ in Unmanaged<FolderWatcher>.fromOpaque(info!).takeUnretainedValue().scheduleScan() }
        stream = FSEventStreamCreate(nil, cb, &ctx, w.map(\.folder) as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 1.0, FSEventStreamCreateFlags(kFSEventStreamCreateFlagFileEvents))
        if let s = stream { FSEventStreamSetDispatchQueue(s, .main); FSEventStreamStart(s) }
    }
    func stop() { if let s = stream { FSEventStreamStop(s); FSEventStreamInvalidate(s); FSEventStreamRelease(s); stream = nil }; timer?.invalidate() }
    func list(_ f: String) -> Set<String> { Set(((try? FileManager.default.contentsOfDirectory(atPath: f)) ?? []).map { (f as NSString).appendingPathComponent($0) }) }
    func scheduleScan() { timer?.invalidate(); timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: false) { [weak self] _ in self?.scan() } }

    func scan() {
        var again = false
        for w in Store.shared.state.watches where unattendedAllowed(w.action) {
            let now = list(w.folder)
            for p in now.subtracting(known[w.folder] ?? []) {
                if Jobs.shared.watchMade.contains(p) { known[w.folder, default: []].insert(p); continue }
                if !extList(w.exts).isEmpty && !extList(w.exts).contains(extOf(p)) { known[w.folder, default: []].insert(p); continue }
                guard let s = fileStamp(p), !(p as NSString).lastPathComponent.hasPrefix(".") else { continue }
                // a file still being written keeps growing; wait for two equal looks
                if pendingSize[p] == s.0, s.0 > 0 {
                    known[w.folder, default: []].insert(p); pendingSize[p] = nil
                    if let m = w.minMB, Double(s.0) < m * 1_048_576 { continue }
                    Jobs.shared.queue([p], w.action, title: w.title, unattended: true)
                } else { pendingSize[p] = s.0; again = true }
            }
        }
        if again { scheduleScan() }
    }
}
