// Apple-framework work on behalf of the Rust engine: one verb, one process.
import AppKit
import CoreImage
import ImageIO
import PDFKit
import Security
import UniformTypeIdentifiers
import Vision
import WebKit

func fail(_ m: String) -> Never {
    FileHandle.standardError.write(Data((m + "\n").utf8))
    exit(1)
}

let maxPixels = 100_000_000

/// Upright, in sRGB, refusing decode bombs.
func load(_ path: String) -> CGImage {
    let url = URL(fileURLWithPath: path) as CFURL
    guard let src = CGImageSourceCreateWithURL(url, [kCGImageSourceShouldCache: false] as CFDictionary) else { fail("can't read this image") }
    guard let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any],
          let w = props[kCGImagePropertyPixelWidth] as? Int, let h = props[kCGImagePropertyPixelHeight] as? Int else { fail("can't read this image") }
    if w * h > maxPixels { fail("this image is too large") }
    let opts: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: max(w, h),
    ]
    guard let raw = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { fail("can't read this image") }
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    guard let c = CGContext(data: nil, width: raw.width, height: raw.height, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { fail("out of memory") }
    c.draw(raw, in: CGRect(x: 0, y: 0, width: raw.width, height: raw.height))
    return c.makeImage()!
}

func save(_ img: CGImage, _ path: String) {
    let lower = path.lowercased()
    let jpg = lower.hasSuffix(".jpg") || lower.hasSuffix(".jpeg")
    let type = (jpg ? UTType.jpeg : UTType.png).identifier as CFString
    guard let d = CGImageDestinationCreateWithURL(URL(fileURLWithPath: path) as CFURL, type, 1, nil) else { fail("can't write the result") }
    CGImageDestinationAddImage(d, img, jpg ? [kCGImageDestinationLossyCompressionQuality: 0.92] as CFDictionary : nil)
    if !CGImageDestinationFinalize(d) { fail("can't write the result") }
}

