import AppKit
import CoreImage
import QuickLookThumbnailing
import QuickLookUI
import SDCore
import SwiftUI

enum Thumb {
    static func load(_ path: String, size: CGFloat, _ done: @escaping (NSImage?) -> Void) {
        let r = QLThumbnailGenerator.Request(fileAt: URL(fileURLWithPath: path), size: CGSize(width: size, height: size), scale: 2, representationTypes: .all)
        QLThumbnailGenerator.shared.generateBestRepresentation(for: r) { t, _ in DispatchQueue.main.async { done(t?.nsImage) } }
    }
}

struct ThumbView: View {
    let path: String; var size: CGFloat = 40
    @State var img: NSImage?
    var body: some View {
        Group {
            if let img { Image(nsImage: img).resizable().aspectRatio(contentMode: .fit) }
            else { Image(nsImage: NSWorkspace.shared.icon(forFile: path)).resizable() }
        }.frame(width: size, height: size).clipShape(RoundedRectangle(cornerRadius: 6)).onAppear { Thumb.load(path, size: size) { img = $0 } }
    }
}

enum Actions {
    /// What choosing any action means, from the wheel, the Hub or the Shelf.
    static func run(_ item: MenuItem, paths all: [String], kind: FileKind) {
        let paths = item.only.map { k in all.filter { kindOf($0).rawValue == k } } ?? all
        guard !paths.isEmpty else { return }
        if let l = item.local { local(l, paths); return }
        if let e = item.editor { Windows.shared.editor(e, path: paths[0]); return }
        if let id = item.app, let a = knownApps.first(where: { $0.id == id }), let u = NSWorkspace.shared.urlForApplication(withBundleIdentifier: a.bundle) {
            NSWorkspace.shared.open(paths.map { URL(fileURLWithPath: $0) }, withApplicationAt: u, configuration: NSWorkspace.OpenConfiguration()); return
        }
        if item.shelf { Shelf.shared.add(paths); return }
        if item.share != nil, Store.shared.state.offline { Toasts.shared.say(T("Offline mode is on: this needs the network"), ok: false); return }
        if item.share == "phone" { Windows.shared.hub(.phone(paths)); return }
        if item.share == "upload" { Windows.shared.hub(.upload(paths)); return }
        if !item.ask.isEmpty || item.danger != nil { Windows.shared.hub(.ask(item, paths)); return }
        if let a = item.action { Jobs.shared.queue(paths, a, title: item.title ?? item.label) }
    }
}

