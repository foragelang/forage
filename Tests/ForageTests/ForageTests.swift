import Testing
@testable import Forage

@Test
func versionIsDefined() {
    #expect(!Forage.version.isEmpty)
}

@Test
func janeWeightKeyReplacesSpacesWithUnderscores() throws {
    let t = TransformImpls()
    #expect(try t.apply("janeWeightKey", value: .string("eighth ounce"), args: []) == .string("eighth_ounce"))
    #expect(try t.apply("janeWeightKey", value: .string("half ounce"), args: []) == .string("half_ounce"))
    #expect(try t.apply("janeWeightKey", value: .string("ounce"), args: []) == .string("ounce"))
    #expect(try t.apply("janeWeightKey", value: .null, args: []) == .null)
}
