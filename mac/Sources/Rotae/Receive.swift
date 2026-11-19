import AppKit
import Network
import SwiftUI

/// Files from a phone to this Mac over the user's own Wi-Fi: a page the phone opens by QR code,
/// served for a few minutes from a port nobody can guess the address of.
final class Inbox: ObservableObject {
    @Published var url: String?
    @Published var got: [String] = []
    @Published var error: String?
    static let limit: Int64 = 4 << 30
    static let folder: URL = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask)[0].appendingPathComponent("Rotae Inbox", isDirectory: true)
    private var listener: NWListener?
    private let q = DispatchQueue(label: "rotae.inbox")
    private var open = 0
    private var idle: DispatchWorkItem?
    fileprivate let token: String = {
        var b = [UInt8](repeating: 0, count: 16)
        _ = SecRandomCopyBytes(kSecRandomDefault, b.count, &b)
        return b.map { String(format: "%02x", $0) }.joined()
    }()

    /// This Mac's address on the local network, Wi-Fi or cable.
    static func address() -> String? {
        var list: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&list) == 0 else { return nil }
        defer { freeifaddrs(list) }
        var p = list
        while let i = p {
            defer { p = i.pointee.ifa_next }
            guard let a = i.pointee.ifa_addr, a.pointee.sa_family == UInt8(AF_INET), String(cString: i.pointee.ifa_name).hasPrefix("en"), Int32(i.pointee.ifa_flags) & IFF_UP != 0 else { continue }
            var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(a, socklen_t(a.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 {
                let s = String(cString: host)
                if !s.hasPrefix("127.") && !s.hasPrefix("169.254.") { return s }
            }
        }
        return nil
    }
    func start() {
        guard listener == nil else { return }
        guard let ip = Self.address() else { error = T("This Mac is not on a network."); return }
        do {
            try FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true)
            let l = try NWListener(using: .tcp, on: .any)
            l.newConnectionHandler = { [weak self] c in
                guard let self, self.open < 8 else { c.cancel(); return }
                self.open += 1
                Conn(c, self).run(on: self.q)
            }
            l.stateUpdateHandler = { [weak self] st in
                DispatchQueue.main.async {
                    guard let self else { return }
                    if case .ready = st, let p = l.port { self.url = "http://\(ip):\(p.rawValue)/\(self.token)" }
                    if case .failed = st { self.error = T("Couldn't open a port on this Mac."); self.stop() }
                }
            }
            l.start(queue: q)
            listener = l
            touch()
        } catch { self.error = T("Couldn't open a port on this Mac.") }
    }
    /// Ten quiet minutes and the door closes by itself.
    fileprivate func touch() {
        DispatchQueue.main.async { [self] in
            idle?.cancel()
            let w = DispatchWorkItem { [weak self] in self?.stop() }
            idle = w
            DispatchQueue.main.asyncAfter(deadline: .now() + 600, execute: w)
        }
    }
    func stop() { idle?.cancel(); listener?.cancel(); listener = nil; url = nil }
    fileprivate func closed() { q.async { self.open = max(0, self.open - 1) } }
    fileprivate func arrived(_ u: URL) {
        // came from another device: macOS should treat it like any download
        try? (u as NSURL).setResourceValue([kLSQuarantineTypeKey as String: kLSQuarantineTypeOtherDownload as String, kLSQuarantineAgentNameKey as String: "Rotae"], forKey: .quarantinePropertiesKey)
        DispatchQueue.main.async { [self] in
            got.append(u.path)
            Shelf.shared.add([u.path])
            Toasts.shared.say("\(T("Received")): \(u.lastPathComponent)")
        }
        touch()
    }
    /// A name from the other side becomes a plain file name here, whatever it tried to be.
    static func safeName(_ raw: String) -> String {
        var n = (raw.replacingOccurrences(of: "\\", with: "/") as NSString).lastPathComponent
        n = String(n.unicodeScalars.filter { $0.value >= 32 && $0 != ":" && $0 != "/" && $0.value != 127 }.map(Character.init))
        while n.hasPrefix(".") { n.removeFirst() }
        n = n.trimmingCharacters(in: .whitespaces)
        if n.utf8.count > 200 { let e = (n as NSString).pathExtension; n = String(n.prefix(80)) + (e.isEmpty || e.count > 10 ? "" : "." + e) }
        return n.isEmpty ? "file" : n
    }
    static func free(_ name: String) -> URL {
        let base = (name as NSString).deletingPathExtension, e = (name as NSString).pathExtension
        var u = folder.appendingPathComponent(name), i = 2
        while FileManager.default.fileExists(atPath: u.path) { u = folder.appendingPathComponent(e.isEmpty ? "\(base) \(i)" : "\(base) \(i).\(e)"); i += 1 }
        return u
    }
    var page: String {
        func h(_ s: String) -> String { T(s).replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: "\"", with: "&quot;") }
        return """
        <!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Rotae</title>
        <style>body{font:17px -apple-system,system-ui,sans-serif;margin:0;padding:28px;background:#f4f4f7;color:#111}@media(prefers-color-scheme:dark){body{background:#111;color:#eee}li{background:#222!important}}
        h1{font-size:24px}label{display:block;padding:36px 16px;border-radius:18px;background:#6b66f5;color:#fff;text-align:center;font-weight:600}input{display:none}
        ul{list-style:none;padding:0}li{background:#fff;border-radius:12px;padding:12px 14px;margin:8px 0;display:flex;justify-content:space-between;gap:12px}li span:first-child{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}</style></head>
        <body><h1>\(h("Send to this Mac"))</h1><label>\(h("Choose photos or files"))<input type="file" multiple id="f"></label><ul id="l"></ul>
        <script>
        const l=document.getElementById('l');
        document.getElementById('f').onchange=async e=>{for(const f of e.target.files){const li=document.createElement('li'),a=document.createElement('span'),b=document.createElement('span');a.textContent=f.name;b.textContent='0%';li.append(a,b);l.prepend(li);
        await new Promise(r=>{const x=new XMLHttpRequest();x.open('POST',location.pathname+'/up?n='+encodeURIComponent(f.name));x.upload.onprogress=p=>{if(p.lengthComputable)b.textContent=Math.round(p.loaded/p.total*100)+'%'};x.onload=()=>{b.textContent=x.status==200?'✓':'✕';r()};x.onerror=()=>{b.textContent='✕';r()};x.send(f)})}e.target.value=''};
        </script></body></html>
        """
    }
}

