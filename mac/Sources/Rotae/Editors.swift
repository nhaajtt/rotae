import AVKit
import AppKit
import PDFKit
import SDCore
import SwiftUI

struct EditorHost: View {
    let tool, path: String; let close: () -> Void
    var body: some View {
        switch tool {
        case "crop": ImageEditor(mode: .crop, path: path, close: close)
        case "addbg": ImageEditor(mode: .background, path: path, close: close)
        case "redact": ImageEditor(mode: .redact, path: path, close: close)
        case "trim": TrimEditor(path: path, close: close)
        case "pages": PagesEditor(path: path, close: close)
        case "sign": SignEditor(path: path, close: close)
        default: Text(tool)
        }
    }
}

/// Footer shared by every editor. Closing with unsaved work asks first.
struct EditorBar: View {
    let dirty: Bool; let canSave: Bool; let close: () -> Void; let save: () -> Void
    @State var ask = false
    var body: some View {
        HStack {
            Spacer()
            Button(T("Cancel")) { if dirty { ask = true } else { close() } }.keyboardShortcut(.cancelAction)
            Button(T("Save")) { save(); close() }.keyboardShortcut(.defaultAction).disabled(!canSave)
        }.padding(12).confirmationDialog(T("Discard your changes?"), isPresented: $ask) { Button(T("Discard"), role: .destructive, action: close) }
    }
}

func loadUpright(_ path: String) -> NSImage? {
    guard let s = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
          let c = CGImageSourceCreateThumbnailAtIndex(s, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: 2400] as CFDictionary) else { return nil }
    return NSImage(cgImage: c, size: NSSize(width: c.width, height: c.height))
}

struct Box: Identifiable, Equatable { let id = UUID(); var r: CGRect }
enum ImgMode { case crop, background, redact }

