// Renders scripts/promo/promo.html to an MP4, frame by frame: the page's `render(t)` draws
// the moment `t` (seconds), WebKit takes a picture of it, AVFoundation encodes it. Nothing
// to install: it's the Mac's own WebKit and video encoder.
//
//   render PAGE.html OUT.mp4 [--fps 30] [--size 1920x1080] [--poster SECONDS OUT.jpg]
//   render PAGE.html DIR --stills 1.5,12,30   (pictures of those moments, to look at)
//
// The page says how long it is (`window.DURATION`). Its window stays off-screen and the
// program never comes to the front.
import AppKit
import AVFoundation
import WebKit

struct Options {
    var page: URL
    var out: URL
    var fps = 30
    var width = 1920
    var height = 1080
    var poster: (Double, URL)?
    var stills: [Double]?
}

func options() -> Options {
    var args = Array(CommandLine.arguments.dropFirst())
    guard args.count >= 2 else {
        FileHandle.standardError.write("usage: render PAGE.html OUT.mp4 [--fps N] [--size WxH] [--poster S OUT.jpg] | PAGE.html DIR --stills S,S,...\n".data(using: .utf8)!)
        exit(2)
    }
    var o = Options(page: URL(fileURLWithPath: args.removeFirst()), out: URL(fileURLWithPath: args.removeFirst()))
    while !args.isEmpty {
        let flag = args.removeFirst()
        switch flag {
        case "--fps": o.fps = Int(args.removeFirst())!
        case "--size":
            let parts = args.removeFirst().split(separator: "x").map { Int($0)! }
            o.width = parts[0]; o.height = parts[1]
        case "--poster": o.poster = (Double(args.removeFirst())!, URL(fileURLWithPath: args.removeFirst()))
        case "--stills": o.stills = args.removeFirst().split(separator: ",").map { Double($0)! }
        default: fatalError("unknown option \(flag)")
        }
    }
    return o
}

@MainActor
final class Renderer: NSObject, WKNavigationDelegate {
    let o: Options
    let web: WKWebView
    let window: NSWindow
    var loaded: CheckedContinuation<Void, Never>?

    init(_ o: Options) {
        self.o = o
        let size = NSRect(x: -20000, y: -20000, width: o.width, height: o.height)
        web = WKWebView(frame: NSRect(origin: .zero, size: size.size), configuration: WKWebViewConfiguration())
        window = NSWindow(contentRect: size, styleMask: .borderless, backing: .buffered, defer: false)
        super.init()
        window.contentView = web
        window.orderFrontRegardless()
        web.navigationDelegate = self
        // (The page leaves out its player's controls for this.)
        web.customUserAgent = "NullFilm"
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loaded?.resume()
        loaded = nil
    }

    func load() async throws {
        await withCheckedContinuation { c in
            loaded = c
            web.loadFileURL(o.page, allowingReadAccessTo: o.page.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent())
        }
        _ = try await web.callAsyncJavaScript("await document.fonts.ready; return 1", contentWorld: .page)
    }

    func frame(at t: Double) async throws -> CGImage {
        _ = try await web.callAsyncJavaScript("window.render(t); return 1", arguments: ["t": t], contentWorld: .page)
        let config = WKSnapshotConfiguration()
        config.rect = NSRect(x: 0, y: 0, width: o.width, height: o.height)
        config.snapshotWidth = NSNumber(value: o.width)
        config.afterScreenUpdates = true
        let image = try await web.takeSnapshot(configuration: config)
        var rect = NSRect(x: 0, y: 0, width: o.width, height: o.height)
        return image.cgImage(forProposedRect: &rect, context: nil, hints: nil)!
    }

    /// At the film's size (WebKit draws at the screen's scale, twice that on a Retina).
    func save(_ image: CGImage, _ url: URL, _ type: NSBitmapImageRep.FileType) {
        let context = CGContext(
            data: nil, width: o.width, height: o.height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: o.width, height: o.height))
        let rep = NSBitmapImageRep(cgImage: context.makeImage()!)
        let props: [NSBitmapImageRep.PropertyKey: Any] = type == .jpeg ? [.compressionFactor: 0.88] : [:]
        try! rep.representation(using: type, properties: props)!.write(to: url)
    }

    func run() async throws {
        try await load()
        if let stills = o.stills {
            try FileManager.default.createDirectory(at: o.out, withIntermediateDirectories: true)
            for t in stills {
                save(try await frame(at: t), o.out.appendingPathComponent(String(format: "%05.2f.png", t)), .png)
            }
            return
        }
        let duration = try await web.callAsyncJavaScript("return window.DURATION", contentWorld: .page) as! Double
        try? FileManager.default.removeItem(at: o.out)
        let writer = try AVAssetWriter(outputURL: o.out, fileType: .mp4)
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: o.width,
            AVVideoHeightKey: o.height,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: 3_500_000,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
                AVVideoMaxKeyFrameIntervalKey: o.fps * 2,
            ],
        ])
        input.expectsMediaDataInRealTime = false
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: o.width,
            kCVPixelBufferHeightKey as String: o.height,
        ])
        writer.add(input)
        writer.startWriting()
        writer.startSession(atSourceTime: .zero)
        let frames = Int((duration * Double(o.fps)).rounded())
        let started = Date()
        for i in 0..<frames {
            let t = Double(i) / Double(o.fps)
            let image = try await frame(at: t)
            if let (at, url) = o.poster, i == Int((at * Double(o.fps)).rounded()) {
                save(image, url, .jpeg)
            }
            while !input.isReadyForMoreMediaData {
                try await Task.sleep(nanoseconds: 2_000_000)
            }
            var buffer: CVPixelBuffer?
            CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &buffer)
            let pixels = buffer!
            CVPixelBufferLockBaseAddress(pixels, [])
            let context = CGContext(
                data: CVPixelBufferGetBaseAddress(pixels), width: o.width, height: o.height, bitsPerComponent: 8,
                bytesPerRow: CVPixelBufferGetBytesPerRow(pixels), space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
            context.interpolationQuality = .high
            context.draw(image, in: CGRect(x: 0, y: 0, width: o.width, height: o.height))
            CVPixelBufferUnlockBaseAddress(pixels, [])
            adaptor.append(pixels, withPresentationTime: CMTime(value: CMTimeValue(i), timescale: CMTimeScale(o.fps)))
            if i % o.fps == 0 {
                let elapsed = Date().timeIntervalSince(started)
                print(String(format: "  %3.0f s of %.0f (%.0f s so far)", t, duration, elapsed))
            }
        }
        input.markAsFinished()
        await writer.finishWriting()
        if writer.status != .completed {
            throw writer.error ?? NSError(domain: "render", code: 1)
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let o = options()
Task { @MainActor in
    let renderer = Renderer(o)
    do {
        try await renderer.run()
        exit(0)
    } catch {
        FileHandle.standardError.write("render: \(error)\n".data(using: .utf8)!)
        exit(1)
    }
}
app.run()
