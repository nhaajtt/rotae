import Foundation

public struct AskField: Hashable, Codable { public var key, label: String; public var kind = "text"; public var confirm = false }

public struct MenuItem: Identifiable, Hashable {
    public var id: String
    public var label: String
    public var symbol = "circle"
    public var title: String?
    public var action: Action?
    public var sub: [MenuItem] = []
    public var def = 0
    public var editor: String?
    public var app: String?
    public var shelf = false
    public var share: String?
    public var ask: [AskField] = []
    public var danger: String?
    public var keywords = ""
    /// Done by the app itself: open, reveal, trash, tags…
    public var local: String?
    public var from: String?
    /// In a mixed selection: the one kind of file this applies to.
    public var only: String?

    public init(_ label: String, _ symbol: String = "circle", title: String? = nil, action: Action? = nil, sub: [MenuItem] = [], def: Int = 0, id: String? = nil) {
        self.id = id ?? label.lowercased(); self.label = label; self.symbol = symbol; self.title = title; self.action = action; self.sub = sub; self.def = def
    }
    /// What dropping here does when no sub-option is chosen.
    public var defaultChoice: MenuItem? {
        if editor != nil || action != nil || app != nil || shelf || share != nil || local != nil { return self }
        return sub.indices.contains(def) ? sub[def].defaultChoice : nil
    }
}

func with(_ i: MenuItem, _ f: (inout MenuItem) -> Void) -> MenuItem { var i = i; f(&i); return i }
let n = { (x: Double) in JSON.n(x) }

