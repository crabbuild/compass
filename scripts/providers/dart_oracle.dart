// Bounded Dart Analyzer source oracle used only by qualification.
//
// The Python boundary supplies an explicit, sorted file list.  This helper
// parses those files with package:analyzer and emits source-only relations;
// it never resolves a package, runs builders, or executes repository code.

import 'dart:convert';
import 'dart:io';

import 'package:analyzer/dart/ast/ast.dart';
import 'package:analyzer/dart/ast/visitor.dart';
import 'package:analyzer/dart/analysis/utilities.dart';

class Relation {
  Relation({
    required this.relation,
    required this.capability,
    required this.owner,
    required this.target,
    required this.qualifier,
    required this.start,
    required this.end,
    required this.line,
  });

  final String relation;
  final String capability;
  final String owner;
  final String target;
  final String? qualifier;
  final int start;
  final int end;
  final int line;

  Map<String, Object?> toJson() => {
        'relation': relation,
        'capability': capability,
        'ownerQualifiedName': owner,
        'targetSpelling': target,
        'qualifier': qualifier,
        'startByte': start,
        'endByte': end,
        'startLine': line,
      };
}

class Arguments {
  Arguments(this.root, this.files, this.output);

  final Directory root;
  final File files;
  final File output;
}

Arguments parseArguments(List<String> values) {
  final options = <String, String>{};
  for (var index = 0; index + 1 < values.length; index += 2) {
    final key = values[index];
    if (!key.startsWith('--')) {
      throw FormatException('expected --root, --files, and --output');
    }
    options[key] = values[index + 1];
  }
  final root = options['--root'];
  final files = options['--files'];
  final output = options['--output'];
  if (root == null || files == null || output == null) {
    throw FormatException('expected --root, --files, and --output');
  }
  return Arguments(Directory(root), File(files), File(output));
}

class ByteOffsets {
  ByteOffsets(this.source) {
    _utf16ToByte.add(0);
    var bytes = 0;
    for (final rune in source.runes) {
      final width = utf8.encode(String.fromCharCode(rune)).length;
      bytes += width;
      // Analyzer offsets are UTF-16 code units.  Supplementary runes occupy
      // two code units while their UTF-8 representation occupies four bytes.
      _utf16ToByte.add(bytes);
      if (rune > 0xffff) {
        _utf16ToByte.add(bytes);
      }
    }
  }

  final String source;
  final List<int> _utf16ToByte = <int>[];

  int byteOffset(int offset) {
    if (offset < 0 || offset >= _utf16ToByte.length) {
      throw RangeError('UTF-16 offset outside source: $offset');
    }
    return _utf16ToByte[offset];
  }
}

class FileEmitter extends RecursiveAstVisitor<void> {
  FileEmitter(this.path, this.source, this.offsets)
      : _lineStarts = _computeLineStarts(source);

  final String path;
  final String source;
  final ByteOffsets offsets;
  final List<int> _lineStarts;
  late final int _sourceByteLength = utf8.encode(source).length;
  final List<Relation> relations = <Relation>[];
  final List<String> _owners = <String>[];

  String get owner => _owners.isEmpty ? path : _owners.join('.');

  void add(
    String relation,
    String capability,
    String target,
    AstNode node, {
    String? qualifier,
    String? explicitOwner,
  }) {
    final range = _range(node);
    if (range == null || target.trim().isEmpty) return;
    _addRange(
      relation,
      capability,
      target,
      qualifier,
      explicitOwner ?? owner,
      range.$1,
      range.$2,
      range.$3,
    );
  }

  void _addSpan(
    String relation,
    String capability,
    String target,
    int startOffset,
    int endOffset, {
    String? qualifier,
    String? explicitOwner,
  }) {
    final range = _rangeFromOffsets(startOffset, endOffset);
    if (range == null || target.trim().isEmpty) return;
    _addRange(
      relation,
      capability,
      target,
      qualifier,
      explicitOwner ?? owner,
      range.$1,
      range.$2,
      range.$3,
    );
  }

