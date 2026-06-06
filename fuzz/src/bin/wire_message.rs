use std::io;

fn main() -> io::Result<()> {
    nodelite_fuzz::run_target_from_args(nodelite_fuzz::fuzz_wire_message)
}
