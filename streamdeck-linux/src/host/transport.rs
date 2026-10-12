//! SDK socket output drains independently of the Host mutex. A completed drain
//! means queued WebSocket frames reached sink flush, not that plugin callbacks or
//! their remote side effects completed. Shutdown waits are deliberately bounded.
use axum::extract::ws::Message;
use futures_util::{Sink, SinkExt};
use serde_json::Value;
use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

const DRAIN_TIMEOUT: Duration = Duration::from_millis(150);

/// Cancelling the reader session also cancels its independent output task.
pub(super) struct Writer(pub(super) tokio::task::JoinHandle<()>);
impl Drop for Writer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone, Default)]
pub(super) struct Drain {
    completion: Arc<(Mutex<Option<bool>>, Condvar)>,
}
impl Drain {
    /// Drop the Host's outgoing Sender before calling this. A congested socket,
    /// retained sender clone, or unavailable runtime worker returns false within
    /// 150 ms; callers may then terminate their owned child without deadlocking.
    pub(super) fn wait(&self) -> bool {
        if tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        }) {
            // A channel send can wake the writer into this worker's local LIFO
            // queue. Hand the scheduler core off before synchronously waiting;
            // otherwise the waiter itself can prevent the writer from running.
            tokio::task::block_in_place(|| self.wait_for(DRAIN_TIMEOUT))
        } else {
            self.wait_for(DRAIN_TIMEOUT)
        }
    }
    fn wait_for(&self, timeout: Duration) -> bool {
        let (lock, changed) = &*self.completion;
        let state = lock.lock().unwrap();
        let (state, _) = changed
            .wait_timeout_while(state, timeout, |state| state.is_none())
            .unwrap();
        *state == Some(true)
    }
    fn finish(&self, flushed: bool) {
        let (lock, changed) = &*self.completion;
        let mut state = lock.lock().unwrap();
        if state.is_none() {
            *state = Some(flushed);
            changed.notify_all();
        }
    }
}
struct Completion(Drain);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.finish(false);
    }
}