  void _addRange(
    String relation,
    String capability,
    String target,
    String? qualifier,
    String explicitOwner,
    int start,
    int end,
    int line,
  ) {
    relations.add(Relation(
      relation: relation,
      capability: capability,
      owner: explicitOwner,
      target: target.trim(),
      qualifier: qualifier,
      start: start,
      end: end,
      line: line,
    ));
  }

  (int, int, int)? _range(AstNode node) {
    return _rangeFromOffsets(node.offset, node.end);
  }

  (int, int, int)? _rangeFromOffsets(int start, int end) {
    if (start < 0 || end <= start) return null;
    try {
      final startByte = offsets.byteOffset(start);
      final endByte = offsets.byteOffset(end);
      if (endByte <= startByte || endByte > _sourceByteLength) {
        return null;
      }
      final line = _lineStarts.lastIndexWhere((item) => item <= start) + 1;
      return (startByte, endByte, line);
    } on RangeError {
      return null;
    }
  }

  void _declaration(String target, AstNode node, {String kind = 'declaration'}) {
    if (node is MethodDeclaration) {
      final body = node.body;
      if (body is ExpressionFunctionBody) {
        _addSpan('contains', 'ownership', target, body.offset, body.end);
        return;
      }
      final parameters = node.parameters;
      if (parameters != null) {
        _addSpan(
          'contains',
          'ownership',
          target,
          node.returnType?.offset ?? node.name.offset,
          parameters.end,
        );
        return;
      }
    } else if (node is FunctionDeclaration) {
      final body = node.functionExpression.body;
      if (body is ExpressionFunctionBody) {
        _addSpan('contains', 'ownership', target, body.offset, body.end);
        return;
      }
      final parameters = node.functionExpression.parameters;
      if (parameters != null) {
        _addSpan(
          'contains',
          'ownership',
          target,
          node.returnType?.offset ?? node.name.offset,
          parameters.end,
        );
        return;
      }
    } else if (node is ConstructorDeclaration) {
      final parameters = node.parameters;
      if (parameters != null) {
        _addSpan(
          'contains',
          'ownership',
          target,
          node.offset,
          parameters.end,
        );
        return;
      }
    }
    _addSpan('contains', 'ownership', target, _declarationStart(node), node.end);
  }

  int _declarationStart(AstNode node) {
    final metadataStart = node is AnnotatedNode && node.metadata.isNotEmpty
        ? node.metadata.first.offset
        : null;
    if (node is ClassDeclaration) return metadataStart ?? node.classKeyword.offset;
    if (node is MixinDeclaration) return metadataStart ?? node.mixinKeyword.offset;
    if (node is ExtensionDeclaration) {
      return metadataStart ?? node.extensionKeyword.offset;
    }
    if (node is ExtensionTypeDeclaration) {
      return metadataStart ?? node.extensionKeyword.offset;
    }
    if (node is EnumDeclaration) return metadataStart ?? node.enumKeyword.offset;
    if (node is TypeAlias) return metadataStart ?? node.typedefKeyword.offset;
    return node.offset;
  }

  void _enter(String name, AstNode node) {
    _declaration(name, node);
    _owners.add(name);
  }

  void _leave() {
    if (_owners.isNotEmpty) _owners.removeLast();
  }

  @override
  void visitCompilationUnit(CompilationUnit node) {
    for (final directive in node.directives) {
      directive.accept(this);
    }
    for (final declaration in node.declarations) {
      declaration.accept(this);
    }
  }

  @override
  void visitImportDirective(ImportDirective node) {
    final uri = node.uri.stringValue ?? node.uri.toSource();
    add('imports', 'imports', uri, node);
    node.prefix?.accept(this);
    for (final combinator in node.combinators) {
      combinator.accept(this);
    }
  }

  @override
  void visitExportDirective(ExportDirective node) {
    final uri = node.uri.stringValue ?? node.uri.toSource();
    add('reexports', 'reexports', uri, node);
    for (final combinator in node.combinators) {
      combinator.accept(this);
    }
  }

  @override
  void visitPartDirective(PartDirective node) {
    final uri = node.uri.stringValue ?? node.uri.toSource();
    add('imports', 'imports', uri, node);
  }

  @override
  void visitPartOfDirective(PartOfDirective node) {
    add('imports', 'imports', node.libraryName?.toSource() ?? node.uri?.toSource() ?? '', node);
  }

