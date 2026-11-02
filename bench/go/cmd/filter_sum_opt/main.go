package main

import (
	"isobench/common"
	"os"
)

var sink float64

// scratch is allocated once, outside the timed loop.
type scratch struct {
	buf   [common.Block]float64
	parts []float64
}

// kernel is the fixed form of section 10 over the kept elements: they are
// compacted, Block at a time, into buf (no branch on the data), each block
// is reduced by BlockSum, and the block results by FixedSum, as if the
// filtered array had been built.
//
//go:noinline
func kernel(x []float64, sc *scratch) float64 {
	parts := sc.parts[:0]
	for pos := 0; ; {
		var k int
		k, pos = common.CompactPositive(x, pos, &sc.buf)
		if k > 0 {
			parts = append(parts, common.BlockSum(sc.buf[:k]))
		}
		if k < common.Block {
			break
		}
	}
	switch len(parts) {
	case 0:
		return 0 // reduce_block of no element: the tree of 32 zeros
	case 1:
		return parts[0] // at most Block kept elements: one reduce_block
	}
	return common.FixedSum(parts)
}

func main() {
	x := common.ReadF64(os.Args[1])
	sc := &scratch{parts: make([]float64, 0, len(x)/common.Block+1)}
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x, sc) })
	common.WriteF64(os.Args[2], []float64{sink})
}
