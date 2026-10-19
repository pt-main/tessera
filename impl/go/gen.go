package tessera

import (
	"encoding/base64"
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
)

type GenErrKind int

const (
	InvalidFieldKey GenErrKind = iota
	InvalidBlockKey
	EmptyBytes
)

type GenError struct {
	Kind GenErrKind
	Key  string
}

func (e GenError) Error() string {
	switch e.Kind {
	case InvalidFieldKey:
		return "field key cannot be represented: " + strconv.Quote(e.Key)
	case InvalidBlockKey:
		return "block key cannot be represented: " + strconv.Quote(e.Key)
	case EmptyBytes:
		return "empty byte value for key " + strconv.Quote(e.Key) + " cannot be encoded"
	}
	return "unknown generation error"
}

// ToTessera serializes cfg; unrepresentable entries are silently dropped.
func ToTessera(cfg *Config) string {
	var b strings.Builder
	writeTessera(cfg, &b, 0, nil)
	return b.String()
}

// TryToTessera returns the first GenError instead of dropping entries.
func TryToTessera(cfg *Config) (string, error) {
	var b strings.Builder
	var errs []GenError
	writeTessera(cfg, &b, 0, &errs)
	if len(errs) > 0 {
		return "", errs[0]
	}
	return b.String(), nil
}

func isValidFieldKey(k string) bool {
	if k == "" || strings.IndexByte(k, ' ') >= 0 {
		return false
	}
	return k[0] != '<' && k[0] != '#'
}

func writeIndent(b *strings.Builder, level int) {
	for i := 0; i < level; i++ {
		b.WriteString("  ")
	}
}

func sortedKeys[V any](m map[string]V) []string {
	if len(m) == 0 {
		return nil
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

func writeTessera(cfg *Config, b *strings.Builder, level int, errs *[]GenError) {
	addErr := func(e GenError) {
		if errs != nil {
			*errs = append(*errs, e)
		}
	}

	for _, k := range sortedKeys(cfg.Ints) {
		if !isValidFieldKey(k) {
			addErr(GenError{InvalidFieldKey, k})
			continue
		}
		writeIndent(b, level)
		b.WriteString(k)
		b.WriteString(" i ")
		b.WriteString(strconv.FormatInt(cfg.Ints[k], 10))
		b.WriteByte('\n')
	}

	for _, k := range sortedKeys(cfg.Strs) {
		if !isValidFieldKey(k) {
			addErr(GenError{InvalidFieldKey, k})
			continue
		}
		writeIndent(b, level)
		b.WriteString(k)
		b.WriteString(" s \"")
		escapeTesseraStr(cfg.Strs[k], b)
		b.WriteString("\"\n")
	}

	for _, k := range sortedKeys(cfg.Floats) {
		if !isValidFieldKey(k) {
			addErr(GenError{InvalidFieldKey, k})
			continue
		}
		writeIndent(b, level)
		b.WriteString(k)
		b.WriteString(" f ")
		b.WriteString(formatTesseraFloat(cfg.Floats[k]))
		b.WriteByte('\n')
	}

	for _, k := range sortedKeys(cfg.Logicals) {
		if !isValidFieldKey(k) {
			addErr(GenError{InvalidFieldKey, k})
			continue
		}
		writeIndent(b, level)
		b.WriteString(k)
		b.WriteString(" l ")
		if cfg.Logicals[k] {
			b.WriteString("true")
		} else {
			b.WriteString("false")
		}
		b.WriteByte('\n')
	}

	for _, k := range sortedKeys(cfg.Bytes) {
		if !isValidFieldKey(k) {
			addErr(GenError{InvalidFieldKey, k})
			continue
		}
		encoded := base64.StdEncoding.EncodeToString(cfg.Bytes[k])
		if encoded == "" {
			addErr(GenError{EmptyBytes, k})
			continue
		}
		writeIndent(b, level)
		b.WriteString(k)
		b.WriteString(" b ")
		b.WriteString(encoded)
		b.WriteByte('\n')
	}

	for _, k := range sortedKeys(cfg.Inners) {
		trimmed := trimASCII(k)
		if trimmed == "" || trimmed != k {
			addErr(GenError{InvalidBlockKey, k})
			continue
		}
		writeIndent(b, level)
		b.WriteByte('<')
		b.WriteString(trimmed)
		b.WriteByte('\n')
		writeTessera(cfg.Inners[k], b, level+1, errs)
		writeIndent(b, level)
		b.WriteString(">\n")
	}
}

func escapeTesseraStr(s string, b *strings.Builder) {
	for _, r := range s {
		switch r {
		case '\\':
			b.WriteString("\\\\")
		case '"':
			b.WriteString("\\\"")
		case '\n':
			b.WriteString("\\n")
		case '\r':
			b.WriteString("\\r")
		case '\t':
			b.WriteString("\\t")
		default:
			if r < 0x20 || (r >= 0x7F && r < 0xA0) {
				fmt.Fprintf(b, "\\u%04x", r)
			} else {
				b.WriteRune(r)
			}
		}
	}
}

func formatTesseraFloat(f float64) string {
	if math.IsNaN(f) {
		return "NaN"
	}
	if math.IsInf(f, 1) {
		return "inf"
	}
	if math.IsInf(f, -1) {
		return "-inf"
	}
	return strconv.FormatFloat(f, 'f', -1, 64)
}
