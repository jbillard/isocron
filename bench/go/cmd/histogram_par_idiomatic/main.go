package main

import (
	"isobench/common"
	"os"
)

var sink [256]uint32

//go:noinline
func kernel(x []byte) [256]uint32 { return common.HistogramPar(x, false) }

func main() {
	x := common.ReadU8(os.Args[1])
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x) })
	out := make([]float64, 256)
	for i, c := range sink {
		out[i] = float64(c)
	}
	common.WriteF64(os.Args[2], out)
}