struct ImageEditor: View {
    let mode: ImgMode; let path: String; let close: () -> Void
    @State var img: NSImage?
    @State var failed = false
    @State var zoom = 1.0
    @State var base = 1.0
    @State var crop = CGRect(x: 0, y: 0, width: 1, height: 1)
    @State var aspect: Double?
    @State var boxes: [Box] = []
    @State var drawing: CGRect?
    @State var color = Color.white
    @State var padding = 8.0
    @State var radius = 3.0
    @State var shadow = 40.0
    @State var touched = false
    let ratios: [(String, Double?)] = [("Free", nil), ("1:1", 1), ("4:3", 4.0 / 3), ("3:2", 1.5), ("16:9", 16.0 / 9), ("9:16", 9.0 / 16)]
    var body: some View {
        HStack(spacing: 0) {
            GeometryReader { g in
                if let img {
                    let fit = min((g.size.width - 40) / img.size.width, (g.size.height - 40) / img.size.height) * zoom
                    let sz = CGSize(width: img.size.width * fit, height: img.size.height * fit)
                    ScrollView([.horizontal, .vertical]) {
                        stage(img, sz).frame(width: max(sz.width + 40, g.size.width), height: max(sz.height + 40, g.size.height))
                    }.gesture(MagnificationGesture().onChanged { zoom = max(0.2, min(8, base * $0)) }.onEnded { _ in base = zoom })
                } else { Text(T(failed ? "This picture couldn't be read." : "Loading…")).frame(maxWidth: .infinity, maxHeight: .infinity) }
            }.background(Color.black.opacity(0.25))
            Divider()
            VStack(alignment: .leading, spacing: 12) { panel; Spacer(); HStack { Button("−") { zoom = max(0.2, zoom / 1.25); base = zoom }; Text("\(Int(zoom * 100))%").monospacedDigit(); Button("+") { zoom = min(8, zoom * 1.25); base = zoom } }; EditorBar(dirty: touched, canSave: img != nil, close: close, save: save) }.padding(12).frame(width: 280)
        }.onAppear { img = loadUpright(path); failed = img == nil }
    }
    @ViewBuilder func stage(_ img: NSImage, _ sz: CGSize) -> some View {
        ZStack(alignment: .topLeading) {
            switch mode {
            case .background:
                let pad = padding / 100 * max(sz.width, sz.height)
                ZStack { color; Image(nsImage: img).resizable().frame(width: sz.width, height: sz.height).clipShape(RoundedRectangle(cornerRadius: radius / 100 * min(sz.width, sz.height))).shadow(color: .black.opacity(shadow / 100 * 0.45), radius: pad * 0.45, y: pad * 0.25) }.frame(width: sz.width + 2 * pad, height: sz.height + 2 * pad)
            case .crop:
                Image(nsImage: img).resizable().frame(width: sz.width, height: sz.height)
                CropOverlay(rect: $crop, aspect: aspect, size: sz) { touched = true }
            case .redact:
                Image(nsImage: img).resizable().frame(width: sz.width, height: sz.height)
                ForEach(boxes) { b in
                    Rectangle().fill(.black).frame(width: b.r.width * sz.width, height: b.r.height * sz.height).offset(x: b.r.minX * sz.width, y: b.r.minY * sz.height)
                        .onTapGesture(count: 2) { boxes.removeAll { $0.id == b.id } }
                }
                if let d = drawing { Rectangle().fill(.black.opacity(0.7)).frame(width: d.width * sz.width, height: d.height * sz.height).offset(x: d.minX * sz.width, y: d.minY * sz.height) }
                Color.clear.frame(width: sz.width, height: sz.height).contentShape(Rectangle()).gesture(DragGesture(minimumDistance: 2).onChanged { v in drawing = norm(v.startLocation, v.location, sz) }.onEnded { v in if let d = drawing, d.width > 0.003, d.height > 0.003 { boxes.append(Box(r: d)); touched = true }; drawing = nil })
            }
        }
    }
    func norm(_ a: CGPoint, _ b: CGPoint, _ s: CGSize) -> CGRect {
        let x0 = min(a.x, b.x) / s.width, y0 = min(a.y, b.y) / s.height, x1 = max(a.x, b.x) / s.width, y1 = max(a.y, b.y) / s.height
        return CGRect(x: max(0, x0), y: max(0, y0), width: min(1, x1) - max(0, x0), height: min(1, y1) - max(0, y0))
    }
    @ViewBuilder var panel: some View {
        switch mode {
        case .crop:
            Text(T("Crop")).font(.headline)
            Picker(T("Aspect"), selection: Binding(get: { ratios.firstIndex { $0.1 == aspect } ?? 0 }, set: { aspect = ratios[$0].1; fitAspect() })) { ForEach(ratios.indices, id: \.self) { Text(T(ratios[$0].0)).tag($0) } }
            if let img { Text("\(Int(crop.width * img.size.width)) × \(Int(crop.height * img.size.height)) px").monospacedDigit().foregroundStyle(.secondary) }
            Button(T("Select all")) { crop = CGRect(x: 0, y: 0, width: 1, height: 1); aspect = nil }
        case .background:
            Text(T("Add BG")).font(.headline); ColorPicker(T("Colour"), selection: $color).onChange(of: color) { touched = true }
            slider("Padding", $padding, 30); slider("Corners", $radius, 20); slider("Shadow", $shadow, 100)
        case .redact:
            Text(T("Redact")).font(.headline); Text("\(boxes.count) \(T("areas"))").foregroundStyle(.secondary)
            Text(T("Drag over what should disappear. Double-click a box to remove it.")).font(.caption).foregroundStyle(.secondary)
            HStack { Button(T("Undo")) { _ = boxes.popLast() }.keyboardShortcut("z").disabled(boxes.isEmpty); Button(T("Clear")) { boxes = [] }.disabled(boxes.isEmpty) }
        }
    }
    func slider(_ l: String, _ b: Binding<Double>, _ m: Double) -> some View { VStack(alignment: .leading) { Text(T(l)); Slider(value: b, in: 0...m) { _ in touched = true } } }
    func fitAspect() {
        guard let a = aspect, let img else { return }
        let target = a / (img.size.width / img.size.height)
        var w = crop.width, h = w / target
        if h > 1 { h = 1; w = target }
        crop = CGRect(x: min(crop.minX, 1 - w), y: min(crop.minY, 1 - h), width: w, height: h); touched = true
    }
    func save() {
        switch mode {
        case .crop: Jobs.shared.queue([path], act("crop", ["rect": .a([.n(crop.minX), .n(crop.minY), .n(crop.width), .n(crop.height)])]), title: "Cropping")
        case .redact: if !boxes.isEmpty { Jobs.shared.queue([path], act("redact", ["boxes": .a(boxes.map { .a([.n($0.r.minX), .n($0.r.minY), .n($0.r.width), .n($0.r.height)]) })]), title: "Redacting") }
        case .background:
            let c = NSColor(color).usingColorSpace(.sRGB) ?? .white
            let hex = String(format: "#%02x%02x%02x", Int(c.redComponent * 255), Int(c.greenComponent * 255), Int(c.blueComponent * 255))
            Jobs.shared.queue([path], act("add_bg", ["color": .s(hex), "padding": .n(padding / 100), "radius": .n(radius / 100), "shadow": .n(shadow / 100)]), title: "Adding background")
        }
    }
}

