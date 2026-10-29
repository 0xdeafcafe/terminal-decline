// Keypress-to-pixels latency of any app, measured from outside it, so
// decline and Ghostty are measured the same way. Built by run.py with
// `swiftc -O -swift-version 5`.
//
//   probe measure <pid> <n> <keycodes> [--window <id>] [--bottom <points>]
//                 [--out <file>]
//   probe keys <pid> <key>...           post keys, e.g. cmd+45 16 14 1 36
//   probe windows <pid>                  the app's windows: id x y width height
//   probe check                          print the permissions this needs
//
// `measure` captures the app's window (`--window`, else its largest) with ScreenCaptureKit (every
// display refresh, at one pixel per point; `--bottom` keeps only that many
// points at the bottom of the window), then n times: waits until nothing
// has changed for 60 ms, posts a key down and up to the app
// (CGEventPostToPid; keycodes are a comma-separated list used in turn),
// and waits for the first captured frame whose pixels differ. The latency
// is that frame's display time (SCStreamFrameInfo.displayTime, the host
// clock) minus the time just before posting. One JSON line per key goes to
// --out (or stdout).
//
// Posting events to another app needs Accessibility, capturing needs
// Screen Recording, both granted to the app running this (the terminal).

import AppKit
import CoreGraphics
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

setvbuf(stdout, nil, _IOLBF, 0)

func now() -> Double {
    var ts = timespec()
    clock_gettime(CLOCK_UPTIME_RAW, &ts)
    return Double(ts.tv_sec) + Double(ts.tv_nsec) * 1e-9
}

var timebase = mach_timebase_info_data_t()
mach_timebase_info(&timebase)
func machSeconds(_ t: UInt64) -> Double {
    Double(t) * Double(timebase.numer) / Double(timebase.denom) * 1e-9
}

func die(_ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(2)
}

/// Post a key down and up; `spec` is a virtual key code, optionally
/// prefixed by `cmd+`, `ctrl+`, `shift+` or `alt+`.
func post(_ spec: String, to pid: pid_t) {
    var parts = spec.split(separator: "+").map(String.init)
    guard let code = UInt16(parts.removeLast()) else { die("bad key \(spec)") }
    var flags = CGEventFlags()
    for m in parts {
        switch m {
        case "cmd": flags.insert(.maskCommand)
        case "ctrl": flags.insert(.maskControl)
        case "shift": flags.insert(.maskShift)
        case "alt": flags.insert(.maskAlternate)
        default: die("bad modifier \(m)")
        }
    }
    let src = CGEventSource(stateID: .privateState)
    for down in [true, false] {
        guard let e = CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: down) else { return }
        e.flags = flags
        e.postToPid(pid)
    }
}

func activate(_ pid: pid_t) {
    NSRunningApplication(processIdentifier: pid)?.activate()
}

/// Frames as they arrive: the display time of the last one whose pixels
/// changed, and when that was seen.
final class Watcher: NSObject, SCStreamOutput {
    let lock = NSLock()
    var previous: [UInt8] = []
    var lastChange: (display: Double, seen: Double)? = nil
    var frames = 0

