package main

import (
	"isobench/common"
	"os"
)

//go:noinline
func kernel(x, out []int64) {
	var s int64
	for i, v := range x {
		s += v
		out[i] = s
	}
}

func main() {
	x := common.ReadI64(os.Args[1])
	out := make([]int64, len(x))
	for i := range out {
		out[i] = 0 // the pages touched before the timer, in every language (DECISIONS D117)
	}
	common.Time(common.Reps(os.Args[3]), func() { kernel(x, out) })
	common.WriteF64(os.Args[2], common.I64ToF64(out))
}
