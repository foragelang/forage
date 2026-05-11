import SwiftUI

@main
struct ToolkitApp: App {
    @State private var library = LibraryStore()
    @State private var preferences = ToolkitPreferences()
    @State private var runResults = RunResultStore()

    var body: some Scene {
        WindowGroup("Forage Toolkit") {
            ContentView()
                .environment(library)
                .environment(preferences)
                .environment(runResults)
                .frame(minWidth: 1100, minHeight: 700)
                .task {
                    library.refresh()
                }
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Recipe") {
                    library.createNewRecipe()
                }
                .keyboardShortcut("n", modifiers: [.command])
            }
            CommandMenu("Recipe") {
                Button("Run Live") {
                    NotificationCenter.default.post(name: .toolkitRunLive, object: nil)
                }
                .keyboardShortcut("r", modifiers: [.command])

                Button("Run Replay") {
                    NotificationCenter.default.post(name: .toolkitRunReplay, object: nil)
                }
                .keyboardShortcut("r", modifiers: [.command, .shift])

                Button("Capture from URL…") {
                    NotificationCenter.default.post(name: .toolkitCapture, object: nil)
                }
                .keyboardShortcut("k", modifiers: [.command])

                Button("Save") {
                    NotificationCenter.default.post(name: .toolkitSave, object: nil)
                }
                .keyboardShortcut("s", modifiers: [.command])
            }
        }
        Settings {
            PreferencesView()
                .environment(preferences)
                .frame(width: 480, height: 240)
        }
    }
}

extension Notification.Name {
    static let toolkitRunLive = Notification.Name("ToolkitRunLive")
    static let toolkitRunReplay = Notification.Name("ToolkitRunReplay")
    static let toolkitCapture = Notification.Name("ToolkitCapture")
    static let toolkitSave = Notification.Name("ToolkitSave")
}
