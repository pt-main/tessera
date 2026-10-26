// Package tessera — decoder from Config into Go structs.
package tessera

import (
	"fmt"
	"reflect"
	"sort"
	"strings"
)

// Unmarshaler is implemented by types that decode themselves from a Tessera
// Config. The decoder passes the parent config and the key under which the
// value lives; the implementation must perform its own lookup.
type Unmarshaler interface {
	UnmarshalTessera(cfg *Config, key string) error
}

var unmarshalerType = reflect.TypeOf((*Unmarshaler)(nil)).Elem()

// Decoder fills Go structs from a *Config.
type Decoder struct {
	// TagName is the struct tag consulted for field names. Defaults to
	// "tessera". Falls back to the Go field name when the tag is absent.
	TagName string

	// ErrorMissing reports an error when an exported struct field has no
	// matching key in Config. Fields tagged with ",optional" are exempt.
	ErrorMissing bool

	// ErrorUnused reports an error when Config contains a key that no
	// struct field consumed.
	ErrorUnused bool

	// CaseInsensitive matches field tags against Config keys
	// case-insensitively. Disabled by default (Tessera is case-sensitive).
	CaseInsensitive bool
}

// Decode is a convenience helper using default settings.
func Decode(cfg *Config, dst any) []error {
	return (&Decoder{}).Decode(cfg, dst)
}

// Decode fills dst (a non-nil pointer to a struct) from cfg. Decoding
// continues past individual errors, mirroring Parse.
func (d *Decoder) Decode(cfg *Config, dst any) []error {
	if cfg == nil {
		return []error{fmt.Errorf("tessera: nil Config")}
	}
	v := reflect.ValueOf(dst)
	if v.Kind() != reflect.Ptr || v.IsNil() {
		return []error{fmt.Errorf("tessera: Decode expects a non-nil pointer, got %T", dst)}
	}
	v = v.Elem()
	if v.Kind() != reflect.Struct {
		return []error{fmt.Errorf("tessera: Decode expects pointer to struct, got %s", v.Kind())}
	}
	var errs []error
	d.decodeStruct(cfg, v, "", &errs)
	return errs
}

// ---------------------------------------------------------------------------
// Tag parsing
// ---------------------------------------------------------------------------

type tagOptions []string

func (o tagOptions) has(name string) bool {
	for _, s := range o {
		if s == name {
			return true
		}
	}
	return false
}

func (d *Decoder) tagName() string {
	if d.TagName == "" {
		return "tessera"
	}
	return d.TagName
}

// fieldTag extracts the Config key and options from a struct field's tag.
func (d *Decoder) fieldTag(field reflect.StructField) (name string, opts tagOptions, skip bool) {
	raw := field.Tag.Get(d.tagName())
	if raw == "" {
		return field.Name, nil, false
	}
	parts := strings.Split(raw, ",")
	if parts[0] == "-" {
		return "", nil, true
	}
	if parts[0] == "" {
		name = field.Name
	} else {
		name = parts[0]
	}
	return name, tagOptions(parts[1:]), false
}

// ---------------------------------------------------------------------------
// Struct traversal
// ---------------------------------------------------------------------------

func (d *Decoder) decodeStruct(cfg *Config, v reflect.Value, path string, errs *[]error) {
	t := v.Type()
	used := make(map[string]bool, len(cfg.Ints)+len(cfg.Strs)+len(cfg.Floats)+
		len(cfg.Bytes)+len(cfg.Logicals)+len(cfg.Inners))

	for i := 0; i < t.NumField(); i++ {
		field := t.Field(i)
		if !field.IsExported() {
			continue
		}
		tag, opts, skip := d.fieldTag(field)
		if skip {
			continue
		}
		key := findKey(cfg, tag, d.CaseInsensitive)
		if key == "" {
			if d.ErrorMissing && !opts.has("optional") {
				*errs = append(*errs, fmt.Errorf("%s%s: missing key %q",
					path, field.Name, tag))
			}
			continue
		}
		used[key] = true
		d.decodeField(cfg, key, v.Field(i), path+field.Name, errs)
	}

	if d.ErrorUnused {
		reportUnused(cfg, used, path, errs)
	}
}

