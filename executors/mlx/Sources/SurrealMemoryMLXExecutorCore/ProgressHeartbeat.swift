import Foundation

/// A progress timer backed by its own Dispatch queue.
///
/// MLX/Metal evaluation may synchronously occupy a Swift cooperative executor
/// thread. A heartbeat implemented as another `Task` can therefore be starved
/// alongside the work it is meant to supervise. Dispatch keeps the liveness
/// signal independent, and `stop()` synchronously drains any in-flight callback
/// before the terminal protocol message is written.
public final class ProgressHeartbeat: @unchecked Sendable {
    private let queue = DispatchQueue(label: "ai.prometheus.surreal-memory.mlx-heartbeat")
    private let timer: DispatchSourceTimer
    private let stateLock = NSLock()
    private var stopped = false

    public init(
        interval: TimeInterval,
        action: @escaping @Sendable () -> Void
    ) {
        precondition(interval > 0)
        timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + interval, repeating: interval)
        timer.setEventHandler(handler: action)
        timer.resume()
    }

    public func stop() {
        stateLock.lock()
        guard !stopped else {
            stateLock.unlock()
            return
        }
        stopped = true
        stateLock.unlock()

        queue.sync {
            timer.setEventHandler {}
            timer.cancel()
        }
    }

    deinit {
        stop()
    }
}
