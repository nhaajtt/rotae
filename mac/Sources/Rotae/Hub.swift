import AppKit
import SDCore
import SwiftUI

enum Route: Equatable {
    case actions([String]), activity, rename([String]), recipes, wheel, settings, welcome, ask(MenuItem, [String]), phone([String]), upload([String]), receive
    var tab: String {
        switch self {
        case .actions, .ask, .phone, .upload, .receive: return "actions"
        case .activity: return "activity"; case .rename: return "rename"; case .recipes: return "recipes"; case .wheel: return "wheel"; case .settings: return "settings"; case .welcome: return "welcome"
        }
    }
}

final class Windows: NSObject, ObservableObject, NSWindowDelegate {
    static let shared = Windows()
    @Published var route = Route.actions([])
    var hubWindow: NSWindow?
    var editors: [NSWindow] = []

    func hub(_ r: Route) {
        route = r
        if hubWindow == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 860, height: 620), styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
            w.titlebarAppearsTransparent = true; w.title = "Rotae"; w.isReleasedWhenClosed = false; w.minSize = NSSize(width: 680, height: 480); w.center()
            if Liquid.available { w.isOpaque = false; w.backgroundColor = .clear }
            w.contentView = NSHostingView(rootView: HubView().environmentObject(self).environmentObject(Store.shared).environmentObject(Jobs.shared))
            hubWindow = w
        }
        NSApp.activate(ignoringOtherApps: true)
        hubWindow?.makeKeyAndOrderFront(nil)
    }
    func hideHub() { hubWindow?.orderOut(nil) }
    func editor(_ tool: String, path: String) {
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 960, height: 680), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        w.title = "\(T(tool.capitalized)) — \((path as NSString).lastPathComponent)"; w.isReleasedWhenClosed = false; w.minSize = NSSize(width: 600, height: 480); w.center(); w.delegate = self
        w.contentView = NSHostingView(rootView: EditorHost(tool: tool, path: path) { [weak w] in w?.close() }.tint(Store.shared.accent))
        editors.append(w)
        NSApp.activate(ignoringOtherApps: true); w.makeKeyAndOrderFront(nil)
    }
    func windowWillClose(_ n: Notification) { editors.removeAll { $0 === n.object as AnyObject } }
}

struct HubView: View {
    @EnvironmentObject var w: Windows
    @EnvironmentObject var s: Store
    let tabs: [(String, String, String)] = [("actions", "Actions", "square.grid.2x2"), ("activity", "Activity", "clock"), ("rename", "Rename", "pencil.line"), ("recipes", "Recipes", "bolt"), ("wheel", "Wheel", "circle.circle"), ("settings", "Settings", "gearshape")]
    var body: some View {
        NavigationSplitView {
            List(tabs, id: \.0, selection: Binding(get: { w.route.tab }, set: { go($0 ?? "actions") })) { t in
                Label { Text(T(t.1)) } icon: { Image(systemName: t.2).foregroundStyle(s.accent) }.tag(t.0)
            }.navigationSplitViewColumnWidth(190)
        } detail: {
            detail.id(w.route.tab).transition(.opacity).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).scrollContentBackground(.hidden)
                .animation(Motion.reduced ? nil : Motion.swap, value: w.route.tab)
                // the sidebar is pure glass; what you read sits on a calmer sheet of it
                .background(Liquid.available ? Color(nsColor: .windowBackgroundColor).opacity(0.62).ignoresSafeArea() : nil)
        }.tint(s.accent).panelGlass()
    }
    func go(_ t: String) {
        switch t {
        case "activity": w.route = .activity; case "rename": w.route = .rename([]); case "recipes": w.route = .recipes
        case "wheel": w.route = .wheel; case "settings": w.route = .settings; default: w.route = .actions([])
        }
    }
    @ViewBuilder var detail: some View {
        switch w.route {
        case .actions(let p): ActionsView(paths: p).id(p)
        case .activity: ActivityView()
        case .rename(let p): RenameView(paths: p)
        case .recipes: RecipesView()
        case .wheel: WheelCustomView()
        case .settings: SettingsView()
        case .welcome: WelcomeView()
        case .ask(let i, let p): AskView(item: i, paths: p).id(i.id)
        case .phone(let p): PhoneView(paths: p).id(p)
        case .upload(let p): UploadView(paths: p).id(p)
        case .receive: ReceiveView()
        }
    }
}

func title(_ s: String) -> some View { Text(T(s)).font(.title2.weight(.semibold)).padding(.bottom, 4) }