pub(super) async fn write<S>(
    mut sink: S,
    mut outgoing: mpsc::UnboundedReceiver<Value>,
    mut controls: mpsc::Receiver<Message>,
    drain: Drain,
) where
    S: Sink<Message> + Unpin,
{
    let completion = Completion(drain);
    let mut controls_open = true;
    loop {
        tokio::select! {
            value=outgoing.recv()=>match value {
                Some(value)=>{
                    if sink.send(Message::Text(value.to_string().into())).await.is_err() { return; }
                }
                None=>{
                    if sink.flush().await.is_ok() { completion.0.finish(true); }
                    return;
                }
            },
            value=controls.recv(), if controls_open=>match value {
                Some(frame)=>{ if sink.send(frame).await.is_err() { return; } }
                None=>controls_open=false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::Pin,
        sync::atomic::{AtomicBool, Ordering},
        task::{Context, Poll},
        time::Instant,
    };
    struct TestSink {
        frames: Arc<Mutex<Vec<Message>>>,
        blocked: bool,
        fail: bool,
        flushed: Arc<AtomicBool>,
    }
    impl Sink<Message> for TestSink {
        type Error = ();
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            if self.blocked {
                Poll::Pending
            } else if self.fail {
                Poll::Ready(Err(()))
            } else {
                Poll::Ready(Ok(()))
            }
        }
        fn start_send(self: Pin<&mut Self>, frame: Message) -> Result<(), ()> {
            self.frames.lock().unwrap().push(frame);
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
            self.flushed.store(true, Ordering::SeqCst);
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
            self.poll_flush(cx)
        }
    }
    fn sink(blocked: bool, fail: bool) -> TestSink {
        TestSink {
            frames: Arc::new(Mutex::new(vec![])),
            blocked,
            fail,
            flushed: Arc::new(AtomicBool::new(false)),
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_release_order_drains_while_caller_holds_host_like_mutex() {
        let host = Mutex::new(());
        let output = sink(false, false);
        let frames = output.frames.clone();
        let flushed = output.flushed.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        let (_control, controls) = mpsc::channel(4);
        let drain = Drain::default();
        let marker = drain.clone();
        let task = tokio::spawn(write(output, rx, controls, marker));
        let guard = host.lock().unwrap();
        tx.send(serde_json::json!({"event":"keyUp"})).unwrap();
        tx.send(serde_json::json!({"event":"willDisappear"}))
            .unwrap();
        drop(tx);
        assert!(drain.wait());
        assert!(flushed.load(Ordering::SeqCst));
        assert_eq!(
            frames
                .lock()
                .unwrap()
                .iter()
                .map(|m| match m {
                    Message::Text(v) => serde_json::from_str::<Value>(v).unwrap()["event"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    _ => panic!("unexpected control"),
                })
                .collect::<Vec<_>>(),
            ["keyUp", "willDisappear"]
        );
        drop(guard);
        task.await.unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn blocking_wait_hands_off_the_only_worker_to_its_queued_writer() {
        // Execute on the sole worker, not the runtime's outer block_on thread.
        tokio::spawn(async {
            let (tx, rx) = mpsc::unbounded_channel();
            let (_control, controls) = mpsc::channel(4);
            let drain = Drain::default();
            let task = tokio::spawn(write(sink(false, false), rx, controls, drain.clone()));
            tx.send(serde_json::json!({"event":"keyUp"})).unwrap();
            drop(tx);
            assert!(drain.wait(), "waiter starved its own queued output task");
            task.await.unwrap();
        })
        .await
        .unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pongs_use_writer_and_closed_control_channel_does_not_spin() {
        let output = sink(false, false);
        let frames = output.frames.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        let (control, controls) = mpsc::channel(4);
        let drain = Drain::default();
        let task = tokio::spawn(write(output, rx, controls, drain.clone()));
        control
            .send(Message::Pong(vec![1, 2, 3].into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !frames.lock().unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(control);
        tx.send(serde_json::json!({"event":"after-pong"})).unwrap();
        drop(tx);
        assert!(drain.wait());
        task.await.unwrap();
        let frames = frames.lock().unwrap();
        assert!(matches!(&frames[0],Message::Pong(bytes) if bytes.as_ref()==[1,2,3]));
        assert_eq!(frames.len(), 2);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn writer_failure_and_abort_wake_waiters_without_false_success() {
        for fail in [true, false] {
            let (tx, rx) = mpsc::unbounded_channel();
            let (_control, controls) = mpsc::channel(4);
            let drain = Drain::default();
            let task = tokio::spawn(write(sink(!fail, fail), rx, controls, drain.clone()));
            tx.send(serde_json::json!({"event":"keyUp"})).unwrap();
            drop(tx);
            if fail {
                task.await.unwrap();
            } else {
                let start = Instant::now();
                assert!(!drain.wait_for(Duration::from_millis(25)));
                assert!(start.elapsed() < Duration::from_millis(200));
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            let start = Instant::now();
            assert!(!drain.wait());
            assert!(start.elapsed() < Duration::from_millis(50));
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retained_sender_times_out_then_drains_after_last_drop() {
        let (tx, rx) = mpsc::unbounded_channel();
        let clone = tx.clone();
        let (_control, controls) = mpsc::channel(4);
        let drain = Drain::default();
        let task = tokio::spawn(write(sink(false, false), rx, controls, drain.clone()));
        drop(tx);
        assert!(!drain.wait_for(Duration::from_millis(20)));
        drop(clone);
        assert!(drain.wait());
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_reader_owner_aborts_output_and_closes_its_receiver() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (_control, controls) = mpsc::channel(4);
        let drain = Drain::default();
        let writer = Writer(tokio::spawn(write(
            sink(true, false),
            rx,
            controls,
            drain.clone(),
        )));
        tx.send(serde_json::json!({"event":"blocked"})).unwrap();
        // Give the task a chance to own the receiver, then cancel the owner.
        tokio::task::yield_now().await;
        drop(writer);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !tx.is_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!drain.wait_for(Duration::from_millis(10)));
    }
}
