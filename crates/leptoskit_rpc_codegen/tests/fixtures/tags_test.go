package tags

import (
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"reflect"
	"strconv"
	"testing"
)

func TestWireTags(t *testing.T) {
	data, err := os.ReadFile("names.json")

	if err != nil {
		t.Fatal(err)
	}

	var names []string

	if err := json.Unmarshal(data, &names); err != nil {
		t.Fatal(err)
	}

	want := map[string]bool{}
	for _, name := range names {
		want[name] = true
		want[name+",omitempty"] = true
	}
	for _, path := range []string{"models.go", "client.go"} {
		file, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)

		if err != nil {

			t.Fatal(err)
		}

		seen := map[string]bool{}
		ast.Inspect(file, func(node ast.Node) bool {
			if fn, ok := node.(*ast.FuncDecl); ok && fn.Name.Name == "injected" {
				t.Fatal("wire name injected a Go declaration")
			}

			field, ok := node.(*ast.Field)
			if !ok || field.Tag == nil {
				return true
			}

			literal, err := strconv.Unquote(field.Tag.Value)
			if err != nil {
				t.Fatal(err)
			}

			wire, ok := reflect.StructTag(literal).Lookup("json")
			if !ok || !want[wire] {
				t.Fatalf("unexpected JSON tag: %q", literal)
			}

			seen[wire] = true
			return true
		})
		for _, name := range names {
			if !seen[name+",omitempty"] {
				t.Errorf("%s lost optional wire name %q", path, name)
			}

			if path == "client.go" && !seen[name] {
				t.Errorf("%s lost required wire name %q", path, name)
			}
		}
	}
}

func TestJSONRoundTrip(t *testing.T) {
	// These punctuation and Unicode names are supported by encoding/json.
	var payload Payload

	if err := json.Unmarshal([]byte(`{"slash/name":"kept","café":"unicode"}`), &payload); err != nil {
		t.Fatal(err)
	}

	data, err := json.Marshal(payload)

	if err != nil {
		t.Fatal(err)
	}

	var got map[string]string

	if err := json.Unmarshal(data, &got); err != nil {

		t.Fatal(err)
	}

	if !reflect.DeepEqual(got, map[string]string{"slash/name": "kept", "café": "unicode"}) {
		t.Fatalf("changed wire names: %s", data)
	}
}
