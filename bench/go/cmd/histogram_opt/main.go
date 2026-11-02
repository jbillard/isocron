package main

import (
	"isobench/common"
	"os"
)

var sink [256]uint32

// Four sub-histograms break the store-to-load dependency on repeated values.
//
//go:noinline
func kernel(x []byte) [256]uint32 {
	var t [4][256]uint32
	i := 0
	for ; i+4 <= len(x); i += 4 {
		c := x[i : i+4 : i+4]
		t[0][c[0]]++
		t[1][c[1]]++
		t[2][c[2]]++
		t[3][c[3]]++
	}
	for ; i < len(x); i++ {
		t[0][x[i]]++
	}
	var h [256]uint32
	for j := 0; j < 256; j++ {
		h[j] = t[0][j] + t[1][j] + t[2][j] + t[3][j]
	}
	return h
}

func main() {
	x := common.ReadU8(os.Args[1])
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x) })
	out := make([]float64, 256)
	for i, c := range sink {
		out[i] = float64(c)
	}
	common.WriteF64(os.Args[2], out)
}
