import Foundation
import SwiftParser
import SwiftSyntax

private struct Relation: Codable {
    let relation: String
    let capability: String
    let ownerQualifiedName: String
    let targetSpelling: String
    let qualifier: String?
    let startByte: Int
    let endByte: Int
    let startLine: Int
}

private struct FileRecord: Codable {
    let path: String
    let status: String
    let bytes: Int
    let relations: [Relation]
}

private struct ProviderOutput: Codable {
    let language: String
    let provider: String
    let toolchain: String
    let implementation: String
    let parserAvailable: Bool
    let files: [FileRecord]
}

private struct Arguments {
    let root: URL
    let files: URL
    let output: URL
}

private func arguments() throws -> Arguments {
    var values: [String: String] = [:]
    var index = 1
    while index < CommandLine.arguments.count {
        let option = CommandLine.arguments[index]
        guard option.hasPrefix("--"), index + 1 < CommandLine.arguments.count else {
            throw ProviderError.message("expected --root, --files, and --output arguments")
        }
        values[option] = CommandLine.arguments[index + 1]
        index += 2
    }
    guard let root = values["--root"], let files = values["--files"], let output = values["--output"] else {
        throw ProviderError.message("expected --root, --files, and --output arguments")
    }
    return Arguments(
        root: URL(fileURLWithPath: root).standardizedFileURL,
        files: URL(fileURLWithPath: files).standardizedFileURL,
        output: URL(fileURLWithPath: output).standardizedFileURL
    )
}

private enum ProviderError: Error, CustomStringConvertible {
    case message(String)

    var description: String {
        switch self {
        case let .message(value): return value
        }
    }
}

private struct Span: Hashable {
    let start: Int
    let end: Int
}

private struct Declaration {
    let name: String
    let qualified: String
    let kind: String
    let span: Span
}

private struct Symbols {
    var types: Set<String> = []
    var callables: Set<String> = []
}

private final class SourceOracle {
    let path: String
    let bytes: [UInt8]
    let tree: SourceFileSyntax
    let symbols: Symbols
    let lineStarts: [Int]
    var relations: [Relation] = []
    var nameSpans: Set<Span> = []
    var baseSpans: Set<Span> = []
    var emitted: Set<String> = []

    init(path: String, bytes: [UInt8], tree: SourceFileSyntax, symbols: Symbols) {
        self.path = path
        self.bytes = bytes
        self.tree = tree
        self.symbols = symbols
        var starts = [0]
        for (index, byte) in bytes.enumerated() where byte == 10 {
            starts.append(index + 1)
        }
        lineStarts = starts
    }

    func run() -> [Relation] {
        walk(Syntax(tree), owner: "")
        return relations.sorted {
            ($0.startByte, $0.endByte, $0.relation, $0.ownerQualifiedName, $0.targetSpelling)
                < ($1.startByte, $1.endByte, $1.relation, $1.ownerQualifiedName, $1.targetSpelling)
        }
    }

