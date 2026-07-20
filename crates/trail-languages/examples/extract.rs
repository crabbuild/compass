use std::error::Error;
use std::path::Path;

use trail_languages::Engine;

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: extract <source-file>")?;
    let extraction = Engine::default().extract(Path::new(&path))?;
    println!("{}", serde_json::to_string_pretty(&extraction)?);
    Ok(())
}
