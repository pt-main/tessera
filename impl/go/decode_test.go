package tessera

import (
	"fmt"
	"strings"
	"testing"
)

func mustParse(t *testing.T, src string) *Config {
	t.Helper()
	cfg, errs := Parse(src)
	if len(errs) != 0 {
		t.Fatalf("parse errors: %v", errs)
	}
	return cfg
}

func TestDecodeBasic(t *testing.T) {
	cfg := mustParse(t, `
name s "Alice"
age i 30
height f 1.75
active l true
avatar b SGVsbG8=
`)
	type User struct {
		Name   string  `tessera:"name"`
		Age    int64   `tessera:"age"`
		Height float64 `tessera:"height"`
		Active bool    `tessera:"active"`
		Avatar []byte  `tessera:"avatar"`
	}
	var u User
	if errs := Decode(cfg, &u); len(errs) != 0 {
		t.Fatalf("decode errors: %v", errs)
	}
	if u.Name != "Alice" || u.Age != 30 || u.Height != 1.75 || !u.Active {
		t.Errorf("got %+v", u)
	}
	if string(u.Avatar) != "Hello" {
		t.Errorf("avatar = %q", u.Avatar)
	}
}

func TestDecodeNested(t *testing.T) {
	cfg := mustParse(t, `
<server
  host s "example.com"
  port i 8080
  <tls
    enabled l true
  >
>
`)
	type TLS struct {
		Enabled bool `tessera:"enabled"`
	}
	type Server struct {
		Host string `tessera:"host"`
		Port int    `tessera:"port"`
		TLS  TLS    `tessera:"tls"`
	}
	type Root struct {
		Server Server `tessera:"server"`
	}
	var r Root
	if errs := Decode(cfg, &r); len(errs) != 0 {
		t.Fatalf("decode errors: %v", errs)
	}
	if r.Server.Host != "example.com" || r.Server.Port != 8080 || !r.Server.TLS.Enabled {
		t.Errorf("got %+v", r)
	}
}

func TestDecodePointer(t *testing.T) {
	cfg := mustParse(t, "x i 42\n")
	type T struct {
		X *int64 `tessera:"x"`
	}
	var tt T
	if errs := Decode(cfg, &tt); len(errs) != 0 {
		t.Fatalf("decode errors: %v", errs)
	}
	if tt.X == nil || *tt.X != 42 {
		t.Errorf("got %+v", tt)
	}
}

func TestDecodeIntRanges(t *testing.T) {
	cfg := mustParse(t, "small i 127\nnegative i -1\n")
	type T struct {
		Small    int8   `tessera:"small"`
		Negative uint32 `tessera:"negative"`
	}
	var tt T
	errs := Decode(cfg, &tt)
	// Small fits, Negative does not.
	if tt.Small != 127 {
		t.Errorf("Small = %d", tt.Small)
	}
	if len(errs) != 1 || !strings.Contains(errs[0].Error(), "negative") {
		t.Errorf("errs = %v", errs)
	}
}

func TestDecodeOverflow(t *testing.T) {
	cfg := mustParse(t, "x i 300\n")
	type T struct {
		X int8 `tessera:"x"`
	}
	var tt T
	errs := Decode(cfg, &tt)
	if len(errs) != 1 || !strings.Contains(errs[0].Error(), "overflows") {
		t.Errorf("errs = %v", errs)
	}
}

func TestDecodeMissingOptional(t *testing.T) {
	cfg := mustParse(t, "x i 1\n")
	type T struct {
		X int64  `tessera:"x"`
		Y int64  `tessera:"y"`
		Z *int64 `tessera:"z,optional"`
	}
	var tt T
	d := &Decoder{ErrorMissing: true}
	errs := d.Decode(cfg, &tt)
	if len(errs) != 1 || !strings.Contains(errs[0].Error(), `missing key "y"`) {
		t.Errorf("errs = %v", errs)
	}
	if tt.X != 1 || tt.Z != nil {
		t.Errorf("got %+v", tt)
	}
}

func TestDecodeUnused(t *testing.T) {
	cfg := mustParse(t, "x i 1\ny i 2\n")
	type T struct {
		X int64 `tessera:"x"`
	}
	var tt T
	d := &Decoder{ErrorUnused: true}
	errs := d.Decode(cfg, &tt)
	if len(errs) != 1 || !strings.Contains(errs[0].Error(), `unused key "y"`) {
		t.Errorf("errs = %v", errs)
	}
}