struct ActionsView: View {
    @State var paths: [String]
    @State var q = ""
    @State var targeted = false
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            title("Actions")
            if paths.isEmpty {
                VStack(spacing: 12) {
                    Image(systemName: "arrow.down.doc").font(.largeTitle).foregroundStyle(.secondary)
                    Text(T("Drop files here")).font(.headline)
                    Button(T("Paste from clipboard")) { paste() }.keyboardShortcut("v")
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
                 .background(RoundedRectangle(cornerRadius: 14).strokeBorder(targeted ? Store.shared.accent : .secondary.opacity(0.4), style: StrokeStyle(lineWidth: 1.5, dash: [6])))
            } else {
                ScrollView(.horizontal, showsIndicators: false) { HStack { ForEach(paths.prefix(12), id: \.self) { p in HStack(spacing: 4) { ThumbView(path: p, size: 20); Text((p as NSString).lastPathComponent).lineLimit(1).font(.caption) }.padding(4).background(.quaternary, in: Capsule()) }; if paths.count > 12 { Text("+\(paths.count - 12)") } } }
                TextField(T("Search actions"), text: $q).textFieldStyle(.roundedBorder)
                let s = Store.shared.state
                let m = menuFor(paths, prefs: s.prefs, recipes: s.recipes, commands: s.commands, apps: Tools.apps, tools: Tools.installedIDs, now: Date().timeIntervalSince1970, offline: s.offline)
                let hit: (MenuItem) -> Bool = { q.isEmpty || matches($0, q) || TL($0.label).lowercased().contains(q.lowercased()) }
                let secs = sections(m).map { ($0.0, $0.1.filter(hit)) }.filter { !$0.1.isEmpty }
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        ForEach(secs, id: \.0.id) { g, list in
                            VStack(alignment: .leading, spacing: 8) {
                                Label(T(g.label), systemImage: g.symbol).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                                LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 8)], alignment: .leading, spacing: 8) {
                                    ForEach(list, id: \.self) { it in ActionChip(item: it) { pick(it, m.kind) } }
                                }
                            }
                        }
                        if secs.isEmpty { Text(T("Nothing matches")).foregroundStyle(.secondary).frame(maxWidth: .infinity).padding(.top, 40) }
                    }.padding(.bottom, 8)
                }.onSubmit(of: .text) { if let it = secs.first?.1.first { pick(it, m.kind) } }
            }
        }.padding(20).onDrop(of: [.fileURL], isTargeted: $targeted) { loadDropped($0) { paths = $0 }; return true }
    }
    /// The wheel's own groups, so the Hub and the wheel are learned once. Anything turned off on
    /// the wheel is still here, under More.
    func sections(_ m: Shown) -> [(MenuItem, [MenuItem])] {
        var out = m.items.map { g in (g, g.sub.isEmpty ? [g] : flatActions(g.sub)) }
        let seen = Set(out.flatMap { $0.1.map(\.id) })
        let rest = flatActions(m.all).filter { !seen.contains($0.id) }
        if !rest.isEmpty { out.append((MenuItem("More", "ellipsis.circle", id: "g-rest"), rest)) }
        return out
    }
    func pick(_ it: MenuItem, _ k: FileKind) {
        Store.shared.noteUse(k, it.from ?? it.id)
        Actions.run(it, paths: paths, kind: k)
        if it.editor == nil && it.share == nil && it.ask.isEmpty && it.danger == nil && it.local != "rename" { Windows.shared.hideHub() }
    }
    func paste() {
        let pb = NSPasteboard.general
        if let u = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL], !u.isEmpty { paths = u.map(\.path); return }
        if let p = pastedImage() { paths = [p] }
    }
}

/// A picture on the clipboard (a screenshot, a copied image) becomes a file Rotae can work on.
private var lastPasted: (count: Int, path: String)?
func pastedImage() -> String? {
    let pb = NSPasteboard.general
    // asking twice about the same clipboard must not write the picture twice
    if let l = lastPasted, l.count == pb.changeCount, FileManager.default.fileExists(atPath: l.path) { return l.path }
    guard let img = NSImage(pasteboard: pb), let t = img.tiffRepresentation, let png = NSBitmapImageRep(data: t)?.representation(using: .png, properties: [:]) else { return nil }
    let d = FileManager.default.urls(for: .picturesDirectory, in: .userDomainMask)[0].appendingPathComponent("Rotae"); try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
    let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd HH.mm.ss"
    var u = d.appendingPathComponent("Pasted image \(f.string(from: Date())).png"), n = 2
    while FileManager.default.fileExists(atPath: u.path) { u = d.appendingPathComponent("Pasted image \(f.string(from: Date())) \(n).png"); n += 1 }
    guard (try? png.write(to: u, options: .withoutOverwriting)) != nil else { return nil }
    lastPasted = (pb.changeCount, u.path)
    return u.path
}

struct ActionChip: View {
    let item: MenuItem; let run: () -> Void
    @State var over = false
    var body: some View {
        Button(action: run) {
            HStack(spacing: 8) {
                Image(systemName: item.symbol).frame(width: 20).foregroundStyle(over ? .white : Store.shared.accent)
                Text(TL(item.label)).lineLimit(1).truncationMode(.middle).foregroundStyle(over ? .white : .primary)
                Spacer(minLength: 0)
                if item.editor != nil { Image(systemName: "slider.horizontal.3").font(.caption).foregroundStyle(over ? .white.opacity(0.8) : .secondary) }
            }
            .padding(.horizontal, 10).padding(.vertical, 8)
            .background(over ? AnyShapeStyle(Store.shared.accent) : AnyShapeStyle(.quaternary.opacity(0.55)), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 10))
        }.buttonStyle(.plain).onHover { over = $0 }.help(TL(item.label)).animation(Motion.reduced ? nil : .easeOut(duration: 0.12), value: over)
    }
}