    private func walk(_ node: Syntax, owner: String) {
        let declaration = declaration(for: node, owner: owner)
        let nextOwner = declaration?.qualified ?? owner
        if let declaration {
            // Accessor bodies, type aliases, and function-type nodes are
            // represented as metadata on their owning Swift declaration by
            // the universal producer, rather than as independently owned
            // graph symbols. Keep them out of the ownership denominator while
            // retaining the surrounding declaration evidence.
            if !["accessor", "type_alias", "function_type", "deinit"].contains(declaration.kind) {
                emit(
                    relation: "contains",
                    capability: "ownership",
                    owner: owner.isEmpty ? path : owner,
                    target: declaration.name,
                    qualifier: nil,
                    span: declaration.span
                )
            }
        }

        if let importDecl = node.as(ImportDeclSyntax.self), let span = span(of: importDecl) {
            let target = importDecl.path.map(\.name.text).joined(separator: ".")
            if !target.isEmpty {
                emit(
                    relation: "imports",
                    capability: "imports",
                    owner: nextOwner.isEmpty ? path : nextOwner,
                    target: target,
                    qualifier: nil,
                    span: span
                )
            }
        }

        if let inherited = node.as(InheritedTypeSyntax.self), let span = span(of: inherited.type) {
            baseSpans.insert(span)
            let raw = text(span)
            if !raw.isEmpty {
                emit(
                    relation: inheritedOwnerIsProtocol(node) ? "implements" : "extends",
                    capability: "base_types",
                    owner: nextOwner.isEmpty ? path : nextOwner,
                    target: terminal(raw),
                    qualifier: qualifier(raw),
                    span: span
                )
            }
        }

        if let call = node.as(FunctionCallExprSyntax.self), let called = span(of: call.calledExpression) {
            let raw = text(called)
            let target = terminal(raw)
            let prefix = qualifier(raw)
            let qualifiedCall = prefix.map { value in
                value.first?.isUppercase == true && !value.hasPrefix("self")
            } ?? false
            if !target.isEmpty && !isControlKeyword(target) &&
                (prefix == nil || (qualifiedCall && symbols.callables.contains(target))) {
                let constructor = symbols.types.contains(target) && target.first?.isUppercase == true
                emit(
                    relation: constructor ? "instantiates" : "calls",
                    capability: constructor ? "construction" : "calls",
                    owner: nextOwner.isEmpty ? path : nextOwner,
                    target: target,
                    qualifier: qualifier(raw),
                    span: called
                )
            }
        }

        if let member = node.as(MemberAccessExprSyntax.self), let span = span(of: member) {
            let raw = text(span)
            let target = terminal(raw)
            let prefix = raw.split(separator: ".", omittingEmptySubsequences: true).dropLast().joined(separator: ".")
            if !target.isEmpty && !prefix.isEmpty {
                emit(
                    relation: "references",
                    capability: "type_references",
                    owner: nextOwner.isEmpty ? path : nextOwner,
                    target: target,
                    qualifier: prefix,
                    span: span
                )
            }
        }

        if let identifier = node.as(IdentifierTypeSyntax.self), let span = span(of: identifier),
           !nameSpans.contains(span), !baseSpans.contains(span) {
            let raw = text(span)
            let target = terminal(raw)
            if !target.isEmpty && symbols.types.contains(target) {
                emit(
                    relation: "references",
                    capability: "type_references",
                    owner: nextOwner.isEmpty ? path : nextOwner,
                    target: target,
                    qualifier: qualifier(raw),
                    span: span
                )
            }
        }

        for child in node.children(viewMode: .sourceAccurate) {
            walk(child, owner: nextOwner)
        }
    }

