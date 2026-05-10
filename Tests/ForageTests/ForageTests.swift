import Testing
@testable import Forage

@Test
func versionIsDefined() {
    #expect(!Forage.version.isEmpty)
}