func same(_ e: String, _ to: String) -> Bool { e == to || (e == "jpeg" && to == "jpg") || (e == "tif" && to == "tiff") }
func convert(_ to: String) -> MenuItem { MenuItem(to.uppercased(), title: "Converting to \(to.uppercased())", action: act("convert", ["to": .s(to)])) }
func convertItem(_ t: [String], _ e: String, _ label: String = "Convert") -> MenuItem {
    MenuItem(label, "arrow.left.arrow.right", sub: t.filter { !same(e, $0) }.map(convert))
}
func levels(_ title: String, _ names: [String] = ["Light", "Med", "Strong"]) -> MenuItem {
    MenuItem("Compress", "arrow.down.right.and.arrow.up.left", sub: zip(names, ["low", "medium", "high"]).map { MenuItem($0, title: title, action: act("compress", ["level": .s($1)])) }, def: 1)
}
let clean = MenuItem("Clean", "eraser", title: "Removing metadata", action: act("strip_metadata"))
func merge(_ t: String = "Merging") -> MenuItem { MenuItem("Merge", "plus.square.on.square", title: t, action: act("merge")) }
func editor(_ label: String, _ sym: String, _ e: String, keywords: String = "") -> MenuItem { with(MenuItem(label, sym)) { $0.editor = e; $0.keywords = keywords } }
let rotateItem = MenuItem("Rotate", "rotate.right", sub: [90, 180, 270].map { d in MenuItem("\(d)°", title: "Rotating \(d)°", action: act("rotate", ["deg": n(Double(d))])) })
let toPhone = with(MenuItem("To phone", "iphone.and.arrow.forward")) { $0.share = "phone"; $0.keywords = "send share wifi lan qr mobile transfer airdrop" }
let upload = with(MenuItem("Upload link", "icloud.and.arrow.up")) { $0.share = "upload"; $0.keywords = "share r2 cloudflare link cloud" }
let shelfItem = with(MenuItem("Shelf", "tray.and.arrow.down")) { $0.shelf = true; $0.keywords = "keep later stash basket collect" }
func kw(_ i: MenuItem, _ k: String) -> MenuItem { with(i) { $0.keywords = k } }
func asks(_ i: MenuItem, _ a: [AskField]) -> MenuItem { with(i) { $0.ask = a } }
let pw = AskField(key: "password", label: "Password", kind: "password", confirm: true)
let pwOnce = AskField(key: "password", label: "Password", kind: "password")
let zipItem = MenuItem("Zip", "doc.zipper", title: "Zipping", action: act("zip"))
let hashItem = kw(MenuItem("Hash", "number", sub: ["md5", "sha1", "sha256", "sha512"].map { a in MenuItem(a.uppercased().replacingOccurrences(of: "SHA", with: "SHA-"), title: "Copying \(a.uppercased())", action: act("hashes", ["algo": .s(a)])) } + [MenuItem("All", title: "Writing hash report", action: act("hashes", ["algo": .s("all")]))], def: 2, id: "hashes"), "checksum md5 sha")
let copyPathItem = MenuItem("Copy", "doc.on.doc", title: "Copying path", action: act("copy_path"))
let identifyItem = kw(MenuItem("Real type", "questionmark.diamond", title: "Checking real type", action: act("identify")), "identify magic fake extension disguised")
let vtItem = kw(MenuItem("VirusTotal", "shield.lefthalf.filled", title: "Looking up on VirusTotal", action: act("virus_total")), "malware virus scan reputation")
let encryptItem = kw(asks(MenuItem("Encrypt", "lock", title: "Encrypting", action: act("encrypt")), [pw]), "password protect lock age")
let decryptItem = asks(MenuItem("Decrypt", "lock.open", title: "Decrypting", action: act("decrypt")), [pwOnce])
let shredItem = with(MenuItem("Shred", "flame", title: "Shredding", action: act("shred"))) { $0.danger = "Overwrites the file and deletes it permanently. On Mac SSDs this is best effort: earlier copies of the data may remain."; $0.keywords = "secure delete wipe destroy" }
let locksItem = kw(MenuItem("In use by", "lock.rectangle.stack", title: "Finding programs using it", action: act("locks")), "locked who using process lsof")
let pathAsItem = MenuItem("Copy path as", "link", sub: [("Quoted", "quoted"), ("file://", "url"), ("Markdown", "markdown")].map { MenuItem($0, title: "Copying path", action: act("copy_path_as", ["style": .s($1)])) }, id: "copy-path-as")
let base64Item = kw(MenuItem("Base64", "textformat.abc", title: "Encoding Base64", action: act("base64", ["decode": .b(false)])), "encode data url")
func local(_ label: String, _ sym: String, _ id: String, keywords: String = "") -> MenuItem { with(MenuItem(label, sym, id: "local-\(id)")) { $0.local = id; $0.keywords = keywords } }
func native(_ label: String, _ title: String, _ verb: String, _ arg: String = "") -> MenuItem { MenuItem(label, title: title, action: act("native", ["verb": .s(verb), "arg": .s(arg)]), id: "\(verb)-\(arg)") }
/// A native action that sits on a ring by itself, with its own icon.
func tool(_ label: String, _ sym: String, _ title: String, _ verb: String, _ arg: String = "", id: String, keywords: String = "") -> MenuItem {
    with(MenuItem(label, sym, title: title, action: act("native", ["verb": .s(verb), "arg": .s(arg)]), id: id)) { $0.keywords = keywords }
}
let searchablePDF = tool("OCR to PDF", "doc.text.magnifyingglass", "Reading the text", "ocrpdf", id: "ocrpdf", keywords: "ocr scan scanned receipt invoice find select copy text layer")
let dataURI = tool("Data URI", "curlybraces.square", "Copying data URI", "dev", "datauri", id: "datauri", keywords: "base64 css html embed inline developer")
let totalsItem = tool("Totals", "sum", "Adding up", "sheet", "totals", id: "totals", keywords: "sum add total average count column accounting ledger money")
let officeMedia = tool("Pictures", "photo.stack", "Getting pictures", "office", "media", id: "office-media", keywords: "extract images media embedded photos")
let officeClean = tool("Remove names", "person.crop.circle.badge.xmark", "Removing names", "office", "clean", id: "office-clean", keywords: "author metadata company privacy anonymous reviewer tracked changes comments")
let officeStats = tool("Count", "textformat.123", "Counting", "office", "stats", id: "office-stats", keywords: "words pages slides rows statistics")
let invoiceLog = tool("Invoice log", "list.bullet.rectangle", "Reading the invoice", "ledger", "invoice", id: "invoice-log", keywords: "hoá đơn hóa đơn invoice receipt register bảng kê accounting excel row tax code mst vat total scan ocr")
let tableTools: [MenuItem] = [
    tool("Tidy table", "wand.and.stars", "Tidying the table", "ledger", "tidy", id: "tidy-table", keywords: "clean trim spaces duplicates empty rows dọn trùng"),
    kw(MenuItem("Split table", "rectangle.split.3x1", sub: [native("By first column", "Splitting the table", "ledger", "split"), asks(native("By column…", "Splitting the table", "split-by"), [AskField(key: "arg", label: "Column header")])], id: "split-table"), "split by column value one file per customer branch month tách chia"),
]
let reconcileItem = tool("Reconcile", "arrow.left.arrow.right.square", "Matching the two tables", "ledger", "reconcile", id: "reconcile", keywords: "đối chiếu bank statement sao kê ledger sổ cái match missing difference accounting")
let mailMerge = tool("Mail merge", "envelope.badge.person.crop", "Filling in the template", "ledger", "mailmerge", id: "mailmerge", keywords: "trộn thư template letters contracts names list {{ }} word")
let devFolder: [MenuItem] = [
    tool("Count code", "number.square", "Counting the code", "kit", "project", id: "kit-project", keywords: "lines of code loc cloc languages stats report largest files project"),
    tool("Find secrets", "key.viewfinder", "Looking for secrets", "kit", "secrets", id: "kit-secrets", keywords: "credentials api key token password leak aws github env private key scan before commit share"),
    tool("Pack clean", "shippingbox", "Packing without the clutter", "kit", "pack", id: "kit-pack", keywords: "zip source share handoff without node_modules git env archive send"),
]
let logDigest = tool("Log digest", "list.bullet.indent", "Reading the log", "kit", "log", id: "kit-log", keywords: "log errors summary group exceptions warnings crash analyse count")
let insidePackage = tool("What's inside", "shippingbox.and.arrow.backward", "Looking inside", "kit", "bundle", id: "kit-bundle", keywords: "ipa apk aab jar size contents info.plist bundle id permissions frameworks dex signed")
let swiftUIShape = tool("SwiftUI shape", "swift", "Writing the shape", "kit", "swiftui", id: "swiftui-shape", keywords: "svg path shape swift icon vector xcode code")
let hidePrivate = tool("Hide private", "eye.slash", "Hiding private details", "redact", id: "auto-redact", keywords: "redact censor black out id card phone email account number cccd privacy sensitive personal automatic")
/// The language model that ships with macOS: nothing leaves the Mac, so no key and no network.
func onDevice(meeting: Bool = true) -> [MenuItem] {
    var out = [tool("Summarize", "text.alignleft", "Summarising on this Mac", "think", "summary", id: "think-summary", keywords: "summary tl;dr ai on-device offline private overview"),
               asks(tool("Ask", "questionmark.bubble", "Reading for the answer", "ask", id: "think-ask", keywords: "question answer find where clause ai chat document"), [AskField(key: "arg", label: "Your question")]),
               kw(MenuItem("Translate", "character.bubble", sub: [native("Tiếng Việt", "Translating on this Mac", "think", "translate:Vietnamese"), native("English", "Translating on this Mac", "think", "translate:English"), native("中文", "Translating on this Mac", "think", "translate:Chinese"), native("日本語", "Translating on this Mac", "think", "translate:Japanese"), native("한국어", "Translating on this Mac", "think", "translate:Korean")], id: "think-translate"), "translation language offline on-device")]
    out.append(tool("Name it", "character.cursor.ibeam", "Reading for a name", "think", "name", id: "think-name", keywords: "rename file name title descriptive date invoice scan organise"))
    if meeting { out.insert(tool("To-do list", "checklist", "Finding decisions and to-dos", "think", "actions", id: "think-actions", keywords: "meeting minutes action items decisions notes tasks who when"), at: 1) }
    return out
}
let verifyItem = with(MenuItem("Check download", "checkmark.shield", id: "local-verify")) { $0.local = "verify"; $0.keywords = "checksum sha256 md5 hash verify integrity compare clipboard" }
let tagsItem = MenuItem("Finder tag", "tag", sub: ["Red", "Orange", "Yellow", "Green", "Blue", "Purple", "Gray"].map { c in with(MenuItem(c, id: "tag-\(c)")) { $0.local = "tag:\(c)" } } + [with(MenuItem("Remove tags", id: "tag-none")) { $0.local = "tag:" }], id: "tags")
func fileTools() -> [MenuItem] { [local("Open", "arrow.up.right.square", "open"), local("Show in Finder", "folder", "reveal"), local("Quick Look", "eye", "peek", keywords: "preview"), local("Copy file", "doc.on.clipboard", "copy-file"), local("Duplicate", "plus.square.on.square", "duplicate"), local("Move to Trash", "trash", "trash", keywords: "delete remove"), tagsItem, local("Terminal here", "terminal", "terminal", keywords: "shell command line cd folder developer"), local("AirDrop", "wave.3.right.circle", "airdrop", keywords: "send share"), local("Mail", "envelope", "mail", keywords: "send email attach"), local("Rename", "pencil.line", "rename", keywords: "batch")] }
func common() -> [MenuItem] { [shelfItem, upload, identifyItem, vtItem, encryptItem, shredItem, locksItem, verifyItem, pathAsItem, base64Item] + fileTools() }

