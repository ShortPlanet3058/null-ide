// The number of the window process <pid> shows (its main one), for `screencapture -l`.
import CoreGraphics
import Foundation

guard CommandLine.arguments.count > 1, let pid = Int(CommandLine.arguments[1]) else {
    FileHandle.standardError.write("usage: winid <pid>\n".data(using: .utf8)!)
    exit(2)
}
let windows = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
let mine = windows.filter {
    ($0[kCGWindowOwnerPID as String] as? Int) == pid && ($0[kCGWindowLayer as String] as? Int) == 0
}
// The biggest: the project window, not a tooltip or a menu.
let biggest = mine.max { a, b in
    func area(_ w: [String: Any]) -> Double {
        let bounds = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        return (bounds["Width"] ?? 0) * (bounds["Height"] ?? 0)
    }
    return area(a) < area(b)
}
guard let window = biggest, let number = window[kCGWindowNumber as String] as? Int else { exit(1) }
print(number)
