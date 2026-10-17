// Package tessera implements the Tessera v1 data format.
//
// Storage layout: each Config is a single map, key -> *Slot. A Slot holds
// all typed values for that key at once (i, f, s, b, l, inners). This is
// equivalent to the spec's "one key can reside in several type tables"
// while keeping a single hash lookup per access.
package tessera

import (
	"encoding/base64"
	"strconv"
	"strings"
)

// ErrKind identifies a parse error category.
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

func (e ErrKind) String() string {
	switch e {
	case MissingName:
		return "MissingName"
	case CantPopStack:
		return "CantPopStack"
	case MissingParts:
		return "MissingParts"
	case InvalidInt:
		return "InvalidInt"
	case InvalidFloat:
		return "InvalidFloat"
	case InvalidBool:
		return "InvalidBool"
	case BadEscape:
		return "BadEscape"
	case BadBase64:
		return "BadBase64"
	case UnknownType:
		return "UnknownType"
	}
	return "Unknown"
}

// ParseError pairs an error kind with the 1-based source line.
type ParseError struct {
	Line int
	Kind ErrKind
}

func (e ParseError) Error() string {
	return "line " + strconv.Itoa(e.Line) + ": " + e.Kind.String()
}

// Slot holds every typed value that may exist for a key.
// Has* flags report presence; a key may hold several types at once.
type Slot struct {
	I      int64
	F      float64
	S      string
	B      []byte
	L      bool
	Inners *Config

	HasI, HasF, HasS, HasB, HasL bool
}

// Config is a Tessera table.
type Config struct {
	Slots map[string]*Slot
}

func NewConfig() *Config {
	return &Config{Slots: make(map[string]*Slot)}
}

// Slot returns the slot for key, or nil if absent.
func (c *Config) Slot(key string) *Slot { return c.Slots[key] }

func (c *Config) Int(key string) (int64, bool) {
	s := c.Slots[key]
	if s == nil || !s.HasI {
		return 0, false
	}
	return s.I, true
}

func (c *Config) Float(key string) (float64, bool) {
	s := c.Slots[key]
	if s == nil || !s.HasF {
		return 0, false
	}
	return s.F, true
}

func (c *Config) Str(key string) (string, bool) {
	s := c.Slots[key]
	if s == nil || !s.HasS {
		return "", false
	}
	return s.S, true
}

func (c *Config) Bytes(key string) ([]byte, bool) {
	s := c.Slots[key]
	if s == nil || !s.HasB {
		return nil, false
	}
	return s.B, true
}

func (c *Config) Bool(key string) (bool, bool) {
	s := c.Slots[key]
	if s == nil || !s.HasL {
		return false, false
	}
	return s.L, true
}

func (c *Config) Inner(key string) *Config {
	s := c.Slots[key]
	if s == nil {
		return nil
	}
	return s.Inners
}

// isWS matches Rust u8::is_ascii_whitespace: space, tab, LF, CR, form feed.
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
		return s, 0 // fast path: nothing to unescape
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

func getOrCreate(c *Config, key string) *Slot {
	s := c.Slots[key]
	if s == nil {
		s = &Slot{}
		c.Slots[key] = s
	}
	return s
}

// Parse consumes src and returns the root Config plus any parse errors.
// Errors do not stop parsing; offending fields or blocks are simply
// not recorded and parsing continues.
func Parse(src string) (*Config, []ParseError) {
	var errors []ParseError
	root := NewConfig()
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
			slot := getOrCreate(parent, name)
			child := NewConfig()
			slot.Inners = child // overwrite per spec
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

		// Field: splitn(3, ' ')
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

		switch vtype {
		case "i":
			n, err := strconv.ParseInt(value, 10, 64)
			if err != nil {
				addErr(InvalidInt)
				continue
			}
			s := getOrCreate(stack[len(stack)-1], key)
			s.I, s.HasI = n, true

		case "f":
			// Rust f64::from_str rejects hex; Go accepts it. Reject it here.
			if strings.IndexAny(value, "xX") >= 0 {
				addErr(InvalidFloat)
				continue
			}
			n, err := strconv.ParseFloat(value, 64)
			if err != nil {
				addErr(InvalidFloat)
				continue
			}
			s := getOrCreate(stack[len(stack)-1], key)
			s.F, s.HasF = n, true

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
			s := getOrCreate(stack[len(stack)-1], key)
			s.S, s.HasS = str, true

		case "b":
			b, err := base64.StdEncoding.DecodeString(value)
			if err != nil {
				addErr(BadBase64)
				continue
			}
			s := getOrCreate(stack[len(stack)-1], key)
			s.B, s.HasB = b, true

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
			s := getOrCreate(stack[len(stack)-1], key)
			s.L, s.HasL = b, true

		default:
			addErr(UnknownType)
		}
	}

	return root, errors
}
