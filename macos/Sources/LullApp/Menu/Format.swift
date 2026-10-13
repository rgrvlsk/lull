// Text for rows: sizes, countdowns, relative times, and the restart command line.
import Foundation
import LullKit

enum Format {
    static func bytes(_ n: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: n, countStyle: .memory)
    }

    /// `12:04` under an hour, `1 h 05 min` above; never negative.
    static func countdown(to date: Date, now: Date) -> String {
        let s = max(0, Int(date.timeIntervalSince(now).rounded(.up)))
        if s >= 3600 { return String(format: "%d h %02d min", s / 3600, s % 3600 / 60) }
        return String(format: "%d:%02d", s / 60, s % 60)
    }

    static func ago(_ date: Date, now: Date = .now) -> String {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f.localizedString(for: date, relativeTo: now)
    }

    static func ports(_ ports: [Int]) -> String {
        guard let first = ports.first else { return "no port" }
        return ports.count > 1 ? ":\(first) +\(ports.count - 1)" : ":\(first)"
    }

    static func origin(_ origin: Origin) -> String {
        origin.kind == .agent ? (origin.agent ?? "agent") : "you"
    }

    static func reason(_ reason: StopReason) -> String {
        switch reason {
        case .idle: "stopped when idle"
        case .manual: "stopped by request"
        case .exited: "exited"
        case .unknown: "stopped"
        }
    }

    /// POSIX shell quoting, for the command a restart runs (§7 Restart).
    static func shellQuote(_ args: [String]) -> String {
        args.map { arg in
            let safe = !arg.isEmpty && arg.allSatisfy { $0.isLetter || $0.isNumber || "-_./:=@%+,".contains($0) }
            return safe ? arg : "'" + arg.replacingOccurrences(of: "'", with: #"'\''"#) + "'"
        }.joined(separator: " ")
    }

    /// What a restart runs: the kept env (`PORT`, `HOST`, `NODE_ENV`) and the launch command,
    /// in the user's login shell.
    static func restartCommand(_ record: StoppedServer) -> String {
        let env = record.restartEnv.sorted { $0.key < $1.key }.map { "\($0.key)=\(shellQuote([$0.value]))" }
        return (env + [shellQuote(record.argv)]).joined(separator: " ")
    }

    static func tilde(_ path: String) -> String {
        let home = NSHomeDirectory()
        return path.hasPrefix(home + "/") ? "~" + path.dropFirst(home.count) : path
    }
}
