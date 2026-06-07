use std::io;

fn main() -> io::Result<()> {
    let iterations = std::env::args()
        .nth(1)
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        .unwrap_or(4096);
    nodelite_fuzz::run_fixed_iteration_smoke(iterations);
    Ok(())
}
