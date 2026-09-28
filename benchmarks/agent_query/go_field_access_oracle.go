// Enumerate direct method-receiver field selectors using Go's parser object bindings.
// This development oracle is independent of Compass and Graphify graph output.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

const maxPaths = 200
const maxFileBytes = 1 << 20
const maxSelectors = 100000

type sourceFile struct {
	Path    string    `json:"path"`
	Package string    `json:"package"`
	Bytes   int       `json:"bytes"`
	SHA256  string    `json:"sha256"`
	Tree    *ast.File `json:"-"`
}

type directField struct {
	File       string `json:"file"`
	Package    string `json:"package"`
	Struct     string `json:"struct"`
	StructLine int    `json:"structLine"`
	Field      string `json:"field"`
	FieldLine  int    `json:"fieldLine"`
}

type contact struct {
	File       string `json:"file"`
	Package    string `json:"package"`
	Struct     string `json:"struct"`
	StructFile string `json:"structFile"`
	StructLine int    `json:"structLine"`
	Method     string `json:"method"`
	MethodLine int    `json:"methodLine"`
	Field      string `json:"field"`
	FieldFile  string `json:"fieldFile"`
	FieldLine  int    `json:"fieldLine"`
	Line       int    `json:"line"`
	ByteOffset int    `json:"byteOffset"`
}

type exclusion struct {
	File       string `json:"file"`
	Method     string `json:"method"`
	MethodLine int    `json:"methodLine"`
	Line       int    `json:"line"`
	ByteOffset int    `json:"byteOffset"`
	Receiver   string `json:"receiver"`
	Field      string `json:"field"`
	Category   string `json:"category"`
}

type output struct {
	Schema        string         `json:"schema"`
	Files         []sourceFile   `json:"files"`
	DirectFields  []directField  `json:"directFields"`
	Contacts      []contact      `json:"contacts"`
	Exclusions    []exclusion    `json:"exclusions"`
	SelectorCount int            `json:"selectorCount"`
	Categories    map[string]int `json:"categories"`
}

func packageTypeKey(path, pkg, name string) string {
	return filepath.ToSlash(filepath.Dir(path)) + "\x00" + pkg + "\x00" + name
}

func baseReceiverType(expr ast.Expr) string {
	switch value := expr.(type) {
	case *ast.Ident:
		return value.Name
	case *ast.StarExpr:
		return baseReceiverType(value.X)
	case *ast.IndexExpr:
		return baseReceiverType(value.X)
	case *ast.IndexListExpr:
		return baseReceiverType(value.X)
	default:
		return ""
	}
}

func sourcePaths() ([]string, error) {
	paths := os.Args[1:]
	if len(paths) == 0 || len(paths) > maxPaths {
		return nil, fmt.Errorf("expected 1..%d registered relative Go paths", maxPaths)
	}
	seen := make(map[string]bool, len(paths))
	for _, path := range paths {
		if filepath.IsAbs(path) || filepath.Clean(path) != path || !strings.HasSuffix(path, ".go") {
			return nil, fmt.Errorf("invalid relative Go path: %q", path)
		}
		for _, part := range strings.Split(filepath.ToSlash(path), "/") {
			if part == ".." || part == "." || part == "" {
				return nil, fmt.Errorf("unsafe source path: %q", path)
			}
		}
		if seen[path] {
			return nil, fmt.Errorf("duplicate source path: %q", path)
		}
		seen[path] = true
	}
	sort.Strings(paths)
	return paths, nil
}

