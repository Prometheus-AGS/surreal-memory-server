import Foundation
import HuggingFace
import MLX
import MLXEmbedders
import MLXHuggingFace
import MLXLMCommon
import SurrealMemoryMLXExecutorCore
import Tokenizers

struct ExecutorSettings: Sendable {
    static let defaultModelID = "BAAI/bge-small-en-v1.5"
    static let defaultRevision = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a"
    static let expectedDimensions = 384

    let modelID: String
    let modelRevision: String
    let dimensions: Int
    let hubCache: URL

    static func fromEnvironment() throws -> ExecutorSettings {
        let environment = ProcessInfo.processInfo.environment
        let modelID = environment["LOCAL_EMBEDDING_MODEL"] ?? defaultModelID
        let revision = environment["LOCAL_EMBEDDING_MODEL_REVISION"] ?? defaultRevision
        let dimensions = Int(environment["LOCAL_EMBEDDING_DIMENSIONS"] ?? "")
            ?? expectedDimensions

        guard modelID == defaultModelID else {
            throw ExecutorError.unsupportedModel(modelID)
        }
        guard revision == defaultRevision else {
            throw ExecutorError.unsupportedRevision(revision)
        }
        guard dimensions == expectedDimensions else {
            throw ExecutorError.dimensionMismatch(
                expected: expectedDimensions,
                actual: dimensions
            )
        }

        let hubCache: URL
        if let configured = environment["HF_HUB_CACHE"] {
            hubCache = URL(filePath: configured, directoryHint: .isDirectory)
        } else if let home = environment["MODEL_CACHE_DIR"] ?? environment["HF_HOME"] {
            hubCache = URL(filePath: home, directoryHint: .isDirectory)
                .appending(path: "hub", directoryHint: .isDirectory)
        } else {
            hubCache = FileManager.default.homeDirectoryForCurrentUser
                .appending(path: ".cache/huggingface/hub", directoryHint: .isDirectory)
        }

        return ExecutorSettings(
            modelID: modelID,
            modelRevision: revision,
            dimensions: dimensions,
            hubCache: hubCache
        )
    }

    var snapshotDirectory: URL {
        let repository = "models--" + modelID.replacingOccurrences(of: "/", with: "--")
        return hubCache
            .appending(path: repository, directoryHint: .isDirectory)
            .appending(path: "snapshots", directoryHint: .isDirectory)
            .appending(path: modelRevision, directoryHint: .isDirectory)
    }
}

enum ExecutorError: Error, LocalizedError {
    case unsupportedModel(String)
    case unsupportedRevision(String)
    case dimensionMismatch(expected: Int, actual: Int)
    case missingSnapshot(URL)
    case snapshotRevisionMismatch(expected: String, actual: String)
    case missingModelFile(String)
    case invalidModelConfiguration
    case inputTooLong(actual: Int, maximum: Int)

    var errorDescription: String? {
        switch self {
        case .unsupportedModel(let model):
            "MLX executor is certified only for \(ExecutorSettings.defaultModelID), got \(model)"
        case .unsupportedRevision(let revision):
            "MLX executor is certified only for revision \(ExecutorSettings.defaultRevision), got \(revision)"
        case .dimensionMismatch(let expected, let actual):
            "embedding dimension mismatch: expected \(expected), got \(actual)"
        case .missingSnapshot(let path):
            "pinned model snapshot is not cached at \(path.path); run --prefetch first"
        case .snapshotRevisionMismatch(let expected, let actual):
            "download resolved revision \(actual), expected \(expected)"
        case .missingModelFile(let file):
            "pinned model snapshot is missing \(file); run --prefetch first"
        case .invalidModelConfiguration:
            "model config.json does not contain a positive max_position_embeddings"
        case .inputTooLong(let actual, let maximum):
            "input_too_long: tokenizer produced \(actual) tokens for model capacity \(maximum)"
        }
    }
}

final class MLXEmbeddingService: Sendable {
    let settings: ExecutorSettings
    let container: EmbedderModelContainer
    let maxInputTokens: Int

    private init(
        settings: ExecutorSettings,
        container: EmbedderModelContainer,
        maxInputTokens: Int
    ) {
        self.settings = settings
        self.container = container
        self.maxInputTokens = maxInputTokens
    }

    static func loadCached(settings: ExecutorSettings) async throws -> MLXEmbeddingService {
        let directory = settings.snapshotDirectory
        try validateSnapshot(directory, expectedRevision: settings.modelRevision)
        let container = try await EmbedderModelFactory.shared.loadContainer(
            from: directory,
            using: #huggingFaceTokenizerLoader()
        )
        let maximum = try readMaximumInputTokens(from: directory)
        return MLXEmbeddingService(
            settings: settings,
            container: container,
            maxInputTokens: maximum
        )
    }

