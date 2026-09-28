// Go standard-library source oracle for the registered named struct fields.
// Build this standalone file with `go build`, then pass the registered paths.
package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
)

type declaration struct {
	File       string `json:"file"`
	Struct     string `json:"struct"`
	StructLine int    `json:"structLine"`
	Field      string `json:"field"`
	FieldLine  int    `json:"fieldLine"`
}

type embedded struct {
	File       string `json:"file"`
	Struct     string `json:"struct"`
	StructLine int    `json:"structLine"`
	Line       int    `json:"line"`
}

type result struct {
	Schema       string        `json:"schema"`
	Declarations []declaration `json:"declarations"`
	Embedded     []embedded    `json:"embedded"`
	Blank        []embedded    `json:"blank"`
}

func main() {
	paths := os.Args[1:]
	relative := len(paths) > 0 && paths[0] == "--relative"
	if relative {
		paths = paths[1:]
	}
	if len(paths) == 0 {
		fmt.Fprintln(os.Stderr, "expected registered source paths")
		os.Exit(2)
	}
	output := result{Schema: "compass.go-struct-field-oracle/1", Declarations: []declaration{}, Embedded: []embedded{}, Blank: []embedded{}}
	for _, path := range paths {
		set := token.NewFileSet()
		file, err := parser.ParseFile(set, path, nil, parser.AllErrors)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		name := filepath.Base(path)
		if relative {
			name = filepath.ToSlash(path)
		}
		for _, top := range file.Decls {
			general, ok := top.(*ast.GenDecl)
			if !ok || general.Tok != token.TYPE {
				continue
			}
			for _, spec := range general.Specs {
				typeSpec, ok := spec.(*ast.TypeSpec)
				if !ok {
					continue
				}
				structure, ok := typeSpec.Type.(*ast.StructType)
				if !ok {
					continue
				}
				structLine := set.Position(typeSpec.Pos()).Line
				for _, field := range structure.Fields.List {
					if len(field.Names) == 0 {
						output.Embedded = append(output.Embedded, embedded{name, typeSpec.Name.Name, structLine, set.Position(field.Pos()).Line})
						continue
					}
					for _, identifier := range field.Names {
						if identifier.Name == "_" {
							output.Blank = append(output.Blank, embedded{name, typeSpec.Name.Name, structLine, set.Position(identifier.Pos()).Line})
							continue
						}
						output.Declarations = append(output.Declarations, declaration{name, typeSpec.Name.Name, structLine, identifier.Name, set.Position(identifier.Pos()).Line})
					}
				}
			}
		}
	}
	sort.Slice(output.Declarations, func(i, j int) bool {
		a, b := output.Declarations[i], output.Declarations[j]
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
	sort.Slice(output.Embedded, func(i, j int) bool {
		a, b := output.Embedded[i], output.Embedded[j]
		if a.File != b.File {
			return a.File < b.File
		}
		if a.StructLine != b.StructLine {
			return a.StructLine < b.StructLine
		}
		return a.Line < b.Line
	})
	sort.Slice(output.Blank, func(i, j int) bool {
		a, b := output.Blank[i], output.Blank[j]
		if a.File != b.File {
			return a.File < b.File
		}
		if a.StructLine != b.StructLine {
			return a.StructLine < b.StructLine
		}
		return a.Line < b.Line
	})
	if err := json.NewEncoder(os.Stdout).Encode(output); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
