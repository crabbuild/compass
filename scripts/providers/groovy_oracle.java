// Qualification-only Apache Groovy CompilationUnit source oracle.
//
// The Python boundary supplies an explicit, sorted file list.  This helper
// parses only those files through Groovy's conversion-phase AST; it never
// runs a script, resolves a project build, or loads repository classes.

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

import org.codehaus.groovy.ast.ASTNode;
import org.codehaus.groovy.ast.ClassCodeVisitorSupport;
import org.codehaus.groovy.ast.ClassNode;
import org.codehaus.groovy.ast.ClassHelper;
import org.codehaus.groovy.ast.ConstructorNode;
import org.codehaus.groovy.ast.FieldNode;
import org.codehaus.groovy.ast.ImportNode;
import org.codehaus.groovy.ast.MethodNode;
import org.codehaus.groovy.ast.ModuleNode;
import org.codehaus.groovy.ast.PropertyNode;
import org.codehaus.groovy.ast.expr.ClassExpression;
import org.codehaus.groovy.ast.expr.ConstructorCallExpression;
import org.codehaus.groovy.ast.expr.MethodCallExpression;
import org.codehaus.groovy.ast.expr.PropertyExpression;
import org.codehaus.groovy.ast.expr.VariableExpression;
import org.codehaus.groovy.control.CompilationUnit;
import org.codehaus.groovy.control.CompilerConfiguration;
import org.codehaus.groovy.control.Phases;
import org.codehaus.groovy.control.SourceUnit;
import org.codehaus.groovy.ast.expr.Expression;

public final class groovy_oracle {
    private groovy_oracle() {}

    private record Relation(
            String relation,
            String capability,
            String owner,
            String target,
            String qualifier,
            int start,
            int end,
            int line) {}

    private record Span(int start, int end, int line) {}

    private static final class SourceText {
        private final String source;
        private final int[] lineStarts;
        private final int[] byteOffsets;

        SourceText(String source) {
            this.source = source;
            List<Integer> lines = new ArrayList<>();
            lines.add(0);
            for (int index = 0; index < source.length(); index++) {
                if (source.charAt(index) == '\n') {
                    lines.add(index + 1);
                }
            }
            lineStarts = lines.stream().mapToInt(Integer::intValue).toArray();
            byteOffsets = new int[source.length() + 1];
            int bytes = 0;
            int index = 0;
            while (index < source.length()) {
                int codePoint = source.codePointAt(index);
                int units = Character.charCount(codePoint);
                bytes += new String(Character.toChars(codePoint)).getBytes(StandardCharsets.UTF_8).length;
                for (int unit = 0; unit < units; unit++) {
                    byteOffsets[index + unit + 1] = bytes;
                }
                index += units;
            }
        }

        Span span(ASTNode node) {
            if (node == null || node.getLineNumber() < 1 || node.getColumnNumber() < 1
                    || node.getLastLineNumber() < 1 || node.getLastColumnNumber() < 1) {
                return null;
            }
            int startLine = node.getLineNumber() - 1;
            int endLine = node.getLastLineNumber() - 1;
            if (startLine >= lineStarts.length || endLine >= lineStarts.length) {
                return null;
            }
            int start = lineStarts[startLine] + node.getColumnNumber() - 1;
            int end = lineStarts[endLine] + node.getLastColumnNumber() - 1;
            if (start < 0 || end <= start || end > source.length()) {
                return null;
            }
            return new Span(byteOffsets[start], byteOffsets[end], startLine + 1);
        }

        String text(Span span) {
            if (span == null) return "";
            // Relation spans are byte-based; use the UTF-8 boundary table to
            // locate the corresponding UTF-16 indices only when needed.
            return "";
        }

        String identifierNear(ASTNode node, String fallback) {
            Span span = span(node);
            if (span == null || fallback == null || fallback.isEmpty()) return fallback;
            int startLine = node.getLineNumber() - 1;
            int endLine = Math.max(startLine, node.getLastLineNumber() - 1);
            int start = lineStarts[startLine] + node.getColumnNumber() - 1;
            int end = Math.min(source.length(), lineStarts[endLine] + node.getLastColumnNumber() - 1);
            int found = source.indexOf(fallback, Math.max(0, start));
            if (found >= 0 && found < end) return fallback;
            return fallback;
        }
    }

    private static final class Emitter extends ClassCodeVisitorSupport {
        private final String path;
        private final String source;
        private final SourceText text;
        private final List<Relation> relations = new ArrayList<>();
        private final List<String> owners = new ArrayList<>();
        private final Set<String> emitted = new HashSet<>();
        private SourceUnit sourceUnit;

        Emitter(String path, String source, SourceText text) {
            this.path = path;
            this.source = source;
            this.text = text;
        }

        List<Relation> relations() {
            relations.sort(Comparator.comparingInt(Relation::start)
                    .thenComparingInt(Relation::end)
                    .thenComparing(Relation::relation)
                    .thenComparing(Relation::owner)
                    .thenComparing(Relation::target));
            return relations;
        }