func ocr(_ path: String) {
    let img = load(path)
    let req = VNRecognizeTextRequest()
    req.recognitionLevel = .accurate
    req.usesLanguageCorrection = true
    req.automaticallyDetectsLanguage = true
    do { try VNImageRequestHandler(cgImage: img).perform([req]) } catch { fail("text recognition failed: \(error.localizedDescription)") }
    let lines = (req.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    print(lines.joined(separator: "\n"))
}

func subject(_ input: String, _ output: String) {
    let img = load(input)
    let req = VNGenerateForegroundInstanceMaskRequest()
    let h = VNImageRequestHandler(cgImage: img)
    do {
        try h.perform([req])
        guard let r = req.results?.first else { fail("couldn't find a subject in this picture") }
        let buf = try r.generateMaskedImage(ofInstances: r.allInstances, from: h, croppedToInstancesExtent: false)
        let ci = CIImage(cvPixelBuffer: buf)
        guard let cg = CIContext().createCGImage(ci, from: ci.extent, format: .RGBA8, colorSpace: CGColorSpace(name: CGColorSpace.sRGB)) else { fail("couldn't make the result") }
        save(cg, output)
    } catch { fail("couldn't separate the subject: \(error.localizedDescription)") }
}

func faces(_ input: String, _ output: String) {
    let img = load(input)
    let req = VNDetectFaceRectanglesRequest()
    do { try VNImageRequestHandler(cgImage: img).perform([req]) } catch { fail("face detection failed: \(error.localizedDescription)") }
    let found = req.results ?? []
    let w = CGFloat(img.width), h = CGFloat(img.height)
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    let c = CGContext(data: nil, width: img.width, height: img.height, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    c.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
    let base = CIImage(cgImage: img)
    let ctx = CIContext()
    for f in found {
        // 15% extra, as on Windows, so hairlines and chins are covered
        let b = f.boundingBox
        var r = CGRect(x: b.minX * w, y: b.minY * h, width: b.width * w, height: b.height * h)
        r = r.insetBy(dx: -r.width * 0.15, dy: -r.height * 0.15).intersection(CGRect(x: 0, y: 0, width: w, height: h)).integral
        let scale = max(r.width, r.height) / 10
        let pix = base.clampedToExtent().applyingFilter("CIPixellate", parameters: [kCIInputScaleKey: max(scale, 8), kCIInputCenterKey: CIVector(x: r.midX, y: r.midY)])
        if let patch = ctx.createCGImage(pix, from: r, format: .RGBA8, colorSpace: space) { c.draw(patch, in: r) }
    }
    save(c.makeImage()!, output)
    print(found.count)
}

func filter(_ name: String, _ input: String, _ output: String) {
    let base = CIImage(cgImage: load(input))
    var img: CIImage
    switch name {
    case "enhance": img = base.autoAdjustmentFilters().reduce(base) { r, f in f.setValue(r, forKey: kCIInputImageKey); return f.outputImage ?? r }
    case "mono": img = base.applyingFilter("CIPhotoEffectNoir")
    case "sepia": img = base.applyingFilter("CISepiaTone", parameters: [kCIInputIntensityKey: 0.9])
    case "invert": img = base.applyingFilter("CIColorInvert")
    case "sharpen": img = base.applyingFilter("CISharpenLuminance", parameters: [kCIInputSharpnessKey: 0.8])
    case "blur": img = base.clampedToExtent().applyingFilter("CIGaussianBlur", parameters: [kCIInputRadiusKey: max(base.extent.width, base.extent.height) / 100]).cropped(to: base.extent)
    case "mirror": img = base.transformed(by: CGAffineTransform(scaleX: -1, y: 1).translatedBy(x: -base.extent.width, y: 0))
    case "flip": img = base.transformed(by: CGAffineTransform(scaleX: 1, y: -1).translatedBy(x: 0, y: -base.extent.height))
    default: fail("not available")
    }
    guard let cg = CIContext().createCGImage(img, from: base.extent, format: .RGBA8, colorSpace: CGColorSpace(name: CGColorSpace.sRGB)) else { fail("couldn't make the result") }
    save(cg, output)
}

/// Longest side becomes `side`; never enlarges.
func fit(_ side: Int, _ input: String, _ output: String) {
    guard side > 0, side <= 16384 else { fail("that size isn't possible") }
    let img = load(input), s = min(1, Double(side) / Double(max(img.width, img.height)))
    let w = max(1, Int(Double(img.width) * s)), h = max(1, Int(Double(img.height) * s))
    let c = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    c.interpolationQuality = .high
    c.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
    save(c.makeImage()!, output)
}

// MARK: Searchable PDF

/// Words Vision found on a page picture, with where they are (0–1, origin bottom-left).
func recognise(_ img: CGImage) -> [(String, CGRect)] {
    let req = VNRecognizeTextRequest()
    req.recognitionLevel = .accurate
    req.usesLanguageCorrection = true
    req.automaticallyDetectsLanguage = true
    do { try VNImageRequestHandler(cgImage: img).perform([req]) } catch { fail("text recognition failed: \(error.localizedDescription)") }
    return (req.results ?? []).compactMap { o in o.topCandidates(1).first.map { ($0.string, o.boundingBox) } }
}

/// Lays each line over where it was read, drawn in the PDF's "invisible" mode: nothing shows,
/// yet the text can be searched, selected and copied.
func overlay(_ lines: [(String, CGRect)], on c: CGContext, page: CGSize) -> Int {
    var words = 0
    c.saveGState()
    c.setTextDrawingMode(.invisible)
    for (text, box) in lines {
        let r = CGRect(x: box.minX * page.width, y: box.minY * page.height, width: box.width * page.width, height: box.height * page.height)
        guard r.width > 1, r.height > 1 else { continue }
        let font = CTFontCreateWithName("Helvetica" as CFString, max(r.height * 0.78, 1), nil)
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [.font: font]))
        let w = CTLineGetTypographicBounds(line, nil, nil, nil)
        guard w > 0 else { continue }
        c.saveGState()
        c.textMatrix = .identity
        // stretched to the width it was read at, so a selection lines up with the picture
        c.translateBy(x: r.minX, y: r.minY + r.height * 0.2)
        c.scaleBy(x: r.width / CGFloat(w), y: 1)
        c.textPosition = .zero
        CTLineDraw(line, c)
        c.restoreGState()
        words += text.split(whereSeparator: { $0.isWhitespace }).count
    }
    c.restoreGState()
    return words
}

