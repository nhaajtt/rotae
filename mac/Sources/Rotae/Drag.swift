import AppKit
import Carbon.HIToolbox
import QuartzCore
import SDCore

/// A short diary of what the wheel saw, in ~/Library/Logs/Rotae/wheel.log: file counts and
/// states only, never names. It is what tells a failed drop from a drag that never began.
enum Trace {
    private static let q = DispatchQueue(label: "rotae.trace")
    private static let url: URL = {
        let d = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)[0].appendingPathComponent("Logs/Rotae", isDirectory: true)
        try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        let u = d.appendingPathComponent("wheel.log")
        if let n = (try? FileManager.default.attributesOfItem(atPath: u.path))?[.size] as? Int, n > 200_000 { try? FileManager.default.removeItem(at: u) }
        return u
    }()
    static func log(_ m: @autoclosure () -> String) {
        let line = String(format: "%.3f ", Date().timeIntervalSince1970) + m() + "\n"
        q.async {
            guard let d = line.data(using: .utf8) else { return }
            if let h = try? FileHandle(forWritingTo: url) { h.seekToEndOfFile(); h.write(d); try? h.close() } else { try? d.write(to: url) }
        }
    }
}

/// Opens the wheel on the file while Shift is held, and takes it away when Shift comes up.
/// Both the mouse button and Shift have to be held: letting go of either ends it.
///  - a drag is under way and Shift goes down (any app);
///  - the button is down on a file in Finder with Shift held, in either order.
/// The wheel follows the pointer itself and acts when the button comes up over a cell, so it
/// does not depend on Finder agreeing to start a drag (with Shift down it often will not).
/// The fast check only runs while the mouse button is down; a slow one, only while Finder is
/// in front, notes what is selected the moment Shift goes down.
final class DragWatcher {
    let wheel = Wheel()
    private var last = 0, down = false, armed = false, shown = false
    private var urls: [String] = []
    private var timer: Timer?, slow: Timer?
    /// What was selected when Shift went down: a press with Shift held can itself change the selection.
    private var before: [String] = [], shiftWas = false
    private var travelled: CGFloat = 0
    private var hotKey: EventHotKeyRef?, undoKey: EventHotKeyRef?
    /// Where the button last went down, and whether Finder was in front when it did.
    private var downAt = NSPoint.zero, downInFinder = false, clicked = false
    private var shiftSince: CFTimeInterval?, downSince: CFTimeInterval = 0
    /// Nothing more opens until Shift comes up: after a drop, a plain click, typing or Esc.
    private var spent = false
    private var asking = false

