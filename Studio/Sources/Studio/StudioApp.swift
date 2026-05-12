import SwiftUI

@main
struct ToolkitApp: App {
    @State private var library = LibraryStore()
    @State private var preferences = ToolkitPreferences()
    @State private var runResults = RunResultStore()
    @StateObject private var mfa = MFAPromptCoordinator()

    var body: some Scene {
        WindowGroup("Forage Toolkit") {
            ContentView()
                .environment(library)
                .environment(preferences)
                .environment(runResults)
                .environmentObject(mfa)
                .frame(minWidth: 1100, minHeight: 700)
                .task {
                    library.refresh()
                }
                .sheet(isPresented: $mfa.isPresented) {
                    MFAPromptSheet(coordinator: mfa)
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

                Divider()

                Button("Validate") {
                    NotificationCenter.default.post(name: .toolkitValidate, object: nil)
                }
                .keyboardShortcut("v", modifiers: [.command, .shift])

                Button("Publish to Hub…") {
                    NotificationCenter.default.post(name: .toolkitPublish, object: nil)
                }
                .keyboardShortcut("p", modifiers: [.command, .shift])

                Divider()

                Button("Import from Hub…") {
                    NotificationCenter.default.post(name: .toolkitImportFromHub, object: nil)
                }
                .keyboardShortcut("i", modifiers: [.command, .shift])
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
    static let toolkitValidate = Notification.Name("ToolkitValidate")
    static let toolkitPublish = Notification.Name("ToolkitPublish")
    static let toolkitImportFromHub = Notification.Name("ToolkitImportFromHub")
}
