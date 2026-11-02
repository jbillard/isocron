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
	m := common.ChunkSums(len(x), func(lo, hi int) float64 {
		s := 0.0
		for _, v := range x[lo:hi] {
			s += v
		}
		return s
	}) / n
	q := common.ChunkSums(len(x), func(lo, hi int) float64 {
		q := 0.0
		for _, v := range x[lo:hi] {
			d := v - m
			q += float64(d * d)
		}
		return q
	})
	return math.Sqrt(q / n)
}

func main() {
	x := common.ReadF64(os.Args[1])
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x) })
	common.WriteF64(os.Args[2], []float64{sink})
}
