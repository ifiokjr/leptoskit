@preconcurrency import AVFoundation
import Foundation
import LeptosKitBridge

@MainActor
public final class SpeechPlugin: NSObject, NativePlugin, AVSpeechSynthesizerDelegate {
    public let name = "speech"
    public let methods: [UInt32] = [0, 1, 2, 3]
    private let synthesizer = AVSpeechSynthesizer()
    private var active: ActiveSpeech?
    private var closed = false

    /// The wire methods the Rust crate calls.
    private enum Method: UInt32 {
        case voices = 0
        case speak = 1
        case stop = 2
        case status = 3
    }

    private struct ActiveSpeech {
        let token: UUID
        let utterance: AVSpeechUtterance
        let completion: CheckedContinuation<Data, any Error>
    }

    override public init() {
        super.init()
        synthesizer.delegate = self
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        try checkOpen(method)

        if method == Method.speak.rawValue { return try await speak(SpeechRequest(payload: payload)) }

        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return try answer(method)
    }

    /// Rejects a closed plugin or a method this host does not answer.
    private func checkOpen(_ method: UInt32) throws {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }
    }

    /// Answers a method that carries no payload.
    private func answer(_ method: UInt32) throws -> Data {
        switch Method(rawValue: method) {
        case .voices: return try voices()
        case .stop:
            cancel()

            return Data()
        default: return Data([active == nil ? 0 : 1, 0, 0, 0])
        }
    }

    public func close() {
        closed = true
        cancel()
        synthesizer.delegate = nil
    }

    /// Reports the installed voices, with the quality and source flags the Rust
    /// crate decodes.
    private func voices() throws -> Data {
        let voices = AVSpeechSynthesisVoice.speechVoices()

        guard voices.count <= 1024 else { throw PluginError.tooLarge }
        var output = SpeechEncoder()
        output.integer(1)
        output.integer(UInt32(voices.count))

        for voice in voices {
            try output.text(voice.identifier)
            try output.text(voice.name)

            try output.text(voice.language)
            switch voice.quality {
            case .default: output.integer(1)
            case .enhanced, .premium: output.integer(2)
            @unknown default: output.integer(0)
            }
            output.integer(0)
        }

        return output.data
    }

    private func speak(_ request: SpeechRequest) async throws -> Data {
        guard active == nil else { throw PluginError.busy }
        let utterance = AVSpeechUtterance(string: request.text)

        try selectVoice(utterance, request)
        applyOptions(utterance, request)
        let token = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { completion in
                begin(token, utterance, completion)
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.cancelIfCurrent(token) }
        }
    }

    /// Selects the voice the request named, or the language it asked for.
    private func selectVoice(_ utterance: AVSpeechUtterance, _ request: SpeechRequest) throws {
        if let identifier = request.voice {
            guard let voice = AVSpeechSynthesisVoice(identifier: identifier) else { throw PluginError.unavailable }
            utterance.voice = voice

            return
        }

        guard let language = request.language else { return }

        guard let voice = AVSpeechSynthesisVoice(language: language) else { throw PluginError.unavailable }
        utterance.voice = voice
    }

    /// Applies the rate, pitch, and volume the request asked for.
    private func applyOptions(_ utterance: AVSpeechUtterance, _ request: SpeechRequest) {
        utterance.rate = min(AVSpeechUtteranceMaximumSpeechRate,
                             max(AVSpeechUtteranceMinimumSpeechRate,
                                 AVSpeechUtteranceDefaultSpeechRate * request.rate))
        utterance.pitchMultiplier = request.pitch
        utterance.volume = request.volume
    }

    /// Registers the waiter, or refuses because the speech can no longer start.
    private func begin(
        _ token: UUID,
        _ utterance: AVSpeechUtterance,
        _ completion: CheckedContinuation<Data, any Error>
    ) {
        guard !Task.isCancelled, !closed else {
            completion.resume(throwing: PluginError.cancelled)

            return
        }

        guard active == nil else {
            completion.resume(throwing: PluginError.busy)

            return
        }

        active = ActiveSpeech(token: token, utterance: utterance, completion: completion)
        synthesizer.speak(utterance)
    }

    /// Cancels the speech the caller abandoned, if it is still the one that runs.
    private func cancelIfCurrent(_ token: UUID) {
        guard active?.token == token else { return }

        cancel()
    }

    private func cancel() {
        guard let current = active else { return }
        active = nil
        synthesizer.stopSpeaking(at: .immediate)
        current.completion.resume(throwing: PluginError.cancelled)
    }

    private func finish(_ identity: ObjectIdentifier, cancelled: Bool) {
        guard let current = active, ObjectIdentifier(current.utterance) == identity else { return }
        active = nil

        if cancelled {
            current.completion.resume(throwing: PluginError.cancelled)
        } else {
            current.completion.resume(returning: Data())
        }
    }

    nonisolated public func speechSynthesizer(
        _ synthesizer: AVSpeechSynthesizer,
        didFinish utterance: AVSpeechUtterance
    ) {
        let identity = ObjectIdentifier(utterance)
        Task { @MainActor [weak self] in self?.finish(identity, cancelled: false) }
    }

    nonisolated public func speechSynthesizer(
        _ synthesizer: AVSpeechSynthesizer,
        didCancel utterance: AVSpeechUtterance
    ) {
        let identity = ObjectIdentifier(utterance)
        Task { @MainActor [weak self] in self?.finish(identity, cancelled: true) }
    }
}
