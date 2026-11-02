package main

import (
	"isobench/common"
	"os"
)

// kernel gives each goroutine a contiguous range of rows; each row is
// reduced alone, so the result does not depend on the number of goroutines.
//
//go:noinline
func kernel(m, v, out []float64) {
	cols := len(v)
	rows := len(out)
	p := common.Parts(rows)
	common.ParallelFor(p, func(k int) {
		lo, hi := common.ChunkOf(rows, p, k)
		common.MatVecFixed(m[lo*cols:hi*cols], v, out[lo:hi])
	})
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