extension Actions {
    static func local(_ id: String, _ paths: [String]) {
        let urls = paths.map { URL(fileURLWithPath: $0) }, fm = FileManager.default
        switch id {
        case "open": urls.forEach { NSWorkspace.shared.open($0) }
        case "reveal": revealAll(paths)
        case "peek": Peek.shared.show(paths)
        case "rename": Windows.shared.hub(.rename(paths))
        case "copy-file":
            NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects(urls as [NSURL]); Toasts.shared.say("Copied")
        case "copy-image":
            if let i = NSImage(contentsOf: urls[0]) { NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects([i]); Toasts.shared.say("Copied") }
        case "wallpaper":
            if let s = NSScreen.main { let ok = (try? NSWorkspace.shared.setDesktopImageURL(urls[0], for: s, options: [:])) != nil; Toasts.shared.say(ok ? "Wallpaper set" : "Couldn't set the wallpaper", ok: ok) }
        case "duplicate":
            var made: [String] = []
            for p in paths {
                let d = (p as NSString).deletingLastPathComponent, e = (p as NSString).pathExtension, s = ((p as NSString).lastPathComponent as NSString).deletingPathExtension
                var n = 1, out: String
                repeat { out = "\(d)/\(s) copy\(n == 1 ? "" : " \(n)")\(e.isEmpty ? "" : ".\(e)")"; n += 1 } while fm.fileExists(atPath: out)
                if (try? fm.copyItem(atPath: p, toPath: out)) != nil { made.append(out) }
            }
            if !made.isEmpty { revealAll(made) }
        case "trash":
            let n = urls.filter { (try? fm.trashItem(at: $0, resultingItemURL: nil)) != nil }.count
            Toasts.shared.say(n == urls.count ? "Moved to Trash" : "Couldn't move everything to Trash", ok: n == urls.count)
        case "terminal":
            // Terminal opens a folder as a new window already standing in it
            let dirs = Set(urls.map { u -> URL in var d: ObjCBool = false; return fm.fileExists(atPath: u.path, isDirectory: &d) && d.boolValue ? u : u.deletingLastPathComponent() })
            if let t = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Terminal") { NSWorkspace.shared.open(Array(dirs.prefix(3)), withApplicationAt: t, configuration: NSWorkspace.OpenConfiguration()) }
        case "chmod":
            var n = 0
            for p in paths {
                guard let mode = (try? fm.attributesOfItem(atPath: p))?[.posixPermissions] as? NSNumber else { continue }
                // whoever may read it may run it
                let m = mode.intValue | ((mode.intValue & 0o444) >> 2)
                if (try? fm.setAttributes([.posixPermissions: m], ofItemAtPath: p)) != nil { n += 1 }
            }
            Toasts.shared.say(n > 0 ? "Can be run now" : "Couldn't change this file", ok: n > 0)
        case "verify":
            // the checksum the user copied from the download page travels with the request
            let pasted = String((NSPasteboard.general.string(forType: .string) ?? "").prefix(600))
            Jobs.shared.queue(paths, act("native", ["verb": .s("verify"), "arg": .s(pasted)]), title: "Checking the download")
        case "airdrop": NSSharingService(named: .sendViaAirDrop)?.perform(withItems: urls)
        case "mail": NSSharingService(named: .composeEmail)?.perform(withItems: urls)
        default:
            if id.hasPrefix("tag:") {
                let tag = String(id.dropFirst(4))
                for u in urls { try? (u as NSURL).setResourceValue(tag.isEmpty ? [] : [tag], forKey: .tagNamesKey) }
                Toasts.shared.say(tag.isEmpty ? "Tags removed" : "Tagged")
            }
        }
    }
}

/// The floating card that follows a job; never takes focus away from what you're doing.
final class FloatPanel: NSPanel {
    override var canBecomeKey: Bool { false }
}

final class Toasts: ObservableObject {
    static let shared = Toasts()
    @Published var live: Job?
    @Published var record: Record?
    /// A one-line confirmation for things that finish at once (Copied, Moved to Trash…).
    @Published var flash: (text: String, ok: Bool)?
    var queue: [Job] = []
    var panel: FloatPanel!
    var hide: DispatchWorkItem?
    @Published var hovering = false
    /// Clear space around the card, inside the panel.
    static var margin: CGFloat { Liquid.available ? 22 : 0 }
    /// Counts every show and hide, so a fade-out that is overtaken by a new card does nothing.
    private var turn = 0