func ocrpdf(_ input: String, _ output: String) {
    let outURL = URL(fileURLWithPath: output) as CFURL
    var words = 0
    if input.lowercased().hasSuffix(".pdf") {
        guard let doc = CGPDFDocument(URL(fileURLWithPath: input) as CFURL) else { fail("can't open PDF") }
        if !doc.isUnlocked { fail("this PDF has a password: unlock it first") }
        guard doc.numberOfPages > 0 else { fail("can't open PDF") }
        if doc.numberOfPages > 2000 { fail("this PDF is too long for this (over 2000 pages)") }
        guard let c = CGContext(outURL, mediaBox: nil, nil) else { fail("can't write the result") }
        let kit = PDFDocument(url: URL(fileURLWithPath: input))
        var already = 0
        for i in 1...doc.numberOfPages {
            autoreleasepool {
                guard let pg = doc.page(at: i) else { return }
                let raw = pg.getBoxRect(.cropBox), turned = pg.rotationAngle % 180 != 0
                let size = turned ? CGSize(width: raw.height, height: raw.width) : raw.size
                guard size.width >= 1, size.height >= 1, size.width < 20000, size.height < 20000 else { return }
                var box = CGRect(origin: .zero, size: size)
                let place = pg.getDrawingTransform(.cropBox, rect: box, rotate: 0, preserveAspectRatio: true)
                c.beginPage(mediaBox: &box)
                c.saveGState(); c.concatenate(place); c.drawPDFPage(pg); c.restoreGState()
                // a page that already has real text is left exactly as it is
                let has = (kit?.page(at: i - 1)?.string ?? "").filter { !$0.isWhitespace }.count > 12
                if has { already += 1 } else {
                    let scale = min(3, 3000 / max(size.width, size.height))
                    let (w, h) = (Int(size.width * scale), Int(size.height * scale))
                    if let b = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) {
                        b.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1)); b.fill(CGRect(x: 0, y: 0, width: w, height: h))
                        b.scaleBy(x: scale, y: scale); b.concatenate(place); b.drawPDFPage(pg)
                        if let img = b.makeImage() { words += overlay(recognise(img), on: c, page: size) }
                    }
                }
                c.endPage()
            }
        }
        c.closePDF()
        if words == 0, already > 0 { try? FileManager.default.removeItem(atPath: output); fail("this PDF already has text that can be searched") }
    } else {
        let img = load(input)
        // an A4-sized page in the picture's own proportions
        let k = 842 / CGFloat(max(img.width, img.height))
        var box = CGRect(x: 0, y: 0, width: CGFloat(img.width) * k, height: CGFloat(img.height) * k)
        // kept as JPEG inside the PDF, or a photo of a receipt becomes tens of megabytes
        let jpeg = NSMutableData()
        guard let d = CGImageDestinationCreateWithData(jpeg, UTType.jpeg.identifier as CFString, 1, nil) else { fail("can't write the result") }
        CGImageDestinationAddImage(d, img, [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        guard CGImageDestinationFinalize(d), let prov = CGDataProvider(data: jpeg), let packed = CGImage(jpegDataProviderSource: prov, decode: nil, shouldInterpolate: true, intent: .defaultIntent),
              let c = CGContext(outURL, mediaBox: &box, nil) else { fail("can't write the result") }
        c.beginPage(mediaBox: &box)
        c.draw(packed, in: box)
        words = overlay(recognise(img), on: c, page: box.size)
        c.endPage(); c.closePDF()
    }
    if words == 0 { try? FileManager.default.removeItem(atPath: output); fail("no text found") }
    print(words)
}

func signature(_ path: String) {
    var code: SecStaticCode?
    guard SecStaticCodeCreateWithPath(URL(fileURLWithPath: path) as CFURL, [], &code) == errSecSuccess, let code else { print("not signed"); return }
    let st = SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSCheckAllArchitectures), nil)
    if st == errSecCSUnsigned { print("not signed"); return }
    if st != errSecSuccess { print("signature is damaged or the file was changed"); return }
    var info: CFDictionary?
    SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &info)
    let d = info as? [String: Any] ?? [:]
    let id = d[kSecCodeInfoIdentifier as String] as? String ?? "?"
    let adhoc = ((d[kSecCodeInfoFlags as String] as? UInt32) ?? 0) & 0x2 != 0
    if let team = d[kSecCodeInfoTeamIdentifier as String] as? String { print("signed by team \(team) (\(id))") }
    else if adhoc { print("signed ad-hoc, no verified developer (\(id))") }
    else { print("signed by the system (\(id))") }
}

