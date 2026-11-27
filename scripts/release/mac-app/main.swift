import AppKit

final class Launcher: NSObject, NSApplicationDelegate {
    let engine = Process()
    let pipe = Pipe()
    var page: URL?
    var quitting = false
    var pending = Data()

    func applicationDidFinishLaunching(_ notification: Notification) {
        let menu = NSMenu()
        let submenu = NSMenu()
        submenu.addItem(withTitle: "Quit Pawket", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        let item = NSMenuItem()
        item.submenu = submenu
        menu.addItem(item)
        NSApp.mainMenu = menu

        let root = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
        let data = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Pawket/Downloads")
        engine.executableURL = root.appendingPathComponent("pawket")
        engine.arguments = ["serve", "--port", "0", "--output", data.path]
        var env = ProcessInfo.processInfo.environment
        env["PATH"] = root.path + ":" + (env["PATH"] ?? "/usr/bin:/bin")
        engine.environment = env
        engine.standardError = pipe
        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let chunk = handle.availableData
            if chunk.isEmpty { handle.readabilityHandler = nil; return }
            DispatchQueue.main.async { self?.read(chunk) }
        }
        engine.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async { if self?.quitting == false { self?.failed("The download engine stopped. Open Pawket again to restart it.") } }
        }
        do { try engine.run() } catch { failed(error.localizedDescription) }
    }

    func read(_ chunk: Data) {
        pending.append(chunk)
        while let newline = pending.firstIndex(of: 10) {
            let line = String(decoding: pending[..<newline], as: UTF8.self)
            pending.removeSubrange(...newline)
            let prefix = "Pawket is ready at "
            if line.hasPrefix(prefix), let url = URL(string: String(line.dropFirst(prefix.count))),
               url.scheme == "http", url.host == "127.0.0.1", url.port != nil {
                page = url
                NSWorkspace.shared.open(url)
            }
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if let page = page { NSWorkspace.shared.open(page) }
        return true
    }

    func failed(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Pawket couldn’t start"
        alert.informativeText = message
        alert.runModal()
        NSApp.terminate(nil)
    }

    func applicationWillTerminate(_ notification: Notification) {
        quitting = true
        if engine.isRunning { engine.terminate() }
    }
}

let launcher = Launcher()
let application = NSApplication.shared
application.setActivationPolicy(.regular)
application.delegate = launcher
application.run()
