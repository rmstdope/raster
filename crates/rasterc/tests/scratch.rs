//! The scratch directory every test in this crate writes into.

use std::{
    collections::HashSet,
    sync::{Arc, Barrier},
    thread,
};

#[path = "common/scratch.rs"]
mod scratch;
use scratch::Scratch;

/// Tests in one binary share a process id and run on parallel threads, so two
/// may ask for a scratch with the same label within one tick of the clock (a
/// microsecond on macOS). Each must still get a directory of its own, or one
/// test's files are overwritten or removed by the other.
#[test]
fn scratches_with_one_label_made_at_once_are_distinct() {
    const THREADS: usize = 16;
    const ROUNDS: usize = 50;

    let mut scratches = Vec::new();
    for _ in 0..ROUNDS {
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    Scratch::new("same")
                })
            })
            .collect();
        scratches.extend(handles.into_iter().map(|h| h.join().unwrap()));
    }

    let paths: HashSet<_> = scratches.iter().map(Scratch::path).collect();

    assert_eq!(paths.len(), scratches.len());
}