let fitSize = kw(MenuItem("Fit size", "scalemass", sub: [8.0, 10, 25, 50].map { mb in MenuItem("\(Int(mb)) MB", title: "Fitting into \(Int(mb)) MB", action: act("target_size", ["mb": n(mb)])) } + [asks(MenuItem("Custom", title: "Fitting size", action: act("target_size")), [AskField(key: "mb", label: "Size limit (MB)", kind: "number")])], def: 1, id: "fit-size"), "target size limit discord zalo email mb")
let speedItem = MenuItem("Speed", "forward", sub: [0.5, 1.5, 2, 4].map { f in MenuItem("\(f.formatted())×", title: "Changing speed to \(f.formatted())×", action: act("speed", ["factor": n(f)])) }, def: 2)
let cleanAudio = kw(MenuItem("Fix sound", "waveform", sub: [MenuItem("Denoise", title: "Removing noise", action: act("audio_fix", ["mode": .s("denoise")])), MenuItem("Loudness", title: "Normalizing loudness", action: act("audio_fix", ["mode": .s("normalize")]))], id: "clean-audio"), "denoise noise loudness normalize volume")
func subtitles(_ video: Bool) -> MenuItem { kw(MenuItem("Subtitles", "captions.bubble", sub: [MenuItem(".srt", title: "Transcribing", action: act("transcribe", ["burn": .b(false)]))] + (video ? [MenuItem("Burn in", title: "Adding subtitles", action: act("transcribe", ["burn": .b(true)]))] : [])), "transcribe speech to text whisper srt captions") }

public struct Menu { public var center: MenuItem; public var items: [MenuItem]; public var extra: [MenuItem] }