/// One request, read by hand: a page, or one file streamed to disk.
private final class Conn {
    let c: NWConnection
    unowned let box: Inbox
    var head = Data()
    var file: FileHandle?, part: URL?, name = "", want: Int64 = 0, have: Int64 = 0
    var over = false
    init(_ c: NWConnection, _ box: Inbox) { self.c = c; self.box = box }
    func run(on q: DispatchQueue) {
        c.start(queue: q)
        // a client that connects and says nothing does not get to hold a slot
        q.asyncAfter(deadline: .now() + 20) { [self] in if file == nil && !over { end() } }
        read()
    }
    func end() {
        if over { return }
        over = true
        try? file?.close()
        if let p = part { try? FileManager.default.removeItem(at: p) }
        c.cancel(); box.closed()
    }
    func reply(_ code: String, _ body: String, type: String = "text/plain; charset=utf-8") {
        let b = Data(body.utf8)
        let h = "HTTP/1.1 \(code)\r\nContent-Type: \(type)\r\nContent-Length: \(b.count)\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'\r\nReferrer-Policy: no-referrer\r\n\r\n"
        c.send(content: Data(h.utf8) + b, completion: .contentProcessed { [self] _ in end() })
    }
    func read() {
        c.receive(minimumIncompleteLength: 1, maximumLength: 1 << 18) { [self] d, _, done, err in
            if over { return }
            guard let d, !d.isEmpty, err == nil else { end(); return }
            if file != nil { body(d) } else {
                head.append(d)
                if let r = head.range(of: Data("\r\n\r\n".utf8)) {
                    let rest = head.subdata(in: r.upperBound..<head.count)
                    request(String(decoding: head.subdata(in: 0..<r.lowerBound), as: UTF8.self), rest)
                } else if head.count > 16384 { reply("431 Too Large", "too large"); return }
            }
            if over { return }
            if done { end() } else { read() }
        }
    }
    func request(_ text: String, _ rest: Data) {
        let lines = text.components(separatedBy: "\r\n")
        let first = lines[0].split(separator: " ")
        guard first.count >= 2 else { reply("400 Bad Request", "bad request"); return }
        let method = String(first[0]), target = String(first[1])
        var headers: [String: String] = [:]
        for l in lines.dropFirst() { if let i = l.firstIndex(of: ":") { headers[l[..<i].lowercased()] = l[l.index(after: i)...].trimmingCharacters(in: .whitespaces) } }
        let path = target.split(separator: "?", maxSplits: 1).first.map(String.init) ?? ""
        // the address is the key: anything else gets the same blank answer
        if method == "GET", path == "/\(box.token)" { reply("200 OK", box.page, type: "text/html; charset=utf-8"); return }
        guard method == "POST", path == "/\(box.token)/up" else { reply("404 Not Found", ""); return }
        guard let n = headers["content-length"].flatMap({ Int64($0) }), n > 0 else { reply("411 Length Required", "length required"); return }
        guard n <= Inbox.limit else { reply("413 Too Large", "too large"); return }
        let raw = URLComponents(string: target)?.queryItems?.first { $0.name == "n" }?.value ?? "file"
        name = Inbox.safeName(raw); want = n
        let p = Inbox.folder.appendingPathComponent(".incoming-\(UUID().uuidString)")
        guard FileManager.default.createFile(atPath: p.path, contents: nil, attributes: [.posixPermissions: 0o600]), let h = try? FileHandle(forWritingTo: p) else { reply("500 Error", "can't save"); return }
        file = h; part = p
        box.touch()
        if !rest.isEmpty { body(rest) }
    }
    func body(_ d: Data) {
        let take = min(Int64(d.count), want - have)
        do { try file?.write(contentsOf: d.prefix(Int(take))) } catch { reply("507 No Space", "no space"); return }
        have += take
        guard have >= want else { return }
        try? file?.close(); file = nil
        guard let p = part else { return }
        let to = Inbox.free(name)
        do { try FileManager.default.moveItem(at: p, to: to); try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: to.path); part = nil; box.arrived(to); reply("200 OK", "ok") }
        catch { reply("500 Error", "can't save") }
    }
}