    private func declaration(for node: Syntax, owner: String) -> Declaration? {
        let result: (String, String, String, TokenSyntax?)?
        if let value = node.as(ClassDeclSyntax.self) {
            result = ("class", value.name.text, value.name.text, value.name)
        } else if let value = node.as(StructDeclSyntax.self) {
            result = ("struct", value.name.text, value.name.text, value.name)
        } else if let value = node.as(ActorDeclSyntax.self) {
            result = ("class", value.name.text, value.name.text, value.name)
        } else if let value = node.as(EnumDeclSyntax.self) {
            result = ("enum", value.name.text, value.name.text, value.name)
        } else if let value = node.as(ProtocolDeclSyntax.self) {
            result = ("protocol", value.name.text, value.name.text, value.name)
        } else if let value = node.as(ExtensionDeclSyntax.self) {
            let name = terminal(value.extendedType.description)
            result = ("class", name, name, nil)
        } else if let value = node.as(FunctionDeclSyntax.self) {
            result = ("function", value.name.text, value.name.text, value.name)
        } else if node.as(InitializerDeclSyntax.self) != nil {
            result = ("constructor", "init", "init", nil)
        } else if node.as(DeinitializerDeclSyntax.self) != nil {
            result = ("method", "deinit", "deinit", nil)
        } else if node.as(SubscriptDeclSyntax.self) != nil {
            result = ("method", "subscript", "subscript", nil)
        } else if let value = node.as(TypeAliasDeclSyntax.self) {
            result = ("type_alias", value.name.text, value.name.text, value.name)
        } else if let value = node.as(EnumCaseDeclSyntax.self) {
            let element = value.elements.first
            result = ("enum", element?.name.text ?? "case", element?.name.text ?? "case", element?.name)
        } else if let value = node.as(VariableDeclSyntax.self) {
            let binding = value.bindings.first
            let pattern = binding?.pattern.as(IdentifierPatternSyntax.self)
            let name = pattern?.identifier.text ?? "variable"
            result = ("field", name, name, pattern?.identifier)
        } else if node.as(ClosureExprSyntax.self) != nil {
            result = ("closure", "closure", "closure", nil)
        } else if node.as(FunctionTypeSyntax.self) != nil {
            result = ("function_type", "function", "function", nil)
        } else if let value = node.as(AccessorDeclSyntax.self) {
            let name = value.accessorSpecifier.text
            result = ("accessor", name, name, value.accessorSpecifier)
        } else {
            result = nil
        }
        guard let result, !result.1.isEmpty, let nodeSpan = span(of: node) else { return nil }
        if let token = result.3, let tokenSpan = span(of: token) {
            nameSpans.insert(tokenSpan)
        }
        let qualified = owner.isEmpty ? result.1 : owner + "." + result.1
        return Declaration(name: result.1, qualified: qualified, kind: result.0, span: nodeSpan)
    }

    private func inheritedOwnerIsProtocol(_ node: Syntax) -> Bool {
        var current: Syntax? = node
        while let value = current {
            if value.as(ProtocolDeclSyntax.self) != nil { return true }
            if value.as(ClassDeclSyntax.self) != nil || value.as(StructDeclSyntax.self) != nil || value.as(EnumDeclSyntax.self) != nil || value.as(ActorDeclSyntax.self) != nil || value.as(ExtensionDeclSyntax.self) != nil {
                return false
            }
            current = value.parent
        }
        return false
    }

    private func emit(relation: String, capability: String, owner: String, target: String, qualifier: String?, span: Span) {
        guard span.start >= 0, span.end > span.start, span.end <= bytes.count else { return }
        let key = [relation, capability, owner, target, qualifier ?? "", String(span.start), String(span.end)].joined(separator: "\u{1f}")
        guard emitted.insert(key).inserted else { return }
        relations.append(Relation(
            relation: relation,
            capability: capability,
            ownerQualifiedName: owner,
            targetSpelling: target,
            qualifier: qualifier,
            startByte: span.start,
            endByte: span.end,
            startLine: line(for: span.start)
        ))
    }

    private func span<T: SyntaxProtocol>(of node: T) -> Span? {
        let syntax = Syntax(node)
        let start = syntax.positionAfterSkippingLeadingTrivia.utf8Offset
        let end = syntax.endPositionBeforeTrailingTrivia.utf8Offset
        return end > start ? Span(start: start, end: end) : nil
    }

