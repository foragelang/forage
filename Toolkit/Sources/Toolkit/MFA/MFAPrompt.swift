import Foundation
import SwiftUI
import Forage

/// MFA provider that drives a modal sheet on the main window. The engine
/// awaits `mfaCode()` from a tokio-style async context; we hop to the
/// MainActor, show the sheet, and resume the continuation on submit / cancel.
///
/// Single-shot per run: the engine asks for a code once, the sheet collects
/// it, and the continuation resolves. If the user closes the sheet without
/// submitting (the Cancel button), the continuation throws
/// `MFAError.cancelled` and the engine surfaces `stallReason: "auth-mfa-cancelled"`.
@MainActor
public final class MFAPromptCoordinator: ObservableObject {
    @Published public var isPresented: Bool = false
    @Published public var draft: String = ""

    private var pendingContinuation: CheckedContinuation<String, Error>?

    public init() {}

    /// Called from the SwiftUI sheet's "Submit" button.
    public func submit() {
        guard let cont = pendingContinuation else { return }
        pendingContinuation = nil
        let code = draft
        draft = ""
        isPresented = false
        cont.resume(returning: code)
    }

    /// Called from the SwiftUI sheet's "Cancel" button or the dismiss handler.
    public func cancel() {
        guard let cont = pendingContinuation else { return }
        pendingContinuation = nil
        draft = ""
        isPresented = false
        cont.resume(throwing: MFAError.cancelled)
    }
}

/// `MFAProvider` adapter that delegates to a `MFAPromptCoordinator`. The
/// coordinator is `@MainActor` because SwiftUI's `@Published` mutations must
/// be on the main thread; the engine call site is async and may be on a
/// background task — we hop via `MainActor.run` to register the
/// continuation, then `await` the result.
public struct SheetMFAProvider: MFAProvider {
    let coordinator: MFAPromptCoordinator

    public init(coordinator: MFAPromptCoordinator) {
        self.coordinator = coordinator
    }

    public func mfaCode() async throws -> String {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<String, Error>) in
            Task { @MainActor in
                // Replace any previous pending continuation (shouldn't happen
                // because the engine only asks for one code per run, but be
                // defensive — leak prevention beats correctness debate).
                coordinator.cancelLingering()
                coordinator.attach(cont)
                coordinator.isPresented = true
            }
        }
    }
}

extension MFAPromptCoordinator {
    fileprivate func attach(_ cont: CheckedContinuation<String, Error>) {
        pendingContinuation = cont
    }

    fileprivate func cancelLingering() {
        if let cont = pendingContinuation {
            pendingContinuation = nil
            cont.resume(throwing: MFAError.cancelled)
        }
    }
}

/// Modal sheet view. Attach to the root view via `.sheet(isPresented:)`
/// bound to `coordinator.isPresented`.
public struct MFAPromptSheet: View {
    @ObservedObject public var coordinator: MFAPromptCoordinator

    public init(coordinator: MFAPromptCoordinator) {
        self.coordinator = coordinator
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Enter MFA code")
                .font(.headline)
            Text("The recipe requires a second-factor authentication code to log in. Enter it now to continue the run.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            SecureField("Code", text: $coordinator.draft)
                .textFieldStyle(.roundedBorder)
                .frame(minWidth: 200)
                .onSubmit { coordinator.submit() }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { coordinator.cancel() }
                Button("Submit") { coordinator.submit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(coordinator.draft.isEmpty)
            }
        }
        .padding(20)
        .frame(minWidth: 320)
    }
}
