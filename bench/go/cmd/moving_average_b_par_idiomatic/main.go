package main

import (
	"isobench/common"
	"os"
)

// kernel is the running sum of moving_average_b_idiomatic on contiguous
// ranges of windows, one per goroutine; each goroutine restarts the sum at
// the start of its range, so the results depend on the number of goroutines.
//
//go:noinline
func kernel(x, out []float64) {
	n := len(out)
	p := common.Parts(n)
	common.ParallelFor(p, func(k int) {
		lo, hi := common.ChunkOf(n, p, k)
		if lo == hi {
			return
		}
		s := 0.0
		for _, v := range x[lo : lo+64] {
			s += v
		}
		out[lo] = s / 64
		for i := lo; i+1 < hi; i++ {
			s += x[i+64] - x[i]
			out[i+1] = s / 64
		}
	})
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
