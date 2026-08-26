import Foundation
import Testing
@testable import SurrealMemoryMLXExecutorCore

@Test func decodesEmbedRequest() throws {
    let json = #"{"request_id":7,"operation_id":"op-1","command":{"command":"embed","part_index":2,"text":"hello"}}"#
    let request = try JSONDecoder().decode(ExecutorRequest.self, from: Data(json.utf8))
    #expect(request.requestID == 7)
    #expect(request.operationID == "op-1")
    guard case .embed(let partIndex, let text) = request.command else {
        Issue.record("expected embed command")
        return
    }
    #expect(partIndex == 2)
    #expect(text == "hello")
}

@Test func readyMessageCarriesCertifiedIdentity() throws {
    let data = try JSONEncoder().encode(
        ExecutorMessage.ready(
            backend: "mlx",
            modelID: "BAAI/bge-small-en-v1.5",
            modelRevision: "revision",
            dimensions: 384
        )
    )
    let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    #expect(object["message"] as? String == "ready")
    #expect(object["protocol_version"] as? Int == executorProtocolVersion)
    #expect(object["backend"] as? String == "mlx")
    #expect(object["dimensions"] as? Int == 384)
}

@Test func plannerUsesDeterministicOverlappingTokenWindows() throws {
    func encode(_ text: String, _ special: Bool) -> [Int] {
        let values = text.split(separator: " ").compactMap { Int($0) }
        return special ? [101] + values + [102] : values
    }
    func decode(_ ids: [Int], _: Bool) -> String {
        ids.map(String.init).joined(separator: " ")
    }

    let input = (1...12).map(String.init).joined(separator: " ")
    let first = try EmbeddingPlanner.plan(
        text: input,
        maxInputTokens: 8,
        encode: encode,
        decode: decode
    )
    let second = try EmbeddingPlanner.plan(
        text: input,
        maxInputTokens: 8,
        encode: encode,
        decode: decode
    )

    #expect(first == second)
    #expect(first.count == 7)
    #expect(first[0].tokenStart == 0)
    #expect(first[0].tokenEnd == 6)
    #expect(first[1].tokenStart == 1)
    #expect(first.allSatisfy { $0.tokenHash.count == 64 })
}

@Test func malformedCommandIsRejected() {
    let json = #"{"request_id":7,"command":{"command":"unknown"}}"#
    #expect(throws: DecodingError.self) {
        try JSONDecoder().decode(ExecutorRequest.self, from: Data(json.utf8))
    }
}

private final class HeartbeatCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0

    func increment() {
        lock.lock()
        value += 1
        lock.unlock()
    }

    func read() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

@Test func progressHeartbeatRunsIndependentlyAndStopsSynchronously() async throws {
    let counter = HeartbeatCounter()
    let heartbeat = ProgressHeartbeat(interval: 0.01) {
        counter.increment()
    }

    try await Task.sleep(for: .milliseconds(80))
    heartbeat.stop()
    let stoppedAt = counter.read()
    #expect(stoppedAt > 0)

    try await Task.sleep(for: .milliseconds(40))
    #expect(counter.read() == stoppedAt)
}
