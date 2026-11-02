package main

import (
	"isobench/common"
	"math"
	"os"
)

var sink float64

//go:noinline
func kernel(x []float64) float64 {
	s := 0.0
	for _, v := range x {
		s += v
	}
	m := s / float64(len(x))
	q := 0.0
	for _, v := range x {
		d := v - m
		q += float64(d * d)
	}
	return math.Sqrt(q / float64(len(x)))
}

func main() {
	x := common.ReadF64(os.Args[1])
	common.Time(common.Reps(os.Args[3]), func() { sink = kernel(x) })
	common.WriteF64(os.Args[2], []float64{sink})
}
