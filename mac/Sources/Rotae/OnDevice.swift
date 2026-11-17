import AppKit
import SDCore
import UniformTypeIdentifiers
#if canImport(FoundationModels)
import FoundationModels
#endif

/// Whether the language model that ships with macOS can be used here. Asked once: it does not
/// change while the app runs, and every wheel wants to know.
enum OnDevice {
    static let ready: Bool = {
        #if canImport(FoundationModels)
        if #available(macOS 26, *), case .available = SystemLanguageModel.default.availability { return true }
        #endif
        return false
    }()
}

/// Recipes as files, to hand to someone else or to another Mac.
enum RecipeFile {
    static let ext = "rotae-recipe"
    struct Box: Codable { var rotae = 1; var recipes: [Recipe] }

    /// A recipe from someone else must not be able to run a program: only built-in steps come in.
    static func safe(_ a: Action) -> Bool {
        let op = a["op"]?.string ?? ""
        if op == "custom" || op == "tool" { return false }
        if case .a(let steps)? = a["steps"] { return steps.allSatisfy { if case .o(let s) = $0 { return safe(s) }; return false } }
        return true
    }
    static func export(_ r: Recipe) {
        let p = NSSavePanel()
        p.nameFieldStringValue = "\(r.name.replacingOccurrences(of: "/", with: "-")).\(ext)"
        NSApp.activate(ignoringOtherApps: true)
        guard p.runModal() == .OK, let u = p.url, let d = try? JSONEncoder().encode(Box(recipes: [r])) else { return }
        do { try d.write(to: u, options: .atomic); Toasts.shared.say(T("Recipe saved")) } catch { Toasts.shared.say(T("Couldn't save the recipe"), ok: false) }
    }
    static func pick() {
        let p = NSOpenPanel(); p.allowsMultipleSelection = true
        if let t = UTType(filenameExtension: ext) { p.allowedContentTypes = [t, .json] }
        NSApp.activate(ignoringOtherApps: true)
        if p.runModal() == .OK { read(p.urls) }
    }
    /// Shows what is about to come in, and takes it only on a yes.
    static func read(_ urls: [URL]) {
        var good: [Recipe] = [], refused = 0, unreadable = 0
        for u in urls {
            guard let n = (try? u.resourceValues(forKeys: [.fileSizeKey]))?.fileSize, n < 1 << 20, let d = try? Data(contentsOf: u), let b = try? JSONDecoder().decode(Box.self, from: d) else { unreadable += 1; continue }
            for var r in b.recipes.prefix(50) {
                guard !r.steps.isEmpty, r.steps.count <= 30, r.steps.allSatisfy({ safe($0.action) }) else { refused += 1; continue }
                r.id = UUID(); r.name = String(r.name.prefix(60))
                good.append(r)
            }
        }
        var lines = good.map { "• \($0.name): \($0.steps.map { T($0.label) }.joined(separator: " → "))" }
        if refused > 0 { lines.append("\(refused) \(T("left out because they run a program on your Mac"))") }
        if unreadable > 0 { lines.append("\(unreadable) \(T("could not be read"))") }
        let a = NSAlert()
        a.messageText = good.isEmpty ? T("Nothing to add") : T("Add these recipes?")
        a.informativeText = lines.joined(separator: "\n")
        if !good.isEmpty { a.addButton(withTitle: T("Add")) }
        a.addButton(withTitle: T("Cancel"))
        NSApp.activate(ignoringOtherApps: true)
        if a.runModal() == .alertFirstButtonReturn, !good.isEmpty {
            Store.shared.update { $0.recipes += good }
            Windows.shared.hub(.recipes)
        }
    }
}