struct CropOverlay: View {
    @Binding var rect: CGRect; let aspect: Double?; let size: CGSize; let touched: () -> Void
    @State var start: CGRect?
    // which edges each handle moves: left, top, right, bottom
    let handles: [(CGFloat, CGFloat, [Bool])] = [(0, 0, [true, true, false, false]), (0.5, 0, [false, true, false, false]), (1, 0, [false, true, true, false]), (1, 0.5, [false, false, true, false]), (1, 1, [false, false, true, true]), (0.5, 1, [false, false, false, true]), (0, 1, [true, false, false, true]), (0, 0.5, [true, false, false, false])]
    var body: some View {
        let r = CGRect(x: rect.minX * size.width, y: rect.minY * size.height, width: rect.width * size.width, height: rect.height * size.height)
        ZStack(alignment: .topLeading) {
            Path { p in p.addRect(CGRect(origin: .zero, size: size)); p.addRect(r) }.fill(.black.opacity(0.5), style: FillStyle(eoFill: true)).allowsHitTesting(false)
            Rectangle().stroke(.white, lineWidth: 1.5).frame(width: r.width, height: r.height).offset(x: r.minX, y: r.minY).allowsHitTesting(false)
            Path { p in for f in [1.0 / 3, 2.0 / 3] { p.move(to: CGPoint(x: r.minX + r.width * f, y: r.minY)); p.addLine(to: CGPoint(x: r.minX + r.width * f, y: r.maxY)); p.move(to: CGPoint(x: r.minX, y: r.minY + r.height * f)); p.addLine(to: CGPoint(x: r.maxX, y: r.minY + r.height * f)) } }.stroke(.white.opacity(0.4)).allowsHitTesting(false)
            Color.white.opacity(0.001).frame(width: r.width, height: r.height).offset(x: r.minX, y: r.minY).gesture(DragGesture().onChanged { v in
                let s = start ?? rect; start = s
                rect.origin = CGPoint(x: max(0, min(1 - s.width, s.minX + v.translation.width / size.width)), y: max(0, min(1 - s.height, s.minY + v.translation.height / size.height))); touched()
            }.onEnded { _ in start = nil })
            ForEach(handles.indices, id: \.self) { i in
                let h = handles[i]
                Circle().fill(.white).frame(width: 12, height: 12).shadow(radius: 1).offset(x: r.minX + r.width * h.0 - 6, y: r.minY + r.height * h.1 - 6).gesture(DragGesture().onChanged { v in drag(h.2, v) }.onEnded { _ in start = nil })
            }
        }.frame(width: size.width, height: size.height, alignment: .topLeading)
    }
    func drag(_ e: [Bool], _ v: DragGesture.Value) {
        let s = start ?? rect; start = s
        let dx = v.translation.width / size.width, dy = v.translation.height / size.height, m = 0.02
        var x0 = s.minX, y0 = s.minY, x1 = s.maxX, y1 = s.maxY
        if e[0] { x0 = max(0, min(x1 - m, s.minX + dx)) }; if e[1] { y0 = max(0, min(y1 - m, s.minY + dy)) }
        if e[2] { x1 = min(1, max(x0 + m, s.maxX + dx)) }; if e[3] { y1 = min(1, max(y0 + m, s.maxY + dy)) }
        if let a = aspect {
            let ratio = a * size.height / size.width
            let h = (x1 - x0) / ratio
            if e[1] { y0 = max(0, y1 - h) } else { y1 = min(1, y0 + h) }
            let realH = y1 - y0
            if abs(realH - h) > 0.0001 { let w = realH * ratio; if e[0] { x0 = x1 - w } else { x1 = x0 + w } }
        }
        rect = CGRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0); touched()
    }
}