func imageMenu(_ e: String, _ multi: Bool, smart: Bool = false) -> Menu {
    var items = [convertItem(["jpg", "png", "webp", "avif", "gif", "pdf", "ico", "bmp", "tiff"], e), levels("Compressing"),
                 MenuItem("Resize", "arrow.up.left.and.arrow.down.right", sub: [75, 50, 25].map { v in MenuItem("\(v)%", title: "Resizing to \(v)%", action: act("resize", ["value": n(Double(v))])) }, def: 1), toPhone, rotateItem]
    if multi { items.append(merge("Merging into PDF")) } else { items += [editor("Crop", "crop", "crop"), editor("Add BG", "photo.on.rectangle", "addbg"), editor("Redact", "rectangle.fill", "redact")] }
    items.append(clean)
    let x: [MenuItem] = [
        kw(MenuItem("Copy text", "text.viewfinder", sub: [MenuItem("Copy", title: "Reading text", action: act("ocr", ["to": .s("clipboard")])), MenuItem(".txt", title: "Reading text", action: act("ocr", ["to": .s("txt")]))], id: "ocr"), "ocr recognise read text scan"),
        kw(MenuItem("Read QR", "qrcode.viewfinder", title: "Reading QR code", action: act("qr_read")), "qr code scan barcode"),
        kw(MenuItem("Icon", "app.dashed", sub: [MenuItem("App .ico", title: "Making icon", action: act("icon_set", ["web": .b(false)])), MenuItem("Favicons", title: "Making favicons", action: act("icon_set", ["web": .b(true)]))], id: "icon"), "ico favicon app icon"),
        kw(MenuItem("Colours", "paintpalette", title: "Finding colours", action: act("palette")), "palette colors hex"),
        asks(MenuItem("Watermark", "c.circle", title: "Adding watermark", action: act("watermark")), [AskField(key: "text", label: "Watermark text")]),
        kw(MenuItem("To SVG", "scribble.variable", title: "Tracing to SVG", action: act("vectorize")), "vector trace svg"),
        kw(MenuItem("Remove BG", "person.crop.rectangle", title: "Removing background", action: act("remove_bg")), "background cut out transparent ai subject"),
        kw(MenuItem("Blur faces", "face.dashed", title: "Blurring faces", action: act("blur_faces")), "privacy anonymize pixelate"),
        kw(MenuItem("Filters", "camera.filters", sub: [native("Auto enhance", "Enhancing", "filter", "enhance"), native("Black & white", "Applying filter", "filter", "mono"), native("Sepia", "Applying filter", "filter", "sepia"), native("Invert", "Applying filter", "filter", "invert"), native("Sharpen", "Sharpening", "filter", "sharpen"), native("Blur", "Blurring", "filter", "blur"), native("Mirror", "Flipping", "filter", "mirror"), native("Upside down", "Flipping", "filter", "flip")], id: "filters"), "effect color photo grayscale"),
        kw(MenuItem("Fit to", "rectangle.compress.vertical", sub: [3840, 1920, 1280, 1080, 800, 512].map { v in native("\(v)px", "Resizing", "fit", "\(v)") }, def: 2, id: "fit-to"), "longest side web social instagram"),
        kw(MenuItem("Beautify", "sparkles.rectangle.stack", sub: [("White", "#ffffff"), ("Black", "#111111"), ("Sky", "#8ec5ff"), ("Peach", "#ffd2b8")].map { n0, c in MenuItem(n0, title: "Adding background", action: act("add_bg", ["color": .s(c), "padding": n(0.08), "radius": n(0.03), "shadow": n(0.5)]), id: "beautify-\(n0)") }, id: "beautify"), "screenshot background shadow"),
        kw(local("Copy image", "photo.on.rectangle", "copy-image"), "clipboard picture"),
        searchablePDF, invoiceLog, hidePrivate, dataURI,
        tool("App icons", "app.gift", "Making the icon set", "kit", "iconset", id: "app-icons", keywords: "xcode appiconset appicon favicon ios macos webmanifest assets developer"),
        tool("CSS colours", "paintbrush.pointed", "Copying CSS variables", "kit", "cssvars", id: "css-vars", keywords: "css variables custom properties palette root hex theme developer"),
        tool("@2x @1x", "square.stack.3d.down.right", "Making smaller sizes", "dev", "scales", id: "scales", keywords: "retina 3x 2x 1x ios app assets developer export"),
        kw(local("Wallpaper", "desktopcomputer", "wallpaper"), "set desktop background"),
        kw(MenuItem("Upscale", "arrow.up.backward.and.arrow.down.forward", sub: [2, 4].map { s in MenuItem("\(s)×", title: "Upscaling \(s)×", action: act("upscale", ["scale": n(Double(s))])) }), "enlarge super resolution esrgan sharpen"),
    ]
    let x2 = x + (smart ? onDevice(meeting: false) : [])
    let extra = multi ? [kw(MenuItem("Contact sheet", "square.grid.3x3", title: "Making contact sheet", action: act("contact_sheet")), "grid collage")] + x2 + [kw(MenuItem("Slideshow", "play.rectangle", sub: [1, 2, 3, 5].map { s in MenuItem("\(s)s each", title: "Making slideshow", action: act("slideshow", ["seconds": n(Double(s))])) }, def: 1), "video from images")] + common() : x2 + common()
    return Menu(center: convert(same(e, "jpg") ? "png" : "jpg"), items: items, extra: extra)
}
func videoMenu(_ e: String, _ multi: Bool) -> Menu {
    var items = [convertItem(["mp4", "mov", "webm", "mkv", "gif", "avi"], e), levels("Compressing video"), fitSize,
                 MenuItem("Resize", "arrow.up.left.and.arrow.down.right", sub: [1080, 720, 480].map { h in MenuItem("\(h)p", title: "Resizing to \(h)p", action: act("resize", ["value": n(Double(h))])) }, def: 1), toPhone,
                 MenuItem("Audio", "music.note", sub: ["mp3", "m4a", "wav"].map { to in MenuItem(to.uppercased(), title: "Extracting audio", action: act("extract_audio", ["to": .s(to)])) }),
                 MenuItem("Mute", "speaker.slash", title: "Removing sound", action: act("mute"))]
    items.append(multi ? merge() : editor("Trim", "scissors", "trim"))
    items.append(clean)
    let extra = [speedItem, MenuItem("Flip", "arrow.left.and.right.righttriangle.left.righttriangle.right", sub: [MenuItem("Mirror", title: "Flipping", action: act("flip", ["dir": .s("h")])), MenuItem("Upside down", title: "Flipping", action: act("flip", ["dir": .s("v")]))]), rotateItem,
                 kw(MenuItem("Frames", "photo.stack", sub: [1, 5, 10].map { s in MenuItem("every \(s)s", title: "Extracting frames", action: act("frames", ["every": n(Double(s))])) }), "screenshots extract images"),
                 kw(MenuItem("Thumbnail", "photo", title: "Making thumbnail", action: act("thumbnail")), "cover poster"), cleanAudio, subtitles(true)] + common()
    return Menu(center: convert(same(e, "mp4") ? "gif" : "mp4"), items: items, extra: extra)
}
func audioMenu(_ e: String, _ multi: Bool) -> Menu {
    let items = [convertItem(["mp3", "m4a", "wav", "flac", "ogg", "opus"], e), levels("Compressing", ["192K", "128K", "96K"]), multi ? merge() : editor("Trim", "scissors", "trim"), toPhone, clean]
    return Menu(center: convert(same(e, "mp3") ? "m4a" : "mp3"), items: items, extra: [fitSize, speedItem, cleanAudio, subtitles(false)] + common())
}
func pdfMenu(_ multi: Bool, smart: Bool = false) -> Menu {
    var items = [convertItem(["jpg", "png", "webp"], "pdf", "Image"), levels("Compressing PDF"), MenuItem("Split", "rectangle.split.1x2", title: "Splitting pages", action: act("split")), toPhone]
    if multi {
        items.append(merge("Merging PDFs"))
        // what a submission portal wants: one file, numbered, and not too heavy
        items.append(kw(MenuItem("Dossier", "books.vertical", title: "Building the dossier", action: act("chain", ["steps": .a([.o(act("merge")), .o(act("page_numbers")), .o(act("compress", ["level": .s("low")]))])]), id: "dossier"), "bundle submit application numbered one file portal upload hồ sơ"))
    } else { items += [editor("Pages", "doc.on.doc", "pages", keywords: "reorder delete remove rotate extract"), editor("Sign", "signature", "sign", keywords: "signature stamp")] }
    items.append(kw(MenuItem("Text", "doc.plaintext", title: "Getting text", action: act("pdf_text")), "ocr extract txt scanned"))
    let extra = [searchablePDF, invoiceLog, hidePrivate] + (multi ? [] : [tool("Statement", "banknote", "Reading the statement", "ledger", "statement", id: "statement", keywords: "bank statement sao kê pdf to excel xlsx table rows")]) + (smart ? onDevice() : []) + [rotateItem, MenuItem("Page numbers", "list.number", title: "Numbering pages", action: act("page_numbers")),
                 kw(asks(MenuItem("Watermark", "c.circle", title: "Adding watermark", action: act("watermark")), [AskField(key: "text", label: "Watermark text")]), "draft confidential"),
                 kw(MenuItem("Images", "photo.stack", title: "Extracting images", action: act("extract_images")), "extract pictures"),
                 kw(asks(MenuItem("Password", "lock", title: "Protecting", action: act("protect")), [pw]), "encrypt lock protect"),
                 kw(asks(MenuItem("Unlock", "lock.open", title: "Unlocking", action: act("unprotect")), [pwOnce]), "remove password decrypt")] + common()
    return Menu(center: convert("jpg"), items: items, extra: extra)
}
func officeMenu(_ e: String, smart: Bool = false, count: Int = 1) -> Menu {
    func to(_ f: String, _ l: String, _ s: String) -> MenuItem { MenuItem(l, s, title: "Converting to \(f.uppercased())", action: act("convert", ["to": .s(f)])) }
    let words = ["doc", "docx", "docm", "odt", "rtf"].contains(e)
    let sheet = ["xlsx", "xlsm"].contains(e), modern = ["docx", "docm", "xlsx", "xlsm", "pptx"].contains(e)
    var items = [to("pdf", "PDF", "arrow.left.arrow.right"), toPhone]
    if words { items.append(to("txt", "Text", "doc.plaintext")) }
    if e != "docx" && words { items.append(to("docx", "DOCX", "doc.richtext")) }
    if e == "docx" || e == "docm" { items.append(tool("Markdown", "m.square", "Converting to Markdown", "paper", "md", id: "markdown", keywords: "md headings tables footnotes pictures lists")) }
    // read straight from the file: no Office or LibreOffice needed
    if sheet { items += [tool("CSV", "tablecells", "Converting to CSV", "sheet", "csv", id: "csv", keywords: "export comma separated sheet"), totalsItem] }
    if sheet { items += tableTools; if count == 2 { items.insert(reconcileItem, at: 0) } }
    if e == "pptx" { items.append(tool("Slide text", "text.below.photo", "Getting text", "office", "text", id: "slide-text", keywords: "outline notes txt")) }
    if modern { items += [officeMedia, officeClean, officeStats,
                          tool("Check to send", "checklist.checked", "Looking for what is hidden", "paper", "check", id: "office-check", keywords: "hidden author tracked changes comments macros links metadata privacy inspect before send sheets notes"),
                          tool("Smaller", "arrow.down.right.and.arrow.up.left", "Shrinking the pictures inside", "paper", "slim", id: "office-slim", keywords: "compress shrink reduce size pictures images email attachment heavy")] }
    return Menu(center: to("pdf", "PDF", "doc"), items: items, extra: (smart && words ? onDevice() : []) + common())
}