/// Prints locked-down HTML with WebKit: JavaScript off and every network load blocked.
final class Printer: NSObject, WKNavigationDelegate {
    let out: String
    var web: WKWebView!
    var window: NSWindow!
    init(out: String) { self.out = out }

    func start(_ url: URL) {
        let rules = #"[{"trigger":{"url-filter":"^https?://.*"},"action":{"type":"block"}},{"trigger":{"url-filter":"^wss?://.*"},"action":{"type":"block"}},{"trigger":{"url-filter":"^ftp://.*"},"action":{"type":"block"}}]"#
        WKContentRuleListStore.default().compileContentRuleList(forIdentifier: "sd-offline-\(getpid())", encodedContentRuleList: rules) { list, err in
            guard let list else { fail("couldn't lock down the page: \(err?.localizedDescription ?? "")") }
            let cfg = WKWebViewConfiguration()
            cfg.websiteDataStore = .nonPersistent()
            cfg.defaultWebpagePreferences.allowsContentJavaScript = false
            cfg.userContentController.add(list)
            self.web = WKWebView(frame: NSRect(x: 0, y: 0, width: 612, height: 792), configuration: cfg)
            self.web.navigationDelegate = self
            self.window = NSWindow(contentRect: self.web.frame, styleMask: [.borderless], backing: .buffered, defer: false)
            self.window.contentView = self.web
            self.web.loadFileURL(url, allowingReadAccessTo: URL(fileURLWithPath: "/"))
        }
    }

    /// Only the document we were given: a page can't redirect itself or a frame anywhere else.
    func webView(_ w: WKWebView, decidePolicyFor a: WKNavigationAction, decisionHandler d: @escaping (WKNavigationActionPolicy) -> Void) {
        if first { first = false; d(.allow) } else { d(.cancel) }
    }
    var first = true

    func webView(_ w: WKWebView, didFinish n: WKNavigation!) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            let info = NSPrintInfo()
            info.paperSize = NSSize(width: 612, height: 792)
            info.topMargin = 36; info.bottomMargin = 36; info.leftMargin = 36; info.rightMargin = 36
            info.horizontalPagination = .fit
            info.verticalPagination = .automatic
            info.jobDisposition = .save
            info.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL] = URL(fileURLWithPath: self.out)
            let op = w.printOperation(with: info)
            op.showsPrintPanel = false
            op.showsProgressPanel = false
            op.view?.frame = NSRect(x: 0, y: 0, width: 540, height: 720)
            op.runModal(for: self.window, delegate: self, didRun: #selector(self.done(_:success:context:)), contextInfo: nil)
        }
    }
    func webView(_ w: WKWebView, didFail n: WKNavigation!, withError e: Error) { fail("couldn't print: \(e.localizedDescription)") }
    func webView(_ w: WKWebView, didFailProvisionalNavigation n: WKNavigation!, withError e: Error) { fail("couldn't print: \(e.localizedDescription)") }
    @objc func done(_ op: NSPrintOperation, success: Bool, context: UnsafeMutableRawPointer?) { exit(success ? 0 : 1) }
}

let a = CommandLine.arguments
if a.count >= 3, let flag = a.first(where: { $0.hasPrefix("--print-to-pdf=") }), let last = a.last, let url = URL(string: last) {
    let app = NSApplication.shared
    app.setActivationPolicy(.prohibited)
    let p = Printer(out: String(flag.dropFirst("--print-to-pdf=".count)))
    p.start(url)
    DispatchQueue.main.asyncAfter(deadline: .now() + 100) { fail("printing took too long") }
    app.run()
} else {
    switch (a.count > 1 ? a[1] : "", a.count) {
    case ("decode", 4): save(load(a[2]), a[3])
    case ("ocr", 3): ocr(a[2])
    case ("subject", 4): subject(a[2], a[3])
    case ("faces", 4): faces(a[2], a[3])
    case ("signature", 3): signature(a[2])
    case ("filter", 5): filter(a[2], a[3], a[4])
    case ("fit", 5): fit(Int(a[2]) ?? 0, a[3], a[4])
    case ("ocrpdf", 4): ocrpdf(a[2], a[3])
    case ("redact", 4): redact(a[2], a[3])
    case ("think", 5): think(a[2], a[3], a[4])
    case ("text", 3): print(plainText(a[2]))
    default: fail("usage: sd-native decode|ocr|subject|faces|signature ...")
    }
}
