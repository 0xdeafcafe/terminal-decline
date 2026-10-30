// Small macOS helpers for run.py, which builds this with
// `swiftc -O -swift-version 5`.
//
//   helper windows <pid>...      one line per normal (layer 0) window of
//                                those processes: pid id x y width height
//                                onscreen (points)
//   helper usage <pid>...         per process: pid cpu_ns idle_wakeups
//                                interrupt_wakeups (proc_pid_rusage; the
//                                user's own processes only)
//   helper font <name> <size>    the NSFont as Terminal.app keeps it in a
//                                profile (an NSKeyedArchiver archive), in hex
//
// `windows` needs no Screen Recording: CGWindowList still gives bounds,
// layer and whether a window is on screen, only not titles. Native macOS
// tabs are windows of their own (only the selected one on screen), so this
// also counts tabs for terminals that use them.

import AppKit
import CoreGraphics
import Foundation

let args = Array(CommandLine.arguments.dropFirst())

func windows(_ pids: Set<Int32>) {
    let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
    for w in list {
        guard let pid = w[kCGWindowOwnerPID as String] as? Int32, pids.contains(pid),
            let layer = w[kCGWindowLayer as String] as? Int, layer == 0,
            let id = w[kCGWindowNumber as String] as? Int,
            let b = w[kCGWindowBounds as String] as? [String: Double]
        else { continue }
        // Tiny windows are helpers (menus, drag images, tab bar
        // accessories), not terminal windows.
        if (b["Width"] ?? 0) < 200 || (b["Height"] ?? 0) < 150 { continue }
        let on = (w[kCGWindowIsOnscreen as String] as? Bool) ?? false
        print(pid, id, Int(b["X"] ?? 0), Int(b["Y"] ?? 0), Int(b["Width"] ?? 0), Int(b["Height"] ?? 0), on ? 1 : 0)
    }
}

func usage(_ pids: [Int32]) {
    var tb = mach_timebase_info_data_t()
    mach_timebase_info(&tb)
    for pid in pids {
        var info = rusage_info_v4()
        let ok = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { proc_pid_rusage(pid, RUSAGE_INFO_V4, $0) }
        }
        guard ok == 0 else { continue }
        // CPU times are in mach time units.
        let ns = (info.ri_user_time + info.ri_system_time) * UInt64(tb.numer) / UInt64(tb.denom)
        print(pid, ns, info.ri_pkg_idle_wkups, info.ri_interrupt_wkups)
    }
}

func font(_ name: String, _ size: Double) {
    guard let f = NSFont(name: name, size: size) else {
        FileHandle.standardError.write("no font \(name)\n".data(using: .utf8)!)
        exit(2)
    }
    let data = try! NSKeyedArchiver.archivedData(withRootObject: f, requiringSecureCoding: false)
    print(data.map { String(format: "%02x", $0) }.joined())
}

switch args.first {
case "windows": windows(Set(args.dropFirst().compactMap { Int32($0) }))
case "usage": usage(args.dropFirst().compactMap { Int32($0) })
case "font" where args.count == 3: font(args[1], Double(args[2]) ?? 13)
default:
    FileHandle.standardError.write("usage: helper windows <pid>... | helper usage <pid>... | helper font <name> <size>\n".data(using: .utf8)!)
    exit(2)
}
