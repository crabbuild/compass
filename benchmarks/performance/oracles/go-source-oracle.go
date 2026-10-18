// Qualification-only source candidates. Never runs source packages, build
// scripts, module loaders, go list, or an implicit compiler export importer.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"go/types"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
)

const maxFiles = 20000
const maxBytes = 128 * 1024 * 1024
const maxRecords = 500000
const maxDiagnostics = 10000
const identityLimit = 4096
const oversizedIdentity = "<oversized_identity>"

type inputFile struct {
	Path     string `json:"path"`
	SHA256   string `json:"sha256"`
	ByteSize int    `json:"byteSize"`
}
type request struct {
	Root  string      `json:"root"`
	Files []inputFile `json:"files"`
}
type anchor struct {
	Start int `json:"startByte"`
	End   int `json:"endByte"`
	Line  int `json:"startLine"`
}
type declaration struct {
	Name          string `json:"name"`
	QualifiedName string `json:"qualifiedName"`
	Kind          string `json:"kind"`
	anchor
	BodyStart int `json:"bodyStartByte"`
	BodyEnd   int `json:"bodyEndByte"`
}
type target struct {
	File          string `json:"file"`
	QualifiedName string `json:"qualifiedName"`
	anchor
}
type call struct {
	Owner     string `json:"owner"`
	Spelling  string `json:"spelling"`
	Qualifier string `json:"qualifier"`
	anchor
	InvocationStart int     `json:"invocationStartByte"`
	InvocationEnd   int     `json:"invocationEndByte"`
	BindingStatus   string  `json:"bindingStatus"`
	Target          *target `json:"target"`
}
type importRecord struct {
	Owner string `json:"owner"`
	Path  string `json:"path"`
	Alias string `json:"alias"`
	anchor
}
type fileRecord struct {
	Path              string         `json:"path"`
	SHA256            string         `json:"sha256"`
	ByteSize          int            `json:"byteSize"`
	Status            string         `json:"status"`
	TypeCheckComplete bool           `json:"typeCheckComplete"`
	Declarations      []declaration  `json:"declarations"`
	Calls             []call         `json:"calls"`
	Imports           []importRecord `json:"imports"`
	tree              *ast.File
}
type diagnostic struct {
	File string `json:"file"`
	Line int    `json:"line"`
	Code string `json:"code"`
}
type response struct {
	Schema          string         `json:"schema"`
	Provider        string         `json:"provider"`
	Toolchain       string         `json:"toolchain"`
	Platform        string         `json:"platform"`
	SelectionPolicy string         `json:"selectionPolicy"`
	ParserAvailable bool           `json:"parserAvailable"`
	SourceDigest    string         `json:"sourceDigest"`
	Files           []fileRecord   `json:"files"`
	Diagnostics     []diagnostic   `json:"diagnostics"`
	Counts          map[string]int `json:"counts"`
}
type rawCall struct {
	node  *ast.CallExpr
	owner string
	file  int
}

// Even the default gc importer can execute `go list -export`. This importer
// deliberately denies imports; unavailable types remain explicit diagnostics.
type denyImports struct{}

func (denyImports) Import(string) (*types.Package, error) {
	return nil, errors.New("external type export data was not supplied")
}

func boundedAnchor(fs *token.FileSet, start, end token.Pos) anchor {
	a, b := fs.PositionFor(start, false), fs.PositionFor(end, false)
	return anchor{a.Offset, b.Offset, a.Line}
}
func receiverName(expr ast.Expr) string {
	switch n := expr.(type) {
	case *ast.Ident:
		return n.Name
	case *ast.StarExpr:
		return receiverName(n.X)
	case *ast.IndexExpr:
		return receiverName(n.X)
	case *ast.IndexListExpr:
		return receiverName(n.X)
	}
	return "<receiver>"
}
func unparen(expr ast.Expr) ast.Expr {
	for {
		switch n := expr.(type) {
		case *ast.ParenExpr:
			expr = n.X
		case *ast.IndexExpr:
			expr = n.X
		case *ast.IndexListExpr:
			expr = n.X
		default:
			return expr
		}
	}
}
func expressionName(expr ast.Expr) string {
	switch n := unparen(expr).(type) {
	case *ast.Ident:
		if len(n.Name) > identityLimit {
			return oversizedIdentity
		}
		return n.Name
	case *ast.SelectorExpr:
		left := expressionName(n.X)
		if left == oversizedIdentity || len(left)+1+len(n.Sel.Name) > identityLimit {
			return oversizedIdentity
		}
		return left + "." + n.Sel.Name
	case *ast.FuncLit:
		return "<function_literal>"
	}
	return "<expression>"
}
func interfaceType(t types.Type) bool {
	if t == nil {
		return false
	}
	if p, ok := t.(*types.Pointer); ok {
		t = p.Elem()
	}
	_, ok := t.Underlying().(*types.Interface)
	return ok
}

