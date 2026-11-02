package main

import (
	"isobench/common"
	"os"
)

// kernel is a two-pass blocked scan that reads x twice and writes out once:
// each goroutine sums its contiguous chunk, an exclusive prefix of the sums
// gives each chunk its starting value, then each goroutine scans its chunk
// from it. Integer addition wraps and is associative: the result is the
// sequential one for any number of goroutines.
//
//go:noinline
func kernel(x, out, sums []int64) {
	n := len(x)
	p := common.Parts(n)
	common.ParallelFor(p, func(k int) {
		lo, hi := common.ChunkOf(n, p, k)
		sums[k] = common.SumI64(x[lo:hi])
	})
	var s int64
	for k := 0; k < p; k++ {
		s, sums[k] = s+sums[k], s
	}
	common.ParallelFor(p, func(k int) {
		lo, hi := common.ChunkOf(n, p, k)
		common.ScanI64(x[lo:hi], out[lo:hi], sums[k])
	})
}

func main() {
	x := common.ReadI64(os.Args[1])
	out := make([]int64, len(x))
	for i := range out {
		out[i] = 0 // the pages touched before the timer, in every language (DECISIONS D117)
	}
	sums := make([]int64, common.Parts(len(x)))
	common.Time(common.Reps(os.Args[3]), func() { kernel(x, out, sums) })
	common.WriteF64(os.Args[2], common.I64ToF64(out))
}