/// The activity list as a spreadsheet, for anyone who has to account for what was done to a file.
enum ActivityLog {
    static func export(_ list: [Record]) {
        let p = NSSavePanel()
        let day = ISO8601DateFormatter(); day.formatOptions = [.withFullDate]
        p.nameFieldStringValue = "Rotae \(day.string(from: Date())).csv"
        NSApp.activate(ignoringOtherApps: true)
        guard p.runModal() == .OK, let u = p.url else { return }
        let when = ISO8601DateFormatter()
        // a cell that starts like a formula is text to us, and must be to a spreadsheet too
        func cell(_ s: String) -> String {
            let v = s.first.map { "=+-@\t\r".contains($0) } == true ? "'" + s : s
            return "\"" + v.replacingOccurrences(of: "\"", with: "\"\"") + "\""
        }
        var out = "\u{FEFF}" + ["When", "Action", "Result", "Files", "Made", "Notes", "Problem", "Undone"].map(cell).joined(separator: ",") + "\r\n"
        for r in list {
            out += [when.string(from: r.date), r.title, r.state.rawValue, r.paths.joined(separator: "\n"), r.outputs.map(\.path).joined(separator: "\n"), r.notes.joined(separator: "\n"), r.error ?? "", r.undone ? "yes" : ""].map(cell).joined(separator: ",") + "\r\n"
        }
        do { try out.write(to: u, atomically: true, encoding: .utf8); Toasts.shared.say(T("Activity saved")) } catch { Toasts.shared.say(T("Couldn't save the list"), ok: false) }
    }
}

/// Notices when the result of one action keeps being fed to the same second action, and
/// offers the pair as a recipe. It only ever suggests; nothing is added without a click.
enum Habits {
    static func key(_ a: Action, _ b: Action) -> String {
        let e = JSONEncoder(); e.outputFormatting = .sortedKeys
        return String(decoding: (try? e.encode([a, b])) ?? Data(), as: UTF8.self)
    }
    static func plain(_ a: Action) -> Bool { !["chain", "custom", "tool"].contains(a["op"]?.string ?? "") && !actionIsSensitive(a) && !usesNetwork(a) }
    /// `first` made the files that `second` was then run on.
    static func note(first: Record, second: Action, title: String, input: String) {
        guard let a = first.retry, plain(a), plain(second) else { return }
        let k = key(a, second)
        let st = Store.shared.state
        let steps = [RecipeStep(label: first.title, action: a), RecipeStep(label: title, action: second)]
        let known = (st.recipes + st.suggested).contains { $0.steps.map(\.action) == steps.map(\.action) }
        let n = (st.habits[k] ?? 0) + 1
        Store.shared.update { s in
            s.habits[k] = n
            if s.habits.count > 300 { s.habits = s.habits.filter { $0.value > 1 } }
            if n == 3, !known, !s.dismissed.contains(k) {
                s.suggested.insert(Recipe(name: "\(T(first.title)) → \(T(title))", kind: kindOf(first.paths.first ?? input).rawValue, steps: steps), at: 0)
                s.suggested = Array(s.suggested.prefix(5))
            }
        }
        if n == 3, !known, !st.dismissed.contains(k) { Toasts.shared.say(T("You do these two together often: see Recipes for a one-drop version")) }
    }
}

enum CommandLineTool {
    /// A link in ~/.local/bin, which needs no administrator password.
    static func install() {
        let fm = FileManager.default
        let dir = fm.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin", isDirectory: true), link = dir.appendingPathComponent("rotae")
        let tool = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/rotae-cli")
        do {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            // only ever replace a link, never a real file someone put there
            if let t = try? fm.destinationOfSymbolicLink(atPath: link.path) { _ = t; try fm.removeItem(at: link) }
            else if fm.fileExists(atPath: link.path) { Toasts.shared.say(T("There is already a file at ~/.local/bin/rotae"), ok: false); return }
            try fm.createSymbolicLink(at: link, withDestinationURL: tool)
            let onPath = (ProcessInfo.processInfo.environment["PATH"] ?? "").contains("/.local/bin")
            Toasts.shared.say(onPath ? T("Installed: type rotae in a terminal") : T("Installed in ~/.local/bin: add that folder to your PATH"))
        } catch { Toasts.shared.say(T("Couldn't install the command"), ok: false) }
    }
}