    private static var finderFront: Bool { NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.apple.finder" }

    func start() {
        last = NSPasteboard(name: .drag).changeCount
        NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp]) { [weak self] e in
            if e.type == .leftMouseDown { self?.wheel.clickedOutside(); self?.begin() }
        }
        // digits during a drag: only reaches us once the user has allowed Accessibility
        NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { [weak self] e in self?.key(e) }
        NSEvent.addGlobalMonitorForEvents(matching: .flagsChanged) { [weak self] _ in self?.shiftEdge() }
        NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { [weak self] _ in self?.watchFinder() }
        watchFinder()
        registerHotKey()
    }
    private func watchFinder() {
        if Self.finderFront {
            guard slow == nil else { return }
            let t = Timer(timeInterval: 0.07, repeats: true) { [weak self] _ in self?.shiftEdge() }
            t.tolerance = 0.02
            RunLoop.main.add(t, forMode: .common)
            slow = t
        } else { slow?.invalidate(); slow = nil; shiftWas = false; before = [] }
    }
    /// Shift just went down in Finder with the button up: remember the selection as it is now.
    private func shiftEdge() {
        let shift = CGEventSource.flagsState(.combinedSessionState).contains(.maskShift)
        defer { shiftWas = shift }
        if !shift { if NSEvent.pressedMouseButtons & 1 == 0 { before = [] }; return }
        guard !shiftWas, NSEvent.pressedMouseButtons & 1 == 0, Self.finderFront, Store.shared.state.enabled else { return }
        DispatchQueue.global(qos: .userInteractive).async {
            let list = Self.finderSelection()
            DispatchQueue.main.async { [self] in
                if CGEventSource.flagsState(.combinedSessionState).contains(.maskShift) { before = list; Trace.log("shift down, selection \(list.count)") }
            }
        }
    }
    private func begin() {
        guard timer == nil else { return }
        let t = Timer(timeInterval: 1.0 / 90, repeats: true) { [weak self] _ in self?.tick() }
        t.tolerance = 0.002
        RunLoop.main.add(t, forMode: .common)
        timer = t
        tick()
    }
    func key(_ e: NSEvent) {
        guard shown || wheel.clickMode else { return }
        if e.keyCode == 53 { spent = true; shown = false; wheel.close(); return }
        if Store.shared.state.digits, let n = Int(e.charactersIgnoringModifiers ?? ""), (1...8).contains(n) { wheel.lock(n - 1) }
    }
    /// The centre goes where the file was pressed, unless the pointer has since gone elsewhere.
    private func anchor() -> NSPoint {
        let p = NSEvent.mouseLocation
        return clicked && hypot(p.x - downAt.x, p.y - downAt.y) < 160 ? downAt : p
    }
    private func tick() {
        let now = CACurrentMediaTime()
        let isDown = NSEvent.pressedMouseButtons & 1 != 0
        let flags = CGEventSource.flagsState(.combinedSessionState)
        let shift = flags.contains(.maskShift)
        let alone = shift && flags.isDisjoint(with: [.maskCommand, .maskControl, .maskAlternate])
        let pb = NSPasteboard(name: .drag)
        let ready = Store.shared.state.enabled && !NSRunningApplication.current.isActive && !wheel.clickMode

        if isDown && !down {
            armed = false; urls = []; last = pb.changeCount
            downSince = now; downAt = NSEvent.mouseLocation; downInFinder = Self.finderFront; clicked = downInFinder; travelled = 0
            if shift || downInFinder { Trace.log("down finder=\(downInFinder) shift=\(shift) before=\(before.count)") }
        }
        if isDown && pb.changeCount != last && !armed {
            armed = true; last = pb.changeCount
            urls = (pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []).map(\.path)
            Trace.log("drag began, files \(urls.count), shown=\(shown)")
            if shown && !urls.isEmpty { wheel.retarget(urls) }
        }
        if isDown { let p = NSEvent.mouseLocation; travelled = max(travelled, hypot(p.x - downAt.x, p.y - downAt.y)) }
        if shift && shiftSince == nil { shiftSince = now }
        if !shift {
            shiftSince = nil; spent = false
            if shown { shown = false; Trace.log("shift up: close"); wheel.closeIfNotDropped() }
        }
        // Shift held for a capital letter is typing, not asking for the wheel
        if let s = shiftSince, !armed, CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .keyDown) < now - s {
            spent = true
            if shown { shown = false; wheel.closeIfNotDropped() }
        }

        if ready && isDown && shift && !shown && !spent {
            if armed && !urls.isEmpty {
                shown = true
                Trace.log("open: drag, files \(urls.count)")
                wheel.open(paths: urls, at: anchor(), click: false)
            } else if alone && !armed, downInFinder, Self.finderFront, now - max(shiftSince ?? now, downSince) >= 0.10 {
                // a moment's hold, so a quick Shift-click to extend the selection stays just that
                if !before.isEmpty {
                    shown = true
                    Trace.log("open: selection before press, files \(before.count)")
                    wheel.open(paths: before, at: anchor(), click: false)
                }
                if !asking { ask() }
            }
        }
        // the wheel follows the pointer on its own account, drag or no drag
        if shown && isDown { wheel.view.follow(NSEvent.mouseLocation) }
        if !isDown && down {
            if shown {
                shown = false
                let v = wheel.view
                v.follow(NSEvent.mouseLocation)
                Trace.log("up: hot=\(v.hot != .none) shift=\(shift) drag=\(armed) travelled=\(Int(travelled)) dropped=\(wheel.dropped)")
                // letting go over a cell is the drop, whether or not a real drag is there to deliver one;
                // letting go where the press began, without having moved, is only a click
                if shift, !wheel.dropped, v.hot != .none, armed || travelled > 10 {
                    v.activate(armed && !urls.isEmpty ? urls : v.paths)
                } else { wheel.closeIfNotDropped() }
            }
            if shift { spent = true }
        }
        down = isDown
        if !isDown { timer?.invalidate(); timer = nil; shiftSince = nil; spent = false }
    }
    /// What is selected in Finder now that the button is down, which is what a drag would carry.
    private func ask() {
        asking = true
        DispatchQueue.global(qos: .userInteractive).async {
            let list = Self.finderSelection()
            DispatchQueue.main.async { [self] in
                asking = false
                Trace.log("selection after press \(list.count), before \(before.count), shown=\(shown)")
                guard CGEventSource.flagsState(.combinedSessionState).contains(.maskShift), NSEvent.pressedMouseButtons & 1 != 0, !spent, !wheel.clickMode, Store.shared.state.enabled else { return }
                if armed && !urls.isEmpty { return } // the drag began meanwhile and knows better
                if list.isEmpty { if !shown { spent = true }; return }
                // a Shift-press on a selected file takes it out of the selection: what was there before stands
                if shown { if !Set(list).isSubset(of: Set(before)) { wheel.retarget(list) }; return }
                shown = true
                wheel.open(paths: list, at: anchor(), click: false)
            }
        }
    }

    /// ⌃⌥Space: the wheel for the Finder selection or copied files, used by clicking.
    private func registerHotKey() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let me = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, ev, ctx in
            let w = Unmanaged<DragWatcher>.fromOpaque(ctx!).takeUnretainedValue()
            var k = EventHotKeyID()
            GetEventParameter(ev, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil, MemoryLayout<EventHotKeyID>.size, nil, &k)
            let id = k.id
            DispatchQueue.main.async { if id == 2 { Jobs.shared.undoLast() } else { w.hotKeyPressed() } }
            return noErr
        }, 1, &spec, me, nil)
        // ⌃⌥Z: take back the last thing Rotae made
        RegisterEventHotKey(UInt32(kVK_ANSI_Z), UInt32(controlKey | optionKey), EventHotKeyID(signature: 0x53444B59, id: 2), GetApplicationEventTarget(), 0, &undoKey)
        RegisterEventHotKey(UInt32(kVK_Space), UInt32(controlKey | optionKey), EventHotKeyID(signature: 0x53444B59, id: 1), GetApplicationEventTarget(), 0, &hotKey)
    }
    private func hotKeyPressed() {
        guard Store.shared.state.enabled else { return }
        if wheel.clickMode { wheel.close(); return }
        DispatchQueue.global(qos: .userInitiated).async {
            let list = Self.finderSelection()
            DispatchQueue.main.async { [self] in
                let copied = (NSPasteboard.general.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []).map(\.path)
                var paths = list.isEmpty ? copied : list
                // nothing selected or copied as a file: a copied picture or screenshot will do
                if paths.isEmpty, let p = pastedImage() { paths = [p] }
                if paths.isEmpty { NSSound.beep(); return }
                wheel.open(paths: paths, at: NSEvent.mouseLocation, click: true)
            }
        }
    }
    private static func finderSelection() -> [String] {
        guard NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.apple.finder" else { return [] }
        let src = "with timeout of 2 seconds\n" + #"tell application "Finder" to set s to selection as alias list"# + "\nend timeout\n" + #"set o to """# + "\n" + #"repeat with f in s"# + "\n" + #"set o to o & POSIX path of f & linefeed"# + "\n" + #"end repeat"# + "\n" + #"return o"#
        var err: NSDictionary?
        guard let r = NSAppleScript(source: src)?.executeAndReturnError(&err).stringValue else { return [] }
        return r.split(separator: "\n").map(String.init)
    }
}

