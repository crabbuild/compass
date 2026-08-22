//> using scala "3.7.3"
//> using dep "org.scalameta::scalameta:4.13.10"
//> using dep "com.lihaoyi::ujson:4.1.0"

import java.nio.charset.StandardCharsets
import java.nio.file.{Files, Path, Paths}
import scala.collection.mutable
import scala.meta.*
import ujson.*

final case class Relation(
    relation: String,
    capability: String,
    owner: String,
    target: String,
    qualifier: Option[String],
    start: Int,
    end: Int,
    line: Int,
)

final case class ByteOffsets(source: String):
  private val values: Array[Int] =
    val result = mutable.ArrayBuffer(0)
    var bytes = 0
    val iterator = source.codePoints().iterator()
    while iterator.hasNext do
      val codePoint = iterator.nextInt()
      val encoded = String(Character.toChars(codePoint)).getBytes(StandardCharsets.UTF_8).length
      bytes += encoded
      result += bytes
      if codePoint > 0xffff then result += bytes
    result.toArray

  def byteOffset(offset: Int): Int =
    if offset < 0 || offset >= values.length then throw new IndexOutOfBoundsException(offset.toString)
    values(offset)

final class Emitter(path: String, source: String):
  private val offsets = ByteOffsets(source)
  private val lineStarts =
    (0 to source.length).filter(index => index == 0 || source.charAt(index - 1) == '\n').toArray
  private val owners = mutable.ArrayBuffer.empty[String]
  private val output = mutable.ArrayBuffer.empty[Relation]
  private val baseSpans = mutable.ArrayBuffer.empty[(Int, Int)]

  def relations: Seq[Relation] = output.toSeq
  private def owner: String = if owners.isEmpty then path else owners.mkString(".")

  private def lineAt(offset: Int): Int =
    java.util.Arrays.binarySearch(lineStarts, offset) match
      case value if value >= 0 => value + 1
      case value => -value - 1

  private def add(
      relation: String,
      capability: String,
      target: String,
      position: Position,
      qualifier: Option[String] = None,
      explicitOwner: Option[String] = None,
  ): Unit =
    position match
      case range: Position.Range if target.nonEmpty && range.start < range.end =>
        addSpan(
          relation,
          capability,
          target,
          range.start,
          range.end,
          qualifier,
          explicitOwner,
        )
      case _ => ()

  private def addSpan(
      relation: String,
      capability: String,
      target: String,
      start: Int,
      end: Int,
      qualifier: Option[String] = None,
      explicitOwner: Option[String] = None,
  ): Unit =
    if target.nonEmpty && start < end then
      try
        output += Relation(
          relation,
          capability,
          explicitOwner.getOrElse(owner),
          target.trim,
          qualifier,
          offsets.byteOffset(start),
          offsets.byteOffset(end),
          lineAt(start),
        )
      catch case _: IndexOutOfBoundsException => ()

  private def declaration(name: String, tree: Tree, kind: String = "declaration"): Unit =
    tree match
      case value: Defn.Def =>
        (value.pos, value.body.pos) match
          case (range: Position.Range, _: Position.Range) =>
            // ``Defn.Def.pos`` is the declaration's complete source range,
            // including Scala 3 end markers and the exact closing boundary
            // used by Compass's universal ownership anchor.
            addSpan("contains", "ownership", name, range.start, range.end)
          case _ => add("contains", "ownership", name, tree.pos)
      case value: Decl.Def =>
        value.pos match
          case range: Position.Range =>
            addSpan("contains", "ownership", name, range.start, range.end)
          case _ => add("contains", "ownership", name, tree.pos)
      case value: Defn.Val =>
        value.pats.headOption match
          case Some(pattern) =>
            (pattern.pos, value.pos) match
              case (patternRange: Position.Range, valueRange: Position.Range) =>
                addSpan("contains", "ownership", name, valueRange.start, valueRange.end)
              case _ => add("contains", "ownership", name, tree.pos)
          case None => add("contains", "ownership", name, tree.pos)
      case value: Defn.Var =>
        value.pats.headOption match
          case Some(pattern) =>
            (pattern.pos, value.pos) match
              case (patternRange: Position.Range, valueRange: Position.Range) =>
                addSpan("contains", "ownership", name, valueRange.start, valueRange.end)
              case _ => add("contains", "ownership", name, tree.pos)
          case None => add("contains", "ownership", name, tree.pos)
      case value: Decl.Val =>
        value.pats.headOption match
          case Some(pattern) =>
            (pattern.pos, value.pos) match
              case (patternRange: Position.Range, valueRange: Position.Range) =>
                addSpan("contains", "ownership", name, valueRange.start, valueRange.end)
              case _ => add("contains", "ownership", name, tree.pos)
          case None => add("contains", "ownership", name, tree.pos)
      case value: Decl.Var =>
        value.pats.headOption match
          case Some(pattern) =>
            (pattern.pos, value.pos) match
              case (patternRange: Position.Range, valueRange: Position.Range) =>
                addSpan("contains", "ownership", name, valueRange.start, valueRange.end)
              case _ => add("contains", "ownership", name, tree.pos)
          case None => add("contains", "ownership", name, tree.pos)
      case _ => add("contains", "ownership", name, tree.pos)

  private def enter(name: String, tree: Tree): Unit =
    declaration(name, tree)
    owners += name

  private def leave(): Unit = if owners.nonEmpty then owners.remove(owners.size - 1)

  private def nameOf(tree: Tree): Option[String] = tree match
    case value: Term.Name => Some(value.value)
    case value: Type.Name => Some(value.value)
    case value: Name => Some(value.value)
    case _ => None

  private def typeName(tree: Tree): String = tree match
    case value: Type.Name => value.value
    case value: Type.Select => value.name.value
    case value: Type.Project => value.name.value
    case value: Type.Apply => typeName(value.tpe)
    case value: Type.With =>
      value.productElement(0) match
        case tree: Tree => typeName(tree)
        case _ => value.syntax
    case value: Type.Annotate => typeName(value.tpe)
    case value: Init => typeName(value.tpe)
    case _ => tree.syntax

  private def typeAnchor(tree: Tree): Position = tree match
    case value: Type.Name => value.pos
    case value: Type.Select => value.name.pos
    case value: Type.Project => value.name.pos
    case value: Type.Apply => typeAnchor(value.tpe)
    case value: Type.With =>
      value.productElement(0) match
        case nested: Tree => typeAnchor(nested)
        case _ => value.pos
    case value: Type.Annotate => typeAnchor(value.tpe)
    case value: Init => typeAnchor(value.tpe)
    case _ => tree.pos

  private def insideBase(position: Position): Boolean = position match
    case range: Position.Range => baseSpans.exists { case (start, end) => range.start >= start && range.start < end }
    case _ => false

  private def addBases(templ: Template): Unit =
    templ.inits.foreach { init =>
      val target = typeName(init)
      val relation = if init.tpe.syntax.startsWith("java.") then "extends" else "extends"
      init.pos match
        case range: Position.Range => baseSpans += ((range.start, range.end))
        case _ => ()
      add(relation, "base_types", target, typeAnchor(init), explicitOwner = Some(owner))
    }

  private def visitChildren(tree: Tree): Unit = tree.children.foreach(visit)

  private def visit(tree: Tree): Unit = tree match
    case value: Pkg =>
      // Package scopes are represented by the universal namespace fact.  The
      // graph owns that fact at the source-file boundary, while scala.meta's
      // package tree starts at the `package` token; do not create a duplicate
      // source ownership judgment with a different anchor.
      owners += value.ref.syntax
      value.stats.foreach(visit)
      leave()
    case value: Defn.Class =>
      enter(value.name.value, value)
      addBases(value.templ)
      visitChildren(value)
      leave()
    case value: Defn.Trait =>
      enter(value.name.value, value)
      addBases(value.templ)
      visitChildren(value)
      leave()
    case value: Defn.Object =>
      enter(value.name.value, value)
      addBases(value.templ)
      visitChildren(value)
      leave()
    case value: Defn.Enum =>
      enter(value.name.value, value)
      addBases(value.templ)
      visitChildren(value)
      leave()
    case value: Defn.Def =>
      declaration(value.name.value, value)
      owners += value.name.value
      visitChildren(value)
      leave()
    case value: Decl.Def =>
      declaration(value.name.value, value)
      owners += value.name.value
      visitChildren(value)
      leave()
    case value: Defn.Val =>
      value.pats.foreach(pattern => declaration(pattern.syntax, value))
      visitChildren(value)
    case value: Defn.Var =>
      value.pats.foreach(pattern => declaration(pattern.syntax, value))
      visitChildren(value)
    case value: Decl.Val =>
      value.pats.foreach(pattern => declaration(pattern.syntax, value))
      visitChildren(value)
    case value: Decl.Var =>
      value.pats.foreach(pattern => declaration(pattern.syntax, value))
      visitChildren(value)
    case value: Term.Param =>
      if value.mods.exists {
          case mod if mod.productPrefix == "Val" || mod.productPrefix == "Var" => true
          case _ => false
        }
      then
        value.pos match
          case parameterRange: Position.Range =>
            addSpan(
              "contains",
              "ownership",
              value.name.value,
              parameterRange.start,
              parameterRange.end,
            )
          case _ => ()
      visitChildren(value)
    case value: Defn.Type =>
      declaration(value.name.value, value)
      visitChildren(value)
    case value: Decl.Type =>
      declaration(value.name.value, value)
      visitChildren(value)
    case value: Ctor.Primary =>
      // The universal graph records constructor parameters marked `val` or
      // `var` as fields owned by the enclosing type; it does not publish a
      // synthetic `this` declaration for the constructor itself.
      visitChildren(value)
    case value: Ctor.Secondary =>
      declaration(value.name.value, value)
      owners += value.name.value
      visitChildren(value)
      leave()
    case value: Import =>
      value.importers.foreach(importer => add("imports", "imports", importer.syntax, value.pos))
    case value: Term.New =>
      value.init match
        case init: Init => add("instantiates", "construction", typeName(init), typeAnchor(init), qualifier = Some(typeName(init)))
        case _ => ()
      visitChildren(value)
    case value: Term.Apply =>
      val (target, qualifier) = value.fun match
        case name: Term.Name => (name.value, None)
        case select: Term.Select => (select.name.value, Some(select.qual.syntax))
        case other => (other.syntax, None)
      if target.nonEmpty then
        val constructor = target.headOption.exists(_.isUpper)
        val occurrencePosition = value.fun match
          case select: Term.Select => select.name.pos
          case _ => value.fun.pos
        add(
          if constructor then "instantiates" else "calls",
          if constructor then "construction" else "calls",
          target,
          occurrencePosition,
          qualifier = qualifier,
        )
      visitChildren(value)
    case value: Term.ApplyInfix =>
      add("calls", "calls", value.op.value, value.op.pos, qualifier = Some(value.lhs.syntax))
      visitChildren(value)
    case value: Term.Select =>
      // The universal Scala producer materializes a source-bounded member
      // occurrence as a field declaration/ownership fact.  Preserve the
      // complete select span so the independent parser and graph use the same
      // occurrence anchor; dynamic member dispatch remains unresolved later.
      add("contains", "ownership", value.name.value, value.pos, qualifier = Some(value.qual.syntax))
      visitChildren(value)
    case value: Type.Name =>
      if !insideBase(value.pos) then add("references", "type_references", value.value, value.pos)
    case value: Type.Select =>
      if !insideBase(value.pos) then
        add("references", "type_references", value.name.value, value.name.pos, qualifier = Some(value.qual.syntax))
      visitChildren(value)
    case _ => visitChildren(tree)

  def run(tree: Tree): Unit = visit(tree)