final class TrimModel: ObservableObject {
    @Published var player: AVPlayer
    @Published var dur = 0.0
    @Published var a = 0.0
    @Published var b = 0.0
    @Published var wave: [Float] = []
    @Published var failed = false
    @Published var now = 0.0
    var token: Any?
    let url: URL
    init(_ path: String) {
        url = URL(fileURLWithPath: path); player = AVPlayer(url: url)
        Task { @MainActor in
            let asset = AVURLAsset(url: url)
            guard let d = try? await asset.load(.duration), d.seconds.isFinite, d.seconds > 0 else { failed = true; return }
            dur = d.seconds; b = d.seconds
            let w = await Self.waveform(asset); wave = w
        }
        token = player.addPeriodicTimeObserver(forInterval: CMTime(seconds: 0.05, preferredTimescale: 600), queue: .main) { [weak self] t in self?.now = t.seconds }
    }
    deinit { if let token { player.removeTimeObserver(token) } }
    /// A peak per column, read once at low quality; files with no sound just get no waveform.
    static func waveform(_ asset: AVAsset) async -> [Float] {
        guard let tr = try? await asset.loadTracks(withMediaType: .audio).first, let r = try? AVAssetReader(asset: asset) else { return [] }
        let o = AVAssetReaderTrackOutput(track: tr, outputSettings: [AVFormatIDKey: kAudioFormatLinearPCM, AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false, AVNumberOfChannelsKey: 1, AVSampleRateKey: 8000])
        guard r.canAdd(o) else { return [] }; r.add(o); r.startReading()
        var peaks: [Float] = [], cur: Int16 = 0, n = 0
        let dur = (try? await asset.load(.duration).seconds) ?? 1, step = max(1, Int(8000 * dur / 700))
        while let sb = o.copyNextSampleBuffer(), let bb = CMSampleBufferGetDataBuffer(sb) {
            var len = 0; var ptr: UnsafeMutablePointer<CChar>?
            CMBlockBufferGetDataPointer(bb, atOffset: 0, lengthAtOffsetOut: nil, totalLengthOut: &len, dataPointerOut: &ptr)
            guard let ptr else { continue }
            ptr.withMemoryRebound(to: Int16.self, capacity: len / 2) { s in
                for i in 0..<(len / 2) { cur = max(cur, s[i] == .min ? .max : abs(s[i])); n += 1; if n >= step { peaks.append(Float(cur) / 32767); cur = 0; n = 0 } }
            }
        }
        return peaks
    }
}
func clock(_ t: Double) -> String { String(format: "%d:%02d.%d", Int(t) / 60, Int(t) % 60, Int((t - floor(t)) * 10)) }

