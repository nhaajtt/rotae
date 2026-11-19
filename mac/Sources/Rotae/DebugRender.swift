#if DEBUG
import AppIntents
import AppKit
import SDCore
import SwiftUI

/// Debug builds only: draws surfaces to PNGs so they can be reviewed without Screen Recording.
func renderForReview(_ dir: String) {
    func save(_ v: NSView, _ name: String, backdrop: Bool = false) {
        v.layoutSubtreeIfNeeded()
        let r = v.bitmapImageRepForCachingDisplay(in: v.bounds)!
        v.cacheDisplay(in: v.bounds, to: r)
        let out = NSImage(size: v.bounds.size)
        out.lockFocus()
        if backdrop { NSGradient(colors: [.systemIndigo, .systemTeal])?.draw(in: v.bounds, angle: 60) }
        r.draw(in: v.bounds)
        out.unlockFocus()
        let png = NSBitmapImageRep(data: out.tiffRepresentation!)!.representation(using: .png, properties: [:])!
        try? png.write(to: URL(fileURLWithPath: "\(dir)/\(name).png"))
    }
    let s = menuFor(["/tmp/a.png"], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0)
    print(s.items.map { "\($0.label):\($0.sub.count)" }, "total actions", leaves(s.items).count)
    func wheel(_ hot: WheelHit, _ name: String) {
        let v = WheelView(frame: NSRect(x: 0, y: 0, width: 640, height: 640))
        v.load(s, paths: ["/tmp/a.png"], click: false)
        v.hot = hot
        // a second pass, as a moving pointer would: an open ring is what makes the next one hittable
        v.content.needsDisplay = true
        save(v, name, backdrop: true)
    }
    wheel(.none, "wheel-closed")
    let edit = s.items.firstIndex { $0.id == "g-edit" } ?? 0
    wheel(.sub(edit, s.items[edit].sub.firstIndex { !$0.sub.isEmpty } ?? 0), "wheel-open")
    let lang = Store.shared.state.lang
    Store.shared.update { $0.lang = "vi" }
    wheel(.wedge(edit), "wheel-vi")
    Store.shared.update { $0.lang = lang }
    let hub = NSHostingView(rootView: HubView().environmentObject(Windows.shared).environmentObject(Store.shared).environmentObject(Jobs.shared))
    hub.frame = NSRect(x: 0, y: 0, width: 860, height: 620)
    let w = NSWindow(contentRect: hub.frame, styleMask: [.titled], backing: .buffered, defer: false); w.contentView = hub
    Windows.shared.route = .settings
    RunLoop.main.run(until: Date().addingTimeInterval(0.6))
    save(hub, "hub-settings")
    Windows.shared.route = .wheel
    RunLoop.main.run(until: Date().addingTimeInterval(0.6))
    save(hub, "hub-wheel")
    exit(0)
}

