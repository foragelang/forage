import SwiftUI

@main
struct StudioApp: App {
    @State private var library = LibraryStore()
    @State private var preferences = StudioPreferences()
    @State private var runResults = RunResultStore()
    @StateObject private var mfa = MFAPromptCoordinator()

    var body: some Scene {
        WindowGroup("Forage Studio") {
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
                    NotificationCenter.default.post(name: .studioRunLive, object: nil)
                }
                .keyboardShortcut("r", modifiers: [.command])

                Button("Run Replay") {
                    NotificationCenter.default.post(name: .studioRunReplay, object: nil)
                }
                .keyboardShortcut("r", modifiers: [.command, .shift])

                Button("Capture from URL…") {
                    NotificationCenter.default.post(name: .studioCapture, object: nil)
                }
                .keyboardShortcut("k", modifiers: [.command])

                Button("Save") {
                    NotificationCenter.default.post(name: .studioSave, object: nil)
                }
                .keyboardShortcut("s", modifiers: [.command])

                Divider()

                Button("Validate") {
                    NotificationCenter.default.post(name: .studioValidate, object: nil)
                }
                .keyboardShortcut("v", modifiers: [.command, .shift])

                Button("Publish to Hub…") {
                    NotificationCenter.default.post(name: .studioPublish, object: nil)
                }
                .keyboardShortcut("p", modifiers: [.command, .shift])

                Divider()

                Button("Import from Hub…") {
                    NotificationCenter.default.post(name: .studioImportFromHub, object: nil)
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
    static let studioRunLive = Notification.Name("StudioRunLive")
    static let studioRunReplay = Notification.Name("StudioRunReplay")
    static let studioCapture = Notification.Name("StudioCapture")
    static let studioSave = Notification.Name("StudioSave")
    static let studioValidate = Notification.Name("StudioValidate")
    static let studioPublish = Notification.Name("StudioPublish")
    static let studioImportFromHub = Notification.Name("StudioImportFromHub")
}
