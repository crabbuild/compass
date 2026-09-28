// Enumerate selectors on explicitly typed ordinary method parameters.
// This source oracle uses only Go's parser object bindings, not graph output.
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

type field struct {
	Name string `json:"name"`
	File string `json:"file"`
	Line int    `json:"line"`
}

type structure struct {
	Name   string             `json:"name"`
	File   string             `json:"file"`
	Line   int                `json:"line"`
	Fields map[string][]field `json:"-"`
	Decl   *ast.TypeSpec      `json:"-"`
}

type parameter struct {
	Name      string
	TypeName  string
	Line      int
	TypeIdent *ast.Ident
}

type contact struct {
	File          string `json:"file"`
	Package       string `json:"package"`
	Method        string `json:"method"`
	MethodLine    int    `json:"methodLine"`
	Parameter     string `json:"parameter"`
	ParameterLine int    `json:"parameterLine"`
	Struct        string `json:"struct"`
	StructFile    string `json:"structFile"`
	StructLine    int    `json:"structLine"`
	Field         string `json:"field"`
	FieldFile     string `json:"fieldFile"`
	FieldLine     int    `json:"fieldLine"`
	Line          int    `json:"line"`
	ByteOffset    int    `json:"byteOffset"`
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
	Contacts      []contact      `json:"contacts"`
	Exclusions    []exclusion    `json:"exclusions"`
	SelectorCount int            `json:"selectorCount"`
	Categories    map[string]int `json:"categories"`
}

func packageTypeKey(path, pkg, name string) string {
	return filepath.ToSlash(filepath.Dir(path)) + "\x00" + pkg + "\x00" + name
}

func baseTypeIdent(expr ast.Expr) *ast.Ident {
	switch value := expr.(type) {
	case *ast.Ident:
		return value
	case *ast.StarExpr:
		return baseTypeIdent(value.X)
	case *ast.IndexExpr:
		return baseTypeIdent(value.X)
	case *ast.IndexListExpr:
		return baseTypeIdent(value.X)
	default:
		return nil
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
		Schema: "compass.go-parameter-field-oracle/1",
		Files:  []sourceFile{}, Contacts: []contact{}, Exclusions: []exclusion{},
		Categories: make(map[string]int),
	}
	positions := token.NewFileSet()
	structs := make(map[string][]structure)
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
		result.Files = append(result.Files, sourceFile{path, tree.Name.Name, len(data), hex.EncodeToString(sum[:]), tree})
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
				body, ok := named.Type.(*ast.StructType)
				if !ok {
					continue
				}
				item := structure{named.Name.Name, path, positions.Position(named.Name.Pos()).Line, make(map[string][]field), named}
				for _, declarationField := range body.Fields.List {
					for _, name := range declarationField.Names {
						if name.Name != "_" {
							item.Fields[name.Name] = append(item.Fields[name.Name], field{name.Name, path, positions.Position(name.Pos()).Line})
						}
					}
				}
				key := packageTypeKey(path, tree.Name.Name, item.Name)
				structs[key] = append(structs[key], item)
			}
		}
	}
	for _, file := range result.Files {
		for _, declaration := range file.Tree.Decls {
			method, ok := declaration.(*ast.FuncDecl)
			if !ok || method.Recv == nil || method.Body == nil || method.Type.Params == nil {
				continue
			}
			parameters := make(map[*ast.Object]parameter)
			for _, declarationField := range method.Type.Params.List {
				typeIdent := baseTypeIdent(declarationField.Type)
				typeName := ""
				if typeIdent != nil {
					typeName = typeIdent.Name
				}
				for _, name := range declarationField.Names {
					if name.Obj != nil && name.Name != "_" {
						parameters[name.Obj] = parameter{name.Name, typeName, positions.Position(name.Pos()).Line, typeIdent}
					}
				}
			}
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
				position := positions.Position(selector.Sel.Pos())
				operand, direct := selector.X.(*ast.Ident)
				category := ""
				var param parameter
				var owner []structure
				var fields []field
				switch {
				case !direct:
					category = "non_identifier_receiver"
				case operand.Obj == nil:
					category = "unbound_identifier"
				default:
					var found bool
					param, found = parameters[operand.Obj]
					if !found {
						category = "not_ordinary_parameter"
					} else if called[selector] {
						category = "called_selector"
					} else if param.TypeName == "" {
						category = "unsupported_parameter_type"
					} else {
						owner = structs[packageTypeKey(file.Path, file.Package, param.TypeName)]
						if len(owner) == 0 {
							category = "unavailable_struct"
						} else if len(owner) > 1 {
							category = "ambiguous_struct"
						} else if param.TypeIdent.Obj != nil && param.TypeIdent.Obj.Decl != owner[0].Decl {
							category = "shadowed_nominal_type"
						} else {
							fields = owner[0].Fields[selector.Sel.Name]
							if len(fields) == 0 {
								category = "not_direct_named_field"
							} else if len(fields) > 1 {
								category = "ambiguous_field"
							}
						}
					}
				}
				if category == "" {
					result.Contacts = append(result.Contacts, contact{
						File: file.Path, Package: file.Package,
						Method: method.Name.Name, MethodLine: positions.Position(method.Name.Pos()).Line,
						Parameter: param.Name, ParameterLine: param.Line,
						Struct: owner[0].Name, StructFile: owner[0].File, StructLine: owner[0].Line,
						Field: fields[0].Name, FieldFile: fields[0].File, FieldLine: fields[0].Line,
						Line: position.Line, ByteOffset: position.Offset,
					})
					result.Categories["accepted_direct_field"]++
				} else {
					receiver := ""
					if direct {
						receiver = operand.Name
					}
					result.Exclusions = append(result.Exclusions, exclusion{
						File: file.Path, Method: method.Name.Name,
						MethodLine: positions.Position(method.Name.Pos()).Line,
						Line:       position.Line, ByteOffset: position.Offset,
						Receiver: receiver, Field: selector.Sel.Name, Category: category,
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
	return result, nil
}

func main() {
	result, err := run()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(result); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