func reportUnused(cfg *Config, used map[string]bool, path string, errs *[]error) {
	set := make(map[string]struct{})
	add := func(k string) {
		if !used[k] {
			set[k] = struct{}{}
		}
	}
	for k := range cfg.Ints {
		add(k)
	}
	for k := range cfg.Strs {
		add(k)
	}
	for k := range cfg.Floats {
		add(k)
	}
	for k := range cfg.Bytes {
		add(k)
	}
	for k := range cfg.Logicals {
		add(k)
	}
	for k := range cfg.Inners {
		add(k)
	}

	if len(set) == 0 {
		return
	}
	keys := make([]string, 0, len(set))
	for k := range set {
		keys = append(keys, k)
	}
	sort.Strings(keys)

	prefix := strings.TrimSuffix(path, ".")
	for _, k := range keys {
		if prefix != "" {
			*errs = append(*errs, fmt.Errorf("%s: unused key %q", prefix, k))
		} else {
			*errs = append(*errs, fmt.Errorf("unused key %q", k))
		}
	}
}

// ---------------------------------------------------------------------------
// Field dispatch
// ---------------------------------------------------------------------------

func (d *Decoder) decodeField(cfg *Config, key string, v reflect.Value, path string, errs *[]error) {
	// (1) Custom Unmarshaler on the pointer type — highest priority.
	if v.Kind() == reflect.Ptr && v.Type().Implements(unmarshalerType) {
		if v.IsNil() {
			v.Set(reflect.New(v.Type().Elem()))
		}
		if err := v.Interface().(Unmarshaler).UnmarshalTessera(cfg, key); err != nil {
			*errs = append(*errs, fmt.Errorf("%s: %w", path, err))
		}
		return
	}
	// (2) Custom Unmarshaler on the addressable value.
	if v.Kind() != reflect.Ptr && v.CanAddr() &&
		v.Addr().Type().Implements(unmarshalerType) {
		if err := v.Addr().Interface().(Unmarshaler).UnmarshalTessera(cfg, key); err != nil {
			*errs = append(*errs, fmt.Errorf("%s: %w", path, err))
		}
		return
	}
	// (3) Generic pointer: allocate and recurse.
	if v.Kind() == reflect.Ptr {
		if v.IsNil() {
			v.Set(reflect.New(v.Type().Elem()))
		}
		d.decodeField(cfg, key, v.Elem(), path, errs)
		return
	}

	t := v.Type()
	switch v.Kind() {
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		n, ok := cfg.Ints[key]
		if !ok {
			*errs = append(*errs, mismatch(path, key, "i", cfg))
			return
		}
		if v.OverflowInt(n) {
			*errs = append(*errs, fmt.Errorf("%s: %d overflows %s", path, n, t))
			return
		}
		v.SetInt(n)

	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32,
		reflect.Uint64, reflect.Uintptr:
		n, ok := cfg.Ints[key]
		if !ok {
			*errs = append(*errs, mismatch(path, key, "i", cfg))
			return
		}
		if n < 0 {
			*errs = append(*errs,
				fmt.Errorf("%s: negative %d cannot fit %s", path, n, t))
			return
		}
		un := uint64(n)
		if v.OverflowUint(un) {
			*errs = append(*errs, fmt.Errorf("%s: %d overflows %s", path, n, t))
			return
		}
		v.SetUint(un)

	case reflect.Float32, reflect.Float64:
		f, ok := cfg.Floats[key]
		if !ok {
			*errs = append(*errs, mismatch(path, key, "f", cfg))
			return
		}
		if v.OverflowFloat(f) {
			*errs = append(*errs, fmt.Errorf("%s: %v overflows %s", path, f, t))
			return
		}
		v.SetFloat(f)

	case reflect.String:
		s, ok := cfg.Strs[key]
		if !ok {
			*errs = append(*errs, mismatch(path, key, "s", cfg))
			return
		}
		v.SetString(s)

	case reflect.Bool:
		b, ok := cfg.Logicals[key]
		if !ok {
			*errs = append(*errs, mismatch(path, key, "l", cfg))
			return
		}
		v.SetBool(b)

	case reflect.Slice:
		if v.Type().Elem().Kind() == reflect.Uint8 {
			b, ok := cfg.Bytes[key]
			if !ok {
				*errs = append(*errs, mismatch(path, key, "b", cfg))
				return
			}
			out := make([]byte, len(b))
			copy(out, b)
			v.SetBytes(out)
			return
		}
		*errs = append(*errs, fmt.Errorf(
			"%s: slice type %s is not supported (only []byte)", path, t))

	case reflect.Struct:
		child, ok := cfg.Inners[key]
		if !ok {
			*errs = append(*errs, mismatch(path, key, "block", cfg))
			return
		}
		d.decodeStruct(child, v, path+".", errs)

	case reflect.Array:
		*errs = append(*errs, fmt.Errorf(
			"%s: array type %s is not supported (only []byte)", path, t))

	case reflect.Map:
		*errs = append(*errs, fmt.Errorf(
			"%s: map type %s is not supported (use a struct with <block)", path, t))

	case reflect.Interface:
		*errs = append(*errs, fmt.Errorf(
			"%s: interface field %s is not supported", path, t))

	default:
		*errs = append(*errs, fmt.Errorf(
			"%s: unsupported kind %s for %s", path, v.Kind(), t))
	}
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// mismatch builds an error for a field whose value is either absent from the
// expected typed map or lives in a different one.
func mismatch(path, key, want string, cfg *Config) error {
	actual := ""
	if _, ok := cfg.Ints[key]; ok {
		actual = "i"
	}
	if _, ok := cfg.Strs[key]; ok {
		actual = "s"
	}
	if _, ok := cfg.Floats[key]; ok {
		actual = "f"
	}
	if _, ok := cfg.Bytes[key]; ok {
		actual = "b"
	}
	if _, ok := cfg.Logicals[key]; ok {
		actual = "l"
	}
	if _, ok := cfg.Inners[key]; ok {
		actual = "block"
	}

	if actual == "" {
		return fmt.Errorf("%s: key %q not found (want %s)", path, key, want)
	}
	return fmt.Errorf("%s: key %q has type %s, want %s", path, key, actual, want)
}

// findKey looks for tag in any of cfg's typed maps.
func findKey(cfg *Config, tag string, ci bool) string {
	if ci {
		for k := range cfg.Ints {
			if strings.EqualFold(k, tag) {
				return k
			}
		}
		for k := range cfg.Strs {
			if strings.EqualFold(k, tag) {
				return k
			}
		}
		for k := range cfg.Floats {
			if strings.EqualFold(k, tag) {
				return k
			}
		}
		for k := range cfg.Bytes {
			if strings.EqualFold(k, tag) {
				return k
			}
		}
		for k := range cfg.Logicals {
			if strings.EqualFold(k, tag) {
				return k
			}
		}
		for k := range cfg.Inners {
			if strings.EqualFold(k, tag) {
				return k
			}
		}
		return ""
	}
	if _, ok := cfg.Ints[tag]; ok {
		return tag
	}
	if _, ok := cfg.Strs[tag]; ok {
		return tag
	}
	if _, ok := cfg.Floats[tag]; ok {
		return tag
	}
	if _, ok := cfg.Bytes[tag]; ok {
		return tag
	}
	if _, ok := cfg.Logicals[tag]; ok {
		return tag
	}
	if _, ok := cfg.Inners[tag]; ok {
		return tag
	}
	return ""
}
