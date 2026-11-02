package main

import (
	"isobench/common"
	"os"
)

var sink float64

//go:noinline
func kernel(x []float64) float64 {
	return common.ChunkSums(len(x), func(lo, hi int) float64 {
		s := 0.0
		for _, v := range x[lo:hi] {
			if v > 0 {
				s += v
			}
		}
		return s
	})
}

func main() {
	x := common.ReadF64(os.Args[1])
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x) })
	common.WriteF64(os.Args[2], []float64{sink})
}
