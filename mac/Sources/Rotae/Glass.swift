import AppKit
import SDCore
import SwiftUI

/// Real Liquid Glass on macOS 26 and later; the drawn look everywhere else.
/// Everything that names a macOS 26 API lives in this file, behind `compiler(>=6.2)`, so an
/// older Xcode still builds the app.
enum Liquid {
    static var available: Bool {
        #if compiler(>=6.2)
        if #available(macOS 26, *) { return true }
        #endif
        return false
    }
}

extension View {
    /// A floating card: real glass where the system has it.
    @ViewBuilder func liquidGlass(cornerRadius r: CGFloat) -> some View {
        #if compiler(>=6.2)
        if #available(macOS 26, *) {
            glassEffect(.regular, in: RoundedRectangle(cornerRadius: r, style: .continuous))
        } else { drawnGlass(cornerRadius: r) }
        #else
        drawnGlass(cornerRadius: r)
        #endif
    }
    /// Clear material, light from above and a bright rim: the look on macOS 14 and 15.
    func drawnGlass(cornerRadius r: CGFloat) -> some View {
        background(VisualEffect(material: .popover))
            .overlay(LinearGradient(colors: [.white.opacity(0.20), .white.opacity(0.02)], startPoint: .top, endPoint: .center).allowsHitTesting(false))
            .clipShape(RoundedRectangle(cornerRadius: r, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: r, style: .continuous).strokeBorder(LinearGradient(colors: [.white.opacity(0.65), .white.opacity(0.12)], startPoint: .top, endPoint: .bottom), lineWidth: 1).allowsHitTesting(false))
    }
    /// The whole of a small floating window.
    @ViewBuilder func panelGlass() -> some View {
        #if compiler(>=6.2)
        if #available(macOS 26, *) {
            background { Color.clear.glassEffect(.regular, in: Rectangle()).ignoresSafeArea() }
        } else { background(VisualEffect(material: .sidebar).ignoresSafeArea()) }
        #else
        background(VisualEffect(material: .sidebar).ignoresSafeArea())
        #endif
    }
    /// The main button of a card.
    @ViewBuilder func primaryGlassButton() -> some View {
        #if compiler(>=6.2)
        if #available(macOS 26, *) { buttonStyle(.glassProminent) } else { buttonStyle(.borderedProminent) }
        #else
        buttonStyle(.borderedProminent)
        #endif
    }
    /// Any other button that sits on glass.
    @ViewBuilder func glassButton() -> some View {
        #if compiler(>=6.2)
        if #available(macOS 26, *) { buttonStyle(.glass) } else { buttonStyle(.bordered) }
        #else
        buttonStyle(.bordered)
        #endif
    }
}

struct VisualEffect: NSViewRepresentable {
    var material: NSVisualEffectView.Material = .sidebar
    func makeNSView(context: Context) -> NSVisualEffectView { let v = NSVisualEffectView(); v.material = material; v.blendingMode = .behindWindow; v.state = .active; return v }
    func updateNSView(_ v: NSVisualEffectView, context: Context) {}
}

// MARK: The wheel's glass

/// A slice of a ring, in the wheel's own terms: degrees clockwise from the top.
struct Arc: Equatable {
    var r0: Double, r1: Double, a0: Double, a1: Double
    var mid: Double { (a0 + a1) / 2 }
    /// The same slice, turned by whole circles so it is the shortest trip from `from`.
    func near(_ from: Arc?) -> Arc {
        guard let f = from else { return self }
        let turns = ((f.mid - mid) / 360).rounded()
        return Arc(r0: r0, r1: r1, a0: a0 + turns * 360, a1: a1 + turns * 360)
    }
}

/// What the glass under the wheel should show. The wheel view writes; the glass follows.
final class WheelGlassModel: ObservableObject {
    @Published var ring2: Arc?
    @Published var ring3: Arc?
    @Published var lens1: Arc?
    @Published var lens2: Arc?
    @Published var lens3: Arc?
    @Published var centre = false
    @Published var accent = Color.accentColor

    func set(ring2 a: Arc?, ring3 b: Arc?, lens1 l1: Arc?, lens2 l2: Arc?, lens3 l3: Arc?, centre c: Bool, animated: Bool) {
        // rings opening and closing settle; the lens only follows the pointer, so it glides
        let rings = { [self] in ring2 = a?.near(ring2); ring3 = b?.near(ring3) }
        let lens = { [self] in lens1 = l1?.near(lens1); lens2 = l2?.near(lens2); lens3 = l3?.near(lens3); centre = c }
        if animated { Motion.animate(Motion.settle, rings); Motion.animate(Motion.glide, lens) } else { rings(); lens() }
    }
}

