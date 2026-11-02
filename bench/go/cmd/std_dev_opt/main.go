package main

import (
	"isobench/common"
	"math"
	"os"
)

var sink float64

//go:noinline
func kernel(x []float64) float64 {
	n := float64(len(x))
	m := common.FixedSum(x) / n
	q := common.Fixed(len(x), func(s, k int) float64 { return common.BlockSq(x[s:s+k], m) })
	return math.Sqrt(q / n)
}

func main() {
	x := common.ReadF64(os.Args[1])
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x) })
	common.WriteF64(os.Args[2], []float64{sink})
}