final class WheelPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

final class Wheel {
    let size: CGFloat = 640
    var panel: WheelPanel!
    let view: WheelView
    var dropped = false
    var clickMode = false
    private var idle: DispatchWorkItem?
    init() {
        view = WheelView(frame: NSRect(x: 0, y: 0, width: size, height: size))
        panel = WheelPanel(contentRect: view.frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false; panel.backgroundColor = .clear; panel.hasShadow = false
        panel.level = .popUpMenu; panel.hidesOnDeactivate = false
        panel.acceptsMouseMovedEvents = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        panel.contentView = view
        view.wheel = self
    }
    private func menu(_ paths: [String]) -> Shown {
        let s = Store.shared.state
        var shown = menuFor(paths, prefs: s.prefs, recipes: s.recipes, commands: s.commands, apps: Tools.apps, tools: Tools.installedIDs, now: Date().timeIntervalSince1970, offline: s.offline, last: s.repeatLast ? s.last[menuKind(paths).rawValue] : nil)
        // transparency would be lost in JPEG, so the one-drop default keeps it
        if s.prefs.center[shown.kind.rawValue] == nil, paths.count == 1, shown.center.action?["to"]?.string == "jpg", Self.hasAlpha(paths[0]) { shown.center = MenuItem("WEBP", title: "Converting to WEBP", action: act("convert", ["to": .s("webp")])) }
        return shown
    }
    /// The drag turned out to carry something other than what the wheel was opened for.
    func retarget(_ paths: [String]) {
        guard panel.isVisible, !clickMode, paths != view.paths else { return }
        view.load(menu(paths), paths: paths, click: false)
        view.softRedraw()
        warn(paths)
    }
    private func warn(_ paths: [String]) {
        let v = view
        let pinned = Store.shared.state.prefs.center[FileKind.pdf.rawValue] != nil || Store.shared.state.repeatLast
        DispatchQueue.global(qos: .userInitiated).async {
            let w = Disguise.warnings(paths)
            // pictures of pages: reading them comes first, unless the user chose their own centre
            let scan = paths.count == 1 && !pinned && Forecast.isScan(paths[0])
            DispatchQueue.main.async {
                guard v.paths == paths else { return }
                v.warning = w.first
                if scan, let ocr = v.shown?.all.first(where: { $0.id == "ocrpdf" }) { v.shown?.center = ocr; v.content.needsDisplay = true }
            }
        }
    }
    func open(paths: [String], at p: NSPoint, click: Bool) {
        dropped = false; clickMode = click
        let shown = menu(paths)
        view.load(shown, paths: paths, click: click)
        let scr = NSScreen.screens.first { $0.frame.contains(p) } ?? NSScreen.main!
        let f = scr.visibleFrame
        // near an edge the wheel slides in rather than running off the screen
        let x = min(max(p.x - size / 2, f.minX), max(f.minX, f.maxX - size)), y = min(max(p.y - size / 2, f.minY), max(f.minY, f.maxY - size))
        panel.setFrameOrigin(NSPoint(x: x, y: y))
        turn += 1 // a close still fading out no longer applies
        panel.alphaValue = Motion.reduced ? 1 : 0
        panel.orderFrontRegardless()
        if !Motion.reduced {
            NSAnimationContext.runAnimationGroup { $0.duration = 0.14; $0.timingFunction = CAMediaTimingFunction(name: .easeOut); panel.animator().alphaValue = 1 }
            view.animateIn()
        }
        idle?.cancel()
        if click { let w = DispatchWorkItem { [weak self] in self?.close() }; idle = w; DispatchQueue.main.asyncAfter(deadline: .now() + 30, execute: w) }
        warn(paths)
    }
    static func hasAlpha(_ p: String) -> Bool {
        guard let s = CGImageSourceCreateWithURL(URL(fileURLWithPath: p) as CFURL, nil), let a = CGImageSourceCopyPropertiesAtIndex(s, 0, nil) as? [CFString: Any] else { return false }
        return a[kCGImagePropertyHasAlpha] as? Bool ?? false
    }
    /// Counts opens and closes, so a fade-out overtaken by a new wheel leaves it alone.
    private var turn = 0
    func close() {
        idle?.cancel(); clickMode = false
        Forecast.shared.cancel()
        guard panel.isVisible else { return }
        turn += 1
        let mine = turn
        if Motion.reduced { panel.orderOut(nil); return }
        view.animateOut()
        NSAnimationContext.runAnimationGroup({ $0.duration = 0.16; $0.timingFunction = CAMediaTimingFunction(name: .easeIn); panel.animator().alphaValue = 0 }, completionHandler: { [weak self] in
            guard let self, turn == mine else { return }
            panel.orderOut(nil)
        })
    }
    func closeIfNotDropped() { if !dropped && !clickMode { close() } }
    func clickedOutside() { if clickMode, !panel.frame.contains(NSEvent.mouseLocation) { close() } }
    func lock(_ i: Int) { view.lock(i) }
    func perform(_ item: MenuItem, kind: FileKind, paths: [String]) {
        dropped = true
        close() // first, so a new wheel can't be closed by this one finishing
        Actions.run(item, paths: paths, kind: kind)
    }
}

enum Disguise {
    static func warnings(_ paths: [String]) -> [String] {
        let p = Process(), i = Pipe(), o = Pipe()
        p.executableURL = Jobs.shared.engine(); p.standardInput = i; p.standardOutput = o; p.standardError = FileHandle.nullDevice
        guard (try? p.run()) != nil else { return [] }
        let d = try? JSONSerialization.data(withJSONObject: ["v": 1, "verb": "disguise", "paths": paths] as [String: Any])
        try? i.fileHandleForWriting.write(contentsOf: (d ?? Data()) + Data([10]))
        let out = o.fileHandleForReading.readDataToEndOfFile()
        try? i.fileHandleForWriting.close()
        p.waitUntilExit()
        for l in out.split(separator: 10) { if let e = EngineEvent.parse(Data(l)), e.event == "done" { return e.notes ?? [] } }
        return []
    }
}

final class WheelView: NSView {
    weak var wheel: Wheel?
    var shown: Shown?
    var paths: [String] = []
    var hot: WheelHit = .none { didSet { if hot != oldValue { changed() } } }
    var openWedge: Int?
    var openCell: Int?
    var locked = false
    var click = false
    var warning: String? { didSet { content.needsDisplay = true } }
    /// What the cell under the pointer would make of the file, once known.
    var preview: String? { didSet { if preview != oldValue { content.needsDisplay = true } } }
    let halo = CALayer()
    let glass = NSVisualEffectView()
    let shape = CAShapeLayer()
    let content = ContentLayerView()
    var thumb: NSImage?
    var radius: CGFloat = 156
    /// Real Liquid Glass when the system has it; `glass` and `halo` are the drawn stand-in.
    let liquid = WheelGlassModel()
    var liquidOn = false
    private var tracking: NSTrackingArea?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        registerForDraggedTypes([.fileURL])
        halo.frame = bounds; halo.shadowColor = NSColor.black.cgColor; halo.shadowOpacity = 0.28; halo.shadowRadius = 26; halo.shadowOffset = CGSize(width: 0, height: -10)
        if let g = makeWheelGlass(liquid, frame: bounds) { addSubview(g); liquidOn = true } else {
            layer?.addSublayer(halo)
            glass.frame = bounds; glass.material = .popover; glass.blendingMode = .behindWindow; glass.state = .active; glass.wantsLayer = true
            addSubview(glass)
            shape.path = circle(radius); glass.layer?.mask = shape; halo.shadowPath = circle(radius)
        }
        content.frame = bounds; content.owner = self
        addSubview(content)
    }
    required init?(coder: NSCoder) { fatalError() }
    override func updateTrackingAreas() {
        if let t = tracking { removeTrackingArea(t) }
        let t = NSTrackingArea(rect: bounds, options: [.activeAlways, .mouseMoved, .inVisibleRect], owner: self); addTrackingArea(t); tracking = t
    }
    func circle(_ r: CGFloat) -> CGPath { CGPath(ellipseIn: CGRect(x: bounds.midX - r, y: bounds.midY - r, width: 2 * r, height: 2 * r), transform: nil) }
    func setGlass(_ r: CGFloat) {
        if liquidOn { return }
        guard r != radius else { return }
        radius = r
        let a = CASpringAnimation(keyPath: "path"); a.fromValue = shape.presentation()?.path ?? shape.path; a.toValue = circle(r)
        a.damping = 22; a.stiffness = 340; a.duration = 0.32
        shape.path = circle(r); shape.add(a, forKey: "path")
        halo.shadowPath = circle(r)
    }
    /// Nearly invisible, yet the whole square takes the drop so nothing falls through to Finder.
    override func draw(_ r: NSRect) { NSColor.black.withAlphaComponent(0.012).setFill(); bounds.fill() }

