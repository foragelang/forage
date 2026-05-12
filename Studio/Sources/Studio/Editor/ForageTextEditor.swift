import SwiftUI
import AppKit

/// `NSTextView` wrapped as an `NSViewRepresentable`. Hosts a Forage recipe
/// source string with debounced syntax highlighting. The view is the source
/// of truth between edits (Swift bindings flow into it on parent updates),
/// but during typing the text view drives the binding.
struct ForageTextEditor: NSViewRepresentable {
    @Binding var text: String

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true

        guard let textView = scrollView.documentView as? NSTextView else {
            return scrollView
        }
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isContinuousSpellCheckingEnabled = false
        textView.isAutomaticLinkDetectionEnabled = false
        textView.isAutomaticDataDetectionEnabled = false
        textView.isAutomaticTextCompletionEnabled = false
        textView.font = Self.editorFont
        textView.textColor = .textColor
        textView.usesFindBar = true
        textView.isHorizontallyResizable = false
        textView.textContainer?.widthTracksTextView = true
        textView.textContainerInset = NSSize(width: 8, height: 8)

        textView.string = text
        context.coordinator.applyHighlighting(textView: textView)
        return scrollView
    }

    func updateNSView(_ nsView: NSScrollView, context: Context) {
        guard let textView = nsView.documentView as? NSTextView else { return }
        if textView.string != text {
            // External change: replace + reapply highlighting.
            let selected = textView.selectedRanges
            textView.string = text
            textView.selectedRanges = selected
            context.coordinator.applyHighlighting(textView: textView)
        }
    }

    static let editorFont: NSFont = {
        let size: CGFloat = 13
        return NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
    }()

    static let boldEditorFont: NSFont = {
        let size: CGFloat = 13
        return NSFont.monospacedSystemFont(ofSize: size, weight: .semibold)
    }()

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        @Binding var text: String
        private var highlightTask: Task<Void, Never>?

        init(text: Binding<String>) {
            _text = text
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            let newValue = textView.string
            if newValue != text { text = newValue }
            scheduleHighlighting(for: textView)
        }

        func scheduleHighlighting(for textView: NSTextView) {
            highlightTask?.cancel()
            highlightTask = Task { @MainActor [weak self, weak textView] in
                try? await Task.sleep(nanoseconds: 250_000_000)
                guard let self, let textView, !Task.isCancelled else { return }
                self.applyHighlighting(textView: textView)
            }
        }

        func applyHighlighting(textView: NSTextView) {
            guard let storage = textView.textStorage else { return }
            let source = storage.string
            let tokens = ForageTokenizer.tokenize(source)
            let fullRange = NSRange(location: 0, length: storage.length)

            storage.beginEditing()
            storage.removeAttribute(.foregroundColor, range: fullRange)
            storage.removeAttribute(.font, range: fullRange)
            storage.addAttribute(.foregroundColor, value: NSColor.textColor, range: fullRange)
            storage.addAttribute(.font, value: ForageTextEditor.editorFont, range: fullRange)

            for token in tokens {
                guard NSMaxRange(token.range) <= storage.length else { continue }
                storage.addAttribute(.foregroundColor, value: token.kind.color, range: token.range)
                if token.kind.isBold {
                    storage.addAttribute(.font, value: ForageTextEditor.boldEditorFont, range: token.range)
                }
            }
            storage.endEditing()
        }
    }
}
