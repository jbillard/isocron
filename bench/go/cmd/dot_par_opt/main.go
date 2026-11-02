package main

import (
	"isobench/common"
	"os"
)

var sink float64

//go:noinline
func kernel(a, b []float64) float64 {
	return common.FixedPar(len(a), func(s, m int) float64 { return common.BlockDot(a[s:s+m], b[s:s+m]) })
}

func main() {
	a := common.ReadF64(os.Args[1])
	b := common.ReadF64(os.Args[2])
	if len(a) != len(b) {
		panic("size mismatch")
	}
	common.Time(common.Reps(os.Args[4]), func() { sink = kernel(a, b) })
	common.WriteF64(os.Args[3], []float64{sink})
}
