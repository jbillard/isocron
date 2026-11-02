package main

import (
	"isobench/common"
	"os"
)

//go:noinline
func kernel(x, out []float64) {
	for i := range out {
		s := 0.0
		for _, v := range x[i : i+64] {
			s += v
		}
		out[i] = s / 64
	}
}

func main() {
	x := common.ReadF64(os.Args[1])
	if len(x) < common.Window {
		panic("fewer elements than the window")
	}
	out := make([]float64, len(x)-common.Window+1)
	for i := range out {
		out[i] = 0 // the pages touched before the timer, in every language (DECISIONS D117)
	}
	common.Time(common.Reps(os.Args[3]), func() { kernel(x, out) })
	common.WriteF64(os.Args[2], out)
}
