import Testing
import Foundation
@testable import Toolkit

@Test
func tokenizerRecognizesKeywords() {
    let tokens = ForageTokenizer.tokenize("recipe \"x\" { engine http }")
    let keywords = tokens.filter { $0.kind == .keyword }
    #expect(keywords.count == 3)   // recipe, engine, http
}

@Test
func tokenizerRecognizesStrings() {
    let src = #"recipe "hello world""#
    let tokens = ForageTokenizer.tokenize(src)
    let strings = tokens.filter { $0.kind == .string }
    #expect(strings.count == 1)
    let range = strings[0].range
    #expect((src as NSString).substring(with: range) == "\"hello world\"")
}

@Test
func tokenizerRecognizesLineComments() {
    let src = "// this is a comment\nrecipe \"x\""
    let tokens = ForageTokenizer.tokenize(src)
    let comments = tokens.filter { $0.kind == .comment }
    #expect(comments.count == 1)
    let range = comments[0].range
    #expect((src as NSString).substring(with: range) == "// this is a comment")
}

@Test
func tokenizerRecognizesBlockComments() {
    let src = "/* block */ recipe \"x\""
    let tokens = ForageTokenizer.tokenize(src)
    let comments = tokens.filter { $0.kind == .comment }
    #expect(comments.count == 1)
    let range = comments[0].range
    #expect((src as NSString).substring(with: range) == "/* block */")
}

@Test
func tokenizerRecognizesNumbers() {
    let tokens = ForageTokenizer.tokenize("page 42 size 200.0")
    let numbers = tokens.filter { $0.kind == .number }
    #expect(numbers.count == 2)
}

@Test
func tokenizerRecognizesDollarVariables() {
    let tokens = ForageTokenizer.tokenize("$input.storeId $menu")
    let dollars = tokens.filter { $0.kind == .dollar }
    #expect(dollars.count == 2)
}

@Test
func tokenizerRecognizesArrowOperator() {
    let src = "name ← $input.name"
    let tokens = ForageTokenizer.tokenize(src)
    let ops = tokens.filter { $0.kind == .op }
    #expect(ops.count == 1)
    let range = ops[0].range
    #expect((src as NSString).substring(with: range) == "←")
}

@Test
func tokenizerRecognizesCaseArrowOperator() {
    let src = "RECREATIONAL → \"Rec\""
    let tokens = ForageTokenizer.tokenize(src)
    let ops = tokens.filter { $0.kind == .op }
    #expect(ops.count == 1)
}

@Test
func tokenizerRecognizesTypeNames() {
    let tokens = ForageTokenizer.tokenize("type Product { name: String }")
    let typeNames = tokens.filter { $0.kind == .typeName }
    // Product, String
    #expect(typeNames.count == 2)
}

@Test
func tokenizerTolerantOfUnterminatedString() {
    // The editor needs to handle half-typed source without throwing.
    let tokens = ForageTokenizer.tokenize("recipe \"unterminated")
    let strings = tokens.filter { $0.kind == .string }
    #expect(strings.count == 1)
}

@Test
func tokenizerHandlesFullSweedRecipeSnippet() {
    // Pull the real reference recipe so we exercise everything the editor
    // will actually meet in the wild. The exact token counts here are
    // brittle on small edits, so we just sanity-check none-zero shapes.
    let src = """
        recipe "sweed" {
            engine http
            type Item { id: String }
            input storeId: String
            for $x in $list[*] {
                emit Item { id ← $x.id | toString }
            }
        }
        """
    let tokens = ForageTokenizer.tokenize(src)
    #expect(tokens.contains(where: { $0.kind == .keyword }))
    #expect(tokens.contains(where: { $0.kind == .typeName }))
    #expect(tokens.contains(where: { $0.kind == .dollar }))
    #expect(tokens.contains(where: { $0.kind == .op }))
    #expect(tokens.contains(where: { $0.kind == .string }))
}