public struct AppEntry: Hashable { public let id, label, bundle: String; public let kinds: [String]; public init(id: String, label: String, bundle: String, kinds: [String]) { self.id = id; self.label = label; self.bundle = bundle; self.kinds = kinds } }
public let knownApps = [
    AppEntry(id: "vscode", label: "VS Code", bundle: "com.microsoft.VSCode", kinds: ["text", "any"]),
    AppEntry(id: "cursor", label: "Cursor", bundle: "com.todesktop.230313mzl4w4u92", kinds: ["text", "any"]),
    AppEntry(id: "sublime", label: "Sublime", bundle: "com.sublimetext.4", kinds: ["text", "any"]),
    AppEntry(id: "bbedit", label: "BBEdit", bundle: "com.barebones.bbedit", kinds: ["text", "any"]),
    AppEntry(id: "hopper", label: "Hopper", bundle: "com.cryptic-apps.hopper-web-4", kinds: ["exe"]),
    AppEntry(id: "wireshark", label: "Wireshark", bundle: "org.wireshark.Wireshark", kinds: ["pcap"]),
]
func openIn(_ apps: [AppEntry], _ kinds: [String], _ label: String = "Open in") -> MenuItem? {
    let sub = apps.filter { $0.kinds.contains { kinds.contains($0) } }.map { a in with(MenuItem(a.label, id: a.id)) { $0.app = a.id } }
    return sub.isEmpty ? nil : MenuItem(label, "arrow.up.forward.app", sub: sub)
}
func textMenu(_ e: String, _ paths: [String], _ apps: [AppEntry], smart: Bool = false) -> Menu {
    var to = ["pdf"]
    if !["html", "htm"].contains(e) { to.append("html") }
    if ["csv", "tsv"].contains(e) { to.append("json") }
    if e == "json" { to.append("csv") }
    for f in ["json", "yaml", "toml"] where ["json", "yaml", "yml", "toml", "xml"].contains(e) && f != e && !(f == "yaml" && e == "yml") && !to.contains(f) { to.append(f) }
    var items = [convertItem(to, e), toPhone]
    if paths.count == 2 { items.insert(kw(MenuItem("Compare", "arrow.left.arrow.right.square", title: "Comparing", action: act("diff")), "diff"), at: 0) }
    if e == "json" {
        items.append(MenuItem("Format", "curlybraces", sub: [MenuItem("Pretty", title: "Formatting JSON", action: act("format_json", ["pretty": .b(true)])), MenuItem("Minify", title: "Minifying JSON", action: act("format_json", ["pretty": .b(false)])), native("Sort keys", "Sorting keys", "dev", "sortkeys")]))
        items.append(kw(MenuItem("Types", "t.square", sub: [native("TypeScript", "Writing TypeScript types", "dev", "types"), native("Swift", "Writing Swift structs", "kit", "struct=swift"), native("Rust", "Writing Rust structs", "kit", "struct=rust"), native("Go", "Writing Go structs", "kit", "struct=go"), native("JSON Schema", "Writing the schema", "kit", "struct=schema")], id: "types"), "typescript interface ts model schema developer codable serde struct golang"))
    }
    if ["csv", "tsv"].contains(e) {
        items.append(kw(MenuItem("Excel", "tablecells", sub: [native("Workbook .xlsx", "Converting to XLSX", "sheet", "xlsx"), native("CSV Excel reads right", "Fixing for Excel", "sheet", "excel")], id: "excel"), "xlsx spreadsheet workbook bom utf-8 vietnamese accents garbled"))
        items.append(totalsItem)
        items += tableTools
        if paths.count == 2 { items.insert(reconcileItem, at: 0) }
        if paths.count > 1 { items.append(tool("Join tables", "rectangle.stack.badge.plus", "Joining tables", "sheet", "merge", id: "merge-tables", keywords: "merge combine append stack csv months reports")) }
    }
    if e == "xml" { items.append(tool("E-invoice", "doc.text.below.ecg", "Reading the e-invoice", "ledger", "einvoice", id: "einvoice", keywords: "hoá đơn điện tử hóa đơn e-invoice xml hddt thuế tax excel xlsx register bảng kê check totals")) }
    if let o = openIn(apps, ["text"]) { items.append(o) }
    items.append(MenuItem("Copy", "doc.on.doc", sub: [MenuItem("Text", title: "Copying text", action: act("copy_text")), MenuItem("Path", title: "Copying path", action: act("copy_path"))]))
    items += [zipItem, hashItem]
    let lang = Locale.current.language.languageCode?.identifier == "vi" ? "Vietnamese" : "English"
    let claude = [
        kw(MenuItem("Translate", "character.bubble", sub: [MenuItem("Tiếng Việt", title: "Translating", action: act("ask_claude", ["task": .s("translate"), "lang": .s("Vietnamese")])), MenuItem("English", title: "Translating", action: act("ask_claude", ["task": .s("translate"), "lang": .s("English")])), asks(MenuItem("Other…", title: "Translating", action: act("ask_claude", ["task": .s("translate")])), [AskField(key: "lang", label: "Into which language?")])]), "claude ai language vietnamese english"),
        kw(MenuItem("Summarize", "text.alignleft", title: "Summarizing", action: act("ask_claude", ["task": .s("summarize"), "lang": .s(lang)])), "claude ai summary tl;dr"),
    ]
    // with a model on the Mac, that is the plain choice; the online one keeps its own name
    let prose = !["json", "csv", "tsv", "yaml", "yml", "toml", "xml", "log"].contains(e)
    let ai: [MenuItem] = smart && prose ? onDevice() + [kw(MenuItem("Claude", "cloud", sub: claude, id: "claude"), "online api anthropic translate summarize")] : claude
    let tidy = kw(MenuItem("Tidy text", "text.badge.checkmark", sub: [native("Sort lines", "Sorting lines", "text", "sort"), native("Remove duplicates", "Removing duplicate lines", "text", "dedupe"), native("UPPERCASE", "Changing case", "text", "upper"), native("lowercase", "Changing case", "text", "lower"), native("Trim spaces", "Trimming", "text", "trim"), native("Word count", "Counting", "text", "count"), native("Unix line ends (LF)", "Changing line endings", "text", "lf"), native("Windows line ends (CRLF)", "Changing line endings", "text", "crlf")], id: "tidy"), "lines sort unique case count crlf lf line endings newline")
    let word = ["md", "markdown"].contains(e) ? [tool("Word", "doc.richtext", "Converting to DOCX", "paper", "docx", id: "word", keywords: "docx word document")] : []
    let dev: [MenuItem] = (["log", "txt", "out"].contains(e) ? [logDigest] : []) + (paths.count == 1 && !["csv", "tsv", "md", "markdown", "txt", "log"].contains(e) ? [devFolder[1]] : [])
    let script = ["sh", "bash", "py", "rb", "lua", "php"].contains(e) ? [local("Make runnable", "play.square", "chmod", keywords: "chmod executable permission script run +x")] : []
    return Menu(center: convert("pdf"), items: items, extra: ai + word + dev + [tidy, dataURI] + script + [kw(MenuItem("QR code", "qrcode", title: "Making QR code", action: act("qr_make")), "qr"), MenuItem("Decode Base64", "textformat.abc", title: "Decoding Base64", action: act("base64", ["decode": .b(true)]))] + common())
}
func exeMenu(_ multi: Bool, _ apps: [AppEntry], _ tools: Set<String>) -> Menu {
    let info = MenuItem("Info", "info.circle", title: "Reading PE header", action: act("pe_info"))
    var items: [MenuItem] = []
    if let o = openIn(apps, ["exe"], "Reverse") { items.append(o) }
    if !multi { items += [info, MenuItem("Strings", "text.quote", title: "Extracting strings", action: act("strings"))] }
    if let o = openIn(apps, ["text"], "IDE") { items.append(o) }
    items += [hashItem, zipItem]
    var extra = [kw(MenuItem("Signature", "checkmark.seal", title: "Verifying signature", action: act("verify_signature")), "signed certificate codesign")]
    if !multi {
        for (id, label, title, k) in [("capa", "Capabilities", "Running capa", "capa mandiant behaviour"), ("floss", "Hidden strings", "Running FLOSS", "floss obfuscated strings"), ("yara", "YARA scan", "Scanning with YARA", "yara rules malware detect"), ("upx", "Unpack UPX", "Unpacking", "upx packer")] where tools.contains(id) {
            extra.append(kw(MenuItem(label, "cpu", title: title, action: act("tool", ["tool": .s(id)])), k))
        }
    }
    return Menu(center: with(info) { _ in }, items: items, extra: extra + common())
}
func archiveMenu(_ apps: [AppEntry]) -> Menu {
    let ex = MenuItem("Extract", "archivebox", title: "Extracting", action: act("extract"))
    var items = [ex, insidePackage, toPhone, hashItem, copyPathItem]
    if let o = openIn(apps, ["any"]) { items.append(o) }
    return Menu(center: ex, items: items, extra: common())
}
func genericMenu(_ paths: [String], _ apps: [AppEntry], lead: [MenuItem] = []) -> Menu {
    let files = paths.allSatisfy { kindOf($0) != .other || !extOf($0).isEmpty }
    var lead = lead
    if paths.count == 1 {
        let e = extOf(paths[0])
        var dir: ObjCBool = false
        if FileManager.default.fileExists(atPath: paths[0], isDirectory: &dir), dir.boolValue, e != "app" { lead += devFolder }
        if ["ipa", "apk", "aab", "jar", "aar", "war"].contains(e) { lead.append(insidePackage) }
        if e == "svg" { lead.append(swiftUIShape) }
    }
    var items = lead + [zipItem, toPhone]
    if files { items.append(hashItem) }
    items.append(copyPathItem)
    if let o = openIn(apps, ["any"]) { items.append(o) }
    var extra = files ? common() : [pathAsItem, shelfItem]
    if files && paths.count == 2 { extra.append(MenuItem("Compare", "arrow.left.arrow.right.square", title: "Comparing", action: act("diff"))) }
    if paths.count == 1 && extOf(paths[0]) == "age" { return Menu(center: decryptItem, items: [decryptItem] + items, extra: extra) }
    if !paths.isEmpty, paths.allSatisfy({ extOf($0) == "app" }) { extra.insert(MenuItem("Signature", "checkmark.seal", title: "Verifying signature", action: act("verify_signature")), at: 0) }
    return Menu(center: zipItem, items: items, extra: extra)
}
let redline = tool("Redline", "text.redaction", "Comparing the two versions", "paper", "compare", id: "redline", keywords: "compare diff changes versions track what changed contract so sánh hợp đồng")
public func buildMenu(_ paths: [String], apps: [AppEntry] = [], tools: Set<String> = []) -> Menu {
    var m = baseMenu(paths, apps: apps, tools: tools)
    // two versions of one document, in whatever formats they were kept
    let prose: Set<String> = ["docx", "docm", "pdf", "md", "markdown", "txt", "rtf", "odt", "html", "htm"]
    if paths.count == 2, paths.allSatisfy({ prose.contains(extOf($0)) }), !m.items.contains(where: { $0.id == "mailmerge" }) { m.items.insert(redline, at: 0) }
    return m
}
func baseMenu(_ paths: [String], apps: [AppEntry] = [], tools: Set<String> = []) -> Menu {
    let kinds = Set(paths.map(kindOf)); let multi = paths.count > 1
    // a Word template and a list of names, dropped together
    let exts = paths.map(extOf)
    if paths.count == 2, exts.contains("docx"), exts.contains(where: { ["csv", "tsv", "xlsx", "xlsm"].contains($0) }) { var m = genericMenu(paths, apps, lead: [mailMerge]); m.center = mailMerge; return m }
    // a statement and a ledger kept in different formats
    if paths.count == 2, exts.allSatisfy({ ["csv", "tsv", "xlsx", "xlsm"].contains($0) }), Set(exts).count == 2, kinds.count == 2 { var m = genericMenu(paths, apps, lead: [reconcileItem]); m.center = reconcileItem; return m }
    let smart = tools.contains("ondevice") && !multi
    if kinds.count == 1, let k = kinds.first {
        let e = extOf(paths[0])
        switch k {
        case .image: return imageMenu(e, multi, smart: smart)
        case .video: return videoMenu(e, multi)
        case .audio: return audioMenu(e, multi)
        case .pdf: return pdfMenu(multi, smart: smart)
        case .text: return textMenu(e, paths, apps, smart: smart)
        case .exe: return exeMenu(multi, apps, tools)
        case .archive where !multi: return archiveMenu(apps)
        case .office: return officeMenu(e, smart: smart, count: paths.count)
        default: break
        }
    }
    // a mixed selection: each kind keeps its own few actions, applied to just those files
    let names: [(FileKind, String, String)] = [(.image, "Images", "photo"), (.pdf, "PDFs", "doc.richtext"), (.video, "Videos", "film"), (.audio, "Audio", "waveform"), (.office, "Documents", "doc.text"), (.text, "Text", "text.alignleft")]
    let byType: [MenuItem] = kinds.count > 1 ? names.filter { kinds.contains($0.0) }.map { k, label, sym in
        let mine = paths.filter { kindOf($0) == k }
        let m = buildMenu(mine, apps: [], tools: tools)
        var picks: [MenuItem] = [], used = Set<String>()
        for (label, c) in [(m.center.label, m.center as MenuItem?)] + m.items.map({ ($0.label, $0.defaultChoice) }) {
            guard var c, c.action != nil, c.ask.isEmpty, c.danger == nil, !usesNetwork(c), used.insert(label).inserted, picks.count < 6 else { continue }
            c.label = label; c.only = k.rawValue; c.id = "only-\(k.rawValue)-\(c.id)"; c.sub = []
            picks.append(c)
        }
        return with(MenuItem("\(label) · \(mine.count)", sym, sub: picks, id: "only-\(k.rawValue)")) { $0.keywords = "by type each kind mixed" }
    }.filter { !$0.sub.isEmpty } : []
    if kinds.isSubset(of: [.image, .pdf]) { return genericMenu(paths, apps, lead: byType + [merge("Merging into PDF")]) }
    return genericMenu(paths, apps, lead: byType)
}

