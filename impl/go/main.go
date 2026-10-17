// Package tessera implements the Tessera v1 data format.
//
// Storage mirrors the spec: six typed maps per Config plus a nested-config
// map. Maps are lazily allocated. Reads on a nil map return zero, false.
// A key may exist in several maps at once, as the spec requires.
package tessera

import (
	"encoding/base64"
	"strconv"
	"strings"
)

type ErrKind int

const (
	MissingName ErrKind = iota
	CantPopStack
	MissingParts
	InvalidInt
	InvalidFloat
	InvalidBool
	BadEscape
	BadBase64
	UnknownType
)

var errNames = [...]string{
	"MissingName", "CantPopStack", "MissingParts", "InvalidInt",
	"InvalidFloat", "InvalidBool", "BadEscape", "BadBase64", "UnknownType",
}

func (e ErrKind) String() string {
	if int(e) < len(errNames) {
		return errNames[e]
	}
	return "Unknown"
}

type ParseError struct {
	Line int
	Kind ErrKind
}

func (e ParseError) Error() string {
	return "line " + strconv.Itoa(e.Line) + ": " + e.Kind.String()
}

// Config is a Tessera table.
type Config struct {
	Ints     map[string]int64
	Strs     map[string]string
	Floats   map[string]float64
	Bytes    map[string][]byte
	Logicals map[string]bool
	Inners   map[string]*Config
}

// isWS matches Rust u8::is_ascii_whitespace.
func isWS(b byte) bool {
	return b == ' ' || b == '\t' || b == '\n' || b == '\r' || b == '\x0C'
}

// trimASCII matches Rust str::trim_ascii.
func trimASCII(s string) string {
	start := 0
	for start < len(s) && isWS(s[start]) {
		start++
	}
	end := len(s)
	for end > start && isWS(s[end-1]) {
		end--
	}
	return s[start:end]
}

func unescape(s string) (string, ErrKind) {
	if strings.IndexByte(s, '\\') < 0 {
		return s, 0 // fast path
	}
	var b strings.Builder
	b.Grow(len(s))
	i := 0
	for i < len(s) {
		c := s[i]
		if c != '\\' {
			b.WriteByte(c)
			i++
			continue
		}
		if i+1 >= len(s) {
			return "", BadEscape
		}
		switch s[i+1] {
		case '\\':
			b.WriteByte('\\')
			i += 2
		case '"':
			b.WriteByte('"')
			i += 2
		case 'n':
			b.WriteByte('\n')
			i += 2
		case 'r':
			b.WriteByte('\r')
			i += 2
		case 't':
			b.WriteByte('\t')
			i += 2
		case 'u':
			if i+6 > len(s) {
				return "", BadEscape
			}
			var cp uint32
			for k := 0; k < 4; k++ {
				d := s[i+2+k]
				var v uint32
				switch {
				case d >= '0' && d <= '9':
					v = uint32(d - '0')
				case d >= 'a' && d <= 'f':
					v = uint32(d-'a') + 10
				case d >= 'A' && d <= 'F':
					v = uint32(d-'A') + 10
				default:
					return "", BadEscape
				}
				cp = cp*16 + v
			}
			if cp >= 0xD800 && cp <= 0xDFFF {
				return "", BadEscape
			}
			b.WriteRune(rune(cp))
			i += 6
		default:
			return "", BadEscape
		}
	}
	return b.String(), 0
}

// Parse consumes src and returns the root Config plus any parse errors.
// Errors do not abort parsing; a bad field is simply not recorded and
// parsing continues on the next line.
func Parse(src string) (*Config, []ParseError) {
	var errors []ParseError
	root := &Config{}
	stack := []*Config{root}
	lineno := 0

	addErr := func(kind ErrKind) {
		errors = append(errors, ParseError{Line: lineno, Kind: kind})
	}

	pos := 0
	for pos < len(src) {
		lineno++
		var line string
		if nl := strings.IndexByte(src[pos:], '\n'); nl >= 0 {
			line = src[pos : pos+nl]
			pos += nl + 1
		} else {
			line = src[pos:]
			pos = len(src)
		}

		line = trimASCII(line)
		if line == "" {
			continue
		}

		first := line[0]

		if first == '<' {
			name := trimASCII(line[1:])
			if name == "" {
				addErr(MissingName)
				continue
			}
			parent := stack[len(stack)-1]
			if parent.Inners == nil {
				parent.Inners = make(map[string]*Config)
			}
			child := &Config{}
			parent.Inners[name] = child
			stack = append(stack, child)
			continue
		}

		if first == '#' {
			continue
		}

		if line == ">" {
			if len(stack) <= 1 {
				addErr(CantPopStack)
			} else {
				stack = stack[:len(stack)-1]
			}
			continue
		}

		// splitn(3, ' ')
		p1 := strings.IndexByte(line, ' ')
		if p1 < 0 {
			addErr(MissingParts)
			continue
		}
		p2rel := strings.IndexByte(line[p1+1:], ' ')
		if p2rel < 0 {
			addErr(MissingParts)
			continue
		}
		p2 := p1 + 1 + p2rel
		key := line[:p1]
		vtype := line[p1+1 : p2]
		value := line[p2+1:]

		cur := stack[len(stack)-1]
		switch vtype {
		case "i":
			n, err := strconv.ParseInt(value, 10, 64)
			if err != nil {
				addErr(InvalidInt)
				continue
			}
			if cur.Ints == nil {
				cur.Ints = make(map[string]int64)
			}
			cur.Ints[key] = n

		case "f":
			// Rust f64::from_str rejects hex; Go accepts it. Reject here.
			if strings.IndexAny(value, "xX") >= 0 {
				addErr(InvalidFloat)
				continue
			}
			n, err := strconv.ParseFloat(value, 64)
			if err != nil {
				addErr(InvalidFloat)
				continue
			}
			if cur.Floats == nil {
				cur.Floats = make(map[string]float64)
			}
			cur.Floats[key] = n

		case "s":
			v := value
			if len(v) >= 2 && v[0] == '"' && v[len(v)-1] == '"' {
				v = v[1 : len(v)-1]
			}
			str, kind := unescape(v)
			if kind != 0 {
				addErr(kind)
				continue
			}
			if cur.Strs == nil {
				cur.Strs = make(map[string]string)
			}
			cur.Strs[key] = str

		case "b":
			b, err := base64.StdEncoding.DecodeString(value)
			if err != nil {
				addErr(BadBase64)
				continue
			}
			if cur.Bytes == nil {
				cur.Bytes = make(map[string][]byte)
			}
			cur.Bytes[key] = b

		case "l":
			v := trimASCII(value)
			var b bool
			switch v {
			case "true", "1":
				b = true
			case "false", "0":
				b = false
			default:
				addErr(InvalidBool)
				continue
			}
			if cur.Logicals == nil {
				cur.Logicals = make(map[string]bool)
			}
			cur.Logicals[key] = b

		default:
			addErr(UnknownType)
		}
	}

	return root, errors
}
