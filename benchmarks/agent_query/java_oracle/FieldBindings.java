/* Independent development oracle using public JDK compiler APIs.
 * Parse and attribute source; never generate or execute project classes.
 */
import com.sun.source.tree.*;
import com.sun.source.util.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import javax.lang.model.element.*;
import javax.lang.model.type.TypeKind;
import javax.tools.*;

public final class FieldBindings {
    private static final int MAX_FILES = 512, MAX_RECORDS = 100000;
    private static final long MAX_FILE = 4194304, MAX_TOTAL = 67108864;
    private final Trees trees;
    private final Path root;
    private final Set<Path> inputs;
    private final Map<String, String> texts = new HashMap<>();
    private final Set<String> declarations = new HashSet<>();
    private final Set<String> occurrences = new HashSet<>();
    private int records;
    private int unboundExpressions;

    private FieldBindings(Trees trees, Path root, Set<Path> inputs) {
        this.trees = trees;
        this.root = root;
        this.inputs = inputs;
    }

    private static Map<String, Object> object(Object... entries) {
        Map<String, Object> map = new LinkedHashMap<>();
        for (int i = 0; i < entries.length; i += 2)
            map.put((String) entries[i], entries[i + 1]);
        return map;
    }

    private static String json(Object value) {
        if (value == null) return "null";
        if (value instanceof Number || value instanceof Boolean) return value.toString();
        if (value instanceof Map<?, ?> map) {
            List<String> parts = new ArrayList<>();
            for (var entry : map.entrySet()) parts.add(json(entry.getKey()) + ":" + json(entry.getValue()));
            return "{" + String.join(",", parts) + "}";
        }
        String text = value.toString();
        StringBuilder out = new StringBuilder("\"");
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c == '\\' || c == '"') out.append('\\').append(c);
            else if (c < 32) out.append(String.format(Locale.ROOT, "\\u%04x", (int) c));
            else out.append(c);
        }
        return out.append('"').toString();
    }

    private void emit(Map<String, Object> record) {
        if (++records > MAX_RECORDS) throw new IllegalStateException("record limit");
        System.out.println(json(record));
    }

    private String file(CompilationUnitTree unit) {
        try {
            Path path = Path.of(unit.getSourceFile().toUri()).toRealPath();
            if (!inputs.contains(path)) throw new IllegalStateException("unregistered source: " + path);
            return root.relativize(path).toString().replace(File.separatorChar, '/');
        } catch (IOException ex) { throw new UncheckedIOException(ex); }
    }

    private String text(CompilationUnitTree unit) {
        return texts.computeIfAbsent(file(unit), key -> {
            try { return unit.getSourceFile().getCharContent(false).toString(); }
            catch (IOException ex) { throw new UncheckedIOException(ex); }
        });
    }

    private static boolean field(Element element) {
        return element != null && (element.getKind() == ElementKind.FIELD
            || element.getKind() == ElementKind.ENUM_CONSTANT);
    }

    private static String qualified(Element element) {
        if (element instanceof TypeElement type) return type.getQualifiedName().toString();
        if (element == null) return "";
        return qualified(element.getEnclosingElement()) + "::" + element.getSimpleName();
    }

    private Map<String, Object> declaration(TreePath path) {
        if (path == null) return null;
        CompilationUnitTree unit = path.getCompilationUnit();
        long start = trees.getSourcePositions().getStartPosition(unit, path.getLeaf());
        long end = trees.getSourcePositions().getEndPosition(unit, path.getLeaf());
        Element element = trees.getElement(path);
        if (element == null || start < 0 || end <= start || end > text(unit).length()) return null;
        String name = element.getSimpleName().toString();
        String kind = element.getKind().toString().toLowerCase(Locale.ROOT);
        String file = file(unit);
        String id = file + ":" + start + ":" + end + ":" + kind + ":" + name;
        return object("id", id, "kind", kind, "file", file, "name", name,
            "qualified", qualified(element), "startUtf16", start, "endUtf16", end,
            "startLine", unit.getLineMap().getLineNumber(start),
            "endLine", unit.getLineMap().getLineNumber(end - 1));
    }

    private Map<String, Object> owner(TreePath path) {
        for (TreePath parent = path.getParentPath(); parent != null; parent = parent.getParentPath()) {
            Tree leaf = parent.getLeaf();
            if (leaf instanceof MethodTree || leaf instanceof ClassTree
                || (leaf instanceof VariableTree && field(trees.getElement(parent))))
                return declaration(parent);
        }
        return null;
    }

    private void declare(TreePath path) {
        Map<String, Object> value = declaration(path);
        if (value != null && declarations.add((String) value.get("id")))
            emit(object("type", "declaration", "declaration", value));
    }

    private void access(TreePath path, String spelling) {
        if (spelling.equals("this") || spelling.equals("super")) return;
        Element element = trees.getElement(path);
        if (element == null) { unboundExpressions++; return; }
        if (!field(element)) return;
        CompilationUnitTree unit = path.getCompilationUnit();
        long start = trees.getSourcePositions().getStartPosition(unit, path.getLeaf());
        long end = trees.getSourcePositions().getEndPosition(unit, path.getLeaf());
        // Attribute-generated trees are not source occurrences.
        if (start < 0 || end <= start || end > text(unit).length()) return;
        long tokenStart = path.getLeaf() instanceof IdentifierTree ? start : end - spelling.length();
        boolean anchored = tokenStart >= start && text(unit).substring((int) tokenStart, (int) end).equals(spelling);
        String key = file(unit) + ":" + start + ":" + end;
        if (!occurrences.add(key)) throw new IllegalStateException("duplicate occurrence: " + key);
        Map<String, Object> target = declaration(trees.getPath(element));
        String origin = target != null ? "source" : "external";
        if (target == null && path.getLeaf() instanceof MemberSelectTree select) {
            if (spelling.equals("class")) origin = "class-literal";
            else if (spelling.equals("length") && trees.getTypeMirror(
                    new TreePath(path, select.getExpression())).getKind() == TypeKind.ARRAY)
                origin = "array-length";
        }
        emit(object("type", "fieldReference", "file", file(unit), "name", spelling,
            "expressionStartUtf16", start, "expressionEndUtf16", end,
            "tokenStartUtf16", anchored ? tokenStart : null, "tokenEndUtf16", anchored ? end : null,
            "anchorStatus", anchored ? "exact" : "unsupported-raw-spelling",
            "line", unit.getLineMap().getLineNumber(end - 1),
            "targetKind", element.getKind().toString().toLowerCase(Locale.ROOT),
            "targetQualified", qualified(element), "targetOrigin", origin, "target", target, "owner", owner(path)));
    }

    private final class Scanner extends TreePathScanner<Void, Void> {
        private int depth;
        @Override public Void scan(Tree tree, Void unused) {
            if (++depth > 512) throw new IllegalStateException("AST depth limit");
            try { return super.scan(tree, unused); }
            finally { depth--; }
        }
        @Override public Void visitImport(ImportTree tree, Void unused) { return null; }
        @Override public Void visitClass(ClassTree tree, Void unused) {
            declare(getCurrentPath()); return super.visitClass(tree, unused);
        }
        @Override public Void visitMethod(MethodTree tree, Void unused) {
            declare(getCurrentPath()); return super.visitMethod(tree, unused);
        }
        @Override public Void visitVariable(VariableTree tree, Void unused) {
            if (field(trees.getElement(getCurrentPath()))) declare(getCurrentPath());
            return super.visitVariable(tree, unused);
        }
        @Override public Void visitIdentifier(IdentifierTree tree, Void unused) {
            access(getCurrentPath(), tree.getName().toString()); return super.visitIdentifier(tree, unused);
        }
        @Override public Void visitMemberSelect(MemberSelectTree tree, Void unused) {
            access(getCurrentPath(), tree.getIdentifier().toString()); return super.visitMemberSelect(tree, unused);
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 4) throw new IllegalArgumentException("root release classpath file-list");
        System.setOut(new PrintStream(System.out, true, StandardCharsets.UTF_8));
        Path root = Path.of(args[0]).toRealPath();
        List<String> names = Files.readAllLines(Path.of(args[3]), StandardCharsets.UTF_8);
        if (names.isEmpty() || names.size() > MAX_FILES) throw new IllegalArgumentException("file count");
        SortedSet<Path> paths = new TreeSet<>();
        long bytes = 0;
        for (String name : names) {
            Path path = root.resolve(name).toRealPath();
            long size = Files.size(path);
            if (!path.startsWith(root) || !name.endsWith(".java") || size > MAX_FILE || !paths.add(path))
                throw new IllegalArgumentException("invalid source " + name);
            bytes += size;
        }
        if (bytes > MAX_TOTAL) throw new IllegalArgumentException("source byte limit");
        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) throw new IllegalStateException("JDK compiler required");
        DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
        try (StandardJavaFileManager manager = compiler.getStandardFileManager(diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
            JavacTask task = (JavacTask) compiler.getTask(new PrintWriter(System.err), manager, diagnostics,
                List.of("--release", args[1], "-encoding", "UTF-8", "-proc:none", "-implicit:none",
                    "-sourcepath", "", "-classpath", args[2], "-Xmaxerrs", "100", "-Xmaxwarns", "100"),
                null, manager.getJavaFileObjectsFromPaths(paths));
            List<CompilationUnitTree> units = new ArrayList<>();
            task.parse().forEach(units::add);
            task.analyze();
            boolean errors = false;
            for (Diagnostic<? extends JavaFileObject> diagnostic : diagnostics.getDiagnostics()) {
                errors |= diagnostic.getKind() == Diagnostic.Kind.ERROR;
                System.err.println(diagnostic.toString());
            }
            if (errors) throw new IllegalStateException("compiler errors; no scored oracle");
            FieldBindings oracle = new FieldBindings(Trees.instance(task), root, paths);
            oracle.emit(object("type", "header", "schema", "compass.javac-field-bindings/1",
                "files", paths.size(), "sourceBytes", bytes, "release", args[1],
                "positionEncoding", "UTF-16 code units", "compilerRuntime", Runtime.version().toString()));
            for (CompilationUnitTree unit : units) oracle.new Scanner().scan(unit, null);
            oracle.emit(object("type", "complete", "declarations", oracle.declarations.size(),
                "fieldReferences", oracle.occurrences.size(), "unboundExpressions", oracle.unboundExpressions));
        }
    }
}