public struct WheelPrefs: Codable, Equatable {
    public var usage: [String: Usage] = [:]
    public var hidden: [String: [String]] = [:]
    public var center: [String: String] = [:]
    public init() {}
}
public struct RecipeStep: Codable, Hashable, Identifiable {
    public var id = UUID(); public var label: String; public var action: Action
    /// Filled in each time the recipe runs.
    public var ask: [AskField] = []
    public init(label: String, action: Action, ask: [AskField] = []) { self.label = label; self.action = action; self.ask = ask }
    public init(from d: Decoder) throws { let c = try d.container(keyedBy: K.self); id = (try? c.decode(UUID.self, forKey: .id)) ?? UUID(); label = try c.decode(String.self, forKey: .label); action = try c.decode(Action.self, forKey: .action); ask = (try? c.decode([AskField].self, forKey: .ask)) ?? [] }
    enum K: String, CodingKey { case id, label, action, ask }
}
public struct Recipe: Codable, Hashable, Identifiable { public var id = UUID(); public var name: String; public var kind: String; public var steps: [RecipeStep]; public init(name: String, kind: String, steps: [RecipeStep]) { self.name = name; self.kind = kind; self.steps = steps } }
public struct UserCommand: Codable, Hashable, Identifiable { public var id = UUID(); public var name, command, exts: String; public init(name: String, command: String, exts: String) { self.name = name; self.command = command; self.exts = exts } }
public func extList(_ s: String) -> [String] { s.split(whereSeparator: { " ,;\n".contains($0) }).map { $0.trimmingCharacters(in: CharacterSet(charactersIn: ".")).lowercased() }.filter { !$0.isEmpty } }

