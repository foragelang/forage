import Testing
@testable import Forage

@MainActor
@Test
func httpProgressStartsAtStarting() {
    let p = HTTPProgress()
    #expect(p.phase == .starting)
    #expect(p.requestsSent == 0)
    #expect(p.recordsEmitted == 0)
    #expect(p.currentURL == nil)
}

@MainActor
@Test
func httpProgressAdvancesThroughExpectedPhases() {
    let p = HTTPProgress()
    let transitions: [HTTPProgress.Phase] = [
        .priming,
        .stepping(name: "auth"),
        .stepping(name: "products"),
        .paginating(name: "products", page: 1),
        .paginating(name: "products", page: 2),
        .paginating(name: "products", page: 3),
        .done,
    ]
    for phase in transitions {
        p.setPhase(phase)
        #expect(p.phase == phase)
    }
}

@MainActor
@Test
func httpProgressFailedCarriesMessage() {
    let p = HTTPProgress()
    p.setPhase(.failed("404 from upstream"))
    #expect(p.phase == .failed("404 from upstream"))
    #expect(p.phase != .failed("connection-reset"))
}

@MainActor
@Test
func httpProgressNoteRequestSentBumpsCounterAndURL() {
    let p = HTTPProgress()
    p.noteRequestSent(url: "https://example.com/api/products?page=1")
    #expect(p.requestsSent == 1)
    #expect(p.currentURL == "https://example.com/api/products?page=1")
    p.noteRequestSent(url: "https://example.com/api/products?page=2")
    #expect(p.requestsSent == 2)
    #expect(p.currentURL == "https://example.com/api/products?page=2")
    p.noteRequestSent(url: nil)
    #expect(p.requestsSent == 3)
    #expect(p.currentURL == nil)
}

@MainActor
@Test
func httpProgressSetRecordsEmittedMonotonicAndOverridable() {
    let p = HTTPProgress()
    p.setRecordsEmitted(0)
    #expect(p.recordsEmitted == 0)
    p.setRecordsEmitted(125)
    #expect(p.recordsEmitted == 125)
    p.setRecordsEmitted(500)
    #expect(p.recordsEmitted == 500)
}

@MainActor
@Test
func httpProgressSteppingPhaseEqualityIsByAssociatedValue() {
    #expect(HTTPProgress.Phase.stepping(name: "products") == .stepping(name: "products"))
    #expect(HTTPProgress.Phase.stepping(name: "products") != .stepping(name: "categories"))
}

@MainActor
@Test
func httpProgressPaginatingPhaseEqualityIsByAssociatedValue() {
    #expect(HTTPProgress.Phase.paginating(name: "products", page: 1)
            == .paginating(name: "products", page: 1))
    #expect(HTTPProgress.Phase.paginating(name: "products", page: 1)
            != .paginating(name: "products", page: 2))
    #expect(HTTPProgress.Phase.paginating(name: "products", page: 1)
            != .paginating(name: "categories", page: 1))
}

@MainActor
@Test
func httpProgressIsTerminalIsFalseForInFlightPhases() {
    let p = HTTPProgress()
    #expect(p.isTerminal == false) // .starting
    let nonTerminal: [HTTPProgress.Phase] = [
        .priming,
        .stepping(name: "auth"),
        .stepping(name: "products"),
        .paginating(name: "products", page: 1),
    ]
    for phase in nonTerminal {
        p.setPhase(phase)
        #expect(p.isTerminal == false, "expected \(phase) to be non-terminal")
    }
}

@MainActor
@Test
func httpProgressIsTerminalIsTrueForDoneAndFailed() {
    // One fresh progress per phase: terminal phases are sticky, so chaining
    // .done → .failed → .failed on a single instance would be ambiguous.
    let done = HTTPProgress()
    done.setPhase(.done)
    #expect(done.isTerminal == true)

    let parseError = HTTPProgress()
    parseError.setPhase(.failed("parse-error"))
    #expect(parseError.isTerminal == true)

    let notFound = HTTPProgress()
    notFound.setPhase(.failed("404"))
    #expect(notFound.isTerminal == true)
}

@MainActor
@Test
func httpProgressTerminalGuardPreventsStepRegressAfterDone() {
    // Terminal phases are sticky: a late transition (e.g. a stale `stepping`
    // from a sibling task) can't rewrite the terminal state.
    let p = HTTPProgress()
    p.setPhase(.done)
    p.setPhase(.stepping(name: "products"))
    #expect(p.phase == .done)
}

@MainActor
@Test
func httpProgressTerminalGuardPreservesFailedPhase() {
    let p = HTTPProgress()
    p.setPhase(.failed("connection-reset"))
    p.setPhase(.paginating(name: "products", page: 4))
    #expect(p.phase == .failed("connection-reset"))
}

@MainActor
@Test
func httpProgressResetReturnsToInitialState() {
    let p = HTTPProgress()
    p.setPhase(.paginating(name: "products", page: 3))
    p.noteRequestSent(url: "https://example.com/x")
    p.noteRequestSent(url: "https://example.com/y")
    p.setRecordsEmitted(42)

    p.reset()

    #expect(p.phase == .starting)
    #expect(p.requestsSent == 0)
    #expect(p.recordsEmitted == 0)
    #expect(p.currentURL == nil)
}
