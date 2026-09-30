use super::MinimaxSolver;
use std::collections::HashSet;
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

#[test]
fn queue_runs_jobs_on_the_configured_number_of_workers() {
    for count in [1, 10, 20] {
        let pool = MinimaxSolver::new(count);
        let barrier = Arc::new(Barrier::new(count));
        let (sender, receiver) = mpsc::channel();
        for _ in 0..count {
            let barrier = Arc::clone(&barrier);
            let sender = sender.clone();
            pool.sender.as_ref().unwrap().send(Box::new(move || {
                // Each worker must hold a job simultaneously to cross this barrier.
                barrier.wait();
                sender.send(std::thread::current().id()).unwrap();
            })).unwrap();
        }
        let ids: HashSet<_> = (0..count)
            .map(|_| receiver.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(ids.len(), count);
        drop(pool); // Closing an idle queue must wake and join every worker.
    }
}