    func stream(_ stream: SCStream, didOutputSampleBuffer sample: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen,
            let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
            let info = attachments.first,
            let raw = info[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
            let display = info[.displayTime] as? UInt64,
            let pixels = CMSampleBufferGetImageBuffer(sample)
        else { return }
        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        let count = CVPixelBufferGetBytesPerRow(pixels) * CVPixelBufferGetHeight(pixels)
        guard let base = CVPixelBufferGetBaseAddress(pixels) else { return }
        let bytes = UnsafeBufferPointer(start: base.assumingMemoryBound(to: UInt8.self), count: count)
        lock.lock()
        defer { lock.unlock() }
        frames += 1
        let changed = previous.count != count || !previous.withUnsafeBufferPointer { memcmp($0.baseAddress!, bytes.baseAddress!, count) == 0 }
        if changed {
            previous = Array(bytes)
            lastChange = (machSeconds(display), now())
        }
    }

    func last() -> (display: Double, seen: Double)? {
        lock.lock()
        defer { lock.unlock() }
        return lastChange
    }
}

func windows(of pid: pid_t) -> [SCWindow] {
    let sem = DispatchSemaphore(value: 0)
    var found: [SCWindow] = []
    var failure: Error? = nil
    SCShareableContent.getExcludingDesktopWindows(true, onScreenWindowsOnly: true) { content, error in
        failure = error
        found = (content?.windows ?? []).filter { $0.owningApplication?.processID == pid && $0.windowLayer == 0 }
        sem.signal()
    }
    sem.wait()
    if let e = failure { die("ScreenCaptureKit: \(e.localizedDescription) (Screen Recording permission?)") }
    return found
}

func window(of pid: pid_t, id: UInt32?) -> SCWindow {
    let all = windows(of: pid)
    let w = id.map { id in all.first { $0.windowID == id } }
        ?? all.max { $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height }
    guard let w else { die("no window on screen for pid \(pid)") }
    return w
}

func measure(pid: pid_t, n: Int, codes: [String], windowID: UInt32?, bottom: Double?, out: FileHandle) {
    let win = window(of: pid, id: windowID)
    let config = SCStreamConfiguration()
    config.width = Int(win.frame.width)
    config.height = Int(win.frame.height)
    if let b = bottom {
        config.sourceRect = CGRect(x: 0, y: win.frame.height - b, width: win.frame.width, height: b)
        config.height = Int(b)
    }
    config.minimumFrameInterval = CMTime(value: 1, timescale: 240)
    config.queueDepth = 5
    config.showsCursor = false
    config.pixelFormat = kCVPixelFormatType_32BGRA
    let watcher = Watcher()
    let stream = SCStream(filter: SCContentFilter(desktopIndependentWindow: win), configuration: config, delegate: nil)
    do {
        try stream.addStreamOutput(watcher, type: .screen, sampleHandlerQueue: DispatchQueue(label: "frames"))
    } catch { die("addStreamOutput: \(error)") }
    let started = DispatchSemaphore(value: 0)
    stream.startCapture { error in
        if let e = error { die("startCapture: \(e.localizedDescription) (Screen Recording permission?)") }
        started.signal()
    }
    started.wait()
    Thread.sleep(forTimeInterval: 0.5)

    var missed = 0
    for i in 0..<n {
        // Quiet first, so the change seen is this key's.
        let quietBy = now() + 2.0
        while let c = watcher.last(), now() - c.seen < 0.06, now() < quietBy {
            Thread.sleep(forTimeInterval: 0.005)
        }
        let before = watcher.last()?.seen ?? 0
        let t = now()
        post(codes[i % codes.count], to: pid)
        var shown: (display: Double, seen: Double)? = nil
        let deadline = t + 1.0
        while now() < deadline {
            if let c = watcher.last(), c.seen > before, c.seen > t {
                shown = c
                break
            }
            Thread.sleep(forTimeInterval: 0.0005)
        }
        var row: [String: Any] = ["seq": i + 1, "t_post": t]
        if let s = shown {
            row["t_shown"] = s.display
            row["t_seen"] = s.seen
            row["ms"] = (s.display - t) * 1000
        } else {
            missed += 1
        }
        let data = try! JSONSerialization.data(withJSONObject: row)
        out.write(data + "\n".data(using: .utf8)!)
        Thread.sleep(forTimeInterval: Double.random(in: 0.05...0.15))
    }
    let done = DispatchSemaphore(value: 0)
    stream.stopCapture { _ in done.signal() }
    done.wait()
    FileHandle.standardError.write("probe: \(n) keys, \(missed) without a change, \(watcher.frames) frames\n".data(using: .utf8)!)
}

let args = Array(CommandLine.arguments.dropFirst())
guard let mode = args.first else { die("usage: probe measure|keys|check …") }
switch mode {
case "check":
    print("screen_recording=\(CGPreflightScreenCaptureAccess()) post_events=\(CGPreflightPostEventAccess())")
case "windows":
    guard args.count == 2, let pid = pid_t(args[1]) else { die("usage: probe windows <pid>") }
    for w in windows(of: pid) { print(w.windowID, Int(w.frame.minX), Int(w.frame.minY), Int(w.frame.width), Int(w.frame.height)) }
case "keys":
    guard args.count >= 3, let pid = pid_t(args[1]) else { die("usage: probe keys <pid> <key>...") }
    activate(pid)
    for k in args[2...] {
        post(k, to: pid)
        Thread.sleep(forTimeInterval: 0.05)
    }
case "measure":
    guard args.count >= 4, let pid = pid_t(args[1]), let n = Int(args[2]) else {
        die("usage: probe measure <pid> <n> <keycodes> [--bottom <points>] [--out <file>]")
    }
    let codes = args[3].split(separator: ",").map(String.init)
    var bottom: Double? = nil
    var windowID: UInt32? = nil
    var out = FileHandle.standardOutput
    var i = 4
    while i < args.count {
        switch args[i] {
        case "--bottom": bottom = Double(args[i + 1]); i += 2
        case "--window": windowID = UInt32(args[i + 1]); i += 2
        case "--out":
            FileManager.default.createFile(atPath: args[i + 1], contents: nil)
            out = FileHandle(forWritingAtPath: args[i + 1])!
            i += 2
        default: die("unknown argument \(args[i])")
        }
    }
    if !CGPreflightScreenCaptureAccess() { die("probe: no Screen Recording permission") }
    if !CGPreflightPostEventAccess() { die("probe: no permission to post events (Accessibility)") }
    activate(pid)
    measure(pid: pid, n: n, codes: codes, windowID: windowID, bottom: bottom, out: out)
default:
    die("unknown mode \(mode)")
}