func run(in request) (response, error) {
	out := response{Schema: "compass.go-source-oracle/1", Provider: "go_stdlib_ast_types_v1", Toolchain: runtime.Version(), Platform: runtime.GOOS + "/" + runtime.GOARCH, SelectionPolicy: "all_selected_source_no_build_filter", ParserAvailable: true, Files: []fileRecord{}, Diagnostics: []diagnostic{}, Counts: map[string]int{}}
	if len(in.Files) == 0 || len(in.Files) > maxFiles {
		return out, errors.New("source file limit")
	}
	root, err := filepath.EvalSymlinks(in.Root)
	if err != nil || !filepath.IsAbs(root) {
		return out, errors.New("absolute source root required")
	}
	fs := token.NewFileSet()
	total := 0
	previous := ""
	diagnosticLimit := false
	addDiagnostic := func(file string, line int, code string) {
		if len(out.Diagnostics) == maxDiagnostics {
			diagnosticLimit = true
			return
		}
		out.Diagnostics = append(out.Diagnostics, diagnostic{file, line, code})
	}
	for _, input := range in.Files {
		rel := input.Path
		if rel <= previous || len(rel) > identityLimit || !strings.HasSuffix(rel, ".go") || strings.Contains(rel, "\\") || !filepath.IsLocal(rel) || filepath.ToSlash(filepath.Clean(rel)) != rel {
			return out, errors.New("source file paths must be unique canonical relatives")
		}
		previous = rel
		p := filepath.Join(root, filepath.FromSlash(rel))
		resolved, e := filepath.EvalSymlinks(p)
		if e != nil {
			return out, errors.New("source file unavailable")
		}
		contained, e := filepath.Rel(root, resolved)
		if e != nil || !filepath.IsLocal(contained) {
			return out, errors.New("source file escapes root")
		}
		stat, e := os.Lstat(p)
		if e != nil || !stat.Mode().IsRegular() || stat.Size() > maxBytes || stat.Size() != int64(input.ByteSize) {
			return out, errors.New("source file type or byte size mismatch")
		}
		file, e := os.Open(p)
		if e != nil {
			return out, e
		}
		raw, e := io.ReadAll(io.LimitReader(file, int64(input.ByteSize)+1))
		closeErr := file.Close()
		if e != nil || closeErr != nil || len(raw) != input.ByteSize {
			return out, errors.New("source file changed while reading")
		}
		total += len(raw)
		if total > maxBytes {
			return out, errors.New("source byte limit")
		}
		digest := sha256.Sum256(raw)
		if hex.EncodeToString(digest[:]) != input.SHA256 {
			return out, errors.New("source digest mismatch")
		}
		f := fileRecord{Path: rel, SHA256: input.SHA256, ByteSize: len(raw), Status: "parsed", Declarations: []declaration{}, Calls: []call{}, Imports: []importRecord{}}
		f.tree, e = parser.ParseFile(fs, p, raw, parser.AllErrors)
		if e != nil {
			f.Status = "rejected"
			f.tree = nil
			addDiagnostic(rel, 1, "parse_error")
		}
		if f.tree != nil && len(f.tree.Name.Name) > identityLimit {
			return out, errors.New("package identity limit")
		}
		out.Files = append(out.Files, f)
	}
	inventoryHash := sha256.New()
	for _, file := range in.Files {
		for _, value := range []string{file.Path, file.SHA256, strconv.Itoa(file.ByteSize)} {
			if err := binary.Write(inventoryHash, binary.LittleEndian, uint64(len(value))); err != nil {
				return out, err
			}
			if _, err := inventoryHash.Write([]byte(value)); err != nil {
				return out, err
			}
		}
	}
	out.SourceDigest = hex.EncodeToString(inventoryHash.Sum(nil))
	groups := map[string][]int{}
	groupByFile := map[int]string{}
	rejectedDirectories := map[string]bool{}
	declTargets := map[token.Pos]target{}
	literalTargets := map[*ast.FuncLit]target{}
	rawCalls := []rawCall{}
	records := 0
	recordLimit := false
	identityExceeded := false
	for at := range out.Files {
		f := &out.Files[at]
		if f.tree == nil {
			rejectedDirectories[filepath.Dir(f.Path)] = true
			continue
		}
		key := filepath.ToSlash(filepath.Dir(f.Path)) + "/" + f.tree.Name.Name
		groups[key] = append(groups[key], at)
		groupByFile[at] = key
		for _, imp := range f.tree.Imports {
			if records >= maxRecords {
				return out, errors.New("source record limit")
			}
			path, e := strconv.Unquote(imp.Path.Value)
			if e != nil {
				return out, errors.New("invalid import string")
			}
			alias := ""
			if imp.Name != nil {
				alias = imp.Name.Name
			}
			if len(path) > identityLimit || len(alias) > identityLimit {
				return out, errors.New("import identity limit")
			}
			f.Imports = append(f.Imports, importRecord{f.Path, path, alias, boundedAnchor(fs, imp.Path.Pos(), imp.Path.End())})
			records++
		}
		var walk func(ast.Node, string)
		walk = func(node ast.Node, owner string) {
			ast.Inspect(node, func(n ast.Node) bool {
				if n == nil {
					return false
				}
				if records >= maxRecords {
					recordLimit = true
					return false
				}
				switch v := n.(type) {
				case *ast.FuncDecl:
					name := f.tree.Name.Name + "."
					kind := "function"
					if v.Recv != nil && len(v.Recv.List) > 0 {
						name += receiverName(v.Recv.List[0].Type) + "."
						kind = "method"
					}
					name += v.Name.Name
					if len(name) > identityLimit {
						identityExceeded = true
						return false
					}
					a := boundedAnchor(fs, v.Name.Pos(), v.Name.End())
					body := boundedAnchor(fs, v.Pos(), v.End())
					f.Declarations = append(f.Declarations, declaration{v.Name.Name, name, kind, a, body.Start, body.End})
					declTargets[v.Name.Pos()] = target{f.Path, name, a}
					records++
					walk(v.Type, name)
					if v.Body != nil {
						walk(v.Body, name)
					}
					return false
				case *ast.FuncLit:
					a := boundedAnchor(fs, v.Pos(), v.End())
					name := fmt.Sprintf("%s.closure@%d", owner, a.Start)
					if len(name) > identityLimit {
						identityExceeded = true
						return false
					}
					f.Declarations = append(f.Declarations, declaration{"<closure>", name, "closure", a, a.Start, a.End})
					literalTargets[v] = target{f.Path, name, a}
					records++
					walk(v.Type, name)
					walk(v.Body, name)
					return false
				case *ast.CallExpr:
					rawCalls = append(rawCalls, rawCall{v, owner, at})
					records++
				}
				return true
			})
		}
		walk(f.tree, f.Path)
		if recordLimit || identityExceeded {
			return out, errors.New("source record limit")
		}
	}
	callsByGroup := map[string][]rawCall{}
	for _, raw := range rawCalls {
		key := groupByFile[raw.file]
		callsByGroup[key] = append(callsByGroup[key], raw)
	}
	keys := make([]string, 0, len(groups))
	for key := range groups {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		indexes := groups[key]
		trees := []*ast.File{}
		for _, at := range indexes {
			trees = append(trees, out.Files[at].tree)
		}
		info := types.Info{Types: map[ast.Expr]types.TypeAndValue{}, Defs: map[*ast.Ident]types.Object{}, Uses: map[*ast.Ident]types.Object{}, Selections: map[*ast.SelectorExpr]*types.Selection{}}
		config := types.Config{Importer: denyImports{}, Sizes: types.SizesFor("gc", runtime.GOARCH), Error: func(e error) {
			if v, ok := e.(types.Error); ok {
				p := fs.PositionFor(v.Pos, false)
				rel, _ := filepath.Rel(root, p.Filename)
				addDiagnostic(filepath.ToSlash(rel), p.Line, "types_error")
			} else {
				addDiagnostic(out.Files[indexes[0]].Path, 1, "types_error")
			}
		}}
		pkg, typeErr := config.Check("source/"+key, fs, trees, &info)
		complete := typeErr == nil && !rejectedDirectories[filepath.Dir(out.Files[indexes[0]].Path)]
		for _, at := range indexes {
			out.Files[at].TypeCheckComplete = complete
		}
		for _, raw := range callsByGroup[key] {
			n := raw.node
			f := &out.Files[raw.file]
			base := unparen(n.Fun)
			a := boundedAnchor(fs, n.Fun.Pos(), n.Fun.End())
			inv := boundedAnchor(fs, n.Pos(), n.End())
			spelling, qualifier := expressionName(n.Fun), ""
			if sel, ok := base.(*ast.SelectorExpr); ok {
				spelling = sel.Sel.Name
				qualifier = expressionName(sel.X)
			}
			if spelling == oversizedIdentity || qualifier == oversizedIdentity || len(spelling) > identityLimit || len(qualifier) > identityLimit {
				return out, errors.New("call identity limit")
			}
			c := call{raw.owner, spelling, qualifier, a, inv.Start, inv.End, "unresolved_call_or_conversion", nil}
			if complete {
				if value, ok := info.Types[n.Fun]; ok && value.IsType() {
					c.BindingStatus = "type_conversion"
				} else {
					var object types.Object
					isInterface := false
					switch v := base.(type) {
					case *ast.Ident:
						object = info.Uses[v]
					case *ast.SelectorExpr:
						if sel := info.Selections[v]; sel != nil {
							object = sel.Obj()
							isInterface = interfaceType(sel.Recv())
						} else {
							object = info.Uses[v.Sel]
						}
					case *ast.FuncLit:
						if value, ok := literalTargets[v]; ok {
							c.BindingStatus = "local_concrete"
							c.Target = &value
						}
					}
					switch obj := object.(type) {
					case *types.Builtin:
						c.BindingStatus = "builtin"
					case *types.Func:
						if sig, ok := obj.Type().(*types.Signature); ok && sig.Recv() != nil {
							isInterface = isInterface || interfaceType(sig.Recv().Type())
						}
						if isInterface {
							c.BindingStatus = "interface_dispatch"
						} else if value, ok := declTargets[obj.Origin().Pos()]; ok {
							c.BindingStatus = "local_concrete"
							c.Target = &value
						} else if obj.Pkg() != pkg {
							c.BindingStatus = "external"
						} else {
							c.BindingStatus = "unresolved_call_or_conversion"
						}
					case *types.Var:
						c.BindingStatus = "function_value"
					}
				}
			}
			f.Calls = append(f.Calls, c)
		}
	}
	if diagnosticLimit {
		return out, errors.New("source diagnostic limit")
	}
	sort.Slice(out.Diagnostics, func(i, j int) bool {
		a, b := out.Diagnostics[i], out.Diagnostics[j]
		if a.File != b.File {
			return a.File < b.File
		}
		if a.Line != b.Line {
			return a.Line < b.Line
		}
		return a.Code < b.Code
	})
	out.Counts = map[string]int{"files": len(out.Files), "parsed": 0, "declarations": 0, "calls": 0, "imports": 0, "diagnostics": len(out.Diagnostics)}
	for at := range out.Files {
		f := &out.Files[at]
		sort.Slice(f.Declarations, func(i, j int) bool { return f.Declarations[i].Start < f.Declarations[j].Start })
		sort.Slice(f.Calls, func(i, j int) bool { return f.Calls[i].Start < f.Calls[j].Start })
		if f.Status == "parsed" {
			out.Counts["parsed"]++
		}
		out.Counts["declarations"] += len(f.Declarations)
		out.Counts["calls"] += len(f.Calls)
		out.Counts["imports"] += len(f.Imports)
	}
	return out, nil
}