    private func text(_ span: Span) -> String {
        String(decoding: bytes[span.start..<span.end], as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func line(for byte: Int) -> Int {
        var low = 0
        var high = lineStarts.count
        while low < high {
            let mid = (low + high) / 2
            if lineStarts[mid] <= byte { low = mid + 1 } else { high = mid }
        }
        return max(1, low)
    }
}

private func collectSymbols(_ node: Syntax, into symbols: inout Symbols) {
    if let value = node.as(ClassDeclSyntax.self) {
        symbols.types.insert(value.name.text)
    } else if let value = node.as(StructDeclSyntax.self) {
        symbols.types.insert(value.name.text)
    } else if let value = node.as(ActorDeclSyntax.self) {
        symbols.types.insert(value.name.text)
    } else if let value = node.as(EnumDeclSyntax.self) {
        symbols.types.insert(value.name.text)
    } else if let value = node.as(ProtocolDeclSyntax.self) {
        symbols.types.insert(value.name.text)
    } else if let value = node.as(TypeAliasDeclSyntax.self) {
        symbols.types.insert(value.name.text)
    } else if let value = node.as(FunctionDeclSyntax.self) {
        symbols.callables.insert(value.name.text)
    } else if node.as(InitializerDeclSyntax.self) != nil {
        symbols.callables.insert("init")
    } else if node.as(DeinitializerDeclSyntax.self) != nil {
        symbols.callables.insert("deinit")
    } else if node.as(SubscriptDeclSyntax.self) != nil {
        symbols.callables.insert("subscript")
    } else if let value = node.as(AccessorDeclSyntax.self) {
        symbols.callables.insert(value.accessorSpecifier.text)
    }
    for child in node.children(viewMode: .sourceAccurate) {
        collectSymbols(child, into: &symbols)
    }
}

private func terminal(_ value: String) -> String {
    let cleaned = value.trimmingCharacters(in: .whitespacesAndNewlines)
    let pieces = cleaned.split(separator: ".", omittingEmptySubsequences: true)
    return pieces.last.map(String.init) ?? cleaned
}

private func qualifier(_ value: String) -> String? {
    let cleaned = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard let separator = cleaned.lastIndex(of: ".") else { return nil }
    let prefix = cleaned[..<separator].trimmingCharacters(in: .whitespacesAndNewlines)
    return prefix.isEmpty ? nil : String(prefix)
}

private func isControlKeyword(_ value: String) -> Bool {
    ["if", "for", "while", "switch", "catch", "guard", "return", "throw", "defer", "repeat"].contains(value)
}

private func sourceFiles(_ url: URL) throws -> [String] {
    let text = try String(contentsOf: url, encoding: .utf8)
    let values = text.split(whereSeparator: \ .isNewline).map(String.init)
    guard values == values.sorted(), Set(values).count == values.count else {
        throw ProviderError.message("file inventory is not sorted and unique")
    }
    return values
}

private func process(_ arguments: Arguments) throws -> ProviderOutput {
    let paths = try sourceFiles(arguments.files)
    var records: [FileRecord] = []
    var parsed: [(String, Data, SourceFileSyntax)] = []
    var symbols = Symbols()
    for path in paths {
        let fileURL = arguments.root.appendingPathComponent(path)
        let data = try Data(contentsOf: fileURL)
        guard let source = String(data: data, encoding: .utf8) else {
            records.append(FileRecord(path: path, status: "partial", bytes: data.count, relations: []))
            continue
        }
        let tree = Parser.parse(source: source)
        collectSymbols(Syntax(tree), into: &symbols)
        parsed.append((path, data, tree))
    }
    for (path, data, tree) in parsed {
        let oracle = SourceOracle(path: path, bytes: Array(data), tree: tree, symbols: symbols)
        records.append(FileRecord(
            path: path,
            status: tree.hasError ? "partial" : "ok",
            bytes: data.count,
            relations: oracle.run()
        ))
    }
    records.sort { $0.path < $1.path }
    return ProviderOutput(
        language: "swift",
        provider: "swift-syntax-source-oracle",
        toolchain: "swift 6.3.3; SwiftSyntax 603.0.0 (qualification contract)",
        implementation: "SwiftSyntax 603.0.0 AST source parser",
        parserAvailable: true,
        files: records
    )
}

do {
    let output = try process(arguments())
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    let data = try encoder.encode(output)
    let arguments = try arguments()
    try data.write(to: arguments.output, options: .atomic)
} catch {
    FileHandle.standardError.write(Data("swift parser provider failed: \(error)\n".utf8))
    exit(1)
}
