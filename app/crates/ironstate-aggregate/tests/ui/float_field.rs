#[derive(ironstate_aggregate::StableHash)]
struct Reading {
    sensor: u32,
    value: f64,
}

fn main() {}