struct ReceiveView: View {
    @StateObject var box = Inbox()
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            title("From phone")
            if let u = box.url {
                if let q = qrImage(u) { Image(nsImage: q).interpolation(.none).resizable().frame(width: 220, height: 220) }
                Text(u).font(.caption.monospaced()).textSelection(.enabled)
                Text(T("Scan it with your phone on the same Wi-Fi, then choose photos or files. They land on the Shelf and in Downloads › Rotae Inbox.")).foregroundStyle(.secondary)
                HStack { Button(T("Stop")) { box.stop() }; Button(T("Show folder")) { NSWorkspace.shared.open(Inbox.folder) } }
            } else if let e = box.error { Text(e).foregroundStyle(.red) }
            else {
                Text(T("Opens a page on your own network only, for ten minutes. Nothing goes to the internet.")).foregroundStyle(.secondary)
                Button(T("Start receiving")) { box.start() }.keyboardShortcut(.defaultAction).disabled(Store.shared.state.offline)
                if Store.shared.state.offline { Text(T("Offline mode is on: this needs the network")).font(.caption).foregroundStyle(.secondary) }
            }
            if !box.got.isEmpty { FileList(paths: box.got.reversed()) }
            Spacer()
        }.padding(20).onDisappear { box.stop() }
    }
}
