use std::time::Instant;

#[derive(ironstate_aggregate::StableHash)]
struct Session {
    id: u64,
    started: Instant,
}

fn main() {}
