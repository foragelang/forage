#if canImport(WebKit)
import Testing
@testable import Forage

@MainActor
@Test
func browserProgressStartsAtStarting() {
    let p = BrowserProgress()
    #expect(p.phase == .starting)
    #expect(p.capturesObserved == 0)
    #expect(p.recordsEmitted == 0)
    #expect(p.currentURL == nil)
    #expect(p.lastObservedURL == nil)
}

@MainActor
@Test
func browserProgressAdvancesThroughExpectedPhases() {
    let p = BrowserProgress()
    let transitions: [BrowserProgress.Phase] = [
        .loading,
        .ageGate,
        .dismissing,
        .warmupClicks,
        .paginating(iteration: 1, maxIterations: 30),
        .paginating(iteration: 2, maxIterations: 30),
        .settling,
        .done,
    ]
    for phase in transitions {
        p.setPhase(phase)
        #expect(p.phase == phase)
    }
}

@MainActor
@Test
func browserProgressFailedCarriesMessage() {
    let p = BrowserProgress()
    p.setPhase(.failed("nav-fail"))
    #expect(p.phase == .failed("nav-fail"))
    // Distinct messages compare unequal — the associated value matters.
    #expect(p.phase != .failed("hard-timeout"))
}

@MainActor
@Test
func browserProgressNoteCaptureBumpsCounterAndURL() {
    let p = BrowserProgress()
    p.noteCapture(responseURL: "https://example.com/api/menu/1")
    #expect(p.capturesObserved == 1)
    #expect(p.lastObservedURL == "https://example.com/api/menu/1")
    p.noteCapture(responseURL: "https://example.com/api/menu/2")
    #expect(p.capturesObserved == 2)
    #expect(p.lastObservedURL == "https://example.com/api/menu/2")
}

@MainActor
@Test
func browserProgressRecordsEmittedAndCurrentURLAreSettable() {
    let p = BrowserProgress()
    p.setRecordsEmitted(0)
    #expect(p.recordsEmitted == 0)
    p.setRecordsEmitted(247)
    #expect(p.recordsEmitted == 247)
    p.setRecordsEmitted(500)
    #expect(p.recordsEmitted == 500)

    p.setCurrentURL("https://example.com/menu")
    #expect(p.currentURL == "https://example.com/menu")
    p.setCurrentURL(nil)
    #expect(p.currentURL == nil)
}

@MainActor
@Test
func browserProgressPaginatingPhaseEqualityIsByAssociatedValue() {
    #expect(BrowserProgress.Phase.paginating(iteration: 1, maxIterations: 30)
            == .paginating(iteration: 1, maxIterations: 30))
    #expect(BrowserProgress.Phase.paginating(iteration: 1, maxIterations: 30)
            != .paginating(iteration: 2, maxIterations: 30))
    #expect(BrowserProgress.Phase.paginating(iteration: 1, maxIterations: 30)
            != .paginating(iteration: 1, maxIterations: 60))
}
#endif
