#!/usr/bin/env ruby
# frozen_string_literal: true

# Qualification-only Ruby source oracle.  It deliberately uses Ripper rather
# than Compass's Tree-sitter parser and emits only facts whose token anchors
# are exact UTF-8 byte ranges.  It is not linked into Compass runtime code.

require "digest"
require "json"
require "optparse"
require "pathname"
require "ripper"

MAX_FILES = 20_000
MAX_BYTES = 64 * 1024 * 1024
MAX_TOTAL_BYTES = 512 * 1024 * 1024
MAX_FACTS = 200_000
SKIP_DIRECTORIES = %w[.git .bundle vendor vendor/bundle node_modules tmp log coverage].freeze

def ruby_shebang?(path)
  first_line = File.binread(path, 256).split("\n", 2).first.to_s
  return false unless first_line.start_with?("#!")

  words = first_line.delete_prefix("#!").strip.split
  return false if words.empty?

  interpreter = File.basename(words.shift)
  if interpreter == "env"
    words.shift while words.first&.start_with?("-") || words.first&.include?("=")
    return false if words.empty?

    interpreter = File.basename(words.first)
  end
  interpreter == "ruby"
rescue Errno::ENOENT, Errno::EACCES
  false
end

def ruby_source_file?(path)
  return true if path.end_with?(".rb", ".rake")
  File.extname(path).empty? && ruby_shebang?(path)
end

options = { root: nil, files: [], output: nil }
OptionParser.new do |parser|
  parser.banner = "usage: ruby_source_oracle.rb --root ROOT [--output FILE] [FILES...]"
  parser.on("--root ROOT", "repository or fixture root") { |value| options[:root] = Pathname(value).expand_path }
  parser.on("--output FILE", "write canonical JSON to FILE") { |value| options[:output] = Pathname(value).expand_path }
end.parse!(ARGV)

abort "--root is required" unless options[:root]

root = options[:root]
paths = if options[:files].empty? && ARGV.empty?
          Dir.glob(root.join("**", "*").to_s).select { |path| File.file?(path) }
        else
          ARGV.map { |path| root.join(path).to_s }
        end
paths = paths.select do |path|
  next false unless ruby_source_file?(path)

  relative_parts = Pathname(path).relative_path_from(root).each_filename.to_a
  (relative_parts & SKIP_DIRECTORIES).empty?
end.sort
abort "file limit exceeded (#{paths.length} > #{MAX_FILES})" if paths.length > MAX_FILES