    func load(_ s: Shown, paths: [String], click: Bool) {
        shown = s; self.paths = paths; self.click = click; hot = .none; openWedge = nil; openCell = nil; locked = false; warning = nil; thumb = nil; preview = nil
        radius = 156; shape.path = circle(radius); halo.shadowPath = circle(radius)
        liquid.accent = Store.shared.accent; syncLiquid(animated: false)
        content.needsDisplay = true; needsDisplay = true
        if let f = paths.first { Thumb.load(f, size: 96) { [weak self] i in if self?.paths.first == f { self?.thumb = i; self?.content.needsDisplay = true } } }
    }
    /// Glass and labels are one object: a single spring moves the layer that holds both.
    func animateIn() {
        guard let l = layer else { return }
        l.removeAnimation(forKey: "out")
        l.anchorPoint = CGPoint(x: 0.5, y: 0.5); l.position = CGPoint(x: bounds.midX, y: bounds.midY)
        let a = CASpringAnimation(keyPath: "transform.scale"); a.fromValue = 0.86; a.toValue = 1
        a.mass = 1; a.stiffness = 380; a.damping = 26; a.initialVelocity = 6; a.duration = a.settlingDuration
        l.add(a, forKey: "in")
    }
    /// Leaving: draws in slightly as it fades, the reverse of arriving.
    func animateOut() {
        guard let l = layer else { return }
        let a = CABasicAnimation(keyPath: "transform.scale"); a.fromValue = 1; a.toValue = 0.94
        a.duration = 0.16; a.timingFunction = CAMediaTimingFunction(name: .easeIn); a.fillMode = .forwards; a.isRemovedOnCompletion = false
        l.add(a, forKey: "out")
    }
    /// Tells the glass which petals are open and where the lens sits.
    func syncLiquid(animated: Bool) {
        guard liquidOn, let m = shown else { return }
        let g = WheelGeometry.self, n = m.items.count, half = 180 / Double(max(n, 1))
        var ring2: Arc?, ring3: Arc?, l1: Arc?, l2: Arc?, l3: Arc?
        var lit: Int?
        switch hot {
        case .wedge(let i), .sub(let i, _), .leaf(let i, _, _): lit = i
        case .center, .none: break
        }
        if let i = openWedge ?? lit {
            let mid = g.mid(i, of: n)
            l1 = Arc(r0: g.core + 5, r1: g.r1 - 3, a0: mid - half + 1.5, a1: mid + half - 1.5)
        }
        if let w = openWedge {
            let sub = m.items[w].sub, width = g.cellWidth(sub.count), c0 = g.mid(w, of: n), span = Double(sub.count) * width / 2
            ring2 = Arc(r0: g.r2In, r1: g.r2, a0: c0 - span - 2, a1: c0 + span + 2)
            var cell = openCell
            if case .sub(w, let j) = hot { cell = j }
            if let j = cell {
                let mid = g.subMid(j, count: sub.count, wedge: w, of: n)
                l2 = Arc(r0: g.r2In + 4, r1: g.r2 - 4, a0: mid - width / 2 + 1, a1: mid + width / 2 - 1)
            }
            if let c = openCell {
                let leaves = sub[c].sub, lw = g.leafWidth(leaves.count), center = g.subMid(c, count: sub.count, wedge: w, of: n), s3 = Double(leaves.count) * lw / 2
                ring3 = Arc(r0: g.r3In, r1: g.r3, a0: center - s3 - 2, a1: center + s3 + 2)
                if case .leaf(w, c, let k) = hot {
                    let mid = g.leafMid(k, count: leaves.count, center: center)
                    l3 = Arc(r0: g.r3In + 4, r1: g.r3 - 4, a0: mid - lw / 2 + 0.8, a1: mid + lw / 2 - 0.8)
                }
            }
        }
        liquid.set(ring2: ring2, ring3: ring3, lens1: l1, lens2: l2, lens3: l3, centre: hot == .center, animated: animated)
    }
    func changed() {
        NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
        guard let m = shown else { return }
        var w: Int?, c: Int?
        switch hot {
        case .wedge(let i): if !m.items[i].sub.isEmpty { w = i }
        case .sub(let i, let j): w = i; if !m.items[i].sub[j].sub.isEmpty { c = j }
        case .leaf(let i, let j, _): w = i; c = j
        case .center, .none: break
        }
        // sliding through the gap between rings keeps them open
        if hot == .none, openWedge != nil { syncLiquid(animated: true); return }
        openWedge = w; openCell = c
        syncLiquid(animated: true)
        softRedraw()
        setGlass(c != nil ? CGFloat(WheelGeometry.r3) + 6 : w != nil ? CGFloat(WheelGeometry.r2) + 6 : 156)
        content.needsDisplay = true
        forecast()
    }
    /// The action a drop would run right now, as `activate` would choose it.
    var aimed: MenuItem? {
        guard let m = shown else { return nil }
        switch hot {
        case .center: return m.center
        case .wedge(let i): return m.items[i].defaultChoice
        case .sub(let i, let j): return m.items[i].sub[j].defaultChoice
        case .leaf(let i, let j, let k): return m.items[i].sub[j].sub[k].defaultChoice
        case .none: return nil
        }
    }
    func forecast() {
        preview = nil
        guard paths.count == 1, let it = aimed, it.ask.isEmpty, let a = it.action else { Forecast.shared.cancel(); return }
        let was = hot, file = paths[0]
        Forecast.shared.ask(a, path: file) { [weak self] t in
            guard let self, hot == was, paths.first == file else { return }
            preview = t
        }
    }
    /// The next redraw of the labels cross-fades, in step with the glass moving under them.
    func softRedraw() {
        guard !Motion.reduced, let l = content.layer else { return }
        let t = CATransition(); t.type = .fade; t.duration = 0.14; t.timingFunction = CAMediaTimingFunction(name: .easeOut)
        l.add(t, forKey: "labels")
    }
    func lock(_ i: Int) { guard let s = shown, s.items.indices.contains(i) else { return }; locked = true; hot = .wedge(i) }
    func track(_ p: NSPoint) {
        guard let m = shown, !locked else { return }
        let sc = openWedge.map { m.items[$0].sub.count } ?? 0
        let lc = openWedge.flatMap { w in openCell.map { m.items[w].sub[$0].sub.count } } ?? 0
        hot = WheelGeometry.hit(dx: p.x - bounds.midX, dy: p.y - bounds.midY, count: m.items.count, open: openWedge, subCount: sc, openCell: openCell, leafCount: lc)
    }
    func update(_ s: NSDraggingInfo) -> NSDragOperation { track(convert(s.draggingLocation, from: nil)); return hot == .none ? [] : .copy }
    override func draggingEntered(_ s: NSDraggingInfo) -> NSDragOperation { Trace.log("real drag entered the wheel"); return update(s) }
    override func draggingUpdated(_ s: NSDraggingInfo) -> NSDragOperation { update(s) }
    override func draggingExited(_ s: NSDraggingInfo?) { if !locked { hot = .none } }
    override func prepareForDragOperation(_ s: NSDraggingInfo) -> Bool { locked || update(s) == .copy }
    /// The pointer, in screen terms, from the watcher rather than from a drag.
    func follow(_ screen: NSPoint) {
        guard let w = window else { return }
        track(convert(w.convertPoint(fromScreen: screen), from: nil))
    }
    override func performDragOperation(_ s: NSDraggingInfo) -> Bool {
        Trace.log("real drop, already handled=\(wheel?.dropped ?? false)")
        if wheel?.dropped == true { return true } // the watcher saw the button come up first
        // what is really being dropped beats what we read at the start of the drag
        let real = (s.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []).map(\.path)
        return activate(real.isEmpty ? paths : real)
    }
    override func mouseMoved(with e: NSEvent) { if click { track(convert(e.locationInWindow, from: nil)) } }
    override func mouseUp(with e: NSEvent) {
        guard click else { return }
        track(convert(e.locationInWindow, from: nil))
        if hot == .none { wheel?.close(); return }
        _ = activate(paths)
    }
    @discardableResult func activate(_ list: [String]) -> Bool {
        guard let m = shown else { return false }
        var item: MenuItem?, noted: String?
        switch hot {
        case .center: item = m.center
        case .wedge(let i): item = m.items[i].defaultChoice; noted = m.items[i].sub.isEmpty ? m.items[i].id : m.items[i].sub[m.items[i].def].id
        case .sub(let i, let j): item = m.items[i].sub[j].defaultChoice; noted = m.items[i].sub[j].id
        case .leaf(let i, let j, let k): item = m.items[i].sub[j].sub[k].defaultChoice; noted = m.items[i].sub[j].id
        case .none: return false
        }
        guard let chosen = item else { return false }
        if let n = noted { Store.shared.noteUse(m.kind, n); Store.shared.update { $0.last[m.kind.rawValue] = "\(n)|\(chosen.id)" } }
        wheel?.perform(chosen, kind: m.kind, paths: list)
        return true
    }
}