  @override
  void visitClassDeclaration(ClassDeclaration node) {
    _enter(node.name.lexeme, node);
    final extendsClause = node.extendsClause;
    if (extendsClause != null) {
      for (final type in _typesIn(extendsClause)) {
        add('extends', 'base_types', type, extendsClause, explicitOwner: owner);
      }
    }
    final withClause = node.withClause;
    if (withClause != null) {
      for (final type in _typesIn(withClause)) {
        add('implements', 'base_types', type, withClause, explicitOwner: owner);
      }
    }
    final implementsClause = node.implementsClause;
    if (implementsClause != null) {
      for (final type in _typesIn(implementsClause)) {
        add('implements', 'base_types', type, implementsClause, explicitOwner: owner);
      }
    }
    super.visitClassDeclaration(node);
    _leave();
  }

  @override
  void visitMixinDeclaration(MixinDeclaration node) {
    _enter(node.name.lexeme, node);
    for (final type in _typesIn(node.onClause)) {
      add('extends', 'base_types', type, node.onClause!, explicitOwner: owner);
    }
    for (final type in _typesIn(node.implementsClause)) {
      add('implements', 'base_types', type, node.implementsClause!, explicitOwner: owner);
    }
    super.visitMixinDeclaration(node);
    _leave();
  }

  @override
  void visitExtensionDeclaration(ExtensionDeclaration node) {
    final name = node.name?.lexeme ?? 'extension';
    _enter(name, node);
    final onClause = node.onClause;
    if (onClause != null) {
      for (final type in _typesIn(onClause)) {
        add('references', 'type_references', type, onClause, explicitOwner: owner);
      }
    }
    super.visitExtensionDeclaration(node);
    _leave();
  }

  @override
  void visitExtensionTypeDeclaration(ExtensionTypeDeclaration node) {
    _enter(node.name.lexeme, node);
    for (final type in _typesIn(node.representation.fieldType)) {
      add('references', 'type_references', type, node.representation.fieldType, explicitOwner: owner);
    }
    super.visitExtensionTypeDeclaration(node);
    _leave();
  }

  @override
  void visitEnumDeclaration(EnumDeclaration node) {
    _enter(node.name.lexeme, node);
    super.visitEnumDeclaration(node);
    _leave();
  }

  @override
  void visitTypeAlias(TypeAlias node) {
    _declaration(node.name.lexeme, node);
    node.visitChildren(this);
  }

  @override
  void visitFunctionDeclaration(FunctionDeclaration node) {
    _declaration(node.name.lexeme, node);
    super.visitFunctionDeclaration(node);
  }

  @override
  void visitTopLevelVariableDeclaration(TopLevelVariableDeclaration node) {
    super.visitTopLevelVariableDeclaration(node);
  }

  @override
  void visitConstructorDeclaration(ConstructorDeclaration node) {
    final name = node.name?.lexeme;
    _declaration(name == null || name.isEmpty ? 'new' : name, node);
    super.visitConstructorDeclaration(node);
  }

  @override
  void visitMethodDeclaration(MethodDeclaration node) {
    _declaration(node.name.lexeme, node);
    super.visitMethodDeclaration(node);
  }

  @override
  void visitFieldDeclaration(FieldDeclaration node) {
    super.visitFieldDeclaration(node);
  }

  @override
  void visitVariableDeclarationStatement(VariableDeclarationStatement node) {
    super.visitVariableDeclarationStatement(node);
  }

  @override
  void visitFunctionExpression(FunctionExpression node) {
    super.visitFunctionExpression(node);
  }

  @override
  void visitInstanceCreationExpression(InstanceCreationExpression node) {
    final constructor = node.constructorName;
    final target = constructor.type.toSource();
    final suffix = constructor.name?.name;
    add(
      'instantiates',
      'construction',
      suffix == null || suffix.isEmpty ? target : '$target.$suffix',
      constructor,
      qualifier: target,
    );
    super.visitInstanceCreationExpression(node);
  }

