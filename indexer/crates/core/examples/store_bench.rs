use std::time::Instant;
use terrain_core::model::Lang;
use terrain_core::store::{FileRow, Store};
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let mut s = Store::open(std::path::Path::new(&p)).unwrap();
    let rows: Vec<FileRow> = (0..2000)
        .map(|i| FileRow {
            path: format!("a/b/file{i}.py"),
            hash: [i as u8; 32],
            size: 100,
            mtime: 5,
            loc: 10,
            lang: Lang::Python,
            flags: 0,
        })
        .collect();
    for _ in 0..3 {
        let t = Instant::now();
        s.write_batch(&rows, &[], &[]).unwrap();
        println!(
            "batch of 2000: {:.1} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
    let t = Instant::now();
    let n = s.load_files().unwrap().len();
    println!("load {n}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
}