        void setSourceUnit(SourceUnit sourceUnit) {
            this.sourceUnit = sourceUnit;
        }

        private String owner() {
            return owners.isEmpty() ? path : String.join(".", owners);
        }

        private void add(String relation, String capability, String target, ASTNode node,
                         String qualifier, String explicitOwner) {
            if (target == null || target.trim().isEmpty()) return;
            Span span = text.span(node);
            if (span == null || span.end() <= span.start()) return;
            String cleanTarget = target.trim();
            String owner = explicitOwner == null ? owner() : explicitOwner;
            String key = relation + "\u0000" + owner + "\u0000" + cleanTarget + "\u0000"
                    + span.start() + "\u0000" + span.end();
            if (emitted.add(key)) {
                relations.add(new Relation(relation, capability, owner, cleanTarget, qualifier,
                        span.start(), span.end(), span.line()));
            }
        }

        private void declaration(String name, ASTNode node) {
            add("contains", "ownership", name, node, null, null);
        }

        private String className(ClassNode node) {
            String value = node.getNameWithoutPackage();
            return value == null || value.isEmpty() ? node.getName() : value;
        }

        private String typeName(ClassNode node) {
            if (node == null) return "";
            String name = node.getName();
            int dollar = name.lastIndexOf('$');
            return dollar >= 0 ? name.substring(dollar + 1) : node.getNameWithoutPackage();
        }

        private void enter(String name, ASTNode node) {
            declaration(name, node);
            owners.add(name);
        }

        private void leave() {
            if (!owners.isEmpty()) owners.remove(owners.size() - 1);
        }

        @Override
        protected SourceUnit getSourceUnit() {
            return sourceUnit;
        }

        @Override
        public void visitClass(ClassNode node) {
            if (node.isScript() || node.isScriptBody() || node.getNameWithoutPackage() == null) {
                super.visitClass(node);
                return;
            }
            String name = className(node);
            enter(name, node);
            ClassNode superClass = node.getUnresolvedSuperClass();
            if (superClass != null && !"java.lang.Object".equals(superClass.getName())) {
                add("extends", "base_types", typeName(superClass), node, null, owner());
            }
            for (ClassNode iface : node.getInterfaces()) {
                add("implements", "base_types", typeName(iface), node, null, owner());
            }
            super.visitClass(node);
            leave();
        }

        @Override
        public void visitMethod(MethodNode node) {
            String name = node.isConstructor() ? "this" : node.getName();
            declaration(name, node);
            owners.add(name);
            super.visitMethod(node);
            leave();
        }

        @Override
        public void visitConstructor(ConstructorNode node) {
            declaration("this", node);
            owners.add("this");
            super.visitConstructor(node);
            leave();
        }

        @Override
        public void visitField(FieldNode node) {
            declaration(node.getName(), node);
            super.visitField(node);
        }

        @Override
        public void visitProperty(PropertyNode node) {
            declaration(node.getName(), node);
            super.visitProperty(node);
        }

        @Override
        public void visitMethodCallExpression(MethodCallExpression node) {
            String name = node.getMethodAsString();
            if (name != null && !name.isEmpty() && !isKeyword(name)) {
                String qualifier = null;
                Expression receiver = node.getObjectExpression();
                if (receiver != null && !node.isImplicitThis()) qualifier = receiver.getText();
                ASTNode methodNode = node.getMethod();
                add("calls", "calls", name, methodNode == null ? node : methodNode, qualifier, null);
            }
            super.visitMethodCallExpression(node);
        }

        @Override
        public void visitConstructorCallExpression(ConstructorCallExpression node) {
            ClassNode type = node.getType();
            String name = typeName(type);
            add("instantiates", "construction", name, node, null, null);
            super.visitConstructorCallExpression(node);
        }

        @Override
        public void visitPropertyExpression(PropertyExpression node) {
            String name = node.getPropertyAsString();
            if (name != null && !name.isEmpty()) {
                String qualifier = node.getObjectExpression() == null ? null : node.getObjectExpression().getText();
                add("accesses", "members", name, node.getProperty(), qualifier, null);
            }
            super.visitPropertyExpression(node);
        }

        @Override
        public void visitClassExpression(ClassExpression node) {
            String name = typeName(node.getType());
            add("references", "type_references", name, node, null, null);
            super.visitClassExpression(node);
        }

        @Override
        public void visitVariableExpression(VariableExpression node) {
            ClassNode type = node.getType();
            if (type != null && !ClassHelper.isDynamicTyped(type) && type.getName() != null) {
                String name = typeName(type);
                if (!name.isEmpty() && !"Object".equals(name)) {
                    add("references", "type_references", name, node, null, null);
                }
            }
            super.visitVariableExpression(node);
        }

        private static boolean isKeyword(String value) {
            return switch (value) {
                case "if", "else", "for", "while", "switch", "case", "catch", "finally",
                        "return", "throw", "new", "this", "super", "assert" -> true;
                default -> false;
            };
        }
    }

