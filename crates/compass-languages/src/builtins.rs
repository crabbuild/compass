const JAVASCRIPT_BUILTIN_GLOBALS: &[&str] = &[
    "String",
    "Number",
    "Boolean",
    "Object",
    "Array",
    "Symbol",
    "BigInt",
    "Date",
    "RegExp",
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
    "Promise",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "JSON",
    "Math",
    "Reflect",
    "Proxy",
    "Intl",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "encodeURI",
    "decodeURI",
    "URL",
    "URLSearchParams",
    "FormData",
    "Blob",
    "File",
    "Headers",
    "Request",
    "Response",
    "AbortController",
    "AbortSignal",
    "TextEncoder",
    "TextDecoder",
    "console",
];

const PYTHON_BUILTIN_GLOBALS: &[&str] = &[
    "str",
    "int",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "bytes",
    "len",
    "range",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sum",
    "min",
    "max",
    "print",
    "open",
    "isinstance",
    "type",
    "super",
    "sorted",
    "reversed",
    "any",
    "all",
    "abs",
    "round",
    "next",
    "iter",
    "hash",
    "id",
    "repr",
    "callable",
    "getattr",
    "setattr",
    "hasattr",
    "delattr",
    "vars",
    "dir",
];

const GO_BUILTIN_GLOBALS: &[&str] = &[
    "append", "cap", "clear", "close", "complex", "copy", "delete", "imag", "len", "make", "max",
    "min", "new", "panic", "print", "println", "real", "recover",
];

const SWIFT_BUILTIN_GLOBALS: &[&str] = &[
    "String",
    "Int",
    "Int8",
    "Int16",
    "Int32",
    "Int64",
    "UInt",
    "UInt8",
    "UInt16",
    "UInt32",
    "UInt64",
    "Double",
    "Float",
    "Bool",
    "Character",
    "Sendable",
    "Codable",
    "Decodable",
    "Encodable",
    "Equatable",
    "Hashable",
    "Identifiable",
    "Comparable",
    "CaseIterable",
    "RawRepresentable",
    "CustomStringConvertible",
    "CustomDebugStringConvertible",
    "AnyObject",
    "LocalizedError",
    "Data",
    "Date",
    "UUID",
    "Decimal",
    "Calendar",
    "Locale",
    "TimeZone",
    "Bundle",
    "URL",
    "IndexPath",
    "IndexSet",
    "NotificationCenter",
    "UserDefaults",
    "FileManager",
    "URLSession",
    "URLRequest",
    "URLComponents",
    "JSONDecoder",
    "JSONEncoder",
    "DateFormatter",
    "NumberFormatter",
    "ISO8601DateFormatter",
    "NSObject",
    "NSString",
    "NSError",
    "NSLock",
    "NSAttributedString",
    "DispatchQueue",
    "DispatchGroup",
    "OperationQueue",
    "RunLoop",
    "Error",
    "View",
    "Color",
    "Font",
    "filter",
    "print",
];

/// Return whether `name` is an unresolved global supplied by `language`.
///
/// This deliberately is not a union table: the same spelling can be a normal
/// user declaration in another language, and callers must attempt proven local
/// or imported resolution before consulting it.
#[must_use]
pub fn is_language_builtin_global(language: &str, name: &str) -> bool {
    match language {
        "javascript" | "typescript" | "tsx" => JAVASCRIPT_BUILTIN_GLOBALS.contains(&name),
        "python" => PYTHON_BUILTIN_GLOBALS.contains(&name),
        "go" => GO_BUILTIN_GLOBALS.contains(&name),
        "swift" => SWIFT_BUILTIN_GLOBALS.contains(&name),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_language_builtin_global;

    #[test]
    fn builtin_globals_are_language_scoped_and_case_sensitive() {
        assert!(is_language_builtin_global("typescript", "Promise"));
        assert!(is_language_builtin_global("python", "len"));
        assert!(is_language_builtin_global("go", "make"));
        assert!(is_language_builtin_global("swift", "Data"));
        assert!(is_language_builtin_global("swift", "Sendable"));

        assert!(!is_language_builtin_global("rust", "Promise"));
        assert!(!is_language_builtin_global("python", "Promise"));
        assert!(!is_language_builtin_global("swift", "data"));
        assert!(!is_language_builtin_global("swift", "ProjectService"));
    }
}