struct AskView: View {
    let item: MenuItem; let paths: [String]
    @State var vals: [String: String] = [:]
    @State var again = ""
    @State var ran = false
    var bad: Bool {
        for f in item.ask {
            let v = vals[f.key] ?? ""
            if v.isEmpty { return true }
            if f.kind == "number" && Double(v.replacingOccurrences(of: ",", with: ".")) == nil { return true }
            if f.confirm && v != again { return true }
        }
        return ran
    }
    var body: some View {
        Form {
            Text(T(item.label)).font(.title2.weight(.semibold))
            Text(paths.map { ($0 as NSString).lastPathComponent }.prefix(5).joined(separator: ", ")).foregroundStyle(.secondary)
            if let d = item.danger { Label(T(d), systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red) }
            ForEach(item.ask, id: \.key) { f in
                if f.kind == "password" {
                    SecureField(T(f.label), text: b(f.key))
                    if f.confirm { SecureField(T("Repeat password"), text: $again) }
                } else { TextField(T(f.label), text: b(f.key)) }
            }
            HStack {
                Button(T("Cancel")) { Windows.shared.route = .actions([]); Windows.shared.hideHub() }
                Button(item.danger != nil ? T("Yes, do it") : T("Run")) { run() }.keyboardShortcut(.defaultAction).disabled(bad)
            }
        }.padding(20).formStyle(.grouped)
    }
    func b(_ k: String) -> Binding<String> { Binding(get: { vals[k] ?? "" }, set: { vals[k] = $0 }) }
    func run() {
        ran = true
        var a = item.action ?? [:]
        for f in item.ask {
            let v = vals[f.key] ?? ""
            let val: JSON = f.kind == "number" ? .n(Double(v.replacingOccurrences(of: ",", with: ".")) ?? 0) : .s(v)
            // "2.password": this answer belongs to step 2 of a recipe
            let parts = f.key.split(separator: ".", maxSplits: 1).map(String.init)
            if parts.count == 2, let i = Int(parts[0]), case .a(var steps)? = a["steps"], steps.indices.contains(i), case .o(var step) = steps[i] {
                step[parts[1]] = val; steps[i] = .o(step); a["steps"] = .a(steps)
            } else { a[f.key] = val }
        }
        Jobs.shared.queue(paths, a, title: item.title ?? item.label)
        Windows.shared.route = .actions([]); Windows.shared.hideHub()
    }
}

struct ActivityView: View {
    @EnvironmentObject var j: Jobs
    @State var filter = "All"
    @State var confirm = false
    var list: [Record] { j.history.filter { filter == "All" || (filter == "Failed" ? $0.state == .failed || $0.state == .partial : $0.state == .done) } }
    var body: some View {
        VStack(alignment: .leading) {
            HStack {
                title("Activity"); Spacer()
                Picker("", selection: $filter) { ForEach(["All", "Failed", "Done"], id: \.self) { Text(T($0)) } }.pickerStyle(.segmented).frame(width: 220)
                Button(T("Export…")) { ActivityLog.export(list) }.disabled(list.isEmpty)
                Button(T("Clear list")) { confirm = true }
            }
            List(list.prefix(100)) { r in
                HStack(alignment: .top, spacing: 10) {
                    if let p = r.outputs.first?.path, !r.undone { ThumbView(path: p, size: 40) } else { Image(systemName: r.state == .failed ? "xmark.octagon" : "checkmark.circle").font(.title2).frame(width: 40) }
                    VStack(alignment: .leading, spacing: 3) {
                        Text(T(r.title)).font(.headline)
                        Text(r.paths.map { ($0 as NSString).lastPathComponent }.prefix(3).joined(separator: ", ")).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        ForEach(r.notes.prefix(3), id: \.self) { Text($0).font(.caption).textSelection(.enabled) }
                        if r.state == .failed || r.state == .partial { Text(T(explain(r.code ?? "").why)).font(.caption).foregroundStyle(.red); if let e = r.error { Text(e).font(.caption2.monospaced()).lineLimit(2).textSelection(.enabled) } }
                    }
                    Spacer()
                    VStack(alignment: .trailing) {
                        Text(r.date, style: .relative).font(.caption2).foregroundStyle(.secondary)
                        HStack {
                            if !r.outputs.isEmpty && !r.undone { Button(T("Show")) { revealAll(r.outputs.map(\.path)) }; Button(T("Undo")) { _ = j.undo(r) } }
                            if r.retry != nil && (r.state == .failed || r.state == .partial) { Button(T("Try again")) { j.retry(r) } }
                        }.controlSize(.small)
                    }
                }.padding(.vertical, 3)
            }
        }.padding(20).confirmationDialog(T("Clear the activity list? Files are not touched."), isPresented: $confirm) { Button(T("Clear list"), role: .destructive) { j.history = []; j.saveHistory() } }
    }
}

