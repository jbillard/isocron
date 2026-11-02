package main

import (
	"isobench/common"
	"os"
)

var sink float64

//go:noinline
func kernel(x []float64) float64 {
	return common.FixedPar(len(x), func(s, m int) float64 { return common.BlockSum(x[s : s+m]) })
}

func main() {
	x := common.ReadF64(os.Args[1])
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x) })
	common.WriteF64(os.Args[2], []float64{sink})
}