/// Debug builds only: puts the real surfaces on screen in a fixed state, to be photographed.
/// `--demo wheel:<file>:<state>` where state is closed, wedge, sub, leaf or centre.
var demoKeep: AnyObject?
func runDemo(_ spec: String, drag: DragWatcher) {
    let part = spec.split(separator: ":", maxSplits: 2).map(String.init)
    if let l = ProcessInfo.processInfo.environment["SD_LANG"] { forcedLang = l }
    if let t = ProcessInfo.processInfo.environment["SD_THEME"] { NSApp.appearance = NSAppearance(named: t == "light" ? .aqua : .darkAqua) }
    let scr = NSScreen.main!.visibleFrame, mid = NSPoint(x: scr.midX, y: scr.midY)
    switch part[0] {
    case "bench":
        // how long one frame of the wheel's labels takes to draw, with rings open: the part of
        // the wheel that is ours to keep fast
        let s = menuFor([part.count > 1 ? part[1] : "/tmp/a.png"], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0)
        let v = WheelView(frame: NSRect(x: 0, y: 0, width: 640, height: 640))
        v.load(s, paths: ["/tmp/a.png"], click: false)
        let edit = s.items.firstIndex { $0.id == "g-edit" } ?? 0
        v.hot = .sub(edit, s.items[edit].sub.firstIndex { !$0.sub.isEmpty } ?? 0)
        let rep = v.content.bitmapImageRepForCachingDisplay(in: v.content.bounds)!
        v.content.cacheDisplay(in: v.content.bounds, to: rep)
        var worst = 0.0
        let n = 300, t0 = CACurrentMediaTime()
        for i in 0..<n {
            let a = CACurrentMediaTime()
            v.hot = i % 2 == 0 ? .wedge(i % s.items.count) : .sub(edit, i % max(1, s.items[edit].sub.count))
            v.content.cacheDisplay(in: v.content.bounds, to: rep)
            worst = max(worst, CACurrentMediaTime() - a)
        }
        let avg = (CACurrentMediaTime() - t0) / Double(n) * 1000
        let t1 = CACurrentMediaTime()
        for _ in 0..<n { v.content.cacheDisplay(in: v.content.bounds, to: rep) }
        print(String(format: "BENCH redraw alone %.2f ms", (CACurrentMediaTime() - t1) / Double(n) * 1000))
        print(String(format: "BENCH avg %.2f ms  worst %.2f ms  (budget 8.3 ms at 120 Hz)", avg, worst * 1000))
        exit(avg < 8.3 ? 0 : 1)
    case "look":
        let s = menuFor([part[1]], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0)
        let picks = leaves(s.items).filter { $0.action != nil && Forecast.worth($0.action!, path: part[1]) }.prefix(4)
        var left = picks.count
        if left == 0 { print("LOOK nothing worth previewing"); exit(0) }
        func next(_ i: Int) {
            guard i < picks.count else { return }
            let it = picks[picks.startIndex + i]
            Forecast.shared.ask(it.action!, path: part[1]) { t in print("LOOK \(it.title ?? it.label): \(t ?? "-")"); left -= 1; if left == 0 { print("LOOK isScan=\(Forecast.isScan(part[1])) temp left: \(((try? FileManager.default.contentsOfDirectory(atPath: NSTemporaryDirectory())) ?? []).filter { $0.hasPrefix("rotae-look") }.count)"); exit(0) }; next(i + 1) }
        }
        next(0)
    case "inbox":
        // headless: the address goes to stderr so a script can post to it
        let b = Inbox(); demoKeep = b
        b.start()
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { FileHandle.standardError.write(Data("INBOX \(b.url ?? b.error ?? "?")\n".utf8)) }
    case "wheel":
        let w = drag.wheel
        w.open(paths: [part[1]], at: mid, click: true)
        let v = w.view, m = v.shown!
        FileHandle.standardError.write(Data("drag types: wheel=\(v.registeredDraggedTypes.count) subviews=\(v.subviews.map { $0.registeredDraggedTypes.count }) statusDelegateIsOurs=\((NSApp.delegate as? AppDelegate)?.status.item.button?.window?.delegate is StatusBar)\n".utf8))
        let i = m.items.firstIndex { $0.id == "g-edit" } ?? 0
        let j = m.items[i].sub.firstIndex { !$0.sub.isEmpty } ?? 0
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
            v.locked = false
            switch part.count > 2 ? part[2] : "closed" {
            case "wedge": v.hot = .wedge(i)
            case "sub": v.hot = .wedge(i); v.hot = .sub(i, j)
            case "leaf": v.hot = .wedge(i); v.hot = .sub(i, j); v.hot = .leaf(i, j, 1)
            case "back": v.hot = .wedge(i); v.hot = .sub(i, j); v.hot = .leaf(i, j, 1); DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { v.locked = false; v.hot = .wedge((i + 1) % m.items.count); v.locked = true }
            case "centre": v.hot = .center
            default: break
            }
            v.locked = true
        }
    case "drop":
        // the same calls AppKit makes for a real drag, with a stand-in for the drag itself
        let w = drag.wheel
        w.open(paths: [part[1]], at: mid, click: false)
        let v = w.view, off = (part.count > 2 ? part[2] : "0,100").split(separator: ",").map { Double($0) ?? 0 }
        let d = StubDrag(paths: [part[1]], at: NSPoint(x: v.bounds.midX + off[0], y: v.bounds.midY + off[1]))
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            let entered = v.draggingEntered(d), ready = v.prepareForDragOperation(d)
            let hot = "\(v.hot)", done = ready && v.performDragOperation(d)
            FileHandle.standardError.write(Data("drop: entered=\(entered.rawValue) hot=\(hot) prepared=\(ready) performed=\(done) wheelVisible=\(w.panel.isVisible)\n".utf8))
        }
        Jobs.shared.onFinished = { j in
            FileHandle.standardError.write(Data("job: \(j.state.rawValue) outputs=\(j.outputs.map { ($0 as NSString).lastPathComponent }) errors=\(j.errors.map(\.code))\n".utf8))
            Toasts.shared.finished(j)
        }
    case "intent":
        // what Shortcuts does: fill in the parameters, call perform, take the files back
        Task { @MainActor in
            func say(_ t: String) { FileHandle.standardError.write(Data("intent: \(t)\n".utf8)) }
            do {
                let c = ConvertFilesIntent(); c.files = [IntentFile(fileURL: URL(fileURLWithPath: part[1]))]; c.format = .webp
                say("convert -> \(try await c.perform().value?.map { $0.filename } ?? [])")
                let bytes = IntentFile(data: try Data(contentsOf: URL(fileURLWithPath: part[1])), filename: "../from bytes.png")
                let z = CompressFilesIntent(); z.files = [bytes]; z.strength = .high
                say("compress bytes -> \(try await z.perform().value?.map { $0.fileURL?.path ?? "?" } ?? [])")
                do { _ = try await IntentRunner.run([bytes], act("shred"), title: "x"); say("shred was allowed: BUG") } catch { say("shred refused: \(error)") }
            } catch { say("failed: \(error)") }
        }
    case "anim":
        // opens, walks outward through the rings and closes, over and over, to be filmed
        let w = drag.wheel
        func cycle() {
            w.open(paths: [part[1]], at: mid, click: true)
            let v = w.view, m = v.shown!
            let i = m.items.firstIndex { $0.id == "g-edit" } ?? 0, j = m.items[i].sub.firstIndex { !$0.sub.isEmpty } ?? 0
            let steps: [(Double, () -> Void)] = [(0.5, { v.hot = .wedge(i) }), (0.9, { v.hot = .sub(i, j) }), (1.3, { v.hot = .leaf(i, j, 1) }), (1.7, { v.hot = .wedge((i + 1) % m.items.count) }), (2.1, { w.close() }), (2.6, { cycle() })]
            for (t, f) in steps { DispatchQueue.main.asyncAfter(deadline: .now() + t, execute: f) }
        }
        cycle()
    case "say": DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { Toasts.shared.say(part[1]) }
    case "toast":
        Jobs.shared.queue([part[1]], act(part.count > 2 ? part[2] : "strip_metadata"), title: "Removing metadata")
    case "shelf": Shelf.shared.add([part[1]])
    case "hub":
        switch part.count > 1 ? part[1] : "" {
        case "activity": Windows.shared.hub(.activity); case "recipes": Windows.shared.hub(.recipes); case "wheel": Windows.shared.hub(.wheel)
        case "settings": Windows.shared.hub(.settings); case "welcome": Windows.shared.hub(.welcome); case "rename": Windows.shared.hub(.rename(part.count > 2 ? [part[2]] : []))
        default: Windows.shared.hub(.actions(part.count > 2 ? [part[2]] : []))
        }
    case "editor": Windows.shared.editor(part[1], path: part[2])
    default: break
    }
}