/// The ring, drawn in one pass in the manner of Liquid Glass: a clear body, a bright rim that
/// catches light from above, and a tinted lens over whatever is under the pointer.
final class ContentLayerView: NSView {
    weak var owner: WheelView?
    override init(frame: NSRect) { super.init(frame: frame); wantsLayer = true; layerContentsRedrawPolicy = .onSetNeedsDisplay }
    required init?(coder: NSCoder) { fatalError() }
    override func hitTest(_ p: NSPoint) -> NSView? { nil }

    func point(_ r: CGFloat, _ deg: Double) -> NSPoint {
        let a = deg * .pi / 180
        return NSPoint(x: bounds.midX + r * CGFloat(sin(a)), y: bounds.midY + r * CGFloat(cos(a)))
    }
    func sector(_ r0: CGFloat, _ r1: CGFloat, _ a0: Double, _ a1: Double) -> NSBezierPath {
        let c = NSPoint(x: bounds.midX, y: bounds.midY), p = NSBezierPath()
        p.appendArc(withCenter: c, radius: r1, startAngle: CGFloat(90 - a0), endAngle: CGFloat(90 - a1), clockwise: true)
        p.appendArc(withCenter: c, radius: r0, startAngle: CGFloat(90 - a1), endAngle: CGFloat(90 - a0), clockwise: false)
        p.close(); return p
    }
    func ring(_ r: CGFloat) -> NSBezierPath { NSBezierPath(ovalIn: bounds.insetBy(dx: bounds.midX - r, dy: bounds.midY - r)) }
    /// A convex highlight: brighter along the top edge, tinted through.
    func lens(_ p: NSBezierPath, _ tint: NSColor) {
        NSGradient(colors: [tint.blended(withFraction: 0.28, of: .white)!.withAlphaComponent(0.94), tint.withAlphaComponent(0.84)])?.draw(in: p, angle: -90)
        NSGraphicsContext.saveGraphicsState(); p.addClip()
        p.lineWidth = 2; NSColor.white.withAlphaComponent(0.42).setStroke(); p.stroke()
        NSGraphicsContext.restoreGraphicsState()
    }
    func text(_ s: String, at p: NSPoint, size: CGFloat, weight: NSFont.Weight = .medium, color: NSColor = .labelColor, width: CGFloat = 86) {
        let st = NSMutableParagraphStyle(); st.alignment = .center; st.lineBreakMode = .byTruncatingTail
        let at: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: size, weight: weight), .foregroundColor: color, .paragraphStyle: st]
        (s as NSString).draw(with: NSRect(x: p.x - width / 2, y: p.y - size * 0.7, width: width, height: size * 1.45), options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine], attributes: at)
    }
    /// Symbols are looked up and tinted once: doing it on every redraw was most of a frame.
    private var symbols: [String: NSImage] = [:]
    func symbol(_ name: String, at p: NSPoint, size: CGFloat, color: NSColor) {
        let key = "\(name)|\(size)|\(color == .white ? "w" : "l")|\(effectiveAppearance.name.rawValue)"
        if symbols[key] == nil {
            guard let i = NSImage(systemSymbolName: name, accessibilityDescription: nil) ?? NSImage(systemSymbolName: "circle", accessibilityDescription: nil), let c = i.withSymbolConfiguration(.init(pointSize: size, weight: .medium).applying(.init(paletteColors: [color]))) else { return }
            symbols[key] = c
        }
        guard let c = symbols[key] else { return }
        c.draw(at: NSPoint(x: p.x - c.size.width / 2, y: p.y - c.size.height / 2), from: .zero, operation: .sourceOver, fraction: 1)
    }
    override func draw(_ d: NSRect) {
        guard let o = owner, let m = o.shown else { return }
        let n = m.items.count, g = WheelGeometry.self, accent = NSColor(Store.shared.accent)
        let R = o.radius - 1
        // the body: a whisper of light from above, and a rim that is bright at the top
        let live = o.liquidOn // real glass brings its own body, rim and lens
        if !live {
            NSGradient(colors: [NSColor.white.withAlphaComponent(0.20), NSColor.white.withAlphaComponent(0.03)])?.draw(in: ring(R), angle: -90)
            NSGraphicsContext.saveGraphicsState()
            let rim = ring(R); rim.lineWidth = 1.5; rim.setClip()
            let stroked = ring(R); stroked.lineWidth = 1.5
            NSColor.white.withAlphaComponent(0.55).setStroke(); stroked.stroke()
            NSGraphicsContext.restoreGraphicsState()
        }
        for (i, it) in m.items.enumerated() {
            let mid = g.mid(i, of: n), half = 180 / Double(max(n, 1))
            var isHot = false
            if case .wedge(i) = o.hot { isHot = true }
            if case .sub(i, _) = o.hot { isHot = true }
            if case .leaf(i, _, _) = o.hot { isHot = true }
            if o.openWedge == i { isHot = true }
            if isHot, !live { lens(sector(CGFloat(g.core) + 5, CGFloat(g.r1) - 3, mid - half + 1.5, mid + half - 1.5), accent) }
            let col: NSColor = isHot ? .white : .labelColor
            // stacked upright, so labels never run into their icons at the sides of the ring
            let at = point(104, mid)
            symbol(it.symbol, at: NSPoint(x: at.x, y: at.y + 10), size: 20, color: col)
            text(T(it.label), at: NSPoint(x: at.x, y: at.y - 13), size: 12, weight: .semibold, color: col, width: 84)
            let badge = point(141, mid)
            text("\(i + 1)", at: badge, size: 9, color: col.withAlphaComponent(0.45), width: 20)
        }
        ring(CGFloat(g.r1) + 2).setLineDash([], count: 0, phase: 0)
        NSColor.white.withAlphaComponent(0.18).setStroke()
        if o.openWedge != nil, !live { ring(CGFloat(g.r1) + 2).stroke() }
        if let w = o.openWedge {
            let sub = m.items[w].sub, width = g.cellWidth(sub.count)
            for (j, s) in sub.enumerated() {
                let mid = g.subMid(j, count: sub.count, wedge: w, of: n)
                var isHot = o.openCell == j
                if o.hot == .sub(w, j) { isHot = true }
                if isHot, !live { lens(sector(CGFloat(g.r2In), CGFloat(g.r2) - 2, mid - width / 2 + 1, mid + width / 2 - 1), accent) }
                let col: NSColor = isHot ? .white : .labelColor
                let cell = point(189, mid)
                symbol(s.symbol == "circle" ? (s.sub.isEmpty ? "smallcircle.filled.circle" : "circle.grid.2x1") : s.symbol, at: NSPoint(x: cell.x, y: cell.y + 9), size: 14, color: col)
                text(T(s.label), at: NSPoint(x: cell.x, y: cell.y - 11), size: 11, weight: j == m.items[w].def ? .bold : .medium, color: col, width: 68)
            }
            if let c = o.openCell {
                let leaves = sub[c].sub, lw = g.leafWidth(leaves.count), center = g.subMid(c, count: sub.count, wedge: w, of: n)
                if !live { NSColor.white.withAlphaComponent(0.18).setStroke(); ring(CGFloat(g.r2) + 2).stroke() }
                for (k, l) in leaves.enumerated() {
                    let mid = g.leafMid(k, count: leaves.count, center: center), isHot = o.hot == .leaf(w, c, k)
                    if isHot, !live { lens(sector(CGFloat(g.r3In), CGFloat(g.r3) - 2, mid - lw / 2 + 0.8, mid + lw / 2 - 0.8), accent) }
                    text(T(l.label), at: point(261, mid), size: 10.5, weight: k == sub[c].def ? .bold : .medium, color: isHot ? .white : .labelColor, width: 62)
                }
            }
        }
        let cHot = o.hot == .center
        if !live {
            if cHot { lens(ring(CGFloat(g.core) - 3), accent) } else { NSColor.white.withAlphaComponent(0.10).setFill(); ring(CGFloat(g.core) - 3).fill() }
            NSColor.white.withAlphaComponent(0.35).setStroke(); ring(CGFloat(g.core) - 3).stroke()
        }
        let c = NSPoint(x: bounds.midX, y: bounds.midY), tc: NSColor = cHot ? .white : .labelColor
        if let t = o.thumb, !cHot {
            let r = NSRect(x: c.x - 19, y: c.y - 2, width: 38, height: 38)
            NSGraphicsContext.saveGraphicsState(); NSBezierPath(roundedRect: r, xRadius: 9, yRadius: 9).addClip()
            t.draw(in: r, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil); NSGraphicsContext.restoreGraphicsState()
        }
        text("→ \(T(m.center.label))", at: NSPoint(x: c.x, y: c.y - 14), size: 12, weight: .semibold, color: tc, width: 92)
        text(o.warning != nil ? T("fake type") : o.preview ?? (o.paths.count > 1 ? "\(o.paths.count) \(T("files"))" : T("drop here")), at: NSPoint(x: c.x, y: c.y - 30), size: 10, color: o.warning != nil ? .systemRed : tc.withAlphaComponent(0.6), width: 92)
        if let w = o.warning {
            NSColor.systemRed.setStroke()
            let p = ring(R - 1); p.lineWidth = 2.5; p.stroke()
            text(w, at: NSPoint(x: c.x, y: bounds.midY - R + 14), size: 11, weight: .semibold, color: .systemRed, width: 200)
        }
    }
}