struct RenameView: View {
    @State var paths: [String]
    @State var pattern = "{name}"
    @State var find = ""
    @State var rep = ""
    @State var regex = false
    @State var mode = "keep"
    @State var start = 1
    @State var message: String?
    @State var lastUndo: [(String, String)] = []
    var preview: [(String, String)] {
        paths.enumerated().map { i, p in
            let n = (p as NSString).lastPathComponent
            return (p, applyRename(name: n, n: start + i, pattern: pattern, find: find, replace: rep, regex: regex, caseMode: mode, date: Date()))
        }
    }
    var problem: String? {
        if regex, !find.isEmpty, (try? NSRegularExpression(pattern: find)) == nil { return T("That pattern isn't valid") }
        let names = preview.map { $0.1.lowercased() }
        if Set(names).count != names.count { return T("Two files would get the same name") }
        if preview.contains(where: { $0.1.isEmpty || $0.1.contains("/") }) { return T("A name can't be empty or contain /") }
        return nil
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            title("Rename")
            if paths.isEmpty { Text(T("Drop files here")).foregroundStyle(.secondary) }
            TextField(T("Pattern: {name} {n} {date}"), text: $pattern).textFieldStyle(.roundedBorder)
            HStack { TextField(T("Find"), text: $find); TextField(T("Replace with"), text: $rep); Toggle(T("Regex"), isOn: $regex) }.textFieldStyle(.roundedBorder)
            HStack {
                Stepper("\(T("Start at")) \(start)", value: $start, in: 0...99999)
                Picker("", selection: $mode) { ForEach(["keep", "lower", "upper", "title", "slug"], id: \.self) { Text(T($0)) } }.frame(width: 140)
            }
            List(preview, id: \.0) { a, b in HStack { Text((a as NSString).lastPathComponent).lineLimit(1); Image(systemName: "arrow.right").foregroundStyle(.secondary); Text(b).lineLimit(1).fontWeight(.medium) } }
            HStack {
                if let p = problem { Text(p).foregroundStyle(.red) } else if let m = message { Text(m).foregroundStyle(.secondary) }
                Spacer()
                if !lastUndo.isEmpty { Button(T("Undo")) { undo() } }
                Button(T("Rename")) { apply() }.keyboardShortcut(.defaultAction).disabled(paths.isEmpty || problem != nil)
            }
        }.padding(20).onDrop(of: [.fileURL], isTargeted: nil) { loadDropped($0) { paths += $0.filter { !paths.contains($0) } }; return true }
    }
    /// All or nothing: through temporary names first, so swaps work and a failure rolls back.
    func batch(_ pairs: [(String, String)]) -> Bool {
        let fm = FileManager.default
        let finalSet = Set(pairs.map { $0.1.lowercased() }), sources = Set(pairs.map { $0.0.lowercased() })
        if pairs.contains(where: { fm.fileExists(atPath: $0.1) && !sources.contains($0.1.lowercased()) }) { return false }
        _ = finalSet
        var temp: [(String, String)] = [], done: [(String, String)] = []
        do {
            for (a, _) in pairs { let t = (a as NSString).deletingLastPathComponent + "/.sd-\(UUID().uuidString)"; try fm.moveItem(atPath: a, toPath: t); temp.append((a, t)) }
            for (i, p) in pairs.enumerated() { try fm.moveItem(atPath: temp[i].1, toPath: p.1); done.append((temp[i].1, p.1)) }
            return true
        } catch {
            for (t, f) in done.reversed() { try? fm.moveItem(atPath: f, toPath: t) }
            for (a, t) in temp.reversed() { try? fm.moveItem(atPath: t, toPath: a) }
            return false
        }
    }
    func apply() {
        let pairs = preview.map { ($0.0, (($0.0 as NSString).deletingLastPathComponent as NSString).appendingPathComponent($0.1)) }.filter { $0.0 != $0.1 }
        if batch(pairs) { lastUndo = pairs; let m = Dictionary(uniqueKeysWithValues: pairs); paths = paths.map { m[$0] ?? $0 }; message = "\(pairs.count) \(T("renamed"))" } else { message = T("Nothing was renamed: a name is taken or a file is locked") }
    }
    func undo() {
        let back = lastUndo.map { ($0.1, $0.0) }
        if batch(back) { let m = Dictionary(uniqueKeysWithValues: back); paths = paths.map { m[$0] ?? $0 }; lastUndo = []; message = T("Put back") }
    }
}

let sampleNames: [FileKind: String] = [.image: "a.png", .video: "a.mp4", .audio: "a.mp3", .pdf: "a.pdf", .text: "a.txt", .office: "a.docx", .exe: "a.exe", .archive: "a.zip", .other: "a.bin"]