public struct Shown { public var center: MenuItem; public var items: [MenuItem]; public var all: [MenuItem]; public var kind: FileKind }

let groups: [(id: String, label: String, symbol: String, members: Set<String>)] = [
    ("g-types", "By type", "square.stack.3d.up", ["only-image", "only-pdf", "only-video", "only-audio", "only-office", "only-text"]),
    ("g-convert", "Convert", "arrow.left.arrow.right", ["convert", "image", "pdf", "text", "docx", "markdown", "word", "audio", "to svg", "icon", "qr code", "decode base64", "base64", "compare", "format", "open in", "reverse", "ide", "excel", "csv", "types", "slide-text", "mailmerge", "redline", "app-icons", "swiftui-shape"]),
    ("g-size", "Size", "arrow.down.right.and.arrow.up.left", ["compress", "resize", "fit-size", "fit-to", "speed", "split", "merge", "upscale", "zip", "extract", "merge-tables", "scales", "dossier", "office-slim", "kit-pack"]),
    ("g-edit", "Edit", "slider.horizontal.3", ["crop", "rotate", "flip", "trim", "mute", "pages", "sign", "add bg", "beautify", "filters", "redact", "watermark", "page numbers", "remove bg", "blur faces", "clean-audio", "tidy", "contact sheet", "slideshow", "tidy-table", "split-table"]),
    ("g-pull", "Get", "text.viewfinder", ["ocr", "read qr", "colours", "frames", "thumbnail", "images", "subtitles", "translate", "summarize", "strings", "info", "copy", "local-copy-image", "local-wallpaper", "ocrpdf", "totals", "office-media", "office-stats", "datauri", "think-summary", "think-actions", "think-ask", "think-translate", "think-name", "claude", "invoice-log", "einvoice", "statement", "reconcile", "kit-project", "kit-log", "kit-bundle", "css-vars"]),
    ("g-share", "Send", "paperplane", ["to phone", "upload link", "shelf", "local-airdrop", "local-mail"]),
    ("g-safe", "Protect", "lock.shield", ["clean", "encrypt", "decrypt", "password", "unlock", "shred", "real type", "virustotal", "signature", "hashes", "capabilities", "hidden strings", "yara scan", "unpack upx", "in use by", "office-clean", "local-verify", "auto-redact", "office-check", "kit-secrets"]),
    ("g-file", "File", "folder", ["local-open", "local-reveal", "local-peek", "local-copy-file", "local-duplicate", "local-trash", "tags", "local-rename", "copy-path-as", "local-terminal", "local-chmod"]),
]
let ringMax = 12

