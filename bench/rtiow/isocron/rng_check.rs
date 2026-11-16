use rand::{Rng, RngCore, SeedableRng};
fn main() {
    let mut r = rand_chacha::ChaCha8Rng::seed_from_u64(42);
    let w: Vec<String> = (0..70).map(|_| r.next_u32().to_string()).collect();
    println!("u32 {}", w.join(" "));
    let mut r = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let _ = r.next_u32();
    let w: Vec<String> = (0..20).map(|_| r.next_u64().to_string()).collect();
    println!("u64 {}", w.join(" "));
    let w: Vec<String> = (0..10).map(|_| format!("{:?}", r.gen::<f64>())).collect();
    println!("f64 {}", w.join(" "));
    let w: Vec<String> = (0..10).map(|_| format!("{:?}", r.gen_range(-1.0, 1.0))).collect();
    println!("range {}", w.join(" "));
    let w: Vec<String> = (0..10).map(|_| r.gen_range(0u8, 3).to_string()).collect();
    println!("u8 {}", w.join(" "));
    let w: Vec<String> = (0..10).map(|i| r.gen_range(0usize, 255 - i).to_string()).collect();
    println!("usize {}", w.join(" "));
    let w: Vec<String> = (0..5).map(|_| format!("{:?}", r.gen::<f32>())).collect();
    println!("f32 {}", w.join(" "));
}
