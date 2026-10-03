// Draws Null's app icon: a dark tile holding nothing but an amber caret, the editor
// waiting for you. Writes an .iconset folder for `iconutil`.
//
//     swift make-icon.swift <out.iconset>

import AppKit

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "Null.iconset"
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)

func draw(size: Int) -> Data {
    let s = CGFloat(size)
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let cg = NSGraphicsContext.current!.cgContext
    cg.scaleBy(x: s / 1024, y: s / 1024)

    // The tile, on Apple's icon grid: 824 points with rounded corners, a soft shadow below.
    let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
    let shape = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
    cg.saveGState()
    cg.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: NSColor(white: 0, alpha: 0.45).cgColor)
    cg.addPath(shape)
    cg.setFillColor(NSColor(red: 0.04, green: 0.04, blue: 0.05, alpha: 1).cgColor)
    cg.fillPath()
    cg.restoreGState()

    // Near-black, a touch lighter at the top.
    cg.saveGState()
    cg.addPath(shape)
    cg.clip()
    let gradient = CGGradient(
        colorsSpace: CGColorSpaceCreateDeviceRGB(),
        colors: [
            NSColor(red: 0.105, green: 0.105, blue: 0.125, alpha: 1).cgColor,
            NSColor(red: 0.035, green: 0.035, blue: 0.045, alpha: 1).cgColor,
        ] as CFArray,
        locations: [0, 1])!
    cg.drawLinearGradient(gradient, start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])
    // A hairline catching the light along the top edge.
    cg.addPath(CGPath(roundedRect: tile.insetBy(dx: 1.5, dy: 1.5), cornerWidth: 184, cornerHeight: 184, transform: nil))
    cg.setStrokeColor(NSColor(white: 1, alpha: 0.07).cgColor)
    cg.setLineWidth(3)
    cg.strokePath()
    cg.restoreGState()

    // The caret: Null's amber, with a quiet glow.
    let amber = NSColor(red: 0xf2 / 255.0, green: 0xb3 / 255.0, blue: 0x5b / 255.0, alpha: 1)
    let caret = CGRect(x: 512 - 22, y: 512 - 170, width: 44, height: 340)
    let bar = CGPath(roundedRect: caret, cornerWidth: 22, cornerHeight: 22, transform: nil)
    cg.saveGState()
    cg.setShadow(offset: .zero, blur: 90, color: amber.withAlphaComponent(0.55).cgColor)
    cg.addPath(bar)
    cg.setFillColor(amber.cgColor)
    cg.fillPath()
    cg.restoreGState()
    cg.addPath(bar)
    cg.setFillColor(amber.cgColor)
    cg.fillPath()

    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for (points, scales) in [(16, [1, 2]), (32, [1, 2]), (128, [1, 2]), (256, [1, 2]), (512, [1, 2])] {
    for scale in scales {
        let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
        try! draw(size: points * scale).write(to: URL(fileURLWithPath: "\(out)/\(name)"))
    }
}
