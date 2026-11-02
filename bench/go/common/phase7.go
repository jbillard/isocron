// Helpers of the four kernels of phase 7 (SPEC section 14): moving average,
// filter then sum, prefix sum, matrix-vector product.

package common

// Window is the width of the moving average.
const Window = 64

// Parts is the number of contiguous chunks of a parallel variant over n
// elements: Workers(), at most n, at least 1.
func Parts(n int) int {
	p := Workers()
	if p > n {
		p = n
	}
	if p < 1 {
		p = 1
	}
	return p
}

// ChunkOf returns the bounds of chunk k of the p contiguous chunks of [0, n).
func ChunkOf(n, p, k int) (lo, hi int) { return n * k / p, n * (k + 1) / p }

// ReadI64 reads integers stored as f64 and converts them to int64.
func ReadI64(path string) []int64 {
	x := ReadF64(path)
	y := make([]int64, len(x))
	for i, v := range x {
		y[i] = int64(v)
	}
	return y
}

// I64ToF64 converts the integers to f64 (for WriteF64).
func I64ToF64(x []int64) []float64 {
	y := make([]float64, len(x))
	for i, v := range x {
		y[i] = float64(v)
	}
	return y
}

// MovingAverageFixed writes into out[i], for i in [0, len(out)), the mean of
// the window x[i : i+64], its sum in the fixed form of section 10:
//
//	acc[j] = (0.0 + w[j]) + w[j+32]          for j < 32
//	acc[j] = acc[j] + acc[j+d]               for d = 16, 8, 4, 2, 1 and j < d
//	result acc[0] / 64.0
//
// written as one tree of the same additions, depth first so that few values
// are live. The 32 additions of 0.0 are replaced by one at the end: adding
// +0.0 only turns -0.0 into +0.0 (round to nearest), and z(a) + z(b) =
// z(a + b) for z(v) = v + 0.0, so z of the tree without them equals the
// tree with them, bit for bit (PHASE7.md). x needs len(out) + 63 elements.
func MovingAverageFixed(x, out []float64) {
	for i := range out {
		w := (*[Window]float64)(x[i : i+Window])
		s0 := (((w[0] + w[32]) + (w[16] + w[48])) + ((w[8] + w[40]) + (w[24] + w[56]))) + (((w[4] + w[36]) + (w[20] + w[52])) + ((w[12] + w[44]) + (w[28] + w[60])))
		s1 := (((w[1] + w[33]) + (w[17] + w[49])) + ((w[9] + w[41]) + (w[25] + w[57]))) + (((w[5] + w[37]) + (w[21] + w[53])) + ((w[13] + w[45]) + (w[29] + w[61])))
		s2 := (((w[2] + w[34]) + (w[18] + w[50])) + ((w[10] + w[42]) + (w[26] + w[58]))) + (((w[6] + w[38]) + (w[22] + w[54])) + ((w[14] + w[46]) + (w[30] + w[62])))
		s3 := (((w[3] + w[35]) + (w[19] + w[51])) + ((w[11] + w[43]) + (w[27] + w[59]))) + (((w[7] + w[39]) + (w[23] + w[55])) + ((w[15] + w[47]) + (w[31] + w[63])))
		t := (s0 + s2) + (s1 + s3)
		out[i] = (t + 0.0) / Window
	}
}

func b2i(b bool) int {
	if b {
		return 1
	}
	return 0
}

// CountPositive is the number of elements > 0.0 of x.
func CountPositive(x []float64) int {
	c := 0
	for _, v := range x {
		c += b2i(v > 0)
	}
	return c
}

// SkipPositive returns the position after the r-th element > 0.0 of x[pos:]
// (pos itself if r is 0, len(x) if there are fewer).
func SkipPositive(x []float64, pos, r int) int {
	if r == 0 {
		return pos
	}
	c := 0
	for j, v := range x[pos:] {
		c += b2i(v > 0)
		if c == r {
			return pos + j + 1
		}
	}
	return len(x)
}

// CompactPositive copies into buf, in order, the elements > 0.0 of x[pos:]
// until buf holds Block of them or x ends, and returns how many it copied
// and the position after the last element read. No branch on the data:
// every element is written, the count advances only for the kept ones.
func CompactPositive(x []float64, pos int, buf *[Block]float64) (int, int) {
	k := 0
	for j, v := range x[pos:] {
		buf[k&(Block-1)] = v
		k += b2i(v > 0)
		if k == Block {
			return k, pos + j + 1
		}
	}
	return k, len(x)
}

// SumI64 is the wrapping sum of x (four accumulators: integer addition is
// associative, the result is the same).
func SumI64(x []int64) int64 {
	var s0, s1, s2, s3 int64
	i := 0
	for ; i+4 <= len(x); i += 4 {
		c := (*[4]int64)(x[i : i+4])
		s0 += c[0]
		s1 += c[1]
		s2 += c[2]
		s3 += c[3]
	}
	for ; i < len(x); i++ {
		s0 += x[i]
	}
	return (s0 + s1) + (s2 + s3)
}

// ScanI64 writes into out the inclusive prefix sums of x (wrapping), the
// first one being s + x[0], and returns the last one. Unrolled by 8, no
// bounds checks in the loop.
func ScanI64(x, out []int64, s int64) int64 {
	out = out[:len(x)]
	i := 0
	for ; i+8 <= len(x); i += 8 {
		a := (*[8]int64)(x[i : i+8])
		b := (*[8]int64)(out[i : i+8])
		s += a[0]
		b[0] = s
		s += a[1]
		b[1] = s
		s += a[2]
		b[2] = s
		s += a[3]
		b[3] = s
		s += a[4]
		b[4] = s
		s += a[5]
		b[5] = s
		s += a[6]
		b[6] = s
		s += a[7]
		b[7] = s
	}
	for ; i < len(x); i++ {
		s += x[i]
		out[i] = s
	}
	return s
}

// MatVecFixed writes into out[i], for i in [0, len(out)), the dot product of
// row i of m (cols = len(v) elements, rows stored one after the other) with
// v, in the fixed form (blocks of Block, BlockDot: each product rounded,
// then added).
func MatVecFixed(m, v, out []float64) {
	cols := len(v)
	for i := range out {
		row := m[i*cols : (i+1)*cols]
		out[i] = Fixed(cols, func(s, k int) float64 { return BlockDot(row[s:s+k], v[s:s+k]) })
	}
}
