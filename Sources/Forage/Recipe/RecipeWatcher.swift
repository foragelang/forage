import Foundation

/// Polls the modification times of `<root>/*/recipe.forage` and fires the
/// callback whenever any of them appears, changes, or disappears.
///
/// Polling — not `DispatchSource.makeFileSystemObjectSource` — because the
/// directory-FD approach has too many sharp edges for a dev hot-reload
/// path: atomic-write saves (the common case for editors) replace the
/// inode the FD points at, so the watcher needs constant re-opening; and
/// the watch only fires for the watched FD itself, so tracking a tree
/// means juggling one watcher per child. A 500ms mtime poll is bulletproof
/// across all of that, cheap on a handful of files, and good enough for a
/// human-driven edit-save loop.
internal final class RecipeWatcher: @unchecked Sendable {
    private let root: URL
    private let interval: TimeInterval
    private let queue: DispatchQueue
    private let onChange: @Sendable @MainActor (URL) -> Void
    private var timer: DispatchSourceTimer?
    private var lastStamps: [URL: Date] = [:]

    init(
        root: URL,
        interval: TimeInterval = 0.5,
        onChange: @escaping @Sendable @MainActor (URL) -> Void
    ) {
        self.root = root
        self.interval = interval
        self.queue = DispatchQueue(label: "fm.forage.RecipeWatcher", qos: .utility)
        self.onChange = onChange
    }

    deinit {
        timer?.cancel()
    }

    func start() {
        queue.async { [weak self] in
            guard let self else { return }
            self.lastStamps = self.scan()
            let t = DispatchSource.makeTimerSource(queue: self.queue)
            t.schedule(deadline: .now() + self.interval, repeating: self.interval)
            t.setEventHandler { [weak self] in self?.tick() }
            self.timer = t
            t.resume()
        }
    }

    private func tick() {
        let current = scan()
        var changed: [URL] = []
        let allKeys = Set(current.keys).union(lastStamps.keys)
        for key in allKeys {
            if current[key] != lastStamps[key] {
                changed.append(key)
            }
        }
        lastStamps = current
        guard !changed.isEmpty else { return }
        let callback = onChange
        Task { @MainActor in
            for url in changed { callback(url) }
        }
    }

    private func scan() -> [URL: Date] {
        let fm = FileManager.default
        guard let entries = try? fm.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey, .contentModificationDateKey]
        ) else {
            return [:]
        }
        var out: [URL: Date] = [:]
        for entry in entries {
            let isDir = (try? entry.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
            guard isDir else { continue }
            let file = entry.appendingPathComponent("recipe.forage")
            guard let attrs = try? fm.attributesOfItem(atPath: file.path),
                  let mtime = attrs[.modificationDate] as? Date else { continue }
            out[file] = mtime
        }
        return out
    }
}
