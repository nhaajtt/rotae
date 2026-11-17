import AppKit
import SDCore
import ServiceManagement
import SwiftUI

@main
enum Main {
    static func main() {
        signal(SIGPIPE, SIG_IGN) // a helper that died must not take the app with it
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        #if DEBUG
        if CommandLine.arguments.count > 2, CommandLine.arguments[1] == "--render" { renderForReview(CommandLine.arguments[2]); return }
        #endif
        let d = AppDelegate()
        app.delegate = d
        app.run()
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    let drag = DragWatcher()
    var status: StatusBar!
    var watcher: FolderWatcher!

    func applicationDidFinishLaunching(_ n: Notification) {
        let s = Store.shared
        s.applyTheme()
        status = StatusBar()
        status.onDrop = { [weak self] p in self?.drag.wheel.open(paths: p, at: NSEvent.mouseLocation, click: true) }
        drag.start()
        watcher = FolderWatcher()
        watcher.reload()
        Toasts.shared.attach()
        Self.sweepIntentFiles()
        NSApp.servicesProvider = self
        NSUpdateDynamicServices()
        NSAppleEventManager.shared().setEventHandler(self, andSelector: #selector(url(_:reply:)), forEventClass: AEEventClass(kInternetEventClass), andEventID: AEEventID(kAEGetURL))
        #if DEBUG
        if CommandLine.arguments.count > 2, CommandLine.arguments[1] == "--demo" { runDemo(CommandLine.arguments[2], drag: drag); return }
        #endif
        if s.state.firstRun {
            s.update { $0.firstRun = false }
            Windows.shared.hub(.welcome)
        }
    }

    /// Files that Shortcuts handed over as bytes are kept a day, long enough for it to read the results.
    static func sweepIntentFiles() {
        DispatchQueue.global(qos: .utility).async {
            let fm = FileManager.default, tmp = fm.temporaryDirectory
            for u in (try? fm.contentsOfDirectory(at: tmp, includingPropertiesForKeys: [.contentModificationDateKey])) ?? [] where u.lastPathComponent.hasPrefix("rotae-intent-") {
                if let d = try? u.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate, d.timeIntervalSinceNow < -86400 { try? fm.removeItem(at: u) }
            }
        }
    }

    func application(_ a: NSApplication, open urls: [URL]) {
        let recipes = urls.filter { $0.pathExtension == RecipeFile.ext }
        if !recipes.isEmpty { RecipeFile.read(recipes) }
        let rest = urls.filter { $0.pathExtension != RecipeFile.ext }
        if rest.isEmpty { return }
        Coalescer.shared.add(rest.map(\.path))
    }

    /// Cmd-Q or Log Out: end the helpers and remove what they were writing.
    func applicationWillTerminate(_ n: Notification) { Jobs.shared.stopEverything() }

    /// Finder › Services › Rotae.
    @objc func openInRotae(_ pb: NSPasteboard, userData: String, error: AutoreleasingUnsafeMutablePointer<NSString>) {
        let u = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
        Coalescer.shared.add(u.map(\.path))
    }

    /// rotae://run?recipe=<name>&path=<p>&path=<p> for Shortcuts and scripts.
    @objc func url(_ e: NSAppleEventDescriptor, reply: NSAppleEventDescriptor) {
        guard let s = e.paramDescriptor(forKeyword: keyDirectObject)?.stringValue, let c = URLComponents(string: s), c.host == "run" else { return }
        let q = c.queryItems ?? []
        let paths = q.filter { $0.name == "path" }.compactMap(\.value).filter { FileManager.default.fileExists(atPath: $0) }
        guard let name = q.first(where: { $0.name == "recipe" })?.value, !paths.isEmpty,
              let r = Store.shared.state.recipes.first(where: { $0.name == name }) else { return }
        // a link can come from any web page: ask, because a recipe can change files
        let a = NSAlert(); a.messageText = T("Run this recipe?"); a.informativeText = "\(r.name)\n\(paths.joined(separator: "\n"))"
        a.addButton(withTitle: T("Run")); a.addButton(withTitle: T("Cancel"))
        NSApp.activate(ignoringOtherApps: true)
        if a.runModal() == .alertFirstButtonReturn {
            Jobs.shared.queue(paths, act("chain", ["steps": .a(r.steps.map { .o($0.action) })]), title: r.name)
        }
    }
}

/// Files handed over one by one within a moment become one selection.
final class Coalescer {
    static let shared = Coalescer()
    var paths: [String] = []
    func add(_ p: [String]) {
        if paths.isEmpty { DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { [self] in let x = paths; paths = []; Windows.shared.hub(.actions(x)) } }
        paths += p
    }
}

final class StatusBar: NSObject, NSMenuDelegate, NSWindowDelegate, NSDraggingDestination {
    var onDrop: (([String]) -> Void)?
    /// Dropping files on the menu bar icon opens the wheel for them: no Shift needed.
    func draggingEntered(_ s: NSDraggingInfo) -> NSDragOperation { Store.shared.state.enabled ? .copy : [] }
    func performDragOperation(_ s: NSDraggingInfo) -> Bool {
        let p = (s.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []).map(\.path)
        guard !p.isEmpty else { return false }
        DispatchQueue.main.async { [self] in onDrop?(p) }
        return true
    }
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    var observer: Any?
    override init() {
        super.init()
        item.button?.image = NSImage(systemSymbolName: "circle.circle", accessibilityDescription: "Rotae")
        item.button?.imagePosition = .imageLeading
        let m = NSMenu(); m.delegate = self; item.menu = m
        item.button?.window?.registerForDraggedTypes([.fileURL])
        item.button?.window?.delegate = self
        Jobs.shared.onChange = { [weak self] in self?.refresh() }
    }
    func refresh() {
        let j = Jobs.shared
        let running = j.jobs.filter(\.isActive)
        if running.isEmpty { item.button?.title = "" } else {
            let f = running.map(\.progress).reduce(0, +) / Double(running.count)
            item.button?.title = " \(Int(f * 100))%" + (running.count > 1 ? " ·\(running.count)" : "")
        }
    }
    func add(_ m: NSMenu, _ title: String, _ sel: Selector, state: Bool? = nil) {
        let i = NSMenuItem(title: T(title), action: sel, keyEquivalent: ""); i.target = self
        if let state { i.state = state ? .on : .off }
        m.addItem(i)
    }
    func menuNeedsUpdate(_ m: NSMenu) {
        m.removeAllItems()
        for job in Jobs.shared.jobs.filter(\.isActive).prefix(5) {
            let i = NSMenuItem(title: "\(T(job.title)) — \(Int(job.progress * 100))%  ✕", action: #selector(cancelJob(_:)), keyEquivalent: "")
            i.target = self; i.representedObject = job; m.addItem(i)
        }
        if Jobs.shared.jobs.contains(where: \.isActive) { m.addItem(.separator()) }
        add(m, "Open Rotae", #selector(openHub))
        add(m, "Activity", #selector(openActivity))
        add(m, "Shelf", #selector(openShelf))
        add(m, "From phone…", #selector(openReceive))
        m.addItem(.separator())
        add(m, "Shift-drag menu", #selector(toggle), state: Store.shared.state.enabled)
        add(m, "Settings…", #selector(openSettings))
        m.addItem(.separator())
        add(m, "Quit Rotae", #selector(quit))
    }
    @objc func cancelJob(_ i: NSMenuItem) { (i.representedObject as? Job).map { Jobs.shared.cancel($0) } }
    @objc func openHub() { Windows.shared.hub(.actions([])) }
    @objc func openActivity() { Windows.shared.hub(.activity) }
    @objc func openShelf() { Shelf.shared.show() }
    @objc func openReceive() { Windows.shared.hub(.receive) }
    @objc func openSettings() { Windows.shared.hub(.settings) }
    @objc func toggle() { Store.shared.update { $0.enabled.toggle() } }
    @objc func quit() { NSApp.terminate(nil) }
}

enum LoginItem {
    static var enabled: Bool { SMAppService.mainApp.status == .enabled }
    static func set(_ on: Bool) throws { if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() } }
}
