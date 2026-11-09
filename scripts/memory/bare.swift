// The floor under Terminal Decline's footprint: a bare AppKit app with
// what any Metal terminal window needs and nothing else, measured beside
// it by scripts/memory/breakdown.py (which builds this with
// `swiftc -O -swift-version 5` and wraps in an app bundle, launched with
// `open` as the terminals are).
//
//   bare [tabs] [text]
//
// `tabs` windows (default 1) of 812×524 points, native tabs of one
// another, each a layer-backed view with a CAMetalLayer of two drawables
// that draws two cleared frames; an app menu with Quit. `text` also makes
// Menlo 13 with Core Text and rasterises ASCII into a bitmap once, as a
// terminal must before it can draw anything.

import AppKit
import CoreText
import Metal
import QuartzCore

let args = Array(CommandLine.arguments.dropFirst())
let tabs = args.compactMap { Int($0) }.first ?? 1
let text = args.contains("text")
// As Decline's DECLINE_TEST_BACKGROUND (crates/term-app/src/background.rs):
// an accessory app whose transparent windows stay behind the others and
// never take focus, so a run on a shared Mac doesn't get in anyone's way.
let background = ProcessInfo.processInfo.environment["DECLINE_TEST_BACKGROUND"].map { !$0.isEmpty && $0 != "0" } ?? false
let device = MTLCreateSystemDefaultDevice()!
let queue = device.makeCommandQueue()!

final class MetalView: NSView {
    override func makeBackingLayer() -> CALayer {
        let l = CAMetalLayer()
        l.device = device
        l.pixelFormat = .bgra8Unorm
        l.maximumDrawableCount = 2
        l.framebufferOnly = true
        return l
    }

    func draw() {
        guard let l = layer as? CAMetalLayer else { return }
        let scale = window?.backingScaleFactor ?? 1
        l.contentsScale = scale
        l.drawableSize = CGSize(width: bounds.width * scale, height: bounds.height * scale)
        guard let d = l.nextDrawable() else { return }
        let rp = MTLRenderPassDescriptor()
        rp.colorAttachments[0].texture = d.texture
        rp.colorAttachments[0].loadAction = .clear
        rp.colorAttachments[0].storeAction = .store
        rp.colorAttachments[0].clearColor = MTLClearColor(red: 0.1, green: 0.1, blue: 0.1, alpha: 1)
        let cb = queue.makeCommandBuffer()!
        cb.makeRenderCommandEncoder(descriptor: rp)!.endEncoding()
        cb.present(d)
        cb.commit()
    }
}

func rasteriseASCII() {
    let font = CTFontCreateWithName("Menlo" as CFString, 13, nil)
    var chars = (32..<127).map { UniChar($0) }
    var glyphs = [CGGlyph](repeating: 0, count: chars.count)
    CTFontGetGlyphsForCharacters(font, &chars, &glyphs, chars.count)
    let w = 16, h = 32
    let ctx = CGContext(data: nil, width: w * glyphs.count, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
    var positions = (0..<glyphs.count).map { CGPoint(x: CGFloat($0 * w), y: 8) }
    CTFontDrawGlyphs(font, &glyphs, &positions, glyphs.count, ctx)
}

final class Delegate: NSObject, NSApplicationDelegate {
    var windows: [NSWindow] = []
    var views: [MetalView] = []

    func applicationDidFinishLaunching(_: Notification) {
        if text { rasteriseASCII() }
        for i in 0..<tabs {
            let w = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 812, height: 524),
                             styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                             backing: .buffered, defer: false)
            w.isReleasedWhenClosed = false
            w.tabbingMode = .preferred
            let v = MetalView(frame: NSRect(x: 0, y: 0, width: 812, height: 524))
            v.wantsLayer = true
            w.contentView = v
            if background { w.alphaValue = 0 }
            if i == 0 {
                if background { w.orderBack(nil) } else { w.makeKeyAndOrderFront(nil) }
            } else {
                windows[0].addTabbedWindow(w, ordered: .above)
                if background { w.orderBack(nil) } else { w.makeKeyAndOrderFront(nil) }
            }
            windows.append(w)
            views.append(v)
        }
        if !background { NSApp.activate(ignoringOtherApps: true) }
        // Two frames, as any terminal draws more than one: the layer then
        // holds both its drawables.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            for v in self.views { v.draw() }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
            for v in self.views { v.draw() }
            print("ready")
            fflush(stdout)
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(background ? .accessory : .regular)
let menu = NSMenu()
let item = NSMenuItem()
menu.addItem(item)
let sub = NSMenu()
sub.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
item.submenu = sub
app.mainMenu = menu
let delegate = Delegate()
app.delegate = delegate
app.run()
