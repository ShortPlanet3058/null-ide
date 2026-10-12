// Renders scripts/promo/promo.html to an MP4, frame by frame: the page's `render(t)` draws
// the moment `t` (seconds), WebKit takes a picture of it, AVFoundation encodes it. Nothing
// to install: it's the Mac's own WebKit and video encoder.
//
//   render PAGE.html OUT.mp4 [--fps 30] [--size 1920x1080] [--also WxH OTHER.mp4] [--poster SECONDS OUT.jpg]
// The page is laid out at 1920×1080 whatever the size: WebKit draws it at the screen's scale
// (twice on a Retina), and the film is that picture at --size (3840x2160 for 4K); --also
// writes a second film at another size from the same pictures.
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
    var also: (Int, Int, URL)?
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
        case "--also":
            let parts = args.removeFirst().split(separator: "x").map { Int($0)! }
            o.also = (parts[0], parts[1], URL(fileURLWithPath: args.removeFirst()))
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
        let size = NSRect(x: -20000, y: -20000, width: 1920, height: 1080)
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
        _ = try await web.callAsyncJavaScript("await document.fonts.ready; if (window.ready) await window.ready; return 1", contentWorld: .page)
    }

    func frame(at t: Double) async throws -> CGImage {
        _ = try await web.callAsyncJavaScript("window.render(t); return 1", arguments: ["t": t], contentWorld: .page)
        let config = WKSnapshotConfiguration()
        config.rect = NSRect(x: 0, y: 0, width: 1920, height: 1080)
        config.snapshotWidth = NSNumber(value: max(o.width, 1920))
        config.afterScreenUpdates = true
        let image = try await web.takeSnapshot(configuration: config)
        var rect = NSRect(x: 0, y: 0, width: image.size.width, height: image.size.height)
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
        var films = [try Film(url: o.out, width: o.width, height: o.height, fps: o.fps)]
        if let (w, h, url) = o.also { films.append(try Film(url: url, width: w, height: h, fps: o.fps)) }
        let frames = Int((duration * Double(o.fps)).rounded())
        let started = Date()
        for i in 0..<frames {
            let t = Double(i) / Double(o.fps)
            let image = try await frame(at: t)
            if let (at, url) = o.poster, i == Int((at * Double(o.fps)).rounded()) {
                save(image, url, .jpeg)
            }
            for film in films { try await film.append(image, frame: i) }
            if i % (o.fps * 5) == 0 {
                print(String(format: "  %3.0f s of %.0f (%.0f s so far)", t, duration, Date().timeIntervalSince(started)))
            }
        }
        for film in films { try await film.finish() }
        // The sounds the film asks for, beside it (score.swift makes them).
        if let sounds = try await web.callAsyncJavaScript("return JSON.stringify(window.SOUNDS || [])", contentWorld: .page) as? String {
            try sounds.write(to: o.out.deletingPathExtension().appendingPathExtension("sounds.json"), atomically: true, encoding: .utf8)
        }
    }
}

/// One film being written: H.264, at its size (the pictures scaled to it).
final class Film {
    let writer: AVAssetWriter
    let input: AVAssetWriterInput
    let adaptor: AVAssetWriterInputPixelBufferAdaptor
    let width: Int, height: Int, fps: Int

    init(url: URL, width: Int, height: Int, fps: Int) throws {
        self.width = width; self.height = height; self.fps = fps
        try? FileManager.default.removeItem(at: url)
        writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
        // Mostly still, dark pictures: the encoder stays well under this.
        let bitrate = Int(Double(width * height * fps) * 0.08)
        input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: bitrate,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
                AVVideoMaxKeyFrameIntervalKey: fps * 2,
            ],
        ])
        input.expectsMediaDataInRealTime = false
        adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
        ])
        writer.add(input)
        writer.startWriting()
        writer.startSession(atSourceTime: .zero)
    }

    func append(_ image: CGImage, frame i: Int) async throws {
        while !input.isReadyForMoreMediaData {
            try await Task.sleep(nanoseconds: 2_000_000)
        }
        var buffer: CVPixelBuffer?
        CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &buffer)
        let pixels = buffer!
        CVPixelBufferLockBaseAddress(pixels, [])
        let context = CGContext(
            data: CVPixelBufferGetBaseAddress(pixels), width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(pixels), space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        CVPixelBufferUnlockBaseAddress(pixels, [])
        adaptor.append(pixels, withPresentationTime: CMTime(value: CMTimeValue(i), timescale: CMTimeScale(fps)))
    }

    func finish() async throws {
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
