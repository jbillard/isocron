package main

import (
	"isobench/common"
	"os"
)

// kernel is comparison B of SPEC section 14: the O(n) running sum. The sum
// of the first window, then each next one by adding the element that enters
// and subtracting the one that leaves. The rounding errors accumulate along
// the array: the results differ from the exact sums of the windows.
//
//go:noinline
func kernel(x, out []float64) {
	s := 0.0
	for _, v := range x[:64] {
		s += v
	}
	out[0] = s / 64
	for i := 0; i+1 < len(out); i++ {
		s += x[i+64] - x[i]
		out[i+1] = s / 64
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