struct RecipesView: View {
    @EnvironmentObject var s: Store
    @State var edit: Recipe?
    @State var cmd = UserCommand(name: "", command: "", exts: "")
    @State var wFolder = ""
    @State var wExts = ""
    @State var wRecipe: UUID?
    @State var wMin = ""
    var body: some View {
        ScrollView { VStack(alignment: .leading, spacing: 16) {
            HStack { title("Recipes"); Spacer(); Button(T("Import…")) { RecipeFile.pick() }; Button(T("New recipe")) { edit = Recipe(name: "", kind: "image", steps: []) } }
            ForEach(s.state.recipes) { r in
                HStack {
                    Image(systemName: "bolt.fill").foregroundStyle(s.accent)
                    VStack(alignment: .leading) { Text(r.name).font(.headline); Text(r.steps.map { T($0.label) }.joined(separator: " → ")).font(.caption).foregroundStyle(.secondary) }
                    Spacer()
                    Text("rotae://run?recipe=…").font(.caption2).foregroundStyle(.tertiary)
                    Button { RecipeFile.export(r) } label: { Image(systemName: "square.and.arrow.up") }.help(T("Save as a file to share"))
                    Button(T("Edit")) { edit = r }
                    Button(role: .destructive) { s.update { $0.recipes.removeAll { $0.id == r.id } } } label: { Image(systemName: "trash") }
                }.padding(10).background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10))
            }
            if !s.state.suggested.isEmpty {
                Text(T("Suggested from what you do")).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                ForEach(s.state.suggested) { r in
                    HStack {
                        Image(systemName: "sparkles").foregroundStyle(s.accent)
                        VStack(alignment: .leading) { Text(r.name).font(.headline); Text(T("You ran these one after the other three times.")).font(.caption).foregroundStyle(.secondary) }
                        Spacer()
                        Button(T("Add")) { s.update { $0.suggested.removeAll { $0.id == r.id }; $0.recipes.append(r) } }
                        Button(T("No thanks")) { s.update { st in st.suggested.removeAll { $0.id == r.id }; if r.steps.count == 2 { st.dismissed.append(Habits.key(r.steps[0].action, r.steps[1].action)); st.dismissed = Array(st.dismissed.suffix(100)) } } }
                    }.padding(10).background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 10))
                }
            }
            Divider(); title("Your commands")
            Text(T("Use {file} {dir} {name} {stem} {ext}. Values are passed safely, whatever the file is called.")).font(.caption).foregroundStyle(.secondary)
            ForEach(s.state.commands) { c in HStack { Text(c.name).font(.headline); Text(c.command).font(.caption.monospaced()).lineLimit(1); Spacer(); Button(role: .destructive) { s.update { $0.commands.removeAll { $0.id == c.id } } } label: { Image(systemName: "trash") } } }
            HStack { TextField(T("Name"), text: $cmd.name); TextField(T("Command"), text: $cmd.command); TextField(T("Only for (png, jpg)"), text: $cmd.exts).frame(width: 130); Button(T("Add")) { s.update { $0.commands.append(cmd) }; cmd = UserCommand(name: "", command: "", exts: "") }.disabled(cmd.name.isEmpty || cmd.command.isEmpty) }.textFieldStyle(.roundedBorder)
            Divider(); title("Watched folders")
            Text(T("New files in a folder go through a recipe by themselves. Files already there are left alone.")).font(.caption).foregroundStyle(.secondary)
            ForEach(s.state.watches) { w in HStack { Image(systemName: "folder"); Text(w.folder).lineLimit(1).truncationMode(.middle); Text("→ \(w.title)").foregroundStyle(.secondary); if let m = w.minMB { Text("≥ \(m.formatted()) MB").font(.caption).foregroundStyle(.secondary) }; Spacer(); Button(role: .destructive) { s.update { $0.watches.removeAll { $0.id == w.id } }; reloadW() } label: { Image(systemName: "trash") } } }
            HStack {
                Button(wFolder.isEmpty ? T("Choose folder…") : (wFolder as NSString).lastPathComponent) { pickFolder() }
                Picker("", selection: $wRecipe) { Text(T("Recipe")).tag(UUID?.none); ForEach(allowed) { Text($0.name).tag(UUID?.some($0.id)) } }.frame(width: 180)
                TextField(T("Only for (png, jpg)"), text: $wExts).textFieldStyle(.roundedBorder).frame(width: 130)
                TextField(T("Only over (MB)"), text: $wMin).textFieldStyle(.roundedBorder).frame(width: 110)
                Button(T("Add")) { addW() }.disabled(wFolder.isEmpty || wRecipe == nil)
            }
        }.padding(20) }
        .sheet(item: $edit) { r in RecipeEditor(recipe: r) { new in s.update { st in if let i = st.recipes.firstIndex(where: { $0.id == new.id }) { st.recipes[i] = new } else { st.recipes.append(new) } }; edit = nil } cancel: { edit = nil } }
    }
    var allowed: [Recipe] { s.state.recipes.filter { r in r.steps.allSatisfy { unattendedAllowed($0.action) && $0.ask.isEmpty } } }
    func pickFolder() { let p = NSOpenPanel(); p.canChooseDirectories = true; p.canChooseFiles = false; if p.runModal() == .OK, let u = p.url { wFolder = u.path } }
    func reloadW() { (NSApp.delegate as? AppDelegate)?.watcher.reload() }
    func addW() {
        guard let r = s.state.recipes.first(where: { $0.id == wRecipe }) else { return }
        let min = Double(wMin.replacingOccurrences(of: ",", with: ".")).flatMap { $0 > 0 ? $0 : nil }
        s.update { $0.watches.append(Watch(folder: wFolder, exts: wExts, title: r.name, action: act("chain", ["steps": .a(r.steps.map { .o($0.action) })]), minMB: min)) }
        wFolder = ""; wExts = ""; wMin = ""; wRecipe = nil; reloadW()
    }
}

