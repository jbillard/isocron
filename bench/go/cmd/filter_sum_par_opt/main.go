package main

import (
	"isobench/common"
	"os"
)

var sink float64

// scratch is allocated once, outside the timed loop.
type scratch struct {
	counts []int                   // kept elements of each chunk
	starts []int                   // index, in the kept sequence, of the first kept element of each chunk
	bufs   [][common.Block]float64 // one compaction buffer per goroutine
	parts  []float64               // one result per block of Block kept elements
}

// kernel computes the fixed form over the kept elements for any number of
// goroutines, with the result of the sequential variant:
//  1. each goroutine counts the kept elements of its contiguous chunk;
//  2. an exclusive prefix of the counts gives each chunk the index, in the
//     kept sequence, of its first kept element;
//  3. block b (kept elements b*Block, ..., b*Block+Block-1) is reduced by the
//     goroutine whose chunk holds its first element: it skips the kept
//     elements of the previous block, then compacts Block kept elements,
//     reading into the next chunks if needed, and reduces them by BlockSum
//     into parts[b];
//  4. the block results are reduced by FixedSum, in order.
//
//go:noinline
func kernel(x []float64, sc *scratch) float64 {
	n := len(x)
	p := common.Parts(n)
	common.ParallelFor(p, func(k int) {
		lo, hi := common.ChunkOf(n, p, k)
		sc.counts[k] = common.CountPositive(x[lo:hi])
	})
	total := 0
	for k := 0; k < p; k++ {
		sc.starts[k] = total
		total += sc.counts[k]
	}
	sc.starts[p] = total
	nb := (total + common.Block - 1) / common.Block
	parts := sc.parts[:nb]
	common.ParallelFor(p, func(k int) {
		first := (sc.starts[k] + common.Block - 1) / common.Block
		end := (sc.starts[k+1] + common.Block - 1) / common.Block
		if first >= end {
			return // no block starts in this chunk
		}
		lo, _ := common.ChunkOf(n, p, k)
		pos := common.SkipPositive(x, lo, first*common.Block-sc.starts[k])
		buf := &sc.bufs[k]
		for b := first; b < end; b++ {
			var c int
			c, pos = common.CompactPositive(x, pos, buf)
			parts[b] = common.BlockSum(buf[:c])
		}
	})
	switch nb {
	case 0:
		return 0 // reduce_block of no element: the tree of 32 zeros
	case 1:
		return parts[0] // at most Block kept elements: one reduce_block
	}
	return common.FixedSum(parts)
}

func main() {
	x := common.ReadF64(os.Args[1])
	p := common.Parts(len(x))
	sc := &scratch{
		counts: make([]int, p),
		starts: make([]int, p+1),
		bufs:   make([][common.Block]float64, p),
		parts:  make([]float64, len(x)/common.Block+1),
	}
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x, sc) })
	common.WriteF64(os.Args[2], []float64{sink})
}