    func attach() {
        panel = FloatPanel(contentRect: NSRect(x: 0, y: 0, width: 380, height: 150), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        // real glass casts its own shadow, and needs room around the card to draw it
        panel.isOpaque = false; panel.backgroundColor = .clear; panel.level = .floating; panel.hasShadow = !Liquid.available; panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        let host = NSHostingView(rootView: ToastView().environmentObject(self))
        host.sizingOptions = [.intrinsicContentSize]
        panel.contentView = host
        Jobs.shared.onFinished = { [weak self] j in self?.finished(j) }
    }
    func target() -> NSRect {
        panel.layoutIfNeeded()
        let s = panel.contentView!.fittingSize
        let f = (NSScreen.screens.first { $0.frame.contains(NSEvent.mouseLocation) } ?? NSScreen.main!).visibleFrame
        let m = 16 - Toasts.margin
        return NSRect(x: f.maxX - s.width - m, y: f.minY + m, width: s.width, height: s.height)
    }
    func track(_ j: Job) {
        hide?.cancel()
        if live == nil && record?.state != .failed { set(live: j, record: nil, flash: nil); show() } else if live !== j { queue.append(j) }
    }
    /// What the card shows changes in one animated step.
    private func set(live l: Job?, record r: Record?, flash f: (text: String, ok: Bool)?) {
        Motion.animate(Motion.swap) { live = l; record = r; flash = f }
    }
    func say(_ text: String, ok: Bool = true) {
        // never over a job that is running or an error still waiting to be read
        guard live == nil, record?.state != .failed else { return }
        set(live: nil, record: nil, flash: (text, ok)); show(); scheduleHide(1.5)
    }
    /// Arrives rising into the corner; if it is already there it only resizes to its new content.
    func show() {
        turn += 1
        DispatchQueue.main.async { [self] in Motion.present(panel, at: target()) }
    }
    func finished(_ j: Job) {
        queue.removeAll { $0 === j }
        guard live === j || live == nil else { return }
        if j.state == .cancelled { live = nil; dismiss(); return }
        set(live: nil, record: Jobs.shared.history.first, flash: nil)
        show()
        if j.state == .done { scheduleHide(3.5) }
    }
    func scheduleHide(_ t: Double) {
        hide?.cancel()
        let w = DispatchWorkItem { [weak self] in
            guard let self else { return }
            if hovering { scheduleHide(1.5) } else { dismiss() }
        }
        hide = w; DispatchQueue.main.asyncAfter(deadline: .now() + t, execute: w)
    }
    func dismiss() {
        hide?.cancel()
        if let n = queue.first { queue.removeFirst(); set(live: n, record: nil, flash: nil); show(); return }
        turn += 1
        let mine = turn
        Motion.dismiss(panel, still: { [weak self] in self?.turn == mine }) { [weak self] in
            guard let self, turn == mine else { return }
            record = nil; flash = nil
        }
    }
}

func revealAll(_ paths: [String]) { NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) }) }

struct FileDragHandle: NSViewRepresentable {
    let paths: [String]
    func makeNSView(context: Context) -> V { V() }
    func updateNSView(_ v: V, context: Context) { v.paths = paths }
    final class V: NSView, NSDraggingSource {
        var paths: [String] = []
        func draggingSession(_ s: NSDraggingSession, sourceOperationMaskFor c: NSDraggingContext) -> NSDragOperation { .copy }
        override func mouseDragged(with e: NSEvent) {
            let items = paths.map { p -> NSDraggingItem in
                let i = NSDraggingItem(pasteboardWriter: URL(fileURLWithPath: p) as NSURL)
                i.setDraggingFrame(NSRect(x: 0, y: 0, width: 32, height: 32), contents: NSWorkspace.shared.icon(forFile: p))
                return i
            }
            beginDraggingSession(with: items, event: e, source: self)
        }
        override var mouseDownCanMoveWindow: Bool { false }
    }
}