func run() (output, error) {
	paths, err := sourcePaths()
	if err != nil {
		return output{}, err
	}
	result := output{
		Schema: "compass.go-field-access-oracle/1", Files: []sourceFile{},
		DirectFields: []directField{}, Contacts: []contact{}, Exclusions: []exclusion{},
		Categories: make(map[string]int),
	}
	positions := token.NewFileSet()
	// A package/type name must identify one declaration before a field can be
	// credited. Build-tag alternatives remain ambiguous in this all-file census.
	types := make(map[string]map[string][]directField)
	for _, path := range paths {
		data, err := os.ReadFile(path)
		if err != nil {
			return output{}, err
		}
		if len(data) > maxFileBytes {
			return output{}, fmt.Errorf("source byte bound exceeded: %s", path)
		}
		tree, err := parser.ParseFile(positions, path, data, parser.AllErrors)
		if err != nil {
			return output{}, fmt.Errorf("parse %s: %w", path, err)
		}
		sum := sha256.Sum256(data)
		pkg := tree.Name.Name
		result.Files = append(result.Files, sourceFile{path, pkg, len(data), hex.EncodeToString(sum[:]), tree})
		for _, declaration := range tree.Decls {
			general, ok := declaration.(*ast.GenDecl)
			if !ok || general.Tok != token.TYPE {
				continue
			}
			for _, spec := range general.Specs {
				named, ok := spec.(*ast.TypeSpec)
				if !ok {
					continue
				}
				structure, ok := named.Type.(*ast.StructType)
				if !ok {
					continue
				}
				key := packageTypeKey(path, pkg, named.Name.Name)
				fields := make(map[string][]directField)
				for _, field := range structure.Fields.List {
					for _, name := range field.Names {
						if name.Name == "_" {
							continue
						}
						item := directField{path, pkg, named.Name.Name,
							positions.Position(named.Name.Pos()).Line, name.Name,
							positions.Position(name.Pos()).Line}
						fields[name.Name] = append(fields[name.Name], item)
						result.DirectFields = append(result.DirectFields, item)
					}
				}
				if types[key] == nil {
					types[key] = fields
				} else {
					for name, items := range fields {
						types[key][name] = append(types[key][name], items...)
					}
					// Even if fields are disjoint, duplicate nominal declarations
					// cannot establish a unique owner.
					types[key]["\x00duplicate-type"] = nil
				}
			}
		}
	}
	for _, file := range result.Files {
		for _, declaration := range file.Tree.Decls {
			method, ok := declaration.(*ast.FuncDecl)
			if !ok || method.Recv == nil || len(method.Recv.List) != 1 || method.Body == nil {
				continue
			}
			receiverField := method.Recv.List[0]
			if len(receiverField.Names) != 1 || receiverField.Names[0].Name == "_" {
				continue
			}
			receiver := receiverField.Names[0]
			typeName := baseReceiverType(receiverField.Type)
			index := types[packageTypeKey(file.Path, file.Package, typeName)]
			called := make(map[*ast.SelectorExpr]bool)
			ast.Inspect(method.Body, func(node ast.Node) bool {
				if call, ok := node.(*ast.CallExpr); ok {
					if selector, ok := call.Fun.(*ast.SelectorExpr); ok {
						called[selector] = true
					}
				}
				return true
			})
			var walkError error
			ast.Inspect(method.Body, func(node ast.Node) bool {
				selector, ok := node.(*ast.SelectorExpr)
				if !ok || walkError != nil {
					return walkError == nil
				}
				result.SelectorCount++
				if result.SelectorCount > maxSelectors {
					walkError = fmt.Errorf("selector bound exceeded")
					return false
				}
				operand, direct := selector.X.(*ast.Ident)
				category := ""
				switch {
				case !direct:
					category = "non_identifier_receiver"
				case operand.Name != receiver.Name:
					category = "other_receiver"
				case receiver.Obj == nil || operand.Obj != receiver.Obj:
					category = "shadowed_or_unresolved_receiver"
				case typeName == "" || index == nil:
					category = "unavailable_struct"
				case len(index) > 0 && duplicateType(index):
					category = "ambiguous_struct"
				case len(index[selector.Sel.Name]) == 0:
					category = "not_direct_named_field"
				case len(index[selector.Sel.Name]) > 1:
					category = "ambiguous_field"
				case called[selector]:
					category = "called_field_selector"
				}
				position := positions.Position(selector.Sel.Pos())
				if category == "" {
					field := index[selector.Sel.Name][0]
					result.Contacts = append(result.Contacts, contact{
						File: file.Path, Package: file.Package, Struct: typeName,
						StructFile: field.File, StructLine: field.StructLine,
						Method: method.Name.Name, MethodLine: positions.Position(method.Name.Pos()).Line,
						Field: field.Field, FieldFile: field.File, FieldLine: field.FieldLine,
						Line: position.Line, ByteOffset: position.Offset,
					})
					result.Categories["accepted_direct_field"]++
				} else {
					receiverName := ""
					if direct {
						receiverName = operand.Name
					}
					result.Exclusions = append(result.Exclusions, exclusion{
						File: file.Path, Method: method.Name.Name,
						MethodLine: positions.Position(method.Name.Pos()).Line,
						Line:       position.Line, ByteOffset: position.Offset,
						Receiver: receiverName, Field: selector.Sel.Name,
						Category: category,
					})
					result.Categories[category]++
				}
				return true
			})
			if walkError != nil {
				return output{}, walkError
			}
		}
	}
	sort.Slice(result.DirectFields, func(i, j int) bool {
		a, b := result.DirectFields[i], result.DirectFields[j]
		if a.File != b.File {
			return a.File < b.File
		}
		if a.StructLine != b.StructLine {
			return a.StructLine < b.StructLine
		}
		if a.FieldLine != b.FieldLine {
			return a.FieldLine < b.FieldLine
		}
		return a.Field < b.Field
	})
	sort.Slice(result.Contacts, func(i, j int) bool {
		a, b := result.Contacts[i], result.Contacts[j]
		if a.File != b.File {
			return a.File < b.File
		}
		if a.ByteOffset != b.ByteOffset {
			return a.ByteOffset < b.ByteOffset
		}
		return a.Field < b.Field
	})
	sort.Slice(result.Exclusions, func(i, j int) bool {
		a, b := result.Exclusions[i], result.Exclusions[j]
		if a.File != b.File {
			return a.File < b.File
		}
		if a.ByteOffset != b.ByteOffset {
			return a.ByteOffset < b.ByteOffset
		}
		return a.Category < b.Category
	})
	return result, nil
}

func duplicateType(fields map[string][]directField) bool {
	_, duplicate := fields["\x00duplicate-type"]
	return duplicate
}

func main() {
	result, err := run()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	encoder := json.NewEncoder(os.Stdout)
	if err := encoder.Encode(result); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
