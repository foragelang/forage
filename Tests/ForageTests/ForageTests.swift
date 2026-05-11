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

@Test
func parseJaneWeightHandlesUnitSuffixedNumerics() throws {
    // The named-weight branches are the common Jane cases; the numeric
    // fallback used to be half-broken — `"1g"` and `"3.5g"` returned 0.0
    // because `Double("1g")` is nil. The fallback now reuses parseSize so a
    // unit-suffixed string resolves to the right scalar.
    let t = TransformImpls()
    #expect(try t.apply("parseJaneWeight", value: .string("1g"), args: []) == .double(1.0))
    #expect(try t.apply("parseJaneWeight", value: .string("3.5g"), args: []) == .double(3.5))
    #expect(try t.apply("parseJaneWeight", value: .string("100mg"), args: []) == .double(100.0))
    #expect(try t.apply("parseJaneWeight", value: .string("1oz"), args: []) == .double(1.0))

    // Named branches still resolve as before.
    #expect(try t.apply("parseJaneWeight", value: .string("half ounce"), args: []) == .double(14.0))
    #expect(try t.apply("parseJaneWeight", value: .string("each"), args: []) == .null)
}
