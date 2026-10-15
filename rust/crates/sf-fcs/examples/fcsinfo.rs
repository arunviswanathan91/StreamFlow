//! `cargo run -p sf-fcs --example fcsinfo -- file.fcs`: per-channel summary for cross-checking
//! against the Python engine.
fn main() {
    let path = std::env::args().nth(1).expect("usage: fcsinfo <file.fcs>");
    let f = sf_fcs::Fcs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    println!("{} events x {} params ({})", f.n_events, f.n_params(), f.version);
    for p in &f.params {
        let v = f.column(&p.name).unwrap();
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let (lo, hi) = v.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        println!("{}\t{}\tmean={:.6}\tmin={:.6}\tmax={:.6}", p.name, p.stain, mean, lo, hi);
    }
}