    private static String json(String value) {
        if (value == null) return "null";
        StringBuilder builder = new StringBuilder("\"");
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            switch (character) {
                case '\\' -> builder.append("\\\\");
                case '"' -> builder.append("\\\"");
                case '\n' -> builder.append("\\n");
                case '\r' -> builder.append("\\r");
                case '\t' -> builder.append("\\t");
                default -> {
                    if (character < 0x20) builder.append(String.format("\\u%04x", (int) character));
                    else builder.append(character);
                }
            }
        }
        return builder.append('"').toString();
    }

    private static String relationJson(Relation relation) {
        return "{\"relation\":" + json(relation.relation())
                + ",\"capability\":" + json(relation.capability())
                + ",\"ownerQualifiedName\":" + json(relation.owner())
                + ",\"targetSpelling\":" + json(relation.target())
                + ",\"qualifier\":" + json(relation.qualifier())
                + ",\"startByte\":" + relation.start()
                + ",\"endByte\":" + relation.end()
                + ",\"startLine\":" + relation.line() + "}";
    }

    private static String fileJson(Path root, String relative) {
        Path path = root.resolve(relative).normalize();
        byte[] bytes;
        try {
            bytes = Files.readAllBytes(path);
        } catch (IOException error) {
            return "{\"path\":" + json(relative) + ",\"status\":\"partial\",\"bytes\":0,\"relations\":[]}";
        }
        try {
            String source = new String(bytes, StandardCharsets.UTF_8);
            CompilerConfiguration configuration = new CompilerConfiguration();
            CompilationUnit unit = new CompilationUnit(configuration);
            SourceUnit sourceUnit = unit.addSource(relative, source);
            unit.compile(Phases.CONVERSION);
            ModuleNode module = sourceUnit.getAST();
            Emitter emitter = new Emitter(relative, source, new SourceText(source));
            emitter.setSourceUnit(sourceUnit);
            emitter.visitImports(module);
            for (ImportNode ignored : module.getImports()) {
                // Imports are emitted below with the exact AST import span.
            }
            for (ImportNode importNode : module.getImports()) {
                String target = importNode.getClassName();
                if (target != null && !target.isEmpty()) emitter.add("imports", "imports", target, importNode, null, null);
            }
            for (ImportNode importNode : module.getStarImports()) {
                String target = importNode.getPackageName();
                if (target != null && !target.isEmpty()) emitter.add("imports", "imports", target, importNode, null, null);
            }
            for (ClassNode classNode : module.getClasses()) emitter.visitClass(classNode);
            for (MethodNode method : module.getMethods()) emitter.visitMethod(method);
            List<Relation> relations = emitter.relations();
            StringBuilder result = new StringBuilder("{\"path\":").append(json(relative))
                    .append(",\"status\":\"ok\",\"bytes\":").append(bytes.length)
                    .append(",\"relations\":[");
            for (int index = 0; index < relations.size(); index++) {
                if (index > 0) result.append(',');
                result.append(relationJson(relations.get(index)));
            }
            return result.append("]}").toString();
        } catch (Throwable error) {
            return "{\"path\":" + json(relative) + ",\"status\":\"partial\",\"bytes\":"
                    + bytes.length + ",\"relations\":[]}";
        }
    }

    private static Map<String, String> options(String[] args) {
        Map<String, String> values = new TreeMap<>();
        if (args.length % 2 != 0) throw new IllegalArgumentException("expected --root, --files, and --output");
        for (int index = 0; index < args.length; index += 2) {
            if (!args[index].startsWith("--")) throw new IllegalArgumentException("expected named options");
            values.put(args[index], args[index + 1]);
        }
        return values;
    }

    public static void main(String[] args) throws Exception {
        try {
            Map<String, String> values = options(args);
            Path root = Paths.get(required(values, "--root")).toAbsolutePath().normalize();
            Path files = Paths.get(required(values, "--files")).toAbsolutePath().normalize();
            Path output = Paths.get(required(values, "--output")).toAbsolutePath().normalize();
            List<String> relative = Files.readAllLines(files, StandardCharsets.UTF_8).stream()
                    .filter(value -> !value.isEmpty()).toList();
            StringBuilder document = new StringBuilder("{\"language\":\"groovy\",\"provider\":\"groovy-compilation-unit-source-oracle\",\"toolchain\":\"Apache Groovy 4.0.27 CompilationUnit conversion phase (qualification contract)\",\"implementation\":\"Apache Groovy CompilationUnit AST source parser\",\"parserAvailable\":true,\"files\":[");
            for (int index = 0; index < relative.size(); index++) {
                if (index > 0) document.append(',');
                document.append(fileJson(root, relative.get(index)));
            }
            Files.writeString(output, document.append("]}\n").toString(), StandardCharsets.UTF_8);
        } catch (Throwable error) {
            System.err.println("Groovy provider failed: " + error.getMessage());
            System.exit(1);
        }
    }

    private static String required(Map<String, String> values, String key) {
        String value = values.get(key);
        if (value == null || value.isEmpty()) throw new IllegalArgumentException("missing " + key);
        return value;
    }
}