func main() {
	raw, err := io.ReadAll(io.LimitReader(os.Stdin, 8*1024*1024+1))
	if err != nil || len(raw) > 8*1024*1024 {
		fmt.Fprintln(os.Stderr, "request byte limit")
		os.Exit(2)
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	var input request
	if err = decoder.Decode(&input); err != nil {
		fmt.Fprintln(os.Stderr, "invalid source request")
		os.Exit(2)
	}
	var extra any
	if decoder.Decode(&extra) != io.EOF {
		fmt.Fprintln(os.Stderr, "extra source request data")
		os.Exit(2)
	}
	output, err := run(input)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	// encoding/json buffers a value before writing it. Check a conservative
	// upper bound first, including worst-case JSON string escaping, so a large
	// repeated declaration identity cannot allocate an oversized JSON buffer.
	estimate := int64(1024)
	for _, f := range output.Files {
		estimate += 512 + 6*int64(len(f.Path))
		for _, d := range f.Declarations {
			estimate += 512 + 6*int64(len(d.Name)+len(d.QualifiedName))
		}
		for _, c := range f.Calls {
			estimate += 512 + 6*int64(len(c.Owner)+len(c.Spelling)+len(c.Qualifier))
			if c.Target != nil {
				estimate += 256 + 6*int64(len(c.Target.File)+len(c.Target.QualifiedName))
			}
		}
		for _, i := range f.Imports {
			estimate += 256 + 6*int64(len(i.Owner)+len(i.Path)+len(i.Alias))
		}
		if estimate > 64*1024*1024 {
			fmt.Fprintln(os.Stderr, "output byte limit")
			os.Exit(2)
		}
	}
	for _, d := range output.Diagnostics {
		estimate += 128 + 6*int64(len(d.File))
	}
	if estimate > 64*1024*1024 {
		fmt.Fprintln(os.Stderr, "output byte limit")
		os.Exit(2)
	}
	if err = json.NewEncoder(os.Stdout).Encode(output); err != nil {
		os.Exit(2)
	}
}