struct ToastView: View {
    @EnvironmentObject var t: Toasts
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let j = t.live { LiveRow(job: j).transition(.opacity) }
            else if let f = t.flash {
                HStack(spacing: 8) {
                    Image(systemName: f.ok ? "checkmark.circle.fill" : "exclamationmark.triangle.fill").foregroundStyle(f.ok ? Color.green : Color.orange).font(.title3)
                    Text(T(f.text)).font(.headline)
                }.transition(.opacity)
            }
            else if let r = t.record { DoneRow(r: r).transition(.opacity) }
        }
        .padding(14).frame(width: t.live == nil && t.flash != nil ? nil : 380, alignment: .leading)
        .liquidGlass(cornerRadius: 22)
        .onHover { t.hovering = $0 }
        .padding(Toasts.margin)
    }
}
struct LiveRow: View {
    @ObservedObject var job: Job
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(T(job.title)).font(.headline)
                Spacer()
                if job.unitsTotal > 1 { Text("\(job.unitsDone)/\(job.unitsTotal)").foregroundStyle(.secondary).monospacedDigit().contentTransition(.numericText()) }
                Button(T("Cancel")) { Jobs.shared.cancel(job) }.controlSize(.small)
            }
            // the bar flows between reports instead of stepping
            ProgressView(value: min(job.progress, 1)).tint(Store.shared.accent).animation(Motion.reduced ? nil : .linear(duration: 0.25), value: job.progress)
        }
    }
}
struct DoneRow: View {
    let r: Record
    var paths: [String] { r.outputs.map(\.path) }
    var body: some View {
        let ex = explain(r.code ?? "")
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                if let p = paths.first { ThumbView(path: p, size: 44).overlay(FileDragHandle(paths: paths)) }
                VStack(alignment: .leading, spacing: 2) {
                    Text(r.state == .failed ? "\(T("Couldn't finish")): \(T(r.title))" : T(r.title)).font(.headline).lineLimit(2)
                    if r.state != .failed, let n = r.notes.first { Text(n).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
                    if let d = delta { Text(d).font(.caption).foregroundStyle(.secondary) }
                }
                Spacer()
                Button { Toasts.shared.dismiss() } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
            }
            if r.state == .failed || r.state == .partial {
                Text(T(ex.why)).font(.callout)
                Text(T(ex.how)).font(.caption).foregroundStyle(.secondary)
                if let d = r.error, r.code == "unknown" || r.code == "ffmpeg_failed" { Text(d).font(.caption2.monospaced()).lineLimit(3).textSelection(.enabled) }
            }
            HStack {
                if !paths.isEmpty {
                    Button(T("Show")) { revealAll(paths) }
                    Button(T("Peek")) { Peek.shared.show(paths) }
                    Button(T("Share")) { share(paths) }
                    Button(T("Undo")) { _ = Jobs.shared.undo(r); Toasts.shared.dismiss() }
                    Button(T("Next…")) { Windows.shared.hub(.actions(paths)); Toasts.shared.dismiss() }
                }
                if r.state == .failed {
                    if ex.addon { Button(T("Open Settings")) { Windows.shared.hub(.settings) } }
                    if r.retry != nil { Button(T("Try again")) { Jobs.shared.retry(r); Toasts.shared.dismiss() } }
                }
            }.controlSize(.small).buttonBorderShape(.capsule)
        }
    }
    /// How much smaller or bigger the result is.
    var delta: String? {
        guard r.paths.count == 1, r.outputs.count == 1, let a = fileStamp(r.paths[0])?.0, a > 0 else { return nil }
        let b = r.outputs[0].size, f = ByteCountFormatter()
        let pct = Int((Double(b) / Double(a) - 1) * 100)
        return "\(f.string(fromByteCount: a)) → \(f.string(fromByteCount: b)) (\(pct > 0 ? "+" : "")\(pct)%)"
    }
    func share(_ p: [String]) {
        guard let v = Toasts.shared.panel.contentView else { return }
        NSSharingServicePicker(items: p.map { URL(fileURLWithPath: $0) }).show(relativeTo: v.bounds, of: v, preferredEdge: .minY)
    }
}

/// Quick Look with Rotae's results, including several at once.
final class Peek: NSObject, QLPreviewPanelDataSource {
    static let shared = Peek()
    var urls: [URL] = []
    func show(_ paths: [String]) {
        urls = paths.map { URL(fileURLWithPath: $0) }
        NSApp.activate(ignoringOtherApps: true)
        guard let p = QLPreviewPanel.shared() else { return }
        p.dataSource = self; p.reloadData(); p.currentPreviewItemIndex = 0; p.makeKeyAndOrderFront(nil)
    }
    func numberOfPreviewItems(in p: QLPreviewPanel!) -> Int { urls.count }
    func previewPanel(_ p: QLPreviewPanel!, previewItemAt i: Int) -> QLPreviewItem! { urls[i] as NSURL }
}

