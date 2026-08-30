use std::sync::{Mutex, PoisonError};
use tokio::sync::broadcast;

/// In-memory ordered delivery for non-authoritative events.
///
/// One short critical section linearizes sequence allocation and broadcast.
/// Slow or disconnected consumers recover from an authoritative snapshot;
/// publishers never perform disk I/O and never wait for consumers.
#[derive(Debug)]
pub(crate) struct OrderedEventDelivery<T> {
    sender: broadcast::Sender<T>,
    sequence: Mutex<u64>,
}

impl<T: Clone> OrderedEventDelivery<T> {
    pub fn new(sender: broadcast::Sender<T>) -> Self {
        Self {
            sender,
            sequence: Mutex::new(0),
        }
    }

    pub fn publish_with(&self, event: T, envelope: impl FnOnce(u64, T) -> T) -> u64 {
        let mut sequence = self.sequence.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(next) = sequence.checked_add(1) else {
            return *sequence;
        };
        *sequence = next;
        let _ = self.sender.send(envelope(next, event));
        next
    }

    pub fn baseline(&self) -> u64 {
        *self.sequence.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Build and broadcast a snapshot-baseline control event while semantic
    /// publishers are excluded. This is intentionally reserved for uncommon
    /// state replacement such as a MEM switch; ordinary reconnect snapshots do
    /// not broadcast to other clients and need no long critical section.
    pub fn broadcast_baseline_with(&self, build: impl FnOnce(u64) -> T) -> u64 {
        let sequence = self.sequence.lock().unwrap_or_else(PoisonError::into_inner);
        let baseline = *sequence;
        let _ = self.sender.send(build(baseline));
        baseline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;

    #[derive(Clone, Debug)]
    struct TestEvent {
        sequence: u64,
    }

    #[test]
    fn concurrent_publishers_are_observed_in_strict_sequence_order() {
        const THREADS: usize = 16;
        const EVENTS_PER_THREAD: usize = 128;
        let (sender, _) = broadcast::channel(THREADS * EVENTS_PER_THREAD + 1);
        let delivery = Arc::new(OrderedEventDelivery::new(sender.clone()));
        let mut receiver = sender.subscribe();
        let barrier = Arc::new(Barrier::new(THREADS));
        let workers = (0..THREADS)
            .map(|_| {
                let delivery = Arc::clone(&delivery);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..EVENTS_PER_THREAD {
                        delivery.publish_with(TestEvent { sequence: 0 }, |sequence, mut event| {
                            event.sequence = sequence;
                            event
                        });
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }

        let expected = THREADS * EVENTS_PER_THREAD;
        let observed = (0..expected)
            .map(|_| receiver.try_recv().unwrap().sequence)
            .collect::<Vec<_>>();
        assert_eq!(observed, (1..=expected as u64).collect::<Vec<_>>());
        assert_eq!(delivery.baseline(), expected as u64);
    }

    #[test]
    fn baseline_snapshot_excludes_publishers_until_the_control_event_is_sent() {
        let (sender, _) = broadcast::channel(8);
        let delivery = Arc::new(OrderedEventDelivery::new(sender.clone()));
        let mut receiver = sender.subscribe();
        delivery.publish_with(TestEvent { sequence: 0 }, |sequence, mut event| {
            event.sequence = sequence;
            event
        });
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let delivery_for_baseline = Arc::clone(&delivery);
        let entered_for_baseline = Arc::clone(&entered);
        let release_for_baseline = Arc::clone(&release);
        let baseline = std::thread::spawn(move || {
            delivery_for_baseline.broadcast_baseline_with(|sequence| {
                entered_for_baseline.wait();
                release_for_baseline.wait();
                TestEvent { sequence }
            })
        });
        entered.wait();
        let publisher_started = Arc::new(Barrier::new(2));
        let publisher_started_in_thread = Arc::clone(&publisher_started);
        let delivery_for_publish = Arc::clone(&delivery);
        let (published_tx, published_rx) = mpsc::channel();
        let publisher = std::thread::spawn(move || {
            publisher_started_in_thread.wait();
            let sequence = delivery_for_publish.publish_with(
                TestEvent { sequence: 0 },
                |sequence, mut event| {
                    event.sequence = sequence;
                    event
                },
            );
            published_tx.send(sequence).unwrap();
        });
        publisher_started.wait();
        assert!(published_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());
        release.wait();
        assert_eq!(baseline.join().unwrap(), 1);
        assert_eq!(
            published_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            2
        );
        publisher.join().unwrap();
        assert_eq!(receiver.try_recv().unwrap().sequence, 1);
        assert_eq!(receiver.try_recv().unwrap().sequence, 1);
        assert_eq!(receiver.try_recv().unwrap().sequence, 2);
    }

    #[test]
    fn publishing_without_receivers_still_advances_the_baseline() {
        let (sender, receiver) = broadcast::channel(4);
        drop(receiver);
        let delivery = OrderedEventDelivery::new(sender);
        assert_eq!(
            delivery.publish_with(TestEvent { sequence: 0 }, |sequence, mut event| {
                event.sequence = sequence;
                event
            }),
            1
        );
        assert_eq!(delivery.baseline(), 1);
    }
}