struct RecipeEditor: View {
    @State var recipe: Recipe
    let save: (Recipe) -> Void, cancel: () -> Void
    @State var adding = false
    var options: [MenuItem] {
        guard let k = FileKind(rawValue: recipe.kind), let n = sampleNames[k] else { return [] }
        let m = buildMenu(["/tmp/\(n)"]); return flatActions(m.items + m.extra).filter { $0.action != nil && $0.danger == nil && $0.action?["op"]?.string != "chain" }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(T("Recipe")).font(.title3.bold())
            TextField(T("Name"), text: $recipe.name).textFieldStyle(.roundedBorder)
            Picker(T("Runs on"), selection: $recipe.kind) { ForEach(FileKind.allCases.filter { sampleNames[$0] != nil }) { Text(T($0.title)).tag($0.rawValue) } }
            List { ForEach(recipe.steps) { s in HStack { Image(systemName: "line.3.horizontal").foregroundStyle(.secondary); Text(T(s.label)); Spacer(); Button { recipe.steps.removeAll { $0.id == s.id } } label: { Image(systemName: "minus.circle") }.buttonStyle(.plain) } }.onMove { recipe.steps.move(fromOffsets: $0, toOffset: $1) } }.frame(minHeight: 160)
            Menu(T("Add step")) { ForEach(options, id: \.self) { o in Button(T(o.label)) { recipe.steps.append(RecipeStep(label: o.label, action: o.action!, ask: o.ask)) } } }
            HStack { Spacer(); Button(T("Cancel"), action: cancel); Button(T("Save")) { save(recipe) }.keyboardShortcut(.defaultAction).disabled(recipe.name.isEmpty || recipe.steps.isEmpty) }
        }.padding(20).frame(width: 460, height: 460)
    }
}

struct WheelCustomView: View {
    @EnvironmentObject var s: Store
    @State var kind = FileKind.image
    var body: some View {
        let list = customizable(["/tmp/\(sampleNames[kind] ?? "a.bin")"])
        let hidden = Set(s.state.prefs.hidden[kind.rawValue] ?? [])
        VStack(alignment: .leading) {
            HStack { title("Wheel"); Spacer(); Button(T("Reset")) { s.update { $0.prefs.hidden[kind.rawValue] = nil; $0.prefs.center[kind.rawValue] = nil } } }
            Picker("", selection: $kind) { ForEach(FileKind.allCases.filter { sampleNames[$0] != nil }) { Text(T($0.title)).tag($0) } }.pickerStyle(.segmented)
            List(list) { it in
                HStack {
                    Image(systemName: it.symbol).frame(width: 22); Text(T(it.label)); Spacer()
                    Button(s.state.prefs.center[kind.rawValue] == it.id ? T("Centre ✓") : T("Set centre")) { s.update { $0.prefs.center[kind.rawValue] = it.id } }.controlSize(.small).disabled(it.defaultChoice == nil)
                    Toggle("", isOn: Binding(get: { !hidden.contains(it.id) }, set: { on in s.update { var h = Set($0.prefs.hidden[kind.rawValue] ?? []); if on { h.remove(it.id) } else { h.insert(it.id) }; $0.prefs.hidden[kind.rawValue] = Array(h) } })).toggleStyle(.switch).labelsHidden()
                }
            }
            Text(T("Everything is on the wheel, grouped by purpose. Turn off what you never use. Places never move; what you use most becomes the wedge's straight-drop action.")).font(.caption).foregroundStyle(.secondary)
        }.padding(20)
    }
}

struct SecretField: View {
    let label, key: String
    @State var v = ""
    @State var has = false
    var body: some View {
        HStack {
            SecureField(has ? T("Saved in Keychain") : T(label), text: $v)
            Button(T("Save")) { Keychain.set(key, v); has = !v.isEmpty; v = "" }.disabled(v.isEmpty)
            if has { Button(T("Remove")) { Keychain.set(key, ""); has = false } }
        }.onAppear { has = !Keychain.get(key).isEmpty }
    }
}