struct TrimEditor: View {
    let path: String; let close: () -> Void
    @StateObject var m: TrimModel
    init(path: String, close: @escaping () -> Void) { self.path = path; self.close = close; _m = StateObject(wrappedValue: TrimModel(path)) }
    var body: some View {
        VStack(spacing: 10) {
            if m.failed { Text(T("This file can't be played here, so it can't be trimmed.")).frame(maxHeight: .infinity) }
            else {
                VideoPlayer(player: m.player)
                GeometryReader { g in
                    let w = g.size.width, x = { (t: Double) in m.dur > 0 ? CGFloat(t / m.dur) * w : 0 }
                    ZStack(alignment: .topLeading) {
                        RoundedRectangle(cornerRadius: 6).fill(.quaternary)
                        Canvas { c, s in
                            let n = max(m.wave.count, 1)
                            for (i, p) in m.wave.enumerated() { let h = CGFloat(p) * s.height, px = CGFloat(i) / CGFloat(n) * s.width; c.fill(Path(CGRect(x: px, y: (s.height - h) / 2, width: max(1, s.width / CGFloat(n)), height: h)), with: .color(.secondary)) }
                        }
                        Rectangle().fill(Store.shared.accent.opacity(0.3)).frame(width: max(0, x(m.b) - x(m.a))).offset(x: x(m.a))
                        Rectangle().fill(.primary).frame(width: 1).offset(x: x(m.now))
                        handle(x(m.a)).gesture(DragGesture().onChanged { v in m.a = max(0, min(m.b - 0.1, Double(v.location.x / w) * m.dur)); seek(m.a) })
                        handle(x(m.b)).gesture(DragGesture().onChanged { v in m.b = min(m.dur, max(m.a + 0.1, Double(v.location.x / w) * m.dur)); seek(m.b) })
                    }.contentShape(Rectangle())
                }.frame(height: 64)
                HStack {
                    Text("\(clock(m.a)) → \(clock(m.b))  (\(clock(max(0, m.b - m.a))))").monospacedDigit()
                    Button(T("Set start (I)")) { m.a = min(m.now, m.b - 0.1) }.keyboardShortcut("i", modifiers: [])
                    Button(T("Set end (O)")) { m.b = max(m.now, m.a + 0.1) }.keyboardShortcut("o", modifiers: [])
                    Button(T("Preview")) { seek(m.a); m.player.play(); DispatchQueue.main.asyncAfter(deadline: .now() + m.b - m.a) { m.player.pause() } }
                    Spacer()
                }
            }
            EditorBar(dirty: m.a > 0 || m.b < m.dur, canSave: !m.failed && m.b > m.a, close: close) { Jobs.shared.queue([path], act("trim", ["start": .n(m.a), "end": .n(m.b)]), title: "Trimming") }
        }.padding(14)
    }
    func handle(_ x: CGFloat) -> some View { RoundedRectangle(cornerRadius: 2).fill(Store.shared.accent).frame(width: 8).offset(x: x - 4) }
    func seek(_ t: Double) { m.player.seek(to: CMTime(seconds: t, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero) }
}

struct PageItem: Identifiable, Equatable { let id = UUID(); let index: Int; var rot = 0 }

struct PagesEditor: View {
    let path: String; let close: () -> Void
    @State var doc: PDFDocument?
    @State var items: [PageItem] = []
    @State var orig: [PageItem] = []
    @State var thumbs: [Int: NSImage] = [:]
    var body: some View {
        VStack(spacing: 0) {
            if doc == nil { Text(T("This PDF couldn't be opened.")).frame(maxHeight: .infinity) }
            else {
                List {
                    ForEach(items) { p in
                        HStack(spacing: 12) {
                            Image(systemName: "line.3.horizontal").foregroundStyle(.secondary)
                            if let t = thumbs[p.index] { Image(nsImage: t).resizable().aspectRatio(contentMode: .fit).frame(width: 90, height: 120).rotationEffect(.degrees(Double(p.rot))).background(Color.white) }
                            Text("\(T("Page")) \(p.index + 1)")
                            Spacer()
                            Button { rotate(p) } label: { Image(systemName: "rotate.right") }
                            Button(role: .destructive) { items.removeAll { $0.id == p.id } } label: { Image(systemName: "trash") }.disabled(items.count == 1)
                        }
                    }.onMove { items.move(fromOffsets: $0, toOffset: $1) }
                }
            }
            HStack { Button(T("Reverse order")) { items.reverse() }; Text("\(items.count) \(T("pages"))").foregroundStyle(.secondary); Spacer() }.padding(.horizontal, 12).padding(.top, 8)
            EditorBar(dirty: items != orig, canSave: doc != nil && !items.isEmpty, close: close) { Jobs.shared.queue([path], act("pages", ["order": .a(items.map { .n(Double($0.index)) }), "rotate": .a(items.map { .n(Double($0.rot)) })]), title: "Saving pages") }
        }.onAppear { load() }
    }
    func rotate(_ p: PageItem) { if let i = items.firstIndex(where: { $0.id == p.id }) { items[i].rot = (items[i].rot + 90) % 360 } }
    func load() {
        guard let d = PDFDocument(url: URL(fileURLWithPath: path)) else { return }
        doc = d; items = (0..<d.pageCount).map { PageItem(index: $0) }; orig = items
        // long documents fill in as they render instead of holding up the window
        DispatchQueue.global(qos: .userInitiated).async {
            for i in 0..<d.pageCount { if let t = d.page(at: i)?.thumbnail(of: NSSize(width: 180, height: 240), for: .cropBox) { DispatchQueue.main.async { thumbs[i] = t } } }
        }
    }
}

struct SignEditor: View {
    let path: String; let close: () -> Void
    @State var doc: PDFDocument?
    @State var page = 0
    @State var strokes: [[CGPoint]] = []
    @State var cur: [CGPoint] = []
    @State var typed = ""
    @State var mode = 0
    @State var saved: NSImage?
    @State var useSaved = false
    @State var place: CGRect?
    @State var startDrag: CGPoint?
    var body: some View {
        HStack(spacing: 0) {
            GeometryReader { g in
                if let d = doc, let p = d.page(at: page) {
                    let b = p.bounds(for: .cropBox), rs = p.rotation % 180 != 0
                    let w0 = rs ? b.height : b.width, h0 = rs ? b.width : b.height
                    let s = min((g.size.width - 30) / w0, (g.size.height - 30) / h0), sz = CGSize(width: w0 * s, height: h0 * s)
                    ZStack(alignment: .topLeading) {
                        Image(nsImage: p.thumbnail(of: NSSize(width: sz.width * 2, height: sz.height * 2), for: .cropBox)).resizable().frame(width: sz.width, height: sz.height)
                        if let r = place { signaturePreview.frame(width: r.width * sz.width, height: r.height * sz.height).border(Store.shared.accent).offset(x: r.minX * sz.width, y: r.minY * sz.height) }
                        Color.white.opacity(0.001).frame(width: sz.width, height: sz.height).gesture(DragGesture(minimumDistance: 2).onChanged { v in
                            let x0 = min(v.startLocation.x, v.location.x), y0 = min(v.startLocation.y, v.location.y)
                            place = CGRect(x: max(0, x0 / sz.width), y: max(0, y0 / sz.height), width: min(1, abs(v.translation.width) / sz.width), height: min(1, abs(v.translation.height) / sz.height))
                        })
                    }.frame(width: sz.width, height: sz.height).frame(maxWidth: .infinity, maxHeight: .infinity)
                } else { Text(T("This PDF couldn't be opened.")).frame(maxWidth: .infinity, maxHeight: .infinity) }
            }.background(Color.black.opacity(0.25))
            Divider()
            VStack(alignment: .leading, spacing: 10) {
                Text(T("Sign")).font(.headline)
                if let d = doc, d.pageCount > 1 { Stepper("\(T("Page")) \(page + 1) / \(d.pageCount)", value: $page, in: 0...(d.pageCount - 1)) }
                Picker("", selection: $mode) { Text(T("Draw")).tag(0); Text(T("Type")).tag(1); if saved != nil { Text(T("Saved")).tag(2) } }.pickerStyle(.segmented)
                if mode == 0 {
                    pad.frame(height: 130).background(.white, in: RoundedRectangle(cornerRadius: 8))
                    HStack { Button(T("Clear")) { strokes = [] }; Button(T("Remember this signature")) { remember() }.disabled(strokes.isEmpty) }.controlSize(.small)
                } else if mode == 1 { TextField(T("Your name"), text: $typed).textFieldStyle(.roundedBorder) }
                else if let saved { Image(nsImage: saved).resizable().aspectRatio(contentMode: .fit).frame(height: 100); Button(T("Forget it")) { Keychain.set("signature", ""); self.saved = nil; mode = 0 }.controlSize(.small) }
                Text(T("Then drag a box on the page where it should go.")).font(.caption).foregroundStyle(.secondary)
                Spacer()
                EditorBar(dirty: !strokes.isEmpty || !typed.isEmpty, canSave: place != nil && render() != nil, close: close, save: save)
            }.padding(12).frame(width: 300)
        }.onAppear {
            doc = PDFDocument(url: URL(fileURLWithPath: path))
            if let d = Data(base64Encoded: Keychain.get("signature")), let i = NSImage(data: d) { saved = i; mode = 2 }
        }
    }
    var pad: some View {
        Canvas { c, _ in
            for s in strokes + [cur] where s.count > 1 { var p = Path(); p.addLines(s); c.stroke(p, with: .color(.black), style: StrokeStyle(lineWidth: 2.4, lineCap: .round, lineJoin: .round)) }
        }.gesture(DragGesture(minimumDistance: 0).onChanged { cur.append($0.location) }.onEnded { _ in strokes.append(cur); cur = [] })
    }
    @ViewBuilder var signaturePreview: some View { if let i = render() { Image(nsImage: i).resizable().aspectRatio(contentMode: .fit) } }
    /// Cropped to the ink, so a signature fills the box you drew.
    func render() -> NSImage? {
        if mode == 2 { return saved }
        if mode == 1 {
            if typed.isEmpty { return nil }
            let f = NSFont(name: "Snell Roundhand", size: 96) ?? .systemFont(ofSize: 96)
            let s = NSAttributedString(string: typed, attributes: [.font: f, .foregroundColor: NSColor.black]); let z = s.size()
            return NSImage(size: z, flipped: false) { _ in s.draw(at: .zero); return true }
        }
        let pts = strokes.flatMap { $0 }
        guard let x0 = pts.map(\.x).min(), let x1 = pts.map(\.x).max(), let y0 = pts.map(\.y).min(), let y1 = pts.map(\.y).max() else { return nil }
        let b = CGRect(x: x0 - 3, y: y0 - 3, width: max(x1 - x0, 1) + 6, height: max(y1 - y0, 1) + 6), scale = 4.0
        let list = strokes
        return NSImage(size: NSSize(width: b.width * scale, height: b.height * scale), flipped: true) { _ in
            NSColor.black.setStroke()
            for s in list where s.count > 1 { let p = NSBezierPath(); p.lineWidth = 2.4 * scale; p.lineCapStyle = .round; p.lineJoinStyle = .round; p.move(to: NSPoint(x: (s[0].x - b.minX) * scale, y: (s[0].y - b.minY) * scale)); for q in s.dropFirst() { p.line(to: NSPoint(x: (q.x - b.minX) * scale, y: (q.y - b.minY) * scale)) }; p.stroke() }
            return true
        }
    }
    func png(_ i: NSImage) -> Data? {
        guard let t = i.tiffRepresentation else { return nil }
        return NSBitmapImageRep(data: t)?.representation(using: .png, properties: [:])
    }
    func remember() { if let i = render(), let d = png(i) { Keychain.set("signature", d.base64EncodedString()); saved = i } }
    func save() {
        guard var r = place, let i = render(), let d = png(i) else { return }
        // keep the signature's own proportions inside the box
        if let p = doc?.page(at: page) {
            let b = p.bounds(for: .cropBox), rs = p.rotation % 180 != 0, pw = rs ? b.height : b.width, ph = rs ? b.width : b.height
            let boxAspect = (r.width * pw) / (r.height * ph), ink = i.size.width / i.size.height
            if ink > boxAspect { r.size.height = (r.width * pw / ink) / ph } else { r.size.width = (r.height * ph * ink) / pw }
        }
        Jobs.shared.queue([path], act("stamp", ["page": .n(Double(page)), "rect": .a([.n(r.minX), .n(r.minY), .n(r.width), .n(r.height)]), "png": .s("data:image/png;base64," + d.base64EncodedString())]), title: "Signing")
    }
}
