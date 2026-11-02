// Package common holds the I/O, the timing loop and the fixed reduction
// form of SPEC section 10, shared by the Go reference programs.
//
// Go may fuse x*y + z into an FMA; an explicit float64(...) conversion
// forbids it (Go specification, "Arithmetic operators"), so products are
// always written float64(a * b).
package common

import (
	"encoding/binary"
	"fmt"
	"math"
	"os"
	"runtime"
	"strconv"
	"sync"
	"time"
)

const Lanes = 32
const Block = 4096

func ReadU8(path string) []byte {
	b, err := os.ReadFile(path)
	if err != nil {
		panic(err)
	}
	return b
}

func ReadF64(path string) []float64 {
	b := ReadU8(path)
	x := make([]float64, len(b)/8)
	for i := range x {
		x[i] = math.Float64frombits(binary.LittleEndian.Uint64(b[8*i:]))
	}
	return x
}

func WriteF64(path string, x []float64) {
	b := make([]byte, 8*len(x))
	for i, v := range x {
		bits := math.Float64bits(v)
		if v != v {
			bits = 0x7FF8000000000000
		}
		binary.LittleEndian.PutUint64(b[8*i:], bits)
	}
	if err := os.WriteFile(path, b, 0o644); err != nil {
		panic(err)
	}
}

// Reps parses the repetition count.
func Reps(s string) int {
	r, err := strconv.Atoi(s)
	if err != nil || r < 1 {
		return 1
	}
	return r
}

// Time calls f reps times and prints the total duration in nanoseconds.
func Time(reps int, f func()) {
	t0 := time.Now()
	for i := 0; i < reps; i++ {
		f()
	}
	fmt.Println(time.Since(t0).Nanoseconds())
}

func tree(acc *[Lanes]float64) float64 {
	for d := Lanes / 2; d >= 1; d /= 2 {
		for j := 0; j < d; j++ {
			acc[j] = acc[j] + acc[j+d]
		}
	}
	return acc[0]
}

// BlockSum is reduce_block with + over x.
func BlockSum(x []float64) float64 {
	var acc [Lanes]float64
	i := 0
	for ; i+Lanes <= len(x); i += Lanes {
		c := x[i : i+Lanes : i+Lanes]
		for j := 0; j < Lanes; j++ {
			acc[j] += c[j]
		}
	}
	for j := 0; i < len(x); i, j = i+1, j+1 {
		acc[j] += x[i]
	}
	return tree(&acc)
}

// BlockDot is reduce_block with + over a[i]*b[i].
func BlockDot(a, b []float64) float64 {
	var acc [Lanes]float64
	i := 0
	for ; i+Lanes <= len(a); i += Lanes {
		ca := a[i : i+Lanes : i+Lanes]
		cb := b[i : i+Lanes : i+Lanes]
		for j := 0; j < Lanes; j++ {
			acc[j] += float64(ca[j] * cb[j])
		}
	}
	for j := 0; i < len(a); i, j = i+1, j+1 {
		acc[j] += float64(a[i] * b[i])
	}
	return tree(&acc)
}

// BlockSq is reduce_block with + over (x[i]-m)^2.
func BlockSq(x []float64, m float64) float64 {
	var acc [Lanes]float64
	i := 0
	for ; i+Lanes <= len(x); i += Lanes {
		c := x[i : i+Lanes : i+Lanes]
		for j := 0; j < Lanes; j++ {
			d := c[j] - m
			acc[j] += float64(d * d)
		}
	}
	for j := 0; i < len(x); i, j = i+1, j+1 {
		d := x[i] - m
		acc[j] += float64(d * d)
	}
	return tree(&acc)
}

// Fixed applies the fixed form: blocks of Block elements, then the block
// results are summed the same way.
func Fixed(n int, block func(start, m int) float64) float64 {
	if n <= Block {
		return block(0, n)
	}
	c := (n + Block - 1) / Block
	parts := make([]float64, c)
	for j := 0; j < c; j++ {
		m := n - j*Block
		if m > Block {
			m = Block
		}
		parts[j] = block(j*Block, m)
	}
	return FixedSum(parts)
}

func FixedSum(x []float64) float64 {
	return Fixed(len(x), func(s, m int) float64 { return BlockSum(x[s : s+m]) })
}

// Workers is the number of goroutines of the parallel variants (GOMAXPROCS).
func Workers() int { return runtime.GOMAXPROCS(0) }

// ParallelFor runs f(i) for i in [0, n) on Workers() goroutines; goroutine k
// runs k, k+P, k+2P, ...
func ParallelFor(n int, f func(i int)) {
	p := Workers()
	if p > n {
		p = n
	}
	var wg sync.WaitGroup
	for k := 0; k < p; k++ {
		wg.Add(1)
		go func(k int) {
			defer wg.Done()
			for i := k; i < n; i += p {
				f(i)
			}
		}(k)
	}
	wg.Wait()
}

// FixedPar is the fixed form with the blocks computed in parallel; each
// block result has its own slot, so the result does not depend on P.
func FixedPar(n int, block func(start, m int) float64) float64 {
	if n <= Block {
		return block(0, n)
	}
	c := (n + Block - 1) / Block
	parts := make([]float64, c)
	ParallelFor(c, func(j int) {
		m := n - j*Block
		if m > Block {
			m = Block
		}
		parts[j] = block(j*Block, m)
	})
	return FixedSum(parts)
}

// ChunkSums splits [0, n) into one contiguous chunk per goroutine, sums each
// with f and adds the partial sums in order (the result depends on P).
func ChunkSums(n int, f func(lo, hi int) float64) float64 {
	p := Workers()
	parts := make([]float64, p)
	ParallelFor(p, func(k int) { parts[k] = f(n*k/p, n*(k+1)/p) })
	s := 0.0
	for _, v := range parts {
		s += v
	}
	return s
}

// SubHistograms adds to h the four sub-histograms of x.
func SubHistograms(x []byte, h *[256]uint32) {
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
	for j := 0; j < 256; j++ {
		h[j] += t[0][j] + t[1][j] + t[2][j] + t[3][j]
	}
}

// HistogramPar computes one histogram per goroutine on a contiguous chunk
// (with four sub-histograms if sub) and merges them.
func HistogramPar(x []byte, sub bool) [256]uint32 {
	p := Workers()
	hs := make([][256]uint32, p)
	ParallelFor(p, func(k int) {
		c := x[len(x)*k/p : len(x)*(k+1)/p]
		if sub {
			SubHistograms(c, &hs[k])
		} else {
			for _, b := range c {
				hs[k][b]++
			}
		}
	})
	var h [256]uint32
	for k := range hs {
		for j := 0; j < 256; j++ {
			h[j] += hs[k][j]
		}
	}
	return h
}
