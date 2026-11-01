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
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        if method == 1 { return try await speak(SpeechRequest(payload: payload)) }

        guard payload.count == 0 else { throw PluginError.invalidArgument }
        switch method {
        case 0: return try voices()
        case 2:
            cancel()
            return Data()
        default:
            return Data([active == nil ? 0 : 1, 0, 0, 0])
        }
    }

    public func close() {
        closed = true
        cancel()
        synthesizer.delegate = nil
    }

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

        if let identifier = request.voice {
            guard let voice = AVSpeechSynthesisVoice(identifier: identifier) else { throw PluginError.unavailable }
            utterance.voice = voice
        } else if let language = request.language {
            guard let voice = AVSpeechSynthesisVoice(language: language) else { throw PluginError.unavailable }
            utterance.voice = voice
        }

        utterance.rate = min(AVSpeechUtteranceMaximumSpeechRate,
                             max(AVSpeechUtteranceMinimumSpeechRate, AVSpeechUtteranceDefaultSpeechRate * request.rate))
        utterance.pitchMultiplier = request.pitch
        utterance.volume = request.volume
        let token = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { completion in

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
        } onCancel: {
            Task { @MainActor [weak self] in

                guard self?.active?.token == token else { return }
                self?.cancel()
            }
        }
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

    nonisolated public func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer,
                                              didFinish utterance: AVSpeechUtterance) {
        let identity = ObjectIdentifier(utterance)
        Task { @MainActor [weak self] in self?.finish(identity, cancelled: false) }
    }

    nonisolated public func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer,
                                              didCancel utterance: AVSpeechUtterance) {
        let identity = ObjectIdentifier(utterance)
        Task { @MainActor [weak self] in self?.finish(identity, cancelled: true) }
    }
}
