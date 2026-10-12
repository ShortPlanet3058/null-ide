// Puts the music under the film: the track is cut on its bars to fit what happens on
// screen, the pieces joined with short crossfades (no clicks), the end faded, then the
// sound and the film's pictures go into one MP4 (the pictures as they are).
//
//   score MUSIC VIDEO.mp4 OUT.mp4
//
// The music (Null's track, 75 BPM: a bar is 3.2 s) and where each piece of it goes:
//   0 – 4.86 s      its quiet start, under the caret alone
//   24.06 – 62.46   from 4.86 s: the build under the clutter, its sudden stop as the
//                   clutter goes into the caret, the silence, the first big beat as the
//                   editor comes in (14.46 s), the groove under the rest
//   136.06 – end    from 43.26 s: its last phrase and fade, under the end card
import AVFoundation

let pieces: [(from: Double, to: Double, at: Double)] = [
    (0, 4.86, 0),
    (24.06, 62.46, 4.86),
    (136.06, 145.4, 43.26),
]
let crossfade = 0.03
let fadeOut = 0.9

let args = CommandLine.arguments
guard args.count == 4 else {
    FileHandle.standardError.write("usage: score MUSIC VIDEO.mp4 OUT.mp4\n".data(using: .utf8)!)
    exit(2)
}
let music = URL(fileURLWithPath: args[1]), video = URL(fileURLWithPath: args[2]), out = URL(fileURLWithPath: args[3])

func run() async throws {
    // The whole track, as samples.
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
    for c in 0..<channels {
        let src = source.floatChannelData![c], dst = sound.floatChannelData![c]
        for i in 0..<total { dst[i] = 0 }
        for piece in pieces {
            // Each piece a little longer at both ends, faded there: neighbours overlap.
            let start = max(0, piece.from - crossfade), end = piece.to + crossfade
            let n = Int((end - start) * sr), fade = Int(crossfade * 2 * sr)
            let from = Int(start * sr), to = Int((piece.at - (piece.from - start)) * sr)
            for k in 0..<n {
                let s = from + k, d = to + k
                if s < 0 || s >= Int(source.frameLength) || d < 0 || d >= total { continue }
                var gain: Float = 1
                if piece.from > 0 && k < fade { gain = Float(k) / Float(fade) }
                if k > n - fade { gain = min(gain, Float(n - k) / Float(fade)) }
                dst[d] += src[s] * gain
            }
        }
        // The last moments fade out with the picture.
        let fadeFrames = Int(fadeOut * sr)
        for k in 0..<fadeFrames {
            let i = total - fadeFrames + k
            if i >= 0 { dst[i] *= Float(fadeFrames - k) / Float(fadeFrames) }
        }
    }

    // The sound as AAC, then with the pictures.
    let m4a = FileManager.default.temporaryDirectory.appendingPathComponent("null-film-sound.m4a")
    try? FileManager.default.removeItem(at: m4a)
    do {
        let writer = try AVAudioFile(forWriting: m4a, settings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: sr,
            AVNumberOfChannelsKey: channels,
            AVEncoderBitRateKey: 192_000,
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