final class Shelf: ObservableObject {
    static let shared = Shelf()
    @Published var paths: [String] = Store.shared.state.shelf.filter { FileManager.default.fileExists(atPath: $0) }
    @Published var selected = Set<String>()
    var panel: NSPanel?
    func add(_ p: [String]) { Motion.animate(Motion.settle) { paths += p.filter { !paths.contains($0) } }; save(); show() }
    func save() { Store.shared.update { $0.shelf = paths } }
    func show() {
        if panel == nil {
            let w = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 300, height: 460), styleMask: [.titled, .closable, .resizable, .utilityWindow, .nonactivatingPanel, .fullSizeContentView], backing: .buffered, defer: false)
            w.titlebarAppearsTransparent = true; w.titleVisibility = .hidden; w.isMovableByWindowBackground = true
            w.level = .floating; w.hidesOnDeactivate = false; w.isReleasedWhenClosed = false
            w.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
            if Liquid.available { w.isOpaque = false; w.backgroundColor = .clear }
            w.contentView = NSHostingView(rootView: ShelfView().environmentObject(self))
            panel = w
            // on the screen you're working on, not always the first one
            let f = (NSScreen.screens.first { $0.frame.contains(NSEvent.mouseLocation) } ?? NSScreen.main!).visibleFrame
            w.setFrameOrigin(NSPoint(x: f.maxX - 316, y: f.midY - 230))
            w.alphaValue = 0
        }
        if let w = panel { Motion.present(w, at: w.frame, rise: 24, from: .maxX) }
    }
}
struct ShelfView: View {
    @EnvironmentObject var s: Shelf
    @State var targeted = false
    @State var confirm = false
    var chosen: [String] { s.selected.isEmpty ? s.paths : s.paths.filter(s.selected.contains) }
    var body: some View {
        VStack(spacing: 0) {
            Text("\(T("Shelf")) · \(s.paths.count)").font(.headline).padding(.top, 8).padding(.bottom, 8)
            if s.paths.isEmpty { Spacer(); Text(T("Drop files here to keep them for later")).foregroundStyle(.secondary).multilineTextAlignment(.center).padding(); Spacer() }
            else {
                List(s.paths, id: \.self, selection: $s.selected) { p in
                    HStack {
                        ThumbView(path: p, size: 32)
                        Text((p as NSString).lastPathComponent).lineLimit(1).truncationMode(.middle)
                        Spacer()
                        Button { Motion.animate(Motion.settle) { s.paths.removeAll { $0 == p } }; s.save() } label: { Image(systemName: "minus.circle") }.buttonStyle(.plain)
                    }.overlay(FileDragHandle(paths: s.selected.contains(p) ? chosen : [p]).padding(.trailing, 30))
                }.scrollContentBackground(.hidden)
            }
            HStack {
                Text(T("Drag all")).padding(.horizontal, 8).padding(.vertical, 4).background(.quaternary, in: RoundedRectangle(cornerRadius: 6)).overlay(FileDragHandle(paths: chosen))
                Button(T("Actions")) { Windows.shared.hub(.actions(chosen)) }.disabled(s.paths.isEmpty)
                Spacer()
                Button(T("Clear")) { confirm = true }.disabled(s.paths.isEmpty)
            }.padding(8).controlSize(.small)
        }
        .panelGlass()
        .overlay(targeted ? RoundedRectangle(cornerRadius: 10).stroke(Store.shared.accent, style: StrokeStyle(lineWidth: 2, dash: [6])).padding(4) : nil)
        .onDrop(of: [.fileURL], isTargeted: $targeted) { loadDropped($0) { s.add($0) }; return true }
        .confirmationDialog(T("Remove everything from the Shelf? The files themselves stay where they are."), isPresented: $confirm) { Button(T("Clear"), role: .destructive) { Motion.animate(Motion.settle) { s.paths = []; s.selected = [] }; s.save() } }
    }
}
func loadDropped(_ providers: [NSItemProvider], _ done: @escaping ([String]) -> Void) {
    var out: [String] = []; let g = DispatchGroup()
    for p in providers {
        g.enter()
        _ = p.loadObject(ofClass: URL.self) { u, _ in if let u, u.isFileURL { DispatchQueue.main.async { out.append(u.path) } }; g.leave() }
    }
    g.notify(queue: .main) { DispatchQueue.main.async { done(out) } }
}

func qrImage(_ s: String) -> NSImage? {
    guard let f = CIFilter(name: "CIQRCodeGenerator", parameters: ["inputMessage": Data(s.utf8), "inputCorrectionLevel": "M"]), let o = f.outputImage?.transformed(by: .init(scaleX: 10, y: 10)) else { return nil }
    let r = NSCIImageRep(ciImage: o); let i = NSImage(size: r.size); i.addRepresentation(r); return i
}