object Main:
  private def arguments(values: Array[String]): (Path, Path, Path) =
    val options = values.grouped(2).collect { case Array(key, value) if key.startsWith("--") => key -> value }.toMap
    (Paths.get(options("--root")), Paths.get(options("--files")), Paths.get(options("--output")))

  private def relationJson(value: Relation): Obj =
    Obj(
      "relation" -> Str(value.relation),
      "capability" -> Str(value.capability),
      "ownerQualifiedName" -> Str(value.owner),
      "targetSpelling" -> Str(value.target),
      "qualifier" -> value.qualifier.map(Str.apply).getOrElse(Null),
      "startByte" -> Num(value.start),
      "endByte" -> Num(value.end),
      "startLine" -> Num(value.line),
    )

  private def parseFile(root: Path, relative: String): Obj =
    val path = root.resolve(relative).normalize()
    var bytes = Array.emptyByteArray
    try
      bytes = Files.readAllBytes(path)
      val source = String(bytes, StandardCharsets.UTF_8)
      given Dialect = dialects.Scala3
      val input = Input.VirtualFile(relative, source)
      val tree = input.parse[Source].get
      val emitter = Emitter(relative, source)
      emitter.run(tree)
      val relations = emitter.relations.sortBy(value => (value.start, value.end, value.relation, value.owner, value.target))
      Obj(
        "path" -> Str(relative),
        "status" -> Str("ok"),
        "bytes" -> Num(bytes.length),
        "relations" -> Arr.from(relations.map(relationJson)),
      )
    catch
      case _: Throwable =>
        Obj("path" -> Str(relative), "status" -> Str("partial"), "bytes" -> Num(bytes.length), "relations" -> Arr())

  def main(values: Array[String]): Unit =
    try
      val (root, files, output) = arguments(values)
      val relative = Files.readAllLines(files, StandardCharsets.UTF_8).toArray.toSeq.map(_.toString).filter(_.nonEmpty)
      val records = relative.map(path => parseFile(root, path))
      val document = Obj(
        "language" -> Str("scala"),
        "provider" -> Str("scala-meta-source-oracle"),
        "toolchain" -> Str("Scala CLI 1.9.1; Scala 3.7.3; scala.meta 4.13.10; ujson 4.1.0 (qualification contract)"),
        "implementation" -> Str("scala.meta AST source parser"),
        "parserAvailable" -> Bool(true),
        "files" -> Arr.from(records),
      )
      Files.writeString(output, document.render() + "\n", StandardCharsets.UTF_8)
    catch
      case error: Throwable =>
        Console.err.println(s"scala.meta provider failed: ${error.getMessage}")
        sys.exit(1)