struct SettingsView: View {
    @EnvironmentObject var s: Store
    @State var login = LoginItem.enabled
    @State var ax = AXIsProcessTrusted()
    func b<T>(_ p: WritableKeyPath<AppState, T>) -> Binding<T> { Binding(get: { s.state[keyPath: p] }, set: { v in s.update { $0[keyPath: p] = v } }) }
    var body: some View {
        Form {
            Section(T("General")) {
                Toggle(T("Shift-drag menu"), isOn: b(\.enabled))
                Toggle(T("Open at login"), isOn: Binding(get: { login }, set: { v in try? LoginItem.set(v); login = LoginItem.enabled }))
                Picker(T("Appearance"), selection: b(\.theme)) { Text(T("System")).tag("system"); Text(T("Light")).tag("light"); Text(T("Dark")).tag("dark") }
                Picker(T("Accent"), selection: b(\.accent)) { ForEach(accents.keys.sorted(), id: \.self) { Text($0.capitalized).tag($0) } }
                Picker(T("Language"), selection: b(\.lang)) { Text(T("System")).tag("system"); Text("English").tag("en"); Text("Tiếng Việt").tag("vi") }
                Toggle(T("The centre repeats my last action"), isOn: b(\.repeatLast))
            }
            Section(T("Privacy")) {
                Toggle(T("Offline mode"), isOn: b(\.offline))
                Text(T("Nothing that reaches the network is offered or run: no uploads, no phone link, no online services. Everything else works as before.")).font(.caption).foregroundStyle(.secondary)
                if OnDevice.ready { Label(T("Summaries, questions and translation use the model on this Mac."), systemImage: "cpu").font(.caption).foregroundStyle(.secondary) }
            }
            Section(T("Keyboard during a drag")) {
                Toggle(T("Keys 1–8 pick a wedge while dragging"), isOn: b(\.digits))
                if s.state.digits && !ax { Button(T("Allow in Accessibility…")) { AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary); ax = AXIsProcessTrusted() } }
                Text(T("Needs Accessibility permission. Everything else works without it.")).font(.caption).foregroundStyle(.secondary)
                Text(T("⌃⌥Space opens the wheel for the Finder selection or copied files; click to choose.")).font(.caption).foregroundStyle(.secondary)
                Text(T("⌃⌥Z takes back the last thing Rotae made: it goes to the Trash.")).font(.caption).foregroundStyle(.secondary)
            }
            Section(T("Add-ons")) {
                ForEach(Tools.addons, id: \.id) { a in
                    HStack { VStack(alignment: .leading) { Text(a.label); Text(T(a.why)).font(.caption).foregroundStyle(.secondary) }; Spacer(); Text(Tools.installed(a) ? T("ready") : T("not installed")).foregroundStyle(Tools.installed(a) ? .green : .secondary) }
                }
                HStack { Text(T("Rotae uses these from Homebrew or your PATH, e.g. brew install ffmpeg qpdf.")).font(.caption).foregroundStyle(.secondary); Spacer(); Button(T("Copy install command")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString("brew install ffmpeg qpdf whisper-cpp yara-x upx", forType: .string) }.controlSize(.small) }
            }
            Section(T("Online services")) {
                SecretField(label: "VirusTotal API key", key: "vt_key")
                SecretField(label: "Claude API key", key: "anthropic_key")
                TextField(T("R2 account ID"), text: b(\.r2Account)); TextField(T("R2 bucket"), text: b(\.r2Bucket))
                SecretField(label: "R2 access key ID", key: "r2_key"); SecretField(label: "R2 secret", key: "r2_secret")
                TextField(T("Public URL"), text: b(\.r2Public))
            }
            Section(T("Command line")) {
                HStack { Text(T("Run the same actions from a terminal or a script: rotae list file.png")).font(.caption).foregroundStyle(.secondary); Spacer(); Button(T("Install command")) { CommandLineTool.install() } }
            }
            Section("YARA") { TextField(T("YARA rules folder"), text: b(\.yaraRules)) }
            Section { Button(T("Show the welcome again")) { Windows.shared.route = .welcome } }
        }.formStyle(.grouped)
    }
}

struct WelcomeView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Rotae").font(.largeTitle.bold())
            Text(T("Shift + drag any file. Pick what to do with it. Done.")).font(.title3).foregroundStyle(.secondary)
            step(1, "Press and hold on a file or folder", "In Finder or on the desktop. From other apps, start dragging the file first.")
            step(2, "Hold Shift too", "The wheel opens with its centre on the file. Let go of Shift or the mouse and it goes away.")
            step(3, "Drag the file onto an action", "The result is saved next to the original, which is never changed.")
            GroupBox(T("Try it")) {
                HStack { Text(T("Drag this into Finder, then press on it there and hold Shift.")); Spacer(); Text("sample.png").padding(8).background(.quaternary, in: RoundedRectangle(cornerRadius: 8)).overlay(FileDragHandle(paths: [sample()])) }.padding(4)
            }
            if !Tools.installedIDs.contains("ffmpeg") { Label(T("Video and audio need FFmpeg: run  brew install ffmpeg"), systemImage: "info.circle").foregroundStyle(.secondary) }
            Spacer()
            HStack { Toggle(T("Open at login"), isOn: Binding(get: { LoginItem.enabled }, set: { try? LoginItem.set($0) })); Spacer(); Button(T("Got it, let me try")) { Windows.shared.hideHub() }.keyboardShortcut(.defaultAction) }
        }.padding(28)
    }
    func step(_ n: Int, _ t: String, _ d: String) -> some View {
        HStack(alignment: .top, spacing: 12) { Text("\(n)").font(.headline).frame(width: 28, height: 28).background(Store.shared.accent.opacity(0.2), in: Circle()); VStack(alignment: .leading) { Text(T(t)).font(.headline); Text(T(d)).foregroundStyle(.secondary) } }
    }
    func sample() -> String {
        let u = supportDir.appendingPathComponent("sample.png")
        if !FileManager.default.fileExists(atPath: u.path), let i = NSImage(named: NSImage.applicationIconName), let t = i.tiffRepresentation, let p = NSBitmapImageRep(data: t)?.representation(using: .png, properties: [:]) { try? p.write(to: u) }
        return u.path
    }
}

