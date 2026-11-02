package main

import (
	"isobench/common"
	"os"
)

// kernel: each goroutine computes the prefix sums of its contiguous chunk,
// then adds to them the total of the chunks before it.
//
//go:noinline
func kernel(x, out []int64) {
	n := len(x)
	p := common.Parts(n)
	last := make([]int64, p)
	common.ParallelFor(p, func(k int) {
		lo, hi := common.ChunkOf(n, p, k)
		var s int64
		for i := lo; i < hi; i++ {
			s += x[i]
			out[i] = s
		}
		last[k] = s
	})
	common.ParallelFor(p, func(k int) {
		var off int64
		for j := 0; j < k; j++ {
			off += last[j]
		}
		lo, hi := common.ChunkOf(n, p, k)
		for i := lo; i < hi; i++ {
			out[i] += off
		}
	})
}

func main() {
	x := common.ReadI64(os.Args[1])
	out := make([]int64, len(x))
	for i := range out {
		out[i] = 0 // the pages touched before the timer, in every language (DECISIONS D117)
	}
	common.Time(common.Reps(os.Args[3]), func() { kernel(x, out) })
	common.WriteF64(os.Args[2], common.I64ToF64(out))
}
