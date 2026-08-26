import CryptoKit
import Foundation

public enum EmbeddingPlannerError: Error, LocalizedError, Equatable {
    case noContentCapacity(maximum: Int, specialTokens: Int)
    case unableToConstructWindow

    public var errorDescription: String? {
        switch self {
        case .noContentCapacity(let maximum, let specialTokens):
            "model capacity \(maximum) does not leave room after \(specialTokens) special tokens"
        case .unableToConstructWindow:
            "unable to construct a model-safe token window"
        }
    }
}
public enum EmbeddingPlanner {
    public static func plan(
        text: String,
        maxInputTokens: Int,
        encode: (String, Bool) throws -> [Int],
        decode: ([Int], Bool) throws -> String
    ) throws -> [EmbeddingPlanPart] {
        let specialTokens = try encode("", true).count
        let usable = maxInputTokens - specialTokens
        guard usable > 0 else {
            throw EmbeddingPlannerError.noContentCapacity(
                maximum: maxInputTokens,
                specialTokens: specialTokens
            )
        }

        let source = try encode(text, false)
        if source.count + specialTokens <= maxInputTokens {
            return [part(index: 0, start: 0, end: source.count, ids: source, content: text)]
        }

        let overlap = min(32, max(0, usable - 1))
        let step = usable - overlap
        var parts: [EmbeddingPlanPart] = []
        var start = 0
        while start < source.count {
            var end = min(start + usable, source.count)
            var content = try decode(Array(source[start..<end]), true)

            while try encode(content, true).count > maxInputTokens {
                end -= 1
                guard end > start else {
                    throw EmbeddingPlannerError.unableToConstructWindow
                }
                content = try decode(Array(source[start..<end]), true)
            }

            parts.append(
                part(
                    index: parts.count,
                    start: start,
                    end: end,
                    ids: Array(source[start..<end]),
                    content: content
                )
            )
            if end == source.count {
                break
            }
            start = min(start + step, end)
        }
        return parts
    }

    private static func part(
        index: Int,
        start: Int,
        end: Int,
        ids: [Int],
        content: String
    ) -> EmbeddingPlanPart {
        var bytes = Data(capacity: ids.count * MemoryLayout<UInt32>.size)
        for id in ids {
            var littleEndian = UInt32(id).littleEndian
            withUnsafeBytes(of: &littleEndian) { bytes.append(contentsOf: $0) }
        }
        let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        return EmbeddingPlanPart(
            partIndex: index,
            tokenStart: start,
            tokenEnd: end,
            tokenCount: end - start,
            tokenHash: digest,
            content: content
        )
    }
}
