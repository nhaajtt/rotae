import Foundation
import SDCore

var failed = 0
func check(_ c: @autoclosure () -> Bool, _ m: String, line: Int = #line) { if !c() { failed += 1; print("FAIL line \(line): \(m)") } }

// the wire shape the engine's serde enum expects
let json = String(data: try! JSONEncoder().encode(act("resize", ["value": .n(50)])), encoding: .utf8)!
check(json.contains("\"value\":50") && !json.contains("50.0"), "whole numbers stay integers: \(json)")
check(EngineEvent.parse(Data(#"{"event":"done","outputs":["/a b.png"],"notes":[]}"#.utf8))?.outputs == ["/a b.png"], "decode done")
check(EngineEvent.parse(Data("not json".utf8)) == nil, "garbage is ignored")

typealias G = WheelGeometry
check(G.hit(dx: 0, dy: 0, count: 8, open: nil, subCount: 0) == .center, "centre")
check(G.hit(dx: 0, dy: 100, count: 8, open: nil, subCount: 0) == .wedge(0), "top")
check(G.hit(dx: 100, dy: 0, count: 8, open: nil, subCount: 0) == .wedge(2), "right")
check(G.hit(dx: 0, dy: -100, count: 8, open: nil, subCount: 0) == .wedge(4), "bottom")
check(G.hit(dx: -100, dy: 0, count: 8, open: nil, subCount: 0) == .wedge(6), "left")
check(G.hit(dx: 0, dy: 190, count: 8, open: 0, subCount: 3) == .sub(0, 1), "outer middle")
check(G.hit(dx: 0, dy: 400, count: 8, open: 0, subCount: 3) == .none, "outside")

var u = Usage(); u.bump(now: 0)
check(abs(u.value(now: 30 * 86400) - 0.5) < 0.001, "half-life")

let img = ["/tmp/a.png"]
let s1 = menuFor(img, prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0)
check(s1.kind == .image && s1.items.count <= 8, "at most 8 wedges, got \(s1.items.count)")
check(s1.center.action?["to"]?.string == "jpg", "png → jpg in the centre")
var p = WheelPrefs(); p.usage["image:upscale"] = { var v = Usage(); v.bump(now: 0); return v }()
let s2 = menuFor(img, prefs: p, recipes: [], commands: [], apps: [], tools: [], now: 0)
check(s1.items.map(\.id) == s2.items.map(\.id), "usage must not move wedges")
check(s1.items.map { $0.sub.map(\.id) } == s2.items.map { $0.sub.map(\.id) }, "usage must not move cells")
check(s2.items.first { $0.id == "g-size" }?.defaultChoice?.title?.hasPrefix("Upscaling") == true, "usage picks the straight-drop default")
check(G.hit(dx: 0, dy: 260, count: 8, open: 0, subCount: 3, openCell: 1, leafCount: 3) == .leaf(0, 1, 1), "third ring")
var h = WheelPrefs(); h.hidden["image"] = ["crop"]
check(!leaves(menuFor(img, prefs: h, recipes: [], commands: [], apps: [], tools: [], now: 0).items).contains { $0.label == "Crop" }, "hiding removes it")
let many = (0..<40).map { Recipe(name: "R\($0)", kind: "image", steps: []) }
let big = menuFor(img, prefs: WheelPrefs(), recipes: many, commands: [], apps: [], tools: [], now: 0)
check(big.items.count <= 8 && big.items.allSatisfy { $0.sub.count <= 12 } && leaves(big.items).filter { $0.label.hasPrefix("R") && $0.action?["op"]?.string == "chain" }.count == 40, "40 recipes all fit")
check(isGroup(act("zip")) && !isGroup(act("resize")) && !isGroup(act("hashes", ["algo": .s("all")])), "group actions")
check(!unattendedAllowed(act("shred")) && unattendedAllowed(act("resize")) && !unattendedAllowed(act("chain", ["steps": .a([.o(act("resize")), .o(act("shred"))])])), "unattended allow-list")
check(applyRename(name: "A b.txt", n: 3, pattern: "{name} {n}", find: "", replace: "", regex: false, caseMode: "slug", date: Date()) == "a-b-3.txt", "rename")
check(applyRename(name: "x1.png", n: 1, pattern: "", find: "\\d", replace: "Y", regex: true, caseMode: "keep", date: Date()) == "xY.png", "regex")
for name in ["a.png", "a.mp4", "a.mp3", "a.pdf", "a.txt", "a.docx", "a.exe", "a.zip", "a.bin"] {
    let s = menuFor(["/tmp/\(name)"], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0)
    check(s.items.count <= 8, "\(name) wedge count \(s.items.count)")
    check(s.items.allSatisfy { $0.sub.count <= 12 && $0.sub.allSatisfy { $0.sub.count <= 13 } }, "\(name) rings fit")
    // the promise: everything is on the wheel
    let onWheel = Set(leaves(s.items).map(\.id)), offered = Set(leaves(s.all).map(\.id))
    check(offered.isSubset(of: onWheel), "\(name) missing \(offered.subtracting(onWheel))")
    check(Set(flatActions(s.all).map(\.label)).count > 0, "\(name) actions")
}
// with the Mac's own model, every kind still fits its rings
for name in ["a.png", "a.pdf", "a.txt", "a.md", "a.docx", "a.csv", "a.json", "a.xlsx", "a.pptx", "a.xml", "a.log", "a.svg", "a.zip", "a.ipa"] {
    let s = menuFor(["/tmp/\(name)"], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: ["ondevice", "ffmpeg"], now: 0)
    check(s.items.count <= 8, "\(name) smart wedge count \(s.items.count)")
    check(s.items.allSatisfy { $0.sub.count <= 12 && $0.sub.allSatisfy { $0.sub.count <= 13 } }, "\(name) smart rings fit: \(s.items.map { "\($0.id):\($0.sub.count)" })")
    let onWheel = Set(leaves(s.items).map(\.id)), offered = Set(leaves(s.all).map(\.id))
    check(offered.isSubset(of: onWheel), "\(name) smart missing \(offered.subtracting(onWheel))")
    let ids = (s.all).map(\.id)
    check(Set(ids).count == ids.count, "\(name) duplicate ids")
}
let smart = menuFor(["/tmp/a.txt"], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: ["ondevice"], now: 0)
check(leaves(smart.all).contains { $0.action?["verb"]?.string == "think" } && leaves(smart.all).contains { $0.action?["op"]?.string == "ask_claude" }, "on-device and online both offered")
// offline: nothing that reaches the network is on the wheel at all
let off = menuFor(["/tmp/a.txt"], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: ["ondevice"], now: 0, offline: true)
check(!leaves(off.items).contains { usesNetwork($0) }, "offline hides network actions")
check(leaves(off.items).contains { $0.action?["verb"]?.string == "think" }, "offline keeps the on-device model")
check(usesNetwork(act("chain", ["steps": .a([.o(act("resize")), .o(act("virus_total"))])])) && !usesNetwork(act("resize")), "network actions are recognised inside recipes")
// a mixed drop: each kind keeps its own actions, for its own files
let mixed = menuFor(["/tmp/a.png", "/tmp/b.png", "/tmp/c.pdf", "/tmp/d.mp4"], prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: ["ffmpeg"], now: 0)
check(mixed.items.count <= 8, "mixed wedge count \(mixed.items.count)")
let types = mixed.items.first { $0.id == "g-types" }
check(types?.sub.map(\.id) == ["only-image", "only-pdf", "only-video"], "mixed: by type \(types?.sub.map(\.id) ?? [])")
check(types?.sub.allSatisfy { !$0.sub.isEmpty && $0.sub.count <= 13 && $0.sub.allSatisfy { $0.only != nil && $0.action != nil } } == true, "mixed: every choice is bound to its kind")
check(types?.sub.first?.label == "Images · 2", "mixed: counts \(types?.sub.first?.label ?? "")")
// the centre can repeat the last action, but never one that asks first
let again = menuFor(img, prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0, last: "compress|strong")
check(again.center.action?["level"]?.string == "high", "repeat last: \(again.center.label)")
check(menuFor(img, prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0, last: "watermark|watermark").center.action?["to"]?.string == "jpg", "repeat last skips what asks")
check(menuFor(img, prefs: WheelPrefs(), recipes: [], commands: [], apps: [], tools: [], now: 0, last: "gone|gone").center.action?["to"]?.string == "jpg", "repeat last survives a stale note")
print(failed == 0 ? "selftest ok" : "selftest FAILED: \(failed)")
exit(failed == 0 ? 0 : 1)
