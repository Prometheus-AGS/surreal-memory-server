import Foundation

public let executorProtocolVersion = 1

public struct ExecutorRequest: Decodable, Sendable {
    public let requestID: UInt64
    public let operationID: String?
    public let command: ExecutorCommand

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case operationID = "operation_id"
        case command
    }
}
public enum ExecutorCommand: Decodable, Sendable {
    case plan(text: String)
    case embed(partIndex: Int, text: String)
    case embedBatch(texts: [String])

    enum CodingKeys: String, CodingKey {
        case command
        case text
        case partIndex = "part_index"
        case texts
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(String.self, forKey: .command) {
        case "plan":
            self = .plan(text: try values.decode(String.self, forKey: .text))
        case "embed":
            self = .embed(
                partIndex: try values.decode(Int.self, forKey: .partIndex),
                text: try values.decode(String.self, forKey: .text)
            )
        case "embed_batch":
            self = .embedBatch(texts: try values.decode([String].self, forKey: .texts))
        case let command:
            throw DecodingError.dataCorruptedError(
                forKey: .command,
                in: values,
                debugDescription: "unsupported executor command '\(command)'"
            )
        }
    }
}

public struct EmbeddingPlanPart: Codable, Equatable, Sendable {
    public let partIndex: Int
    public let tokenStart: Int
    public let tokenEnd: Int
    public let tokenCount: Int
    public let tokenHash: String
    public let content: String

    public init(
        partIndex: Int,
        tokenStart: Int,
        tokenEnd: Int,
        tokenCount: Int,
        tokenHash: String,
        content: String
    ) {
        self.partIndex = partIndex
        self.tokenStart = tokenStart
        self.tokenEnd = tokenEnd
        self.tokenCount = tokenCount
        self.tokenHash = tokenHash
        self.content = content
    }

    enum CodingKeys: String, CodingKey {
        case partIndex = "part_index"
        case tokenStart = "token_start"
        case tokenEnd = "token_end"
        case tokenCount = "token_count"
        case tokenHash = "token_hash"
        case content
    }
}

public enum ExecutorResult: Encodable, Sendable {
    case plan(parts: [EmbeddingPlanPart])
    case embedding([Float])
    case batch([[Float]])

    enum CodingKeys: String, CodingKey {
        case result
        case parts
        case embedding
        case embeddings
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .plan(let parts):
            try values.encode("plan", forKey: .result)
            try values.encode(parts, forKey: .parts)
        case .embedding(let embedding):
            try values.encode("embedding", forKey: .result)
            try values.encode(embedding, forKey: .embedding)
        case .batch(let embeddings):
            try values.encode("batch", forKey: .result)
            try values.encode(embeddings, forKey: .embeddings)
        }
    }
}

public enum ExecutorMessage: Encodable, Sendable {
    case ready(
        backend: String,
        modelID: String,
        modelRevision: String,
        dimensions: Int
    )
    case progress(requestID: UInt64, phase: String)
    case completed(requestID: UInt64, result: ExecutorResult)
    case failed(requestID: UInt64, error: String)

    enum CodingKeys: String, CodingKey {
        case message
        case protocolVersion = "protocol_version"
        case backend
        case modelID = "model_id"
        case modelRevision = "model_revision"
        case dimensions
        case requestID = "request_id"
        case phase
        case result
        case error
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .ready(let backend, let modelID, let modelRevision, let dimensions):
            try values.encode("ready", forKey: .message)
            try values.encode(executorProtocolVersion, forKey: .protocolVersion)
            try values.encode(backend, forKey: .backend)
            try values.encode(modelID, forKey: .modelID)
            try values.encode(modelRevision, forKey: .modelRevision)
            try values.encode(dimensions, forKey: .dimensions)
        case .progress(let requestID, let phase):
            try values.encode("progress", forKey: .message)
            try values.encode(requestID, forKey: .requestID)
            try values.encode(phase, forKey: .phase)
        case .completed(let requestID, let result):
            try values.encode("completed", forKey: .message)
            try values.encode(requestID, forKey: .requestID)
            try values.encode(result, forKey: .result)
        case .failed(let requestID, let error):
            try values.encode("failed", forKey: .message)
            try values.encode(requestID, forKey: .requestID)
            try values.encode(error, forKey: .error)
        }
    }
}
