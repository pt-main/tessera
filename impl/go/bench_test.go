package main

import (
	"fmt"
	"strings"
	"testing"
)

func genFlatInts(n int) string {
	var b strings.Builder
	for i := 0; i < n; i++ {
		fmt.Fprintf(&b, "key%d i %d\n", i, i)
	}
	return b.String()
}

func genMixed(n int) string {
	var b strings.Builder
	for i := 0; i < n; i++ {
		fmt.Fprintf(&b, "i%d i %d\n", i, i)
		fmt.Fprintf(&b, "s%d s \"v%d\"\n", i, i)
		fmt.Fprintf(&b, "l%d l true\n", i)
	}
	return b.String()
}

func genNested(n int) string {
	var b strings.Builder
	for i := 0; i < n; i++ {
		fmt.Fprintf(&b,
			"<block%d\n  a i %d\n  b s \"text%d\"\n  <inner\n    c f %d.5\n  >\n>\n",
			i, i, i, i)
	}
	return b.String()
}

func genEscaped(n int) string {
	var b strings.Builder
	for i := 0; i < n; i++ {
		fmt.Fprintf(&b, "s%d s \"line \\\"quoted\\\" and \\\\ escaped\"\n", i)
	}
	return b.String()
}

func bench(b *testing.B, src string) {
	b.Helper()
	if _, errs := Parse(src); len(errs) != 0 {
		b.Fatalf("%v", errs[0])
	}
	b.SetBytes(int64(len(src)))
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		Parse(src)
	}
}

func BenchmarkFlatInts(b *testing.B) { bench(b, genFlatInts(200_000)) }
func BenchmarkMixed(b *testing.B)    { bench(b, genMixed(100_000)) }
func BenchmarkNested(b *testing.B)   { bench(b, genNested(20_000)) }
func BenchmarkEscaped(b *testing.B)  { bench(b, genEscaped(100_000)) }
