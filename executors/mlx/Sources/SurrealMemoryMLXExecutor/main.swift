import Foundation
import SurrealMemoryMLXExecutorCore

final class OutputWriter: @unchecked Sendable {
    private let lock = NSLock()

    func write(_ message: ExecutorMessage) throws {
        lock.lock()
        defer { lock.unlock() }
        var data = try JSONEncoder().encode(message)
        data.append(0x0A)
        FileHandle.standardOutput.write(data)
    }
}
@main
enum SurrealMemoryMLXExecutorMain {
    static func main() async {
        do {
            try await run()
        } catch {
            writeStandardError("surreal-memory-mlx-executor: \(error.localizedDescription)\n")
            Foundation.exit(1)
        }
    }

    private static func run() async throws {
        let arguments = Array(CommandLine.arguments.dropFirst())
        let settings = try ExecutorSettings.fromEnvironment()
        switch arguments.first {
        case "--version", "-V":
            print("surreal-memory-mlx-executor 1.0.0 mlx-swift-lm 3.31.4")
        case "--prefetch":
            let service = try await MLXEmbeddingService.prefetch(settings: settings)
            try await printSmoke(service: service, mode: "prefetch")
        case "--smoke":
            let service = try await MLXEmbeddingService.loadCached(settings: settings)
            try await printSmoke(service: service, mode: "smoke")
        case "embedding-executor", nil:
            let service = try await MLXEmbeddingService.loadCached(settings: settings)
            _ = try await service.warmup()
            try await runProtocol(service: service)
        default:
            throw CocoaError(.executableLoad)
        }
    }

    private static func printSmoke(
        service: MLXEmbeddingService,
        mode: String
    ) async throws {
        let embedding = try await service.warmup()
        let norm = sqrt(embedding.map { $0 * $0 }.reduce(0, +))
        let output: [String: Any] = [
            "backend": "mlx",
            "dimensions": embedding.count,
            "mode": mode,
            "model_id": service.settings.modelID,
            "model_revision": service.settings.modelRevision,
            "norm": norm,
            "status": "ok"
        ]
        let data = try JSONSerialization.data(withJSONObject: output, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    }

    private static func runProtocol(service: MLXEmbeddingService) async throws {
        let writer = OutputWriter()
        try writer.write(
            .ready(
                backend: "mlx",
                modelID: service.settings.modelID,
                modelRevision: service.settings.modelRevision,
                dimensions: service.settings.dimensions
            )
        )

        while let line = readLine(strippingNewline: true) {
            let request: ExecutorRequest
            do {
                request = try JSONDecoder().decode(
                    ExecutorRequest.self,
                    from: Data(line.utf8)
                )
            } catch {
                try writer.write(.failed(requestID: 0, error: "decode request: \(error)"))
                continue
            }

            try writer.write(.progress(requestID: request.requestID, phase: "accepted"))
            // MLX evaluation can synchronously occupy a Swift cooperative
            // executor thread while Metal finishes the graph. Drive watchdog
            // progress from a dedicated Dispatch queue so a busy inference
            // cannot starve its own heartbeat and be mistaken for a hang.
            let heartbeat = ProgressHeartbeat(interval: 0.25) {
                try? writer.write(
                    .progress(requestID: request.requestID, phase: "working")
                )
            }

            let message: ExecutorMessage
            do {
                let result: ExecutorResult
                switch request.command {
                case .plan(let text):
                    result = .plan(parts: try await service.plan(text: text))
                case .embed(_, let text):
                    result = .embedding(try await service.embedBatch([text])[0])
                case .embedBatch(let texts):
                    result = .batch(try await service.embedBatch(texts))
                }
                message = .completed(requestID: request.requestID, result: result)
            } catch {
                message = .failed(requestID: request.requestID, error: String(describing: error))
            }
            heartbeat.stop()
            try writer.write(message)
        }
    }

    private static func writeStandardError(_ text: String) {
        FileHandle.standardError.write(Data(text.utf8))
    }
}