func TestDecodeTypeMismatch(t *testing.T) {
	cfg := mustParse(t, "x i 1\n")
	type T struct {
		X string `tessera:"x"`
	}
	var tt T
	errs := Decode(cfg, &tt)
	if len(errs) != 1 || !strings.Contains(errs[0].Error(), "want s") {
		t.Errorf("errs = %v", errs)
	}
}

func TestDecodeCaseInsensitive(t *testing.T) {
	cfg := mustParse(t, "PORT i 8080\n")
	type T struct {
		Port int64 `tessera:"port"`
	}

	var strict T
	if errs := (&Decoder{ErrorMissing: true}).Decode(cfg, &strict); len(errs) == 0 {
		t.Error("strict mode should have reported a missing key")
	}
	if strict.Port != 0 {
		t.Errorf("strict Port = %d, want 0", strict.Port)
	}

	var relaxed T
	if errs := (&Decoder{CaseInsensitive: true}).Decode(cfg, &relaxed); len(errs) != 0 {
		t.Fatalf("decode errors: %v", errs)
	}
	if relaxed.Port != 8080 {
		t.Errorf("relaxed Port = %d, want 8080", relaxed.Port)
	}
}
func TestDecodeSkipField(t *testing.T) {
	cfg := mustParse(t, "x i 1\ny i 2\n")
	type T struct {
		X int64 `tessera:"x"`
		Y int64 `tessera:"-"`
	}
	var tt T
	d := &Decoder{ErrorUnused: false}
	// Even with ErrorUnused, "y" is reported unused because it was skipped.
	d.ErrorUnused = true
	errs := d.Decode(cfg, &tt)
	if len(errs) != 1 || !strings.Contains(errs[0].Error(), `unused key "y"`) {
		t.Errorf("errs = %v", errs)
	}
}

func TestDecodeUnsupported(t *testing.T) {
	cfg := mustParse(t, "x i 1\n")
	cases := []any{
		&struct {
			X []int64 `tessera:"x"`
		}{},
		&struct {
			X [4]byte `tessera:"x"`
		}{},
		&struct {
			X map[string]any `tessera:"x"`
		}{},
		&struct {
			X any `tessera:"x"`
		}{},
	}
	for _, c := range cases {
		if errs := Decode(cfg, c); len(errs) == 0 {
			t.Errorf("expected error for %T", c)
		}
	}
}

// ---- Custom Unmarshaler -------------------------------------------------

type doubleInt struct{ v int64 }

func (d *doubleInt) UnmarshalTessera(cfg *Config, key string) error {
	v, ok := cfg.Ints[key]
	if !ok {
		return fmt.Errorf("key %q is not an int", key)
	}
	d.v = v * 2
	return nil
}

func TestDecodeUnmarshalerValue(t *testing.T) {
	cfg := mustParse(t, "x i 21\n")
	type T struct {
		X doubleInt `tessera:"x"`
	}
	var tt T
	if errs := Decode(cfg, &tt); len(errs) != 0 {
		t.Fatalf("decode errors: %v", errs)
	}
	if tt.X.v != 42 {
		t.Errorf("X.v = %d", tt.X.v)
	}
}

func TestDecodeUnmarshalerPointer(t *testing.T) {
	cfg := mustParse(t, "x i 21\n")
	type T struct {
		X *doubleInt `tessera:"x"`
	}
	var tt T
	if errs := Decode(cfg, &tt); len(errs) != 0 {
		t.Fatalf("decode errors: %v", errs)
	}
	if tt.X == nil || tt.X.v != 42 {
		t.Errorf("X = %+v", tt.X)
	}
}

func TestDecodeDuplicateKeyAcrossTypes(t *testing.T) {
	// One Tessera key may live in several type maps; each Go field type
	// picks the matching one.
	cfg := mustParse(t, `
<thing
  x i 10
  x s "hello"
>
`)
	type Thing struct {
		X int64 `tessera:"x"`
		// Second field with the same tag but a different kind:
		Y string `tessera:"x"`
	}
	type Root struct {
		Thing Thing `tessera:"thing"`
	}
	var r Root
	if errs := Decode(cfg, &r); len(errs) != 0 {
		t.Fatalf("decode errors: %v", errs)
	}
	if r.Thing.X != 10 || r.Thing.Y != "hello" {
		t.Errorf("got %+v", r.Thing)
	}
}