final class StubDrag: NSObject, NSDraggingInfo {
    let pb = NSPasteboard(name: NSPasteboard.Name("sd-test-\(UUID().uuidString)")), at: NSPoint
    init(paths: [String], at: NSPoint) { self.at = at; super.init(); pb.clearContents(); pb.writeObjects(paths.map { URL(fileURLWithPath: $0) as NSURL }) }
    var draggingDestinationWindow: NSWindow? { nil }
    var draggingSourceOperationMask: NSDragOperation { [.copy, .move] }
    var draggingLocation: NSPoint { at }
    var draggedImageLocation: NSPoint { at }
    var draggedImage: NSImage? { nil }
    var draggingPasteboard: NSPasteboard { pb }
    var draggingSource: Any? { nil }
    var draggingSequenceNumber: Int { 1 }
    func slideDraggedImage(to p: NSPoint) {}
    var draggingFormation: NSDraggingFormation = .default
    var animatesToDestination = false
    var numberOfValidItemsForDrop = 1
    func enumerateDraggingItems(options: NSDraggingItemEnumerationOptions, for v: NSView?, classes: [AnyClass], searchOptions: [NSPasteboard.ReadingOptionKey: Any], using block: @escaping (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void) {}
    var springLoadingHighlight: NSSpringLoadingHighlight { .none }
    func resetSpringLoading() {}
}
#endif
