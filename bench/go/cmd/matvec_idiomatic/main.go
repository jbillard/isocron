package main

import (
	"isobench/common"
	"os"
)

// kernel: float64(a * b) forbids the FMA, as in cmd/dot_idiomatic.
//
//go:noinline
func kernel(m, v, out []float64) {
	cols := len(v)
	for i := range out {
		s := 0.0
		for j, a := range m[i*cols : (i+1)*cols] {
			s += float64(a * v[j])
		}
		out[i] = s
	}
}

func main() {
	m := common.ReadF64(os.Args[1])
	v := common.ReadF64(os.Args[2])
	if len(v) == 0 {
		panic("empty vector")
	}
	out := make([]float64, len(m)/len(v))
	for i := range out {
		out[i] = 0 // the pages touched before the timer, in every language (DECISIONS D117)
	}
	common.Time(common.Reps(os.Args[4]), func() { kernel(m, v, out) })
	common.WriteF64(os.Args[3], out)
}