inventory = []
total_bytes = 0
paths.each do |path|
  relative = Pathname(path).relative_path_from(root).to_s.tr("\\", "/")
  bytes = File.binread(path)
  abort "byte limit exceeded" if bytes.bytesize > MAX_BYTES
  total_bytes += bytes.bytesize
  abort "repository byte limit exceeded" if total_bytes > MAX_TOTAL_BYTES
  begin
    source = bytes.force_encoding(Encoding::UTF_8)
    unless source.valid_encoding?
      inventory << { "path" => relative, "status" => "partial", "declarations" => [], "relations" => [] }
      next
    end
    sexp = Ripper.sexp(source)
  rescue EncodingError
    sexp = nil
  end
  if sexp.nil?
    inventory << { "path" => relative, "status" => "partial", "declarations" => [], "relations" => [] }
    next
  end

  line_starts = [0]
  source.each_byte.with_index { |byte, index| line_starts << index + 1 if byte == 10 }
  tokens = Ripper.lex(source)
  significant = tokens.each_index.select do |index|
    !%i[on_sp on_nl on_ignored_nl on_comment].include?(tokens[index][1])
  end
  facts = { declarations: [], relations: [] }
  frames = []
  owner_by_token = {}
  call_owner_by_token = {}
  declaration_name_tokens = {}
  block_depth = 0

  byte_at = lambda do |position|
    line, column = position
    line_start = line_starts.fetch(line - 1, source.bytesize)
    line_start + source.byteslice(line_start, column).to_s.bytesize
  end
  anchor = lambda do |token|
    position, _kind, text, _state = token
    start_byte = byte_at.call(position)
    end_byte = start_byte + text.to_s.encode(Encoding::UTF_8).bytesize
    {
      "sourceFile" => relative,
      "startByte" => start_byte,
      "endByte" => end_byte,
      "startLine" => position[0],
      "startColumn" => position[1],
      "endLine" => position[0],
      "endColumn" => position[1] + text.to_s.bytesize
    }
  end
  range_anchor = lambda do |first_token, last_token|
    first_position, _first_kind, first_text, _first_state = first_token
    last_position, _last_kind, last_text, _last_state = last_token
    start_byte = byte_at.call(first_position)
    last_start_byte = byte_at.call(last_position)
    {
      "sourceFile" => relative,
      "startByte" => start_byte,
      "endByte" => last_start_byte + last_text.to_s.encode(Encoding::UTF_8).bytesize,
      "startLine" => first_position[0],
      "startColumn" => first_position[1],
      "endLine" => last_position[0],
      "endColumn" => last_position[1] + last_text.to_s.bytesize
    }
  end
  text_of = lambda { |token| token[2].to_s }
  next_token = lambda do |index, step = 1|
    significant.fetch(significant.index(index).to_i + step, nil).then { |next_index| next_index && tokens[next_index] }
  end
  current = lambda do
    owner = frames.reverse_each.find { |frame| frame[:kind] == :class || frame[:kind] == :module }&.fetch(:qualified_name, relative)
    owner = relative if owner.nil? || owner.empty?
    owner
  end
  mixin_owner = lambda do
    # A top-level include/extend/prepend mutates an ambient runtime receiver;
    # a mixin inside a method or anonymous `Class.new` block is likewise not a
    # source-grounded type relationship.  Keep those facts out of the oracle
    # denominator instead of asking publication to invent a file/method owner.
    next nil if frames.reverse_each.any? { |frame| %i[def dynamic_block].include?(frame[:kind]) }

    owner_frame = frames.reverse_each.find do |frame|
      frame[:kind] == :class || frame[:kind] == :module
    end
    owner = owner_frame&.fetch(:qualified_name, nil)
    next nil if owner.nil? || owner.empty? || owner == relative || owner.end_with?("::")

    owner
  end
  method_space_by_token = {}
  qualify = lambda do |raw|
    raw = raw.sub(/^::/, "")
    raw.include?("::") || current.call == relative ? raw : "#{current.call}::#{raw}"
  end
  literal = lambda do |value|
    value.to_s.strip.sub(/^:/, "").sub(/\A['"]/, "").sub(/['"]\z/, "")
  end
  receiver_atom = lambda do |token|
    next false unless token

    kind = token[1]
    value = text_of.call(token)
    %i[on_ident on_const on_ivar on_cvar on_gvar].include?(kind) ||
      (kind == :on_kw && %w[self super].include?(value))
  end
  receiver_path = lambda do |ordinal|
    cursor = ordinal - 1
    separator = text_of.call(tokens[significant[cursor]]) if cursor >= 0
    next nil unless separator == "." || separator == "::"

    parts = []
    expect_atom = true
    cursor -= 1
    while cursor >= 0
      index = significant[cursor]
      token = tokens[index]
      value = text_of.call(token)
      if expect_atom
        unless receiver_atom.call(token)
          parts.clear
          break
        end

        parts.unshift(value)
        expect_atom = false
      elsif value == "." || value == "::"
        # A leading absolute-constant marker is part of the receiver's
        # spelling, but it does not introduce another atom to consume.
        break if value == "::" && cursor.zero?

        parts.unshift(value)
        expect_atom = true
      else
        break
      end
      cursor -= 1
    end
    next nil if expect_atom || parts.empty?

    parts.join
  end

  significant.each_with_index do |token_index, ordinal|
    token = tokens[token_index]
    owner_by_token[token_index] = current.call
    call_owner_by_token[token_index] = frames.reverse_each.find { |frame| frame[:kind] == :def }&.fetch(:qualified_name, nil) || current.call
    def_frame = frames.reverse_each.find { |frame| frame[:kind] == :def }
    method_space_by_token[token_index] = if def_frame.nil? || def_frame.fetch(:qualified_name, "").include?(".")
                                           :singleton
                                         else
                                           :instance
                                         end
    kind = token[1]
    value = text_of.call(token)
    next if kind != :on_kw && kind != :on_ident && kind != :on_const && kind != :on_op

    next_index = significant[ordinal + 1]
    next_token_value = next_index && text_of.call(tokens[next_index])
    previous_index = ordinal.positive? ? significant[ordinal - 1] : nil
    previous_value = previous_index && text_of.call(tokens[previous_index])
    case [kind, value]
    when [:on_kw, "class"], [:on_kw, "module"]
      next unless next_index
      if value == "class" && text_of.call(tokens[next_index]) == "<<"
        receiver_index = significant[ordinal + 2]
        if receiver_index && text_of.call(tokens[receiver_index]) == "self"
          # `class << self` opens the owner's singleton scope; it does not
          # declare a literal `<<` type.  Compass keeps methods in this scope
          # under the same owner identity, so the oracle does the same.
          frames << { kind: :singleton_class, qualified_name: current.call }
          next
        end
      end
      name_indices = [next_index]
      cursor = ordinal + 2
      while (candidate_index = significant[cursor])
        candidate_value = text_of.call(tokens[candidate_index])
        break unless candidate_value == "::"

        component_index = significant[cursor + 1]
        break unless component_index

        name_indices << candidate_index << component_index
        cursor += 2
      end
      name_token = tokens[name_indices.first]
      raw_name = name_indices.map { |index| text_of.call(tokens[index]) }.join
      next if raw_name.empty?

      qualified_name = qualify.call(raw_name)
      next if qualified_name.empty?

      declaration_kind = value == "module" ? "module" : "class"
      facts[:declarations] << {
        "kind" => declaration_kind,
        "qualifiedName" => qualified_name,
        "anchor" => anchor.call(name_token)
      }
      if significant[ordinal + 2] && text_of.call(tokens[significant[ordinal + 2]]) == "<"
        base_ordinal = ordinal + 3
        base_index = significant[base_ordinal]
        base_index = significant[base_ordinal + 1] if base_index && text_of.call(tokens[base_index]) == "::"
        base_token = base_index && tokens[base_index]
        if base_token
          base_parts = [text_of.call(base_token)]
          base_first_token = base_token
          base_last_token = base_token
          if base_index != significant[base_ordinal]
            base_first_token = tokens[significant[base_ordinal]]
          end
          cursor = base_ordinal + (base_index == significant[base_ordinal] ? 1 : 2)
          while (separator_index = significant[cursor]) && text_of.call(tokens[separator_index]) == "::"
            component_index = significant[cursor + 1]
            break unless component_index

            base_parts << text_of.call(tokens[separator_index]) << text_of.call(tokens[component_index])
            base_last_token = tokens[component_index]
            cursor += 2
          end
          base_name = base_parts.join
          next if base_name.empty?

          facts[:relations] << {
            "relation" => "extends",
            "source" => qualified_name,
            "target" => qualify.call(base_name),
            "anchor" => range_anchor.call(base_first_token, base_last_token)
          }
        end
      end
      frames << { kind: value.to_sym, qualified_name: qualified_name }
    when [:on_kw, "def"]
      name_token = next_index && tokens[next_index]
      # Methods declared inside `class << self` inherit the owner's
      # singleton dispatch space even when the `def` spelling is just
      # `def application`.  Keeping that scope bit here makes the oracle
      # agree with Compass's singleton-class identity (`Rails.application`),
      # rather than manufacturing an instance method (`Rails#application`).
      singleton = frames.reverse_each.any? { |frame| frame[:kind] == :singleton_class }
      if name_token && text_of.call(name_token) == "self"
        dot_index = significant[ordinal + 2]
        name_index = significant[ordinal + 3]
        singleton = true
        name_token = name_index && tokens[name_index]
      elsif significant[ordinal + 2] && [".", "::"].include?(text_of.call(tokens[significant[ordinal + 2]]))
        # `def Receiver.method` is a singleton method even when the receiver
        # is a constant path rather than the literal `self`.
        singleton = text_of.call(tokens[significant[ordinal + 2]]) == "."
        name_token = tokens[significant[ordinal + 3]] if singleton && significant[ordinal + 3]
      end
      next unless name_token
      owner = current.call
      separator = singleton ? "." : "#"
      qualified_name = "#{owner}#{separator}#{text_of.call(name_token)}"
      declaration_name_tokens[name_token.object_id] = true
      facts[:declarations] << {
        "kind" => "method",
        "qualifiedName" => qualified_name,
        "anchor" => anchor.call(name_token)
      }
      frames << { kind: :def, qualified_name: qualified_name }
    when [:on_ident, "include"], [:on_ident, "prepend"], [:on_ident, "extend"]
      if next_index && (mixin_source = mixin_owner.call)
        target_indices = []
        cursor = ordinal + 1
        if text_of.call(tokens[significant[cursor]]) == "::"
          target_indices << significant[cursor]
          cursor += 1
        end
        first_component = significant[cursor]
        unless first_component && tokens[first_component][1] == :on_const
          next
        end
        target_indices << first_component
        cursor += 1
        while (separator_index = significant[cursor]) && text_of.call(tokens[separator_index]) == "::"
          component_index = significant[cursor + 1]
          break unless component_index && tokens[component_index][1] == :on_const

          target_indices << separator_index << component_index
          cursor += 2
        end
        target = literal.call(target_indices.map { |index| text_of.call(tokens[index]) }.join)
        next if target.empty?

        target_first_token = tokens[target_indices.first]
        target_last_token = tokens[target_indices.last]

        facts[:relations] << {
          "relation" => "uses_trait",
          "operation" => value,
          "source" => mixin_source,
          "target" => begin
            if target.include?("::") || mixin_source == relative
              target
            else
              namespace = mixin_source.rpartition("::").first
              namespace.empty? || namespace == relative ? target : "#{namespace}::#{target}"
            end
          end,
          "anchor" => range_anchor.call(target_first_token, target_last_token)
        }
      end
    when [:on_ident, "require"], [:on_ident, "require_relative"], [:on_ident, "autoload"]
      literal_index = next_index
      literal_index = significant[ordinal + 2] if literal_index && tokens[literal_index][1] == :on_tstring_beg
      if literal_index && %i[on_tstring_content on_ident on_const].include?(tokens[literal_index][1])
        literal_value = text_of.call(tokens[literal_index])
        next if literal.call(literal_value).empty?

        literal_first_token = tokens[literal_index]
        literal_last_token = literal_first_token
        if significant[ordinal + 2] && tokens[significant[ordinal + 2]][1] == :on_tstring_beg
          literal_first_token = tokens[significant[ordinal + 2]]
          cursor = ordinal + 2
          while (candidate_index = significant[cursor])
            literal_last_token = tokens[candidate_index]
            break if literal_last_token[1] == :on_tstring_end

            cursor += 1
          end
        end

        facts[:relations] << {
          "relation" => "imports",
          "operation" => value,
          "source" => owner_by_token.fetch(token_index, relative).to_s.then { |owner| owner.empty? ? relative : owner },
          "target" => literal.call(literal_value),
          "anchor" => range_anchor.call(literal_first_token, literal_last_token)
        }
      end
    when [:on_kw, "end"]
      frames.pop unless frames.empty?
    end

    end_bearing = case value
                  when "do", "case", "begin", "for"
                    true
                  when "if", "unless", "while", "until"
                    previous_index = ordinal.positive? ? significant[ordinal - 1] : nil
                    statement_boundary = previous_index.nil? || tokens[(previous_index + 1)...token_index].any? do |candidate|
                      candidate[1] == :on_nl || candidate[1] == :on_ignored_nl || candidate[2] == ";"
                    end
                    statement_boundary || ["=", "(", "[", "{", "&&", "||"].include?(previous_value)
                  else
                    false
                  end
    if end_bearing
      dynamic_owner_block = if ordinal >= 2 && value == "do" && previous_value == "new"
                              significant[ordinal - 2] &&
                                text_of.call(tokens[significant[ordinal - 2]]) == "."
                            else
                              false
                            end
      frames << { kind: dynamic_owner_block ? :dynamic_block : :block }
      block_depth += 1
    elsif value == "end" && block_depth.positive?
      block_depth -= 1
    end
  end

  # Add only token-grounded calls and literal metaprogramming.  The oracle is
  # intentionally less clever than Compass: it supplies independently
  # reviewable positive strata, never a guessed target.  Calls without an
  # explicit argument list are omitted unless they have an explicit receiver.
  control_words = %w[begin break case class def do else elsif end ensure for if in module next redo rescue return self super then undef unless until when while yield].freeze
  # These operations are either represented by a more precise relation or
  # deliberately left unresolved by the product adapter.  Counting them as
  # ordinary calls would make the independent oracle claim a local target for
  # Ruby's dynamic dispatch/metaprogramming surface.
  non_evidence_calls = %w[alias_method class_eval eval extend include method_missing module_eval prepend public_send send].freeze
  significant.each_with_index do |token_index, ordinal|
    token = tokens[token_index]
    kind = token[1]
    value = text_of.call(token)
    next unless %i[on_ident on_const on_kw].include?(kind)
    next if control_words.include?(value)

    previous_token = ordinal.positive? && tokens[significant[ordinal - 1]]
    previous_value = previous_token && text_of.call(previous_token)
    next_value = significant[ordinal + 1] && text_of.call(tokens[significant[ordinal + 1]])
    next if previous_value == "def" || non_evidence_calls.include?(value) || declaration_name_tokens[token.object_id]
    method_value = value.dup
    method_value << "=" if next_value == "=" && (previous_value == "." || previous_value == "::")

    explicit_call = next_value == "(" || previous_value == "." || previous_value == "::"
    next unless explicit_call

    receiver = receiver_path.call(ordinal)
    if (previous_value == "." || previous_value == "::") && receiver.nil?
      # Do not turn a chained call whose receiver is not token-grounded (for
      # example `relation.where(...)`) into an unqualified positive fact.
      next
    end
    next if previous_value == "::" && kind == :on_const
    owner = call_owner_by_token.fetch(token_index, relative).to_s
    owner = relative if owner.empty?
    relation = value == "new" && receiver ? "constructs" : "calls"
    target = if relation == "constructs"
               receiver == "self" ? owner : receiver
             elsif receiver == "self"
               separator = method_space_by_token.fetch(token_index, :instance) == :singleton ? "." : "#"
               "#{owner}#{separator}#{method_value}"
             elsif receiver
               separator = receiver.match?(/\A(?:::)?[A-Z][A-Za-z0-9_:]*\z/) ? "." : "#"
               "#{receiver}#{separator}#{method_value}"
             else
               method_value
             end
    next if target.empty?

    facts[:relations] << {
      "relation" => relation,
      "source" => owner,
      "target" => target,
      "anchor" => anchor.call(token)
    }
  end

  significant.each_with_index do |token_index, ordinal|
    token = tokens[token_index]
    value = text_of.call(token)
    next unless value == "alias" || value == "alias_method"

    names = significant[(ordinal + 1)..].to_a
      .take_while { |index| text_of.call(tokens[index]) != ")" }
      .select { |index| %i[on_ident on_const on_tstring_content].include?(tokens[index][1]) }
      .first(2)
    next unless names.length == 2

    facts[:relations] << {
      "relation" => "aliases",
      "source" => owner_by_token.fetch(token_index, relative).to_s.then { |owner| owner.empty? ? relative : owner },
      "target" => "#{literal.call(text_of.call(tokens[names[0]]))}=>#{literal.call(text_of.call(tokens[names[1]]))}",
      "anchor" => anchor.call(token)
    }
  end

  facts[:declarations].sort_by! { |fact| [fact["anchor"]["startByte"], fact["kind"], fact["qualifiedName"]] }
  facts[:relations].sort_by! { |fact| [fact["anchor"]["startByte"], fact["relation"], fact["target"]] }
  abort "fact limit exceeded: #{relative}" if facts.values.sum(&:length) > MAX_FACTS
  inventory << { "path" => relative, "status" => "ok", **facts }
end

inventory.sort_by! { |file| file["path"] }
document = {
  "schema" => "compass.ruby-source-oracle/1",
  "rubyVersion" => RUBY_VERSION,
  "rubyRevision" => RUBY_REVISION,
  "files" => inventory
}
canonical_json = lambda do |value|
  case value
  when Hash
    "{" + value.keys.map(&:to_s).sort.map { |key|
      original_key = value.keys.find { |candidate| candidate.to_s == key }
      JSON.generate(key, ascii_only: true) + ":" + canonical_json.call(value[original_key])
    }.join(",") + "}"
  when Array
    "[" + value.map { |item| canonical_json.call(item) }.join(",") + "]"
  else
    JSON.generate(value, ascii_only: true)
  end
end
canonical = canonical_json.call(document)
document["inventorySha256"] = Digest::SHA256.hexdigest(canonical)
output = canonical_json.call(document) + "\n"
if options[:output]
  File.write(options[:output], output)
else
  $stdout.write(output)
end
