import AppKit
import QuartzCore
import SwiftUI

/// One set of curves for the whole app, so every surface moves like part of the same thing.
/// With Reduce Motion on, things change in place instead of travelling.
enum Motion {
    static var reduced: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }
    /// Following the pointer: fast, no overshoot.
    static let glide = Animation.spring(response: 0.24, dampingFraction: 0.86)
    /// Something arriving or changing shape.
    static let settle = Animation.spring(response: 0.34, dampingFraction: 0.80)
    /// Content swapping inside a surface that stays put.
    static let swap = Animation.easeInOut(duration: 0.18)

    static func animate(_ a: Animation, _ body: () -> Void) {
        if reduced { body() } else { withAnimation(a, body) }
    }

    /// A window arriving: fades in while rising `rise` points into place.
    static func present(_ w: NSWindow, at frame: NSRect, rise: CGFloat = 14, from edge: NSRectEdge = .minY) {
        if reduced { w.setFrame(frame, display: true); w.alphaValue = 1; w.orderFrontRegardless(); return }
        // already there: it only glides to its new size
        var start = frame
        if !w.isVisible || w.alphaValue < 0.05 {
            switch edge { case .maxX: start.origin.x += rise; default: start.origin.y -= rise }
            w.setFrame(start, display: true)
            w.alphaValue = 0
        }
        w.orderFrontRegardless()
        NSAnimationContext.runAnimationGroup { c in
            c.duration = 0.30
            c.timingFunction = CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.25, 1) // quick start, long soft landing
            w.animator().setFrame(frame, display: true)
            w.animator().alphaValue = 1
        }
    }

    /// A window leaving: fades while sinking, then is taken off screen unless `still` says it came back.
    static func dismiss(_ w: NSWindow, sink: CGFloat = 8, still: @escaping () -> Bool, then: @escaping () -> Void = {}) {
        guard w.isVisible else { then(); return }
        if reduced { w.orderOut(nil); w.alphaValue = 1; then(); return }
        var end = w.frame; end.origin.y -= sink
        NSAnimationContext.runAnimationGroup({ c in
            c.duration = 0.18
            c.timingFunction = CAMediaTimingFunction(name: .easeIn)
            w.animator().alphaValue = 0
            w.animator().setFrame(end, display: true)
        }, completionHandler: {
            if still() { w.orderOut(nil); w.alphaValue = 1 }
            then()
        })
    }
}
