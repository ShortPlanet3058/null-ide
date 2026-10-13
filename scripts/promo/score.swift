// The film's sound: the music cut on its bars to fit what happens on screen, the small
// sounds the film asks for (keys, pops, a whoosh, ticks: made here, from noise and sine
// waves), mixed, then put with the pictures in one MP4 (the pictures as they are).
//
//   score MUSIC VIDEO.mp4 OUT.mp4 [SOUNDS.json] [--pieces FROM-TO@AT,...]
//
// --pieces gives another cut of the music (the 30-second film's: see render.sh).
//
// The music (Null's track, 75 BPM: a bar is 3.2 s; its bars fall on 1.66 + 3.2k s) and
// where each piece of it goes in the film:
//   0 – 59.26 s      intro, the three entries (the noise), the build and its stop, the
//                    silence, the first big beat (33.66 s: the editor), eight bars of groove
//   84.86 – 123.26   from 59.26 s: a swell, the quiet breakdown (the turn: AI off), the
//                    full return on a bar (65.66 s: ⌘I), ten bars of it
//   136.06 – end     from 97.66 s: the last phrase and its fade, under the end card
import AVFoundation

var pieces: [(from: Double, to: Double, at: Double)] = [
    (0, 59.26, 0),
    (84.86, 123.26, 59.26),
    (136.06, 145.4, 97.66),
]
let crossfade = 0.03
let fadeOut = 1.0

var args = CommandLine.arguments
if let at = args.firstIndex(of: "--pieces"), at + 1 < args.count {
    pieces = args[at + 1].split(separator: ",").map { piece in
        let parts = piece.split(separator: "@"), range = parts[0].split(separator: "-")
        return (Double(range[0])!, Double(range[1])!, Double(parts[1])!)
    }
    args.removeSubrange(at...(at + 1))
}
guard args.count == 4 || args.count == 5 else {
    FileHandle.standardError.write("usage: score MUSIC VIDEO.mp4 OUT.mp4 [SOUNDS.json] [--pieces FROM-TO@AT,...]\n".data(using: .utf8)!)
    exit(2)
}
let music = URL(fileURLWithPath: args[1]), video = URL(fileURLWithPath: args[2]), out = URL(fileURLWithPath: args[3])
let cues: [(Double, String, Double)] = {
    guard args.count == 5, let data = try? Data(contentsOf: URL(fileURLWithPath: args[4])),
          let list = try? JSONSerialization.jsonObject(with: data) as? [[Any]] else { return [] }
    return list.compactMap { c in
        guard c.count == 3, let t = c[0] as? Double, let kind = c[1] as? String, let gain = c[2] as? Double else { return nil }
        return (t, kind, gain)
    }
}()

/// Repeatable noise.
struct Noise {
    var state: UInt64
    mutating func next() -> Float {
        state = state &* 6364136223846793005 &+ 1442695040888963407
        return Float(Int64(bitPattern: state >> 11) % 2_000_000) / 1_000_000 - 1
    }
}

/// A one-pole filter: `low` keeps what's under the cutoff, the rest is what's above.
struct OnePole {
    var y: Float = 0
    mutating func low(_ x: Float, _ cutoff: Float, _ sr: Float) -> Float {
        let a = 1 - expf(-2 * .pi * cutoff / sr)
        y += a * (x - y)
        return y
    }
}

/// One small sound, mono, as samples.
func make(_ kind: String, sr: Float, noise: inout Noise) -> [Float] {
    func samples(_ seconds: Float) -> Int { Int(seconds * sr) }
    var out: [Float] = []
    switch kind {
    case "key", "enter":
        // A soft key: a short burst of filtered noise and a small low knock.
        let enter = kind == "enter"
        var lp = OnePole(), hp = OnePole()
        let n = samples(enter ? 0.06 : 0.04)
        for i in 0..<n {
            let t = Float(i) / sr
            let x = noise.next()
            let band = lp.low(x, enter ? 3200 : 5200, sr) - hp.low(x, 900, sr)
            let env = expf(-t / (enter ? 0.009 : 0.006))
            let knock = sinf(2 * .pi * (enter ? 180 : 260) * t) * expf(-t / 0.008)
            out.append((band * 0.9 + knock * 0.35) * env * (enter ? 0.14 : 0.1))
        }
    case "press":
        // A shortcut: a rounder, deeper key.
        var lp = OnePole()
        for i in 0..<samples(0.09) {
            let t = Float(i) / sr
            let body = sinf(2 * .pi * 150 * t) * expf(-t / 0.022)
            let click = lp.low(noise.next(), 2600, sr) * expf(-t / 0.005)
            out.append((body * 0.7 + click * 0.8) * 0.15)
        }
    case "pop":
        // A notification landing: a small falling blip.
        var phase: Float = 0
        for i in 0..<samples(0.12) {
            let t = Float(i) / sr
            let f = 820 - 360 * min(1, t / 0.07)
            phase += 2 * .pi * f / sr
            out.append(sinf(phase) * expf(-t / 0.035) * min(1, t / 0.003) * 0.1)
        }
    case "thock":
        // A panel arriving: a low, soft thump.
        var lp = OnePole()
        for i in 0..<samples(0.14) {
            let t = Float(i) / sr
            let x = lp.low(noise.next(), 420, sr) * 2.2 + sinf(2 * .pi * 90 * t) * 0.5
            out.append(x * expf(-t / 0.04) * 0.11)
        }
    case "swish":
        // A montage cut: air, opening and closing.
        var lp = OnePole()
        let n = samples(0.28)
        for i in 0..<n {
            let p = Float(i) / Float(n)
            let cutoff = 1500 + 4500 * sinf(.pi * p)
            out.append(lp.low(noise.next(), cutoff, sr) * sinf(.pi * p) * 0.08)
        }
    case "whoosh":
        // Everything pulled into the caret: rising air that stops dead.
        var lp = OnePole()
        let n = samples(0.47)
        for i in 0..<n {
            let p = Float(i) / Float(n)
            let cutoff = 250 + 6000 * p * p
            let env = p * p * (p > 0.96 ? (1 - p) / 0.04 : 1)
            out.append(lp.low(noise.next(), cutoff, sr) * env * 0.32)
        }
    case "tick":
        // Kept, switched on, committed: a clear, short tick.
        for i in 0..<samples(0.08) {
            let t = Float(i) / sr
            let x = sinf(2 * .pi * 2300 * t) + 0.4 * sinf(2 * .pi * 3450 * t)
            out.append(x * expf(-t / 0.014) * min(1, t / 0.001) * 0.09)
        }
    default:
        break
    }
    return out
}

