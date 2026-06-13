use std::collections::HashMap;

#[derive(ironstate_aggregate::StableHash)]
struct Scores {
    by_player: HashMap<u32, u32>,
}

fn main() {}