    static func prefetch(settings: ExecutorSettings) async throws -> MLXEmbeddingService {
        try FileManager.default.createDirectory(
            at: settings.hubCache,
            withIntermediateDirectories: true
        )
        let hub = HubClient(cache: HubCache(cacheDirectory: settings.hubCache))
        let configuration = ModelConfiguration(
            id: settings.modelID,
            revision: settings.modelRevision
        )
        let container = try await EmbedderModelFactory.shared.loadContainer(
            from: #hubDownloader(hub),
            using: #huggingFaceTokenizerLoader(),
            configuration: configuration,
            useLatest: false
        )
        let resolvedDirectory = try await container.modelDirectory.resolvingSymlinksInPath()
        guard resolvedDirectory.lastPathComponent == settings.modelRevision else {
            throw ExecutorError.snapshotRevisionMismatch(
                expected: settings.modelRevision,
                actual: resolvedDirectory.lastPathComponent
            )
        }
        try validateSnapshot(resolvedDirectory, expectedRevision: settings.modelRevision)
        return MLXEmbeddingService(
            settings: settings,
            container: container,
            maxInputTokens: try readMaximumInputTokens(from: resolvedDirectory)
        )
    }

    func warmup() async throws -> [Float] {
        let embedding = try await embedBatch(["warmup"])[0]
        guard embedding.count == settings.dimensions else {
            throw ExecutorError.dimensionMismatch(
                expected: settings.dimensions,
                actual: embedding.count
            )
        }
        return embedding
    }

    func plan(text: String) async throws -> [EmbeddingPlanPart] {
        try await container.perform { context in
            try EmbeddingPlanner.plan(
                text: text,
                maxInputTokens: maxInputTokens,
                encode: { value, addSpecialTokens in
                    context.tokenizer.encode(
                        text: value,
                        addSpecialTokens: addSpecialTokens
                    )
                },
                decode: { tokenIDs, skipSpecialTokens in
                    context.tokenizer.decode(
                        tokenIds: tokenIDs,
                        skipSpecialTokens: skipSpecialTokens
                    )
                }
            )
        }
    }

    func embedBatch(_ texts: [String]) async throws -> [[Float]] {
        guard !texts.isEmpty else { return [] }
        let embeddings = try await container.perform { context in
            let encoded = texts.map {
                context.tokenizer.encode(text: $0, addSpecialTokens: true)
            }
            if let tooLong = encoded.first(where: { $0.count > maxInputTokens }) {
                throw ExecutorError.inputTooLong(
                    actual: tooLong.count,
                    maximum: maxInputTokens
                )
            }

            let maximum = encoded.map(\.count).max() ?? 0
            let paddingToken = context.tokenizer.convertTokenToId("[PAD]") ?? 0
            let paddedTokens = encoded.map {
                $0 + Array(repeating: paddingToken, count: maximum - $0.count)
            }
            let masks = encoded.map {
                Array(repeating: Float(1), count: $0.count)
                    + Array(repeating: Float(0), count: maximum - $0.count)
            }
            let inputs = stacked(paddedTokens.map { MLXArray($0) })
            let attentionMask = stacked(masks.map { MLXArray($0) })
            let tokenTypes = MLXArray.zeros(like: inputs)
            let output = context.model(
                inputs,
                positionIds: nil,
                tokenTypeIds: tokenTypes,
                attentionMask: attentionMask
            )
            let pooled = Pooling(
                strategy: .mean,
                dimension: settings.dimensions
            )(
                output,
                mask: attentionMask,
                normalize: true,
                applyLayerNorm: false
            )
            pooled.eval()
            return pooled.map { $0.asArray(Float.self) }
        }

        for embedding in embeddings where embedding.count != settings.dimensions {
            throw ExecutorError.dimensionMismatch(
                expected: settings.dimensions,
                actual: embedding.count
            )
        }
        return embeddings
    }

    private static func validateSnapshot(
        _ directory: URL,
        expectedRevision: String
    ) throws {
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(
            atPath: directory.path,
            isDirectory: &isDirectory
        ), isDirectory.boolValue else {
            throw ExecutorError.missingSnapshot(directory)
        }
        guard directory.lastPathComponent == expectedRevision else {
            throw ExecutorError.snapshotRevisionMismatch(
                expected: expectedRevision,
                actual: directory.lastPathComponent
            )
        }
        for file in ["config.json", "tokenizer.json", "model.safetensors"] {
            guard FileManager.default.fileExists(
                atPath: directory.appending(path: file).path
            ) else {
                throw ExecutorError.missingModelFile(file)
            }
        }
    }

    private static func readMaximumInputTokens(from directory: URL) throws -> Int {
        let data = try Data(contentsOf: directory.appending(path: "config.json"))
        let object = try JSONSerialization.jsonObject(with: data)
        guard let values = object as? [String: Any],
              let maximum = values["max_position_embeddings"] as? Int,
              maximum > 0 else {
            throw ExecutorError.invalidModelConfiguration
        }
        return maximum
    }
}