func run() async throws {
    let file = try AVAudioFile(forReading: music)
    let format = file.processingFormat
    let sr = format.sampleRate
    let source = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(file.length))!
    try file.read(into: source)
    let channels = Int(format.channelCount)

    let film = AVURLAsset(url: video)
    let length = try await film.load(.duration).seconds
    let total = Int(length * sr)
    let sound = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(total))!
    sound.frameLength = AVAudioFrameCount(total)

    // The music, piece by piece; neighbours overlap a little, faded there.
    for c in 0..<channels {
        let src = source.floatChannelData![c], dst = sound.floatChannelData![c]
        for i in 0..<total { dst[i] = 0 }
        for piece in pieces {
            let start = max(0, piece.from - crossfade), end = piece.to + crossfade
            let n = Int((end - start) * sr), fade = Int(crossfade * 2 * sr)
            let from = Int(start * sr), to = Int((piece.at - (piece.from - start)) * sr)
            for k in 0..<n {
                let s = from + k, d = to + k
                if s < 0 || s >= Int(source.frameLength) || d < 0 || d >= total { continue }
                var gain: Float = 0.72
                if piece.from > 0 && k < fade { gain *= Float(k) / Float(fade) }
                if k > n - fade { gain *= Float(n - k) / Float(fade) }
                dst[d] += src[s] * gain
            }
        }
    }

    // The small sounds, each a little to the left or right.
    var noise = Noise(state: 0x9E3779B97F4A7C15)
    for (t, kind, gain) in cues {
        let samples = make(kind, sr: Float(sr), noise: &noise)
        let pan = (noise.next() * 0.25)
        let at = Int(t * sr)
        for c in 0..<channels {
            let dst = sound.floatChannelData![c]
            let side: Float = channels == 2 ? (c == 0 ? 1 - pan : 1 + pan) : 1
            for (k, x) in samples.enumerated() where at + k >= 0 && at + k < total {
                dst[at + k] += x * Float(gain) * side
            }
        }
    }

    // The end fades with the picture; nothing clips.
    let fadeFrames = Int(fadeOut * sr)
    for c in 0..<channels {
        let dst = sound.floatChannelData![c]
        for k in 0..<fadeFrames where total - fadeFrames + k >= 0 {
            dst[total - fadeFrames + k] *= Float(fadeFrames - k) / Float(fadeFrames)
        }
        for i in 0..<total where abs(dst[i]) > 0.7 {
            // (Under 0.85: AAC overshoots a little.)
            dst[i] = (dst[i] > 0 ? 1 : -1) * (0.7 + 0.15 * tanhf((abs(dst[i]) - 0.7) / 0.15))
        }
    }

    let m4a = FileManager.default.temporaryDirectory.appendingPathComponent("null-film-sound-\(getpid()).m4a")
    try? FileManager.default.removeItem(at: m4a)
    do {
        let writer = try AVAudioFile(forWriting: m4a, settings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: sr,
            AVNumberOfChannelsKey: channels,
            AVEncoderBitRateKey: 256_000,
        ], commonFormat: .pcmFormatFloat32, interleaved: false)
        try writer.write(from: sound)
    }
    let composition = AVMutableComposition()
    let range = CMTimeRange(start: .zero, duration: try await film.load(.duration))
    let pictures = try await film.loadTracks(withMediaType: .video)[0]
    let videoTrack = composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!
    try videoTrack.insertTimeRange(range, of: pictures, at: .zero)
    let soundAsset = AVURLAsset(url: m4a)
    let soundTrack = try await soundAsset.loadTracks(withMediaType: .audio)[0]
    let soundRange = CMTimeRange(start: .zero, duration: min(range.duration, try await soundAsset.load(.duration)))
    let audioTrack = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
    try audioTrack.insertTimeRange(soundRange, of: soundTrack, at: .zero)

    try? FileManager.default.removeItem(at: out)
    let export = AVAssetExportSession(asset: composition, presetName: AVAssetExportPresetPassthrough)!
    try await export.export(to: out, as: .mp4)
    try? FileManager.default.removeItem(at: m4a)
}

let done = DispatchSemaphore(value: 0)
var failure: Error?
Task {
    do { try await run() } catch { failure = error }
    done.signal()
}
done.wait()
if let failure {
    FileHandle.standardError.write("score: \(failure)\n".data(using: .utf8)!)
    exit(1)
}