/// One long-running helper for a share; ending the process ends the share.
final class ShareTask: ObservableObject {
    @Published var lan: EngineEvent.LanShare?
    @Published var links: [String] = []
    @Published var error: String?
    @Published var running = false
    var p: Process?
    func start(_ verb: String, _ paths: [String]) {
        running = true
        let pr = Process(), i = Pipe(), o = Pipe(); pr.executableURL = Jobs.shared.engine()
        pr.environment = ["PATH": Tools.searchPath, "HOME": NSHomeDirectory(), "LANG": "en_US.UTF-8"]; pr.standardInput = i; pr.standardOutput = o; pr.standardError = FileHandle.nullDevice
        var buf = Data()
        o.fileHandleForReading.readabilityHandler = { [weak self] h in
            let d = h.availableData; if d.isEmpty { h.readabilityHandler = nil; DispatchQueue.main.async { self?.running = false }; return }
            buf.append(d)
            while let n = buf.firstIndex(of: 10) {
                let l = Data(buf[buf.startIndex..<n]); buf = Data(buf[buf.index(after: n)...])
                if let e = EngineEvent.parse(l) { DispatchQueue.main.async { self?.on(e) } }
            }
        }
        guard (try? pr.run()) != nil else { error = "crashed"; running = false; return }
        p = pr
        let req: [String: Any] = ["v": 1, "verb": verb, "paths": paths, "secrets": Keychain.all, "settings": Store.shared.settingsForEngine]
        if var d = try? JSONSerialization.data(withJSONObject: req) { d.append(10); try? i.fileHandleForWriting.write(contentsOf: d) }
        stdinKeep = i
    }
    var stdinKeep: Pipe?
    func on(_ e: EngineEvent) {
        switch e.event {
        case "share": lan = e.lan
        case "clipboard": NSPasteboard.general.clearContents(); NSPasteboard.general.setString(e.text ?? "", forType: .string)
        case "done": links = e.links ?? []
        case "error": error = e.code
        default: break
        }
    }
    func stop() { if let p, p.isRunning { kill(p.processIdentifier, SIGTERM) }; p = nil; running = false; lan = nil }
    deinit { stop() }
}

struct FileList: View { let paths: [String]; var body: some View { VStack(alignment: .leading) { ForEach(paths.prefix(6), id: \.self) { p in HStack { ThumbView(path: p, size: 28); Text((p as NSString).lastPathComponent).lineLimit(1) } }; if paths.count > 6 { Text("+\(paths.count - 6)").foregroundStyle(.secondary) } } } }

struct PhoneView: View {
    let paths: [String]
    @StateObject var t = ShareTask()
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            title("To phone"); FileList(paths: paths)
            if let l = t.lan {
                if let q = qrImage(l.url) { Image(nsImage: q).interpolation(.none).resizable().frame(width: 220, height: 220) }
                Text(l.url).font(.caption.monospaced()).textSelection(.enabled)
                Text("\(T("Scan it on the same Wi-Fi. The link closes after")) \(l.minutes) \(T("minutes"))").foregroundStyle(.secondary)
                HStack { Button(T("Copy link")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(l.url, forType: .string) }; Button(T("Stop sharing")) { t.stop() } }
            } else if let e = t.error { Text(T(explain(e).why)).foregroundStyle(.red); Text(T(explain(e).how)).foregroundStyle(.secondary) }
            else if t.running { ProgressView() }
            else {
                Text(T("Opens a one-time link on your own network only. Nothing goes to the internet.")).foregroundStyle(.secondary)
                HStack {
                    Button(T("Create link")) { t.start("share_lan", paths) }.keyboardShortcut(.defaultAction)
                    AirDropButton(paths: paths)
                }
            }
            Spacer()
        }.padding(20).onDisappear { t.stop() }
    }
}
struct AirDropButton: View {
    let paths: [String]
    var body: some View { Button("AirDrop") { NSSharingService(named: .sendViaAirDrop)?.perform(withItems: paths.map { URL(fileURLWithPath: $0) }) } }
}
struct UploadView: View {
    let paths: [String]
    @StateObject var t = ShareTask()
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            title("Upload link"); FileList(paths: paths)
            Text("\(T("Uploads to your Cloudflare R2 bucket:")) \(Store.shared.state.r2Bucket)").foregroundStyle(.secondary)
            if !t.links.isEmpty { Label(T("Links copied"), systemImage: "checkmark.circle").foregroundStyle(.green); ForEach(t.links, id: \.self) { Text($0).font(.caption.monospaced()).textSelection(.enabled) } }
            else if let e = t.error { Text(T(explain(e).why)).foregroundStyle(.red); Button(T("Open Settings")) { Windows.shared.route = .settings } }
            else if t.running { ProgressView() }
            else { Button(T("Upload")) { t.start("share_r2", paths) }.keyboardShortcut(.defaultAction) }
            Spacer()
        }.padding(20)
    }
}