/// More than a ring can hold folds into cells that open their own outer ring, so nothing is ever cut.
func fold(_ items: [MenuItem]) -> [MenuItem] {
    guard items.count > ringMax else { return items }
    let size = max(ringMax, Int(Double(items.count).squareRoot().rounded(.up)))
    return stride(from: 0, to: items.count, by: size).map { start in
        let part = Array(items[start..<min(start + size, items.count)])
        return MenuItem("\(start + 1)–\(start + part.count)", "square.stack", sub: part, id: "fold-\(start)")
    }
}

/// All actions, on the wheel, by purpose. Places never change with use: usage only decides what
/// a straight drop on a wedge does.
public func menuFor(_ paths: [String], prefs: WheelPrefs, recipes: [Recipe], commands: [UserCommand], apps: [AppEntry], tools: Set<String>, now: Double, offline: Bool = false, last: String? = nil) -> Shown {
    let kind = menuKind(paths)
    var raw = buildMenu(paths, apps: apps, tools: tools)
    if offline {
        // offline: what would reach the network is not offered at all
        func local(_ l: [MenuItem]) -> [MenuItem] { l.compactMap { i in if usesNetwork(i) { return nil }; if i.sub.isEmpty { return i }; var c = i; c.sub = local(i.sub); c.def = min(c.def, max(0, c.sub.count - 1)); return c.sub.isEmpty ? nil : c } }
        raw.items = local(raw.items); raw.extra = local(raw.extra)
        if usesNetwork(raw.center) { raw.center = zipItem }
    }
    let own = recipes.filter { $0.kind == kind.rawValue || $0.kind == "any" }.map { r -> MenuItem in
        var i = kw(MenuItem(r.name, "bolt", title: r.name, action: act("chain", ["steps": .a(r.steps.map { .o($0.action) })]), id: "recipe-\(r.id)"), "recipe")
        i.ask = r.steps.enumerated().flatMap { n, s in s.ask.map { AskField(key: "\(n).\($0.key)", label: "\(s.label): \($0.label)", kind: $0.kind, confirm: $0.confirm) } }
        return i
    } + commands.filter { c in extList(c.exts).isEmpty || paths.allSatisfy { extList(c.exts).contains(extOf($0)) } }.map { c in kw(MenuItem(c.name, "terminal", title: c.name, action: act("custom", ["command": .s(c.command)]), id: "cmd-\(c.id)"), "command custom") }
    var seen = Set<String>()
    raw.items = raw.items.filter { seen.insert($0.id).inserted }
    raw.extra = raw.extra.filter { seen.insert($0.id).inserted }
    let all = raw.items + raw.extra + own
    let hidden = Set(prefs.hidden[kind.rawValue] ?? [])
    let visible = (raw.items + raw.extra).filter { !hidden.contains($0.id) }
    let score = { (it: MenuItem) in prefs.usage["\(kind.rawValue):\(it.id)"]?.value(now: now) ?? 0 }
    var rest = visible, wedges: [MenuItem] = []
    func wedge(_ id: String, _ label: String, _ sym: String, _ list: [MenuItem]) {
        guard !list.isEmpty else { return }
        if list.count == 1 { wedges.append(list[0]); return }
        let folded = fold(list)
        let best = folded.indices.max { (score(folded[$0]), -$0) < (score(folded[$1]), -$1) } ?? 0
        wedges.append(MenuItem(label, sym, sub: folded, def: best, id: id))
    }
    for g in groups {
        let inGroup = rest.filter { g.members.contains($0.id) }
        rest.removeAll { g.members.contains($0.id) }
        wedge(g.id, g.label, g.symbol, inGroup)
    }
    // anything not classified still has a place, as does everything the user made
    wedge("g-yours", own.isEmpty ? "More" : "Yours", own.isEmpty ? "ellipsis" : "bolt", rest + own.filter { !hidden.contains($0.id) })
    let pinned = all.first { $0.id == prefs.center[kind.rawValue] }?.defaultChoice
    // "group|choice", as the wheel noted it the last time this kind of file was dropped
    let again: MenuItem? = last.flatMap { l in
        let p = l.split(separator: "|", maxSplits: 1, omittingEmptySubsequences: false).map(String.init)
        guard p.count == 2, let top = visible.first(where: { $0.id == p[0] }) ?? own.first(where: { $0.id == p[0] }) else { return nil }
        let found = top.id == p[1] ? top : leaves([top]).first { $0.id == p[1] }
        // never something that asks or warns first: the centre is a one-drop place
        return found.flatMap { $0.ask.isEmpty && $0.danger == nil && $0.defaultChoice != nil ? $0.defaultChoice : nil }
    }
    return Shown(center: pinned ?? again ?? raw.center, items: wedges, all: all, kind: kind)
}
/// What each kind's Customize list offers.
public func customizable(_ paths: [String]) -> [MenuItem] { let m = buildMenu(paths, apps: [], tools: ["capa", "floss", "yara", "upx", "ondevice"]); var s = Set<String>(); return (m.items + m.extra).filter { s.insert($0.id).inserted } }
/// Every action anywhere on a wheel, for checking that nothing got left off.
public func leaves(_ items: [MenuItem]) -> [MenuItem] { items.flatMap { $0.sub.isEmpty ? [$0] : leaves($0.sub) } }
public func flatActions(_ all: [MenuItem]) -> [MenuItem] {
    all.flatMap { it -> [MenuItem] in
        if it.sub.isEmpty { return [it] }
        return it.sub.map { s in var c = s; c.label = "\(it.label) › \(s.label)"; c.symbol = it.symbol; c.from = it.id; c.keywords = it.keywords; return c }
    }
}
public func matches(_ item: MenuItem, _ q: String) -> Bool {
    let hay = (item.label + " " + (item.title ?? "") + " " + item.keywords).lowercased()
    return q.lowercased().split(separator: " ").allSatisfy { hay.contains($0) }
}