/// A ring slice with soft corners. Animatable, so a lens glides from one cell to the next.
struct Sector: Shape {
    var arc: Arc
    var corner: Double = 10
    var animatableData: AnimatablePair<AnimatablePair<Double, Double>, AnimatablePair<Double, Double>> {
        get { .init(.init(arc.r0, arc.r1), .init(arc.a0, arc.a1)) }
        set { arc = Arc(r0: newValue.first.first, r1: newValue.first.second, a0: newValue.second.first, a1: newValue.second.second) }
    }
    func path(in rect: CGRect) -> Path {
        let c = CGPoint(x: rect.midX, y: rect.midY)
        if arc.a1 - arc.a0 >= 300 {
            // nearly all the way round: a whole ring reads better than a ring with a notch
            var p = Path()
            p.addArc(center: c, radius: arc.r1, startAngle: .zero, endAngle: .degrees(360), clockwise: false)
            p.closeSubpath()
            p.addArc(center: c, radius: arc.r0, startAngle: .degrees(360), endAngle: .zero, clockwise: true)
            p.closeSubpath()
            return p
        }
        let k = max(1, min(corner, (arc.r1 - arc.r0) / 2 - 0.5))
        let rm = (arc.r0 + arc.r1) / 2
        // pull every edge in by the corner, then grow it back with a round stroke
        let pad = min(k / rm * 180 / .pi, (arc.a1 - arc.a0) / 2 - 0.1)
        let a0 = Angle.degrees(arc.a0 + pad - 90), a1 = Angle.degrees(arc.a1 - pad - 90)
        var p = Path()
        p.addArc(center: c, radius: arc.r1 - k, startAngle: a0, endAngle: a1, clockwise: false)
        p.addArc(center: c, radius: arc.r0 + k, startAngle: a1, endAngle: a0, clockwise: true)
        p.closeSubpath()
        return p.strokedPath(StrokeStyle(lineWidth: k * 2, lineJoin: .round)).union(p)
    }
}

/// The lens over whatever is under the pointer. Colour is painted, not left to the system:
/// glass in a panel that never becomes key is drawn as inactive, which would grey the tint.
struct Lens<S: Shape>: View {
    let shape: S, tint: Color
    var body: some View {
        shape.fill(tint.opacity(0.92)).overlay(shape.fill(LinearGradient(colors: [.white.opacity(0.30), .white.opacity(0)], startPoint: .top, endPoint: .bottom)))
            .overlay(shape.stroke(LinearGradient(colors: [.white.opacity(0.75), .white.opacity(0.10)], startPoint: .top, endPoint: .bottom), lineWidth: 1))
            .shadow(color: tint.opacity(0.55), radius: 9, y: 2)
            .transition(.opacity.combined(with: .scale(scale: 0.96)))
    }
}

/// Never takes a click or a drop: the wheel view underneath owns both.
final class PassthroughHost<V: View>: NSHostingView<V> {
    override func hitTest(_ p: NSPoint) -> NSView? { nil }
    override var mouseDownCanMoveWindow: Bool { false }
}

#if compiler(>=6.2)
@available(macOS 26, *)
struct WheelGlass: View {
    @ObservedObject var m: WheelGlassModel
    @Namespace var ns
    var body: some View {
        let g = WheelGeometry.self
        ZStack {
            // the body: one disc, and petals that flow out of it as rings open
            GlassEffectContainer(spacing: 3) {
                ZStack {
                    Color.clear.frame(width: g.r1 * 2, height: g.r1 * 2)
                        .glassEffect(.regular, in: Circle()).glassEffectID("disc", in: ns)
                    if let a = m.ring2 {
                        Color.clear.glassEffect(.regular, in: Sector(arc: a, corner: 18))
                            .glassEffectID("ring2", in: ns).glassEffectTransition(.matchedGeometry)
                    }
                    if let a = m.ring3 {
                        Color.clear.glassEffect(.regular, in: Sector(arc: a, corner: 18))
                            .glassEffectID("ring3", in: ns).glassEffectTransition(.matchedGeometry)
                    }
                }
            }
            if let a = m.lens1 { Lens(shape: Sector(arc: a, corner: 12), tint: m.accent) }
            if let a = m.lens2 { Lens(shape: Sector(arc: a, corner: 12), tint: m.accent) }
            if let a = m.lens3 { Lens(shape: Sector(arc: a, corner: 10), tint: m.accent) }
            ZStack {
                Color.clear.glassEffect(.clear, in: Circle())
                if m.centre { Lens(shape: Circle(), tint: m.accent) }
            }.frame(width: g.core * 2 - 6, height: g.core * 2 - 6)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
#endif

/// The glass layer for the wheel, or nil where the system has no Liquid Glass.
func makeWheelGlass(_ m: WheelGlassModel, frame: NSRect) -> NSView? {
    #if compiler(>=6.2)
    if #available(macOS 26, *) {
        let h = PassthroughHost(rootView: WheelGlass(m: m))
        h.frame = frame; h.autoresizingMask = [.width, .height]
        h.unregisterDraggedTypes() // the drop belongs to the wheel view, whatever SwiftUI asked for
        return h
    }
    #endif
    return nil
}