  @override
  void visitMethodInvocation(MethodInvocation node) {
    final target = node.methodName.name;
    final qualifier = node.target?.toSource();
    final constructor = target.isNotEmpty && target.codeUnitAt(0) >= 65 && target.codeUnitAt(0) <= 90;
    add(
      constructor ? 'instantiates' : 'calls',
      constructor ? 'construction' : 'calls',
      target,
      node.methodName,
      qualifier: qualifier,
    );
    if (node.target != null) node.target!.accept(this);
    node.typeArguments?.accept(this);
    node.argumentList.accept(this);
  }

  @override
  void visitFunctionExpressionInvocation(FunctionExpressionInvocation node) {
    add('calls', 'calls', node.function.toSource(), node.function,
        qualifier: null);
    super.visitFunctionExpressionInvocation(node);
  }

  @override
  void visitPropertyAccess(PropertyAccess node) {
    add('accesses', 'members', node.propertyName.name, node.propertyName,
        qualifier: node.target?.toSource());
    super.visitPropertyAccess(node);
  }

  @override
  void visitPrefixedIdentifier(PrefixedIdentifier node) {
    add('accesses', 'members', node.identifier.name, node.identifier,
        qualifier: node.prefix.name);
    super.visitPrefixedIdentifier(node);
  }

  @override
  void visitIndexExpression(IndexExpression node) {
    add('accesses', 'members', '[]', node.index ?? node);
    super.visitIndexExpression(node);
  }

  @override
  @override
  void visitNamedType(NamedType node) {
    _addSpan(
      'references',
      'type_references',
      node.name.lexeme,
      node.name.offset,
      node.name.end,
      qualifier: node.importPrefix?.toSource(),
    );
    super.visitNamedType(node);
  }

  @override
  void visitAnnotation(Annotation node) {
    add('references', 'type_references', node.name.toSource(), node.name);
    super.visitAnnotation(node);
  }

  Iterable<String> _typesIn(AstNode? node) sync* {
    if (node == null) return;
    for (final child in node.childEntities.whereType<NamedType>()) {
      yield child.name.lexeme;
    }
  }

  static List<int> _computeLineStarts(String source) {
    final starts = <int>[0];
    for (var index = 0; index < source.length; index++) {
      if (source.codeUnitAt(index) == 0x0a) starts.add(index + 1);
    }
    return starts;
  }
}

List<Relation> parseFile(String path, String source) {
  final parsed = parseString(content: source, path: path, throwIfDiagnostics: false);
  final emitter = FileEmitter(path, source, ByteOffsets(source));
  parsed.unit.accept(emitter);
  emitter.relations.sort((left, right) {
    final byStart = left.start.compareTo(right.start);
    if (byStart != 0) return byStart;
    final byEnd = left.end.compareTo(right.end);
    if (byEnd != 0) return byEnd;
    final byRelation = left.relation.compareTo(right.relation);
    if (byRelation != 0) return byRelation;
    return left.target.compareTo(right.target);
  });
  return emitter.relations;
}

Map<String, Object?> run(Arguments arguments) {
  final paths = arguments.files
      .readAsLinesSync()
      .where((line) => line.isNotEmpty)
      .toList(growable: false);
  final files = <Map<String, Object?>>[];
  for (final relative in paths) {
    final file = File('${arguments.root.path}/$relative');
    try {
      final bytes = file.readAsBytesSync();
      final source = utf8.decode(bytes, allowMalformed: false);
      files.add({
        'path': relative,
        'status': 'ok',
        'bytes': bytes.length,
        'relations': parseFile(relative, source).map((item) => item.toJson()).toList(),
      });
    } catch (_) {
      files.add({'path': relative, 'status': 'partial', 'bytes': 0, 'relations': []});
    }
  }
  return {
    'language': 'dart',
    'provider': 'dart-analyzer-source-oracle',
    'toolchain': 'Dart SDK 3.13.1; package:analyzer 8.4.0 (qualification contract)',
    'implementation': 'package:analyzer AST source parser',
    'parserAvailable': true,
    'files': files,
  };
}

void main(List<String> values) {
  try {
    final arguments = parseArguments(values);
    arguments.output.writeAsStringSync(
      '${const JsonEncoder.withIndent(null).convert(run(arguments))}\n',
    );
  } catch (error, stack) {
    stderr.writeln('dart analyzer provider failed: $error');
    stderr.writeln(stack);
    exitCode = 1;
  }
}
